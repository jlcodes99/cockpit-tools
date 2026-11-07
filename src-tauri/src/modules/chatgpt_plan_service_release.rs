#[derive(Serialize)]
pub struct ChatgptPlanAuthorizationActionResult {
    pub authorization: chatgpt_plan::ChatgptPlanAccountSummary,
    pub state: CodexLocalAccessState,
    pub remote_revoked: bool,
}

pub async fn release_authorization(
    id: &str,
    disposition: &str,
) -> Result<ChatgptPlanAuthorizationActionResult, String> {
    manage_authorization(id, disposition, true, true).await
}

pub async fn disconnect_with_disposition(
    id: &str,
    disposition: &str,
) -> Result<ChatgptPlanAuthorizationActionResult, String> {
    manage_authorization(id, disposition, true, false).await
}

pub async fn unbind_with_disposition(
    id: &str,
    disposition: &str,
) -> Result<ChatgptPlanAuthorizationActionResult, String> {
    manage_authorization(id, disposition, false, true).await
}

async fn manage_authorization(
    id: &str,
    disposition: &str,
    revoke: bool,
    detach: bool,
) -> Result<ChatgptPlanAuthorizationActionResult, String> {
    if !matches!(disposition, "local_gateway" | "pause") {
        return Err("CHATGPT_PLAN_SERVICE_MODE_INVALID".into());
    }
    let lock = action_lock(id)?;
    let _guard = lock.lock().await;
    let summary = chatgpt_plan::get_account(id).await?;
    let projection = codex_account::chatgpt_plan_projection_id(id)?;
    let (_, members) = codex_local_access::chatgpt_plan_pool_snapshot().await?;
    // An authorization action cannot add a member or change an existing local
    // member. The CAS transaction also rechecks membership after waiting.
    let bindings = codex_local_access::chatgpt_plan_selected_bindings().await?;
    let canonical = bindings
        .iter()
        .find(|(source, selected)| selected.as_str() == id && members.contains(source))
        .map(|(source, _)| source.clone());
    if let Some(source) = canonical {
        if disposition == "pause" {
            codex_local_access::pause_official_service_binding(&source, id).await?;
        } else {
            let association =
                chatgpt_plan::validate_source_association_snapshot(id, &source).await?;
            codex_local_access::switch_chatgpt_plan_service_member_intent(
                &source,
                Some(&projection),
                false,
                None,
                Some(association),
                true,
                false,
            )
            .await?;
            remember_committed_mode(id, Some(&source), "codex").await?;
        }
    } else if members.contains(&projection) {
        if let Some(source) = summary
            .source_account_id
            .as_deref()
            .filter(|_| summary.source_binding_valid)
        {
            let association =
                chatgpt_plan::validate_source_association_snapshot(id, source).await?;
            let changed = codex_local_access::switch_chatgpt_plan_service_member_intent(
                source,
                Some(&projection),
                false,
                None,
                Some(association),
                true,
                disposition == "pause",
            )
            .await;
            if let Err(error) = changed {
                if error != "CHATGPT_PLAN_SERVICE_MEMBER_REMOVED" {
                    return Err(error);
                }
            } else if disposition == "local_gateway" {
                remember_committed_mode(id, Some(source), "codex").await?;
            }
        } else if disposition == "pause" {
            // No trusted source: keep the old endpoint isolated instead of
            // handing its ACL to another principal with a reused local ID.
            codex_local_access::pause_chatgpt_plan_service_endpoint(&projection).await?;
        } else {
            return Err(summary
                .source_binding_error
                .clone()
                .unwrap_or_else(|| "CHATGPT_PLAN_BINDING_REQUIRED".into()));
        }
    }
    let remote_revoked = if revoke {
        if detach {
            chatgpt_plan::disconnect_and_unbind(id).await?
        } else {
            chatgpt_plan::disconnect(id).await?
        }
        .remote_revoked
    } else {
        if detach {
            if let Some(source) = summary.source_account_id.as_deref() {
                chatgpt_plan::unbind_account(id, source).await?;
            }
        }
        false
    };
    emit_service_state_updated();
    Ok(ChatgptPlanAuthorizationActionResult {
        authorization: chatgpt_plan::get_account(id).await?,
        state: codex_local_access::chatgpt_plan_state_snapshot().await?,
        remote_revoked,
    })
}
