// Codex 账号模块：将 ZCode API Key/OAuth 账号以引用方式接入本地 Provider Gateway。
//
// 该模块只保存 ZCode 账号 ID，不把 API Key 或 OAuth Token 复制到 Codex 账号或 auth.json。
// 请求真正发送前由 native_zcode 读取加密 ZCode 账号存储并在内存中组装认证头。

const ZCODE_API_KEY_PROVIDER_KIND: &str = "zcode_api_key";
const ZCODE_OAUTH_PROVIDER_KIND: &str = "zcode_oauth";
pub(crate) const ZCODE_NATIVE_PROVIDER: &str = "zcode";
use crate::modules::zcode_account::ZCODE_PLAN_API_BASE_URL;
pub(crate) const ZCODE_PROVIDER_MODEL_CATALOG: &[&str] = &["GLM-5.2", "glm-5-turbo"];

pub(crate) fn is_zcode_upstream_provider(account: &CodexAccount) -> bool {
    account.is_api_key_auth()
        && matches!(
            account.api_credential_kind.as_deref(),
            Some(kind) if kind == ZCODE_API_KEY_PROVIDER_KIND || kind == ZCODE_OAUTH_PROVIDER_KIND
        )
        && account
            .api_credential_ref
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
}

pub(crate) fn is_zcode_oauth_upstream_provider(account: &CodexAccount) -> bool {
    is_zcode_upstream_provider(account)
        && account.api_credential_kind.as_deref() == Some(ZCODE_OAUTH_PROVIDER_KIND)
}

pub(crate) fn zcode_upstream_account(
    account: &CodexAccount,
) -> Option<crate::models::zcode::ZcodeAccount> {
    let id = account.api_credential_ref.as_deref()?.trim();
    (!id.is_empty())
        .then(|| crate::modules::zcode_account::load_account(id))
        .flatten()
}

pub(crate) fn build_zcode_provider_account_id(zcode_account_id: &str) -> String {
    format!(
        "codex_zcode_{:x}",
        md5::compute(zcode_account_id.trim().as_bytes())
    )
}

pub(crate) fn zcode_provider_model_catalog() -> Vec<String> {
    ZCODE_PROVIDER_MODEL_CATALOG
        .iter()
        .map(|model| (*model).to_string())
        .collect()
}

pub(crate) fn upsert_zcode_provider_account(
    zcode_account_id: &str,
    account_name: Option<String>,
) -> Result<CodexAccount, String> {
    let zcode_account_id = zcode_account_id.trim();
    if zcode_account_id.is_empty() {
        return Err("请选择要绑定的 ZCode 账号".to_string());
    }
    let source = crate::modules::zcode_account::load_account(zcode_account_id)
        .ok_or_else(|| "ZCode 账号不存在，请先在 ZCode 页面登录或导入".to_string())?;
    let is_oauth = source.auth_mode == crate::models::zcode::ZcodeAuthMode::Oauth;
    if is_oauth {
        crate::modules::zcode_account::oauth_inference_session(&source)?;
    } else if source.api_key.as_deref().is_none_or(|key| key.trim().is_empty()) {
        return Err("ZCode 账号缺少 API Key".into());
    }
    let provider_id = if source.provider.eq_ignore_ascii_case("bigmodel") {
        crate::modules::zcode_account::BIGMODEL_API_KEY_PROVIDER_ID
    } else {
        crate::modules::zcode_account::ZAI_API_KEY_PROVIDER_ID
    };
    let provider_name = if source.provider.eq_ignore_ascii_case("bigmodel") {
        "BigModel"
    } else {
        "Z.ai"
    };
    let base_url = if is_oauth {
        ZCODE_PLAN_API_BASE_URL
    } else if source.provider.eq_ignore_ascii_case("bigmodel") {
        crate::modules::zcode_account::BIGMODEL_API_BASE_URL
    } else {
        crate::modules::zcode_account::ZAI_API_BASE_URL
    };
    let account_id = build_zcode_provider_account_id(zcode_account_id);
    let existing = load_account(&account_id);
    let mut account = existing.unwrap_or_else(|| {
        CodexAccount::new_api_key(
            account_id.clone(),
            source.email.clone(),
            String::new(),
            CodexApiProviderMode::Custom,
            Some(base_url.to_string()),
            Some(provider_id.to_string()),
            Some(provider_name.to_string()),
            zcode_provider_model_catalog(),
        )
    });
    account.auth_mode = CodexAuthMode::Apikey;
    account.email = source.email.clone();
    account.openai_api_key = None;
    account.api_provider_mode = CodexApiProviderMode::Custom;
    account.api_provider_id = Some(provider_id.to_string());
    account.api_provider_name = Some(provider_name.to_string());
    account.api_base_url = Some(base_url.to_string());
    if account.api_model_catalog.is_empty() {
        account.api_model_catalog = zcode_provider_model_catalog();
    }
    account.api_wire_api = Some("anthropic".to_string());
    account.api_auth_scheme = Some(if is_oauth { "bearer" } else { "x-api-key" }.to_string());
    account.api_supports_websockets = false;
    account.api_supports_vision = false;
    account.api_sync_model_catalog_to_codex = false;
    account.plan_type = Some(COCKPIT_API_LOGIN_PLAN_TYPE.to_string());
    account.api_credential_kind = Some(
        if is_oauth {
            ZCODE_OAUTH_PROVIDER_KIND
        } else {
            ZCODE_API_KEY_PROVIDER_KIND
        }
        .to_string(),
    );
    account.api_credential_ref = Some(zcode_account_id.to_string());
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
        "Codex ZCode API Key 供应商账号已保存: account_id={}, zcode_account_id={}",
        account.id, zcode_account_id
    ));
    Ok(account)
}

#[cfg(test)]
mod zcode_provider_tests {
    use super::*;

    #[test]
    fn provider_id_is_stable_and_trimmed() {
        assert_eq!(
            build_zcode_provider_account_id("zcode-source"),
            build_zcode_provider_account_id("  zcode-source  ")
        );
        assert_ne!(
            build_zcode_provider_account_id("zcode-source"),
            build_zcode_provider_account_id("other-source")
        );
    }

    #[test]
    fn oauth_linked_provider_is_recognized_without_secret_material() {
        let mut account = CodexAccount::new_api_key(
            "linked".into(),
            "fixture@example.invalid".into(),
            String::new(),
            CodexApiProviderMode::Custom,
            Some(ZCODE_PLAN_API_BASE_URL.into()),
            Some("builtin:zai".into()),
            Some("Z.ai".into()),
            zcode_provider_model_catalog(),
        );
        account.api_credential_kind = Some(ZCODE_OAUTH_PROVIDER_KIND.to_string());
        account.api_credential_ref = Some("zcode-source".to_string());
        account.openai_api_key = None;
        assert!(is_zcode_upstream_provider(&account));
        assert!(is_zcode_oauth_upstream_provider(&account));
    }

    #[test]
    fn default_catalog_contains_zcode_models() {
        let models = zcode_provider_model_catalog();
        assert_eq!(models, vec!["GLM-5.2", "glm-5-turbo"]);
    }
}
