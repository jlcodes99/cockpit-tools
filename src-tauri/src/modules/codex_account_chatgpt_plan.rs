// API-service-only references to independently stored ChatGPT plan registrations.
pub const CHATGPT_PLAN_PROVIDER_ID: &str = "cockpit_chatgpt_plan";
pub const CHATGPT_PLAN_ACCOUNT_PREFIX: &str = "chatgpt_plan_";

pub fn is_chatgpt_plan_account(account: &CodexAccount) -> bool {
    account.api_provider_id.as_deref() == Some(CHATGPT_PLAN_PROVIDER_ID)
}

pub fn chatgpt_plan_projection_id(registration_id: &str) -> Result<String, String> {
    uuid::Uuid::parse_str(registration_id).map_err(|_| "CHATGPT_PLAN_INVALID_ID".to_string())?;
    Ok(format!("{CHATGPT_PLAN_ACCOUNT_PREFIX}{registration_id}"))
}

pub fn save_chatgpt_plan_projection(
    registration_id: &str,
    email: &str,
    label: &str,
    models: Vec<String>,
) -> Result<CodexAccount, String> {
    let id = chatgpt_plan_projection_id(registration_id)?;
    let _guard = CODEX_ACCOUNT_MUTATION_LOCK
        .lock()
        .map_err(|_| "CHATGPT_PLAN_STORAGE_BUSY")?;
    let mut index = load_account_index();
    let mut account = load_account(&id).unwrap_or_else(|| {
        CodexAccount::new_api_key(
            id.clone(),
            email.to_string(),
            "chatgpt-plan-managed".into(),
            CodexApiProviderMode::Custom,
            Some("https://api.openai.com/v1".into()),
            Some(CHATGPT_PLAN_PROVIDER_ID.into()),
            Some("ChatGPT".into()),
            Vec::new(),
        )
    });
    account.email = email.to_string();
    account.account_name = Some(label.to_string());
    account.api_model_catalog = models;
    account.api_wire_api = Some("responses".into());
    account.api_supports_websockets = false;
    account.api_supports_vision = true;
    account.api_sync_model_catalog_to_codex = false;
    // The subscription value is unknown; never manufacture a translated tier.
    account.plan_type = None;
    account.update_last_used();
    let tombstone = read_account_tombstone(&id);
    if let Some(deleted) = tombstone.as_ref() {
        account.token_generation = account
            .token_generation
            .max(deleted.generation.saturating_add(1));
    }
    save_account_unchecked(&account)?;
    if tombstone.is_some() {
        write_account_tombstone(
            &id,
            false,
            account.token_generation,
            account_credential_hash(&account),
        )?;
    }
    let summary = CodexAccountSummary {
        id: account.id.clone(),
        email: account.email.clone(),
        plan_type: None,
        subscription_active_until: None,
        created_at: account.created_at,
        last_used: account.last_used,
    };
    if let Some(existing) = index.accounts.iter_mut().find(|item| item.id == id) {
        *existing = summary;
    } else {
        index.accounts.push(summary);
    }
    save_account_index(&index)?;
    Ok(account)
}

pub(crate) fn update_chatgpt_plan_projection_health(
    registration: &crate::modules::chatgpt_plan::ChatgptPlanAccountSummary,
) -> Result<(), String> {
    let id = chatgpt_plan_projection_id(&registration.id)?;
    let _guard = CODEX_ACCOUNT_MUTATION_LOCK.lock().map_err(|_| "CHATGPT_PLAN_STORAGE_BUSY")?;
    let Some(mut account) = load_account(&id).filter(is_chatgpt_plan_account) else { return Ok(()); };
    account.requires_reauth = !registration.connected || registration.needs_reauth;
    account.reauth_reason = account.requires_reauth.then(|| registration.check_error.clone()
        .unwrap_or_else(|| "CHATGPT_PLAN_REAUTH_REQUIRED".into()));
    save_account_unchecked(&account)
}

pub(crate) fn set_chatgpt_plan_projection_source(registration_id: &str, source: &str) -> Result<(), String> {
    let id = chatgpt_plan_projection_id(registration_id)?;
    let _guard = CODEX_ACCOUNT_MUTATION_LOCK.lock().map_err(|_| "CHATGPT_PLAN_STORAGE_BUSY")?;
    let Some(mut account) = load_account(&id).filter(is_chatgpt_plan_account) else { return Ok(()); };
    if account.api_service_source_account_id.as_deref().is_some_and(|old| old != source) {
        return Err("CHATGPT_PLAN_BINDING_CONFLICT".into());
    }
    if account.api_service_source_account_id.as_deref() != Some(source) {
        account.api_service_source_account_id = Some(source.into());
        save_account_unchecked(&account)?;
    }
    Ok(())
}

pub(crate) fn reject_chatgpt_plan_client_injection(account: &CodexAccount) -> Result<(), String> {
    if is_chatgpt_plan_account(account) {
        Err("CHATGPT_PLAN_API_SERVICE_ONLY".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod chatgpt_plan_projection_tests {
    use super::*;

    #[test]
    fn projection_reference_is_not_a_codex_login_or_secret() {
        let id = chatgpt_plan_projection_id("12345678-1234-4234-8234-123456789abc").unwrap();
        let mut account = CodexAccount::new_api_key(
            id,
            "user@example.com".into(),
            "chatgpt-plan-managed".into(),
            CodexApiProviderMode::Custom,
            None,
            Some(CHATGPT_PLAN_PROVIDER_ID.into()),
            None,
            vec![],
        );
        assert!(is_chatgpt_plan_account(&account));
        assert!(reject_chatgpt_plan_client_injection(&account).is_err());
        assert_eq!(
            build_auth_file_value(&account).unwrap_err(),
            "CHATGPT_PLAN_API_SERVICE_ONLY"
        );
        assert!(account.tokens.access_token.is_empty());
        assert!(account.tokens.refresh_token.is_none());
        let plan = account.clone();
        account.api_provider_id = None;
        account.openai_api_key = Some("sk-synthetic-normal".into());
        assert!(reject_chatgpt_plan_client_injection(&account).is_ok());
        assert!(build_auth_file_value(&account).is_ok());
        let exported = serialize_accounts_for_export(&[plan, account.clone()]).unwrap();
        let values: serde_json::Value = serde_json::from_str(&exported).unwrap();
        assert_eq!(values.as_array().unwrap().len(), 1);
        assert!(values[0]["api_provider_id"].is_null());
        assert!(!exported.contains("chatgpt-plan-managed"));
        assert!(chatgpt_plan_projection_id("../../auth").is_err());
    }
}
