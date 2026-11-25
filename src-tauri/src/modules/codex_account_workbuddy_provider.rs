// Codex 账号模块：将 WorkBuddy 账号以引用方式接入 Native ACP Gateway。
// 只保存 WorkBuddy 账号 ID，不把 access token/refresh token 写入 Codex 账号或 auth.json。

pub(crate) const WORKBUDDY_ACP_PROVIDER_KIND: &str = "workbuddy_acp";
pub(crate) const WORKBUDDY_NATIVE_PROVIDER: &str = "workbuddy_acp";
pub(crate) const WORKBUDDY_ACP_BASE_URL: &str = "workbuddy-acp://native";
pub(crate) const WORKBUDDY_PROVIDER_MODEL_CATALOG: &[&str] = &["workbuddy-default"];

pub(crate) fn is_workbuddy_upstream_provider(account: &CodexAccount) -> bool {
    account.is_api_key_auth()
        && account.api_credential_kind.as_deref() == Some(WORKBUDDY_ACP_PROVIDER_KIND)
        && account
            .api_credential_ref
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
}

pub(crate) fn workbuddy_upstream_account(
    account: &CodexAccount,
) -> Option<crate::models::workbuddy::WorkbuddyAccount> {
    let id = account.api_credential_ref.as_deref()?.trim();
    (!id.is_empty())
        .then(|| crate::modules::workbuddy_account::load_account(id))
        .flatten()
}

pub(crate) fn build_workbuddy_provider_account_id(workbuddy_account_id: &str) -> String {
    format!(
        "codex_workbuddy_{:x}",
        md5::compute(workbuddy_account_id.trim().as_bytes())
    )
}

pub(crate) fn workbuddy_provider_model_catalog() -> Vec<String> {
    WORKBUDDY_PROVIDER_MODEL_CATALOG
        .iter()
        .map(|model| (*model).to_string())
        .collect()
}

pub(crate) fn upsert_workbuddy_provider_account(
    workbuddy_account_id: &str,
    account_name: Option<String>,
) -> Result<CodexAccount, String> {
    let workbuddy_account_id = workbuddy_account_id.trim();
    if workbuddy_account_id.is_empty() {
        return Err("请选择要绑定的 WorkBuddy 账号".to_string());
    }
    let source = crate::modules::workbuddy_account::load_account(workbuddy_account_id)
        .ok_or_else(|| "WorkBuddy 账号不存在，请先在 WorkBuddy 页面登录或导入".to_string())?;
    if source.access_token.trim().is_empty()
        || source
            .expires_at
            .is_some_and(|expires_at| expires_at <= chrono::Utc::now().timestamp_millis())
        || source.status.as_deref().is_some_and(|status| {
            matches!(status, "expired" | "reauth_required" | "reauthorization_required")
        })
    {
        return Err("WorkBuddy 账号缺少可用的访问令牌或已过期，请先重新登录".to_string());
    }

    let account_id = build_workbuddy_provider_account_id(workbuddy_account_id);
    let existing = load_account(&account_id);
    let mut account = existing.unwrap_or_else(|| {
        CodexAccount::new_api_key(
            account_id.clone(),
            source.email.clone(),
            String::new(),
            CodexApiProviderMode::Custom,
            Some(WORKBUDDY_ACP_BASE_URL.to_string()),
            Some("workbuddy_acp".to_string()),
            Some("WorkBuddy".to_string()),
            workbuddy_provider_model_catalog(),
        )
    });
    account.auth_mode = CodexAuthMode::Apikey;
    account.email = source.email.clone();
    account.openai_api_key = None;
    account.api_provider_mode = CodexApiProviderMode::Custom;
    account.api_provider_id = Some("workbuddy_acp".to_string());
    account.api_provider_name = Some("WorkBuddy".to_string());
    account.api_base_url = Some(WORKBUDDY_ACP_BASE_URL.to_string());
    account.api_model_catalog = workbuddy_provider_model_catalog();
    account.api_wire_api = Some("responses".to_string());
    account.api_auth_scheme = Some("workbuddy_acp".to_string());
    account.api_supports_websockets = false;
    account.api_supports_vision = false;
    account.api_sync_model_catalog_to_codex = false;
    account.plan_type = Some(COCKPIT_API_LOGIN_PLAN_TYPE.to_string());
    account.api_credential_kind = Some(WORKBUDDY_ACP_PROVIDER_KIND.to_string());
    account.api_credential_ref = Some(workbuddy_account_id.to_string());
    account.upstream_grok_account_id = None;
    if let Some(name) = normalize_optional_value(account_name) {
        account.account_name = Some(name);
    }
    account.update_last_used();
    save_account_from_user_action(&mut account)?;

    let mut index = load_account_index();
    if let Some(summary) = index.accounts.iter_mut().find(|item| item.id == account.id) {
        summary.email = account.email.clone();
        summary.plan_type = account.plan_type.clone();
        summary.subscription_active_until = account.subscription_active_until.clone();
        summary.last_used = account.last_used;
    } else {
        index.accounts.push(CodexAccountSummary {
            id: account.id.clone(),
            email: account.email.clone(),
            plan_type: account.plan_type.clone(),
            subscription_active_until: account.subscription_active_until.clone(),
            created_at: account.created_at,
            last_used: account.last_used,
        });
    }
    save_account_index(&index)?;
    logger::log_info(&format!(
        "Codex WorkBuddy ACP 供应商账号已保存: account_id={}, workbuddy_account_id={}",
        account.id, workbuddy_account_id
    ));
    Ok(account)
}

#[cfg(test)]
mod workbuddy_provider_tests {
    use super::*;

    #[test]
    fn provider_id_is_stable_and_trimmed() {
        assert_eq!(
            build_workbuddy_provider_account_id("workbuddy-source"),
            build_workbuddy_provider_account_id("  workbuddy-source  ")
        );
        assert_ne!(
            build_workbuddy_provider_account_id("workbuddy-source"),
            build_workbuddy_provider_account_id("other-source")
        );
    }

    #[test]
    fn provider_catalog_is_text_only() {
        assert_eq!(
            workbuddy_provider_model_catalog(),
            vec!["workbuddy-default"]
        );
    }
}
