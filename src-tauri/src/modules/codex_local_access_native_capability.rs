// Capability knowledge is deliberately three-state. Ordinary route selection
// keeps its existing behavior; fallback excludes known unsupported targets and
// lets an unknown target answer, within the existing bounded pre-execution retry.
fn native_capability_support(
    collection: &CodexLocalAccessCollection,
    account: &CodexAccount,
    upstream_model: &str,
    capability: &str,
) -> Option<CodexLocalAccessCapabilitySupport> {
    use CodexLocalAccessCapabilitySupport::{Supported, Unsupported};
    if !CODEX_CAPABILITY_IDS.contains(&capability) {
        // An unfamiliar extension is not proof that the current upstream cannot
        // handle it. The replay validator still forbids unknown cross-account tools.
        return None;
    }
    let hosted = matches!(
        capability,
        "tool_search"
            | "web_search"
            | "image_generation"
            | "file_search"
            | "code_interpreter"
            | "computer_use"
            | "mcp"
            | "programmatic_tool_calling"
    );
    if native_is_chatgpt_plan_account(account)
        && ((hosted && capability != "web_search")
            || matches!(capability, "audio_input" | "video_input" | "compact"))
    {
        return Some(Unsupported);
    }
    if native_wire_api(account).ok() == Some(native_contract::WireApi::Chat)
        && (hosted || capability == "compact")
    {
        return Some(Unsupported);
    }
    let exact = collection
        .capability_fallback
        .overrides
        .iter()
        .find(|entry| entry.account_id == account.id && entry.model == upstream_model);
    let wildcard = collection
        .capability_fallback
        .overrides
        .iter()
        .find(|entry| entry.account_id == account.id && entry.model == "*");
    let declared = exact
        .and_then(|entry| entry.capabilities.get(capability))
        .or_else(|| wildcard.and_then(|entry| entry.capabilities.get(capability)));
    match declared {
        Some(Supported) => Some(Supported),
        Some(Unsupported) => Some(Unsupported),
        None => None,
    }
}

fn native_filter_capability_backups(
    collection: &CodexLocalAccessCollection,
    plan: &NativeModelRoutePlan,
    request: &native_contract::RouteRequest,
    accounts: &mut Vec<CodexAccount>,
) -> Result<(), native_contract::ServiceError> {
    let Some(source) = request.capability_fallback_from.as_deref() else {
        return Ok(());
    };
    // Do not create a new grant by appending backup IDs to an existing model
    // route/Key scope. The original filter has already intersected that scope.
    accounts.retain(|account| {
        account.id != source
            && native_operation_model(
                collection,
                plan,
                account,
                request.auxiliary.as_deref(),
                request.image_input,
            )
            .is_some_and(|model| {
                request.required_capabilities.iter().all(|capability| {
                    native_capability_support(collection, account, &model, capability)
                        != Some(CodexLocalAccessCapabilitySupport::Unsupported)
                })
            })
    });
    Ok(())
}

async fn native_capability_fallback_decision(
    host: &NativeHostBridge,
    caller: &native_contract::Caller,
    route: &native_contract::Route,
    request: &native_contract::RouteRequest,
) -> Result<native_contract::CapabilityFallbackDecision, native_contract::ServiceError> {
    let collection = host.collection().await?;
    let key = native_caller_key(&collection, caller)?;
    let public_model = native_model_for_key(&collection, key, &request.model)?;
    let plan = native_model_route_plan(&collection, key, &public_model)?;
    if caller.id != NATIVE_INTERNAL_CALLER && !plan.account_ids.contains(&route.account_id) {
        return Err(native_error(
            403,
            "account_not_allowed",
            "Selected account is outside this API key's model scope",
        ));
    }
    let Some(account) = host.load_route_account(route).await? else {
        return Err(native_error(
            503,
            "account_not_found",
            "Selected account is unavailable",
        ));
    };
    let mut unsupported = Vec::new();
    for capability in &request.required_capabilities {
        if native_capability_support(&collection, &account, &route.model, capability)
            == Some(CodexLocalAccessCapabilitySupport::Unsupported)
            && !unsupported.contains(capability)
        {
            unsupported.push(capability.clone());
        }
    }
    Ok(native_contract::CapabilityFallbackDecision {
        enabled: true,
        unsupported,
    })
}

#[cfg(test)]
#[path = "codex_local_access_native_capability_tests.rs"]
mod native_capability_tests;
