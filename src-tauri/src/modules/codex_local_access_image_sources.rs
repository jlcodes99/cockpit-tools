/// 保留真实上游目录完整 ID；用户手动添加的自定义图片型号也按原值调用。
fn image_api_model_ids(account: &CodexAccount) -> Vec<String> {
    if !account.is_api_key_auth() {
        return Vec::new();
    }
    normalize_image_model_ids(account.api_model_catalog.clone())
}

fn image_model_matches_selection(actual: &str, selected: &str) -> bool {
    let actual = actual.trim();
    let selected = selected.trim();
    actual == selected
        || (matches!(selected, "gpt-image-2" | "gpt-image-2.5")
            && actual
                .strip_prefix(selected)
                .and_then(|suffix| suffix.strip_prefix('-'))
                .is_some_and(|suffix| !suffix.is_empty()))
}

/// 显式型号必须精确存在且属于所选族；缺少选择仅允许裸型号，不能猜测后缀。
pub(crate) fn resolve_image_generation_api_model(
    account: &CodexAccount,
    selected_model: &str,
    explicit_model: Option<&str>,
) -> Option<String> {
    if !account.is_api_key_auth() || !is_image_generation_source_for_model(account, selected_model)
    {
        return None;
    }
    let actual = explicit_model.unwrap_or(selected_model).trim();
    (image_model_matches_selection(actual, selected_model)
        && image_api_model_ids(account)
            .iter()
            .any(|model| model == actual))
    .then(|| actual.to_string())
}

pub(crate) fn resolve_image_generation_source_model(
    account: &CodexAccount,
    collection: &CodexLocalAccessCollection,
    explicit_model: Option<&str>,
) -> Option<String> {
    let actual = if account.is_api_key_auth() {
        resolve_image_generation_api_model(
            account,
            &collection.image_generation_model,
            explicit_model,
        )?
    } else if is_image_generation_source_for_model(account, &collection.image_generation_model) {
        collection.image_generation_model.trim().to_string()
    } else {
        return None;
    };
    (!collection
        .image_generation_deleted_models
        .iter()
        .any(|deleted| deleted.trim() == actual))
    .then_some(actual)
}

fn image_generation_source_is_ready(
    account: &CodexAccount,
    collection: &CodexLocalAccessCollection,
    explicit_model: Option<&str>,
) -> bool {
    resolve_image_generation_source_model(account, collection, explicit_model).is_some()
}

pub(crate) fn configured_image_generation_settings() -> Result<CodexLocalAccessCollection, String> {
    load_collection_from_disk()?
        .map(Ok)
        .unwrap_or_else(new_local_access_collection)
}

fn image_generation_manifest_policy(
    account: &CodexAccount,
    collection: &CodexLocalAccessCollection,
) -> &'static str {
    // Native OAuth images can only execute a live bare model. Keep its credential
    // admission behavior unchanged; this guard owns only image model availability.
    if !account.is_api_key_auth()
        && (!matches!(
            collection.image_generation_model.trim(),
            "gpt-image-2" | "gpt-image-2.5"
        ) || collection
            .image_generation_deleted_models
            .iter()
            .any(|model| model.trim() == collection.image_generation_model.trim()))
    {
        return "disabled";
    }
    match collection
        .image_generation_account_policies
        .get(&account.id)
    {
        Some(CodexLocalAccessImageGenerationPolicy::Enabled) => "enabled",
        Some(CodexLocalAccessImageGenerationPolicy::Disabled) => "disabled",
        _ => "inherit",
    }
}

pub(crate) fn is_image_generation_source_for_model(account: &CodexAccount, model: &str) -> bool {
    if account.is_api_key_auth() {
        image_api_model_ids(account)
            .iter()
            .any(|item| image_model_matches_selection(item, model))
            && account
                .openai_api_key
                .as_deref()
                .is_some_and(|key| !key.trim().is_empty())
            && (account.api_provider_mode == CodexApiProviderMode::OpenaiBuiltin
                || account.api_base_url.as_deref().is_some_and(|raw| {
                    Url::parse(raw.trim()).is_ok_and(|url| {
                        matches!(url.scheme(), "http" | "https") && url.host_str().is_some()
                    })
                }))
    } else {
        matches!(model.trim(), "gpt-image-2" | "gpt-image-2.5")
            && !account.is_agent_identity_auth()
            && codex_account::account_has_refresh_token(account)
    }
}

fn image_generation_upstream_manifest_value(
    account: &CodexAccount,
    collection: &CodexLocalAccessCollection,
    proxy_url: Option<&str>,
) -> Option<Value> {
    let explicitly_enabled = collection
        .image_generation_account_ids
        .contains(&account.id)
        || collection
            .image_generation_account_policies
            .get(&account.id)
            == Some(&CodexLocalAccessImageGenerationPolicy::Enabled);
    if !explicitly_enabled
        || !account.is_api_key_auth()
        || !is_image_generation_source_for_model(account, &collection.image_generation_model)
    {
        return None;
    }
    let model = resolve_image_generation_source_model(
        account,
        collection,
        collection
            .image_generation_account_models
            .get(&account.id)
            .map(String::as_str),
    )?;
    let base_url = resolve_sidecar_upstream_base_url(account, collection)?;
    Some(json!({
        "baseUrl": base_url,
        "apiKey": account.openai_api_key.as_deref()?.trim(),
        "modelIds": image_api_model_ids(account),
        "model": model,
        "proxyUrl": proxy_url,
    }))
}

fn image_configuration_update_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    &LOCK
}

fn apply_image_generation_source_selection(
    collection: &mut CodexLocalAccessCollection,
    account_ids: Vec<String>,
    models: HashMap<String, String>,
    accounts: &[CodexAccount],
) -> bool {
    let mut changed = collection.image_generation_account_ids != account_ids
        || collection.image_generation_account_models != models;
    let removed_api_sources = collection
        .image_generation_account_ids
        .iter()
        .filter(|id| !account_ids.contains(id))
        .filter(|id| {
            collection.image_generation_account_models.contains_key(*id)
                || accounts
                    .iter()
                    .find(|account| account.id == **id)
                    .map(|account| account.is_api_key_auth())
                    .unwrap_or(true)
        })
        .cloned()
        .collect::<Vec<_>>();
    for id in removed_api_sources {
        if collection.image_generation_account_policies.get(&id)
            == Some(&CodexLocalAccessImageGenerationPolicy::Enabled)
        {
            // Restore the default when an explicit API source is cleared. An Enabled
            // residue would itself generate an image upstream after its scope is gone.
            collection.image_generation_account_policies.remove(&id);
            changed = true;
        }
    }
    collection.image_generation_account_ids = account_ids;
    collection.image_generation_account_models = models;
    for account in accounts {
        if account.is_api_key_auth()
            && collection
                .image_generation_account_ids
                .contains(&account.id)
            && image_generation_source_is_ready(
                account,
                collection,
                collection
                    .image_generation_account_models
                    .get(&account.id)
                    .map(String::as_str),
            )
        {
            changed |= collection.image_generation_account_policies.insert(
                account.id.clone(),
                CodexLocalAccessImageGenerationPolicy::Enabled,
            ) != Some(CodexLocalAccessImageGenerationPolicy::Enabled);
        }
    }
    changed
}

fn remove_disabled_image_generation_sources(collection: &mut CodexLocalAccessCollection) -> bool {
    let disabled = collection
        .image_generation_account_policies
        .iter()
        .filter_map(|(id, policy)| {
            (*policy == CodexLocalAccessImageGenerationPolicy::Disabled).then_some(id.clone())
        })
        .collect::<HashSet<_>>();
    let ids_before = collection.image_generation_account_ids.len();
    let models_before = collection.image_generation_account_models.len();
    collection
        .image_generation_account_ids
        .retain(|id| !disabled.contains(id));
    collection
        .image_generation_account_models
        .retain(|id, _| !disabled.contains(id));
    ids_before != collection.image_generation_account_ids.len()
        || models_before != collection.image_generation_account_models.len()
}
