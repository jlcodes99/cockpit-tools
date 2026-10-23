//! Ordinary proxy command adapters for desktop-only API identities.
//! API accounts remain excluded from upstream account proxy eligibility.
use super::{codex_client_proxy as desktop, codex_proxy_runtime as runtime};
use crate::models::codex::CodexAccount;

pub async fn desktop_target(raw: &str) -> Result<Option<String>, String> {
    let target = desktop::normalize_target(raw)?;
    if super::codex_instance::is_api_service_bind_account_id(&target) {
        return Ok(Some(target));
    }
    let account = runtime::load_stored(&target).await?;
    Ok(account.is_api_key_auth().then_some(target))
}

pub async fn runtime_id(raw: &str) -> Result<String, String> {
    Ok(match desktop_target(raw).await? {
        Some(target) => desktop::route_id(&target),
        None => raw.trim().to_string(),
    })
}

pub async fn view(raw: &str) -> Result<desktop::TargetView, String> {
    match desktop_target(raw).await? {
        Some(target) => desktop::target_view(&target).await,
        None => {
            let source = runtime::load(raw).await?;
            if !super::codex_account_proxy::eligible(&source) {
                return Err("PROXY_ACCOUNT_UNSUPPORTED".into());
            }
            let mut account = CodexAccount::new(
                source.id,
                source.email,
                crate::models::codex::CodexTokens {
                    access_token: String::new(),
                    id_token: String::new(),
                    refresh_token: None,
                },
            );
            account.account_name = source.account_name;
            account.plan_type = source.plan_type;
            account.egress_proxy_url = source.egress_proxy_url;
            account.egress_proxy_disabled = source.egress_proxy_disabled;
            account.created_at = source.created_at;
            account.last_used = source.last_used;
            Ok(desktop::TargetView {
                account,
                egress_proxy_error: None,
                egress_proxy_legacy_bound: false,
            })
        }
    }
}

pub fn desktop_status(
    mut status: runtime::RuntimeStatus,
    desktop_only: bool,
) -> runtime::RuntimeStatus {
    if desktop_only {
        status.shared_entry = false;
        status.account = "unbound";
        status.sidecar = "unbound";
        status.account_port = None;
        status.sidecar_port = None;
        status.account_node = None;
        status.sidecar_node = None;
        status.account_selection = None;
        status.sidecar_selection = None;
    }
    status
}

pub async fn status(raw: &str, measure: bool) -> Result<runtime::RuntimeStatus, String> {
    let id = runtime_id(raw).await?;
    let status = if measure {
        runtime::measure_current_latency(&id).await?
    } else {
        runtime::status(&id).await?
    };
    Ok(desktop_status(status, desktop::route_target(&id).is_some()))
}

pub async fn save(
    raw: String,
    input: Option<String>,
    disabled: bool,
) -> Result<CodexAccount, String> {
    if let Some(target) = desktop_target(&raw).await? {
        return desktop::save_account_binding(target, input, disabled).await;
    }
    runtime::save_binding_with_mode(raw, input, disabled).await
}

/// UI totals include desktop-only cards, without changing upstream eligibility.
pub async fn account_totals() -> Result<(Vec<String>, Vec<String>, Vec<String>), String> {
    let accounts = runtime::stored_accounts().await?;
    let mut eligible = Vec::new();
    let mut independent = Vec::new();
    let mut disabled = Vec::new();
    let mut special = vec![super::codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID.to_string()];
    for account in accounts {
        if account.is_api_key_auth() {
            special.push(account.id);
            continue;
        }
        if !super::codex_account_proxy::eligible(&account) {
            continue;
        }
        if account.egress_proxy_disabled {
            disabled.push(account.id.clone());
        } else if account
            .egress_proxy_url
            .as_ref()
            .is_some_and(|raw| !raw.trim().is_empty())
        {
            independent.push(account.id.clone());
        }
        eligible.push(account.id);
    }
    let modes = desktop::modes().await?;
    for id in special {
        match modes.get(&id).copied().unwrap_or_default() {
            desktop::Mode::Direct => disabled.push(id.clone()),
            desktop::Mode::Independent | desktop::Mode::BoundOauth => independent.push(id.clone()),
            desktop::Mode::Inherit => (),
        }
        eligible.push(id);
    }
    Ok((eligible, independent, disabled))
}

pub async fn desktop_ids() -> Result<Vec<String>, String> {
    let accounts = runtime::stored_accounts().await?;
    let mut ids = accounts
        .into_iter()
        .filter(CodexAccount::is_api_key_auth)
        .map(|account| account.id)
        .collect::<Vec<_>>();
    ids.push(super::codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID.to_string());
    Ok(ids)
}
