// Codex 账号模块：将 WorkBuddy 账号以引用方式接入 Native ACP Gateway。
// 只保存 WorkBuddy 账号 ID，不把 access token/refresh token 写入 Codex 账号或 auth.json。

pub(crate) const WORKBUDDY_ACP_PROVIDER_KIND: &str = "workbuddy_acp";
pub(crate) const WORKBUDDY_NATIVE_PROVIDER: &str = "workbuddy_acp";
pub(crate) const WORKBUDDY_ACP_BASE_URL: &str = "workbuddy-acp://native";
pub(crate) const WORKBUDDY_PROVIDER_MODEL_CATALOG: &[&str] = &["workbuddy-default"];
pub(crate) const WORKBUDDY_CLIENT_UNSUPPORTED: &str = "Direct WorkBuddy account injection into Codex is not supported. For Codex coding tools, create the dedicated Native DelegateTool profile from the WorkBuddy page. The legacy WorkBuddy ACP route remains stateless text-only.";

pub(crate) fn reject_workbuddy_client_injection(account: &CodexAccount) -> Result<(), String> {
    if is_workbuddy_upstream_provider(account) {
        Err(WORKBUDDY_CLIENT_UNSUPPORTED.to_string())
    } else {
        Ok(())
    }
}

pub(crate) fn reject_workbuddy_client_binding(binding: Option<&str>) -> Result<(), String> {
    reject_workbuddy_client_binding_with(binding, load_account)
}

fn reject_workbuddy_client_binding_with(
    binding: Option<&str>,
    load: impl FnOnce(&str) -> Option<CodexAccount>,
) -> Result<(), String> {
    let Some(binding) = binding.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    if crate::modules::codex_instance::is_api_service_bind_account_id(binding) {
        return Ok(());
    }
    let account_id =
        crate::modules::codex_instance::parse_provider_gateway_bind_account_id(binding)
            .unwrap_or_else(|| binding.to_string());
    if let Some(account) = load(&account_id) {
        reject_workbuddy_client_injection(&account)?;
    }
    Ok(())
}

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
            matches!(
                status,
                "expired" | "reauth_required" | "reauthorization_required"
            )
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
    account.plan_type = Some("WorkBuddy".to_string());
    account.quota = None;
    account.quota_error = None;
    account.team_quota_history = None;
    account.usage_updated_at = None;
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

    fn linked_account() -> CodexAccount {
        let mut account = CodexAccount::new_api_key(
            "workbuddy-client-fixture".into(),
            "fixture@example.invalid".into(),
            String::new(),
            CodexApiProviderMode::Custom,
            Some(WORKBUDDY_ACP_BASE_URL.into()),
            Some(WORKBUDDY_NATIVE_PROVIDER.into()),
            Some("WorkBuddy".into()),
            workbuddy_provider_model_catalog(),
        );
        account.openai_api_key = None;
        account.api_credential_kind = Some(WORKBUDDY_ACP_PROVIDER_KIND.into());
        account.api_credential_ref = Some("fixture-source".into());
        // Existing 1.5.1 links must work without being removed and re-added.
        account.plan_type = Some(COCKPIT_API_LOGIN_PLAN_TYPE.into());
        account
    }

    #[test]
    fn workbuddy_client_projection_reports_capability_not_missing_key() {
        let account = linked_account();
        assert_eq!(
            build_auth_file_value(&account).unwrap_err(),
            WORKBUDDY_CLIENT_UNSUPPORTED
        );
        let mut ordinary = account.clone();
        ordinary.api_credential_kind = None;
        ordinary.api_credential_ref = None;
        ordinary.openai_api_key = Some("fixture-api-key".into());
        ordinary.api_base_url = Some("https://example.invalid/v1".into());
        assert_eq!(
            build_auth_file_value(&ordinary).unwrap()["OPENAI_API_KEY"],
            "fixture-api-key"
        );
        ordinary.openai_api_key = None;
        assert_eq!(
            build_auth_file_value(&ordinary).unwrap_err(),
            "API Key 账号缺少 OPENAI_API_KEY"
        );
    }

    #[test]
    fn workbuddy_client_writes_leave_existing_profile_untouched() {
        let directory = tempfile::tempdir().unwrap();
        let auth_path = directory.path().join("auth.json");
        let config_path = directory.path().join("config.toml");
        let previous_auth = r#"{"OPENAI_API_KEY":"existing-fixture-key"}"#;
        let previous_config =
            "model = \"existing-model\"\nmodel_provider = \"existing-provider\"\n";
        std::fs::write(&auth_path, previous_auth).unwrap();
        std::fs::write(&config_path, previous_config).unwrap();
        let mut account = linked_account();
        assert_eq!(
            write_auth_file_to_dir(directory.path(), &account).unwrap_err(),
            WORKBUDDY_CLIENT_UNSUPPORTED
        );
        assert_eq!(
            write_prepared_account_bundle_to_dir(directory.path(), &account).unwrap_err(),
            WORKBUDDY_CLIENT_UNSUPPORTED
        );
        assert_eq!(
            write_account_bundle_to_dir(directory.path(), &account).unwrap_err(),
            WORKBUDDY_CLIENT_UNSUPPORTED
        );
        // Binding an OAuth account must not bypass the provider's capability guard.
        let mut oauth = account.clone();
        oauth.id = "oauth-fixture".into();
        account.bound_oauth_account_id = Some(oauth.id.clone());
        assert_eq!(
            write_api_key_account_bundle_with_oauth_to_dir(directory.path(), &account, &oauth)
                .unwrap_err(),
            WORKBUDDY_CLIENT_UNSUPPORTED
        );
        assert_eq!(std::fs::read_to_string(auth_path).unwrap(), previous_auth);
        assert_eq!(
            std::fs::read_to_string(config_path).unwrap(),
            previous_config
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
    }

    #[test]
    fn workbuddy_launch_binding_rejects_direct_and_provider_gateway_but_not_api_service() {
        let account = linked_account();
        let wrapped =
            crate::modules::codex_instance::provider_gateway_bind_account_id(&account.id).unwrap();
        for binding in [account.id.as_str(), wrapped.as_str()] {
            assert_eq!(
                reject_workbuddy_client_binding_with(Some(binding), |id| {
                    assert_eq!(id, account.id);
                    Some(account.clone())
                })
                .unwrap_err(),
                WORKBUDDY_CLIENT_UNSUPPORTED
            );
        }
        for binding in [
            None,
            Some(" "),
            Some(crate::modules::codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID),
        ] {
            assert!(reject_workbuddy_client_binding_with(binding, |_| panic!(
                "no source lookup for API service or unbound client"
            ))
            .is_ok());
        }
    }

    #[tokio::test]
    async fn workbuddy_launch_preflight_rejects_before_bound_oauth_or_profile_changes() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let directory = tempfile::tempdir().unwrap();
        struct RestoreEnv(Option<std::ffi::OsString>);
        impl Drop for RestoreEnv {
            fn drop(&mut self) {
                match self.0.take() {
                    Some(value) => std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", value),
                    None => std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR"),
                }
            }
        }
        let _restore = RestoreEnv(std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR"));
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", directory.path());
        let mut account = linked_account();
        account.bound_oauth_account_id = Some("missing-oauth-fixture".into());
        save_account(&account).unwrap();
        let profile = directory.path().join("untouched-profile");
        assert_eq!(
            prepare_account_for_instance_launch_preflight(&account.id)
                .await
                .unwrap_err(),
            WORKBUDDY_CLIENT_UNSUPPORTED
        );
        assert_eq!(
            project_preflighted_account_for_instance_launch(&account.id, &profile)
                .await
                .unwrap_err(),
            WORKBUDDY_CLIENT_UNSUPPORTED
        );
        assert_eq!(
            prepare_account_for_injection_from_auth_dir(&account.id, Some(&profile))
                .await
                .unwrap_err(),
            WORKBUDDY_CLIENT_UNSUPPORTED
        );
        assert!(!profile.exists());
    }

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
