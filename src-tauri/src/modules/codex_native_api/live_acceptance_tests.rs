//! Opt-in process driver for real-account acceptance; never runs in normal tests.
//! Credentials are supplied by an explicitly selected private file, never fixtures.
use std::{io::Read, path::PathBuf, sync::Arc, time::Duration};

use axum::http::{header::AUTHORIZATION, HeaderMap, HeaderValue};
use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{contract::*, start, Options};
use crate::modules::codex_api_identity::InstallationProfile;

#[derive(Deserialize)]
struct LiveAccount {
    id: String,
    owner: String,
    access_token: String,
    workspace: String,
    proxy: Option<String>,
    installation: InstallationProfile,
}

#[derive(Deserialize)]
struct LiveConfig {
    caller: String,
    key: String,
    identity_path: PathBuf,
    ready_path: PathBuf,
    accounts: Vec<LiveAccount>,
}

struct LiveHost(LiveConfig);

impl Host for LiveHost {
    fn authorize<'a>(&'a self, bearer: &'a str) -> BoxFuture<'a, Result<Caller, ServiceError>> {
        Box::pin(async move {
            if bearer != self.0.key {
                return Err(ServiceError::new(
                    401,
                    "invalid_api_key",
                    "Invalid acceptance key",
                ));
            }
            Ok(Caller {
                id: self.0.caller.clone(),
                label: "live acceptance".into(),
            })
        })
    }

    fn route<'a>(
        &'a self,
        _: &'a Caller,
        request: RouteRequest,
    ) -> BoxFuture<'a, Result<Route, ServiceError>> {
        Box::pin(async move {
            let id = request.preferred_account_id.as_deref();
            let account = self
                .0
                .accounts
                .iter()
                .find(|account| {
                    id.is_none_or(|id| id == account.id)
                        && !request.excluded_account_ids.contains(&account.id)
                })
                .ok_or_else(|| {
                    ServiceError::new(
                        503,
                        "no_available_account",
                        "Acceptance account unavailable",
                    )
                })?;
            let mut client = reqwest::Client::builder();
            if let Some(proxy) = &account.proxy {
                client = client.proxy(reqwest::Proxy::all(proxy).map_err(|_| {
                    ServiceError::new(503, "invalid_proxy", "Invalid acceptance proxy")
                })?);
            } else {
                client = client.no_proxy();
            }
            let client = client.build().map_err(|_| {
                ServiceError::new(
                    503,
                    "transport_unavailable",
                    "Acceptance transport unavailable",
                )
            })?;
            let headers = HeaderMap::from_iter([
                (
                    AUTHORIZATION,
                    HeaderValue::from_str(&format!("Bearer {}", account.access_token)).map_err(
                        |_| {
                            ServiceError::new(
                                503,
                                "invalid_credential",
                                "Invalid acceptance credential",
                            )
                        },
                    )?,
                ),
                (
                    "chatgpt-account-id".parse().unwrap(),
                    HeaderValue::from_str(&account.workspace).map_err(|_| {
                        ServiceError::new(503, "invalid_workspace", "Invalid acceptance workspace")
                    })?,
                ),
                (
                    "user-agent".parse().unwrap(),
                    HeaderValue::from_static("codex-tui/0.153.4 (Mac OS 26.5.0; arm64)"),
                ),
                (
                    "originator".parse().unwrap(),
                    HeaderValue::from_static("codex_cli_rs"),
                ),
            ]);
            Ok(Route {
                account_id: account.id.clone(),
                owner_id: account.owner.clone(),
                continuity_account_id: None,
                installation_profile: Some(account.installation.clone()),
                official_authorization_revision: None,
                official_registration_id: None,
                official_fallback_registration_id: None,
                official_fallback_authorization_revision: None,
                authorization_scope_fingerprint: None,
                official_fallback_failure: None,
                transport_identity: None,
                base_url: "https://chatgpt.com/backend-api/codex".into(),
                model: request.model,
                headers,
                client,
                proxy_url: account.proxy.clone(),
                proxy_route: None,
                chatgpt: true,
                chatgpt_plan: false,
                wire_api: WireApi::Responses,
                compact_endpoint: None,
                websocket_endpoint: None,
                default_service_tier: None,
                permit: None,
            })
        })
    }

    fn models<'a>(&'a self, _: &'a Caller) -> BoxFuture<'a, Result<Value, ServiceError>> {
        Box::pin(async { Ok(json!({"data":[{"id":"gpt-6.1-sol"}]})) })
    }

    fn execution_policy(&self) -> BoxFuture<'_, Result<ExecutionPolicy, ServiceError>> {
        Box::pin(async {
            Ok(ExecutionPolicy {
                open_timeout: Duration::from_secs(120),
                idle_timeout: Duration::from_secs(120),
                ..ExecutionPolicy::default()
            })
        })
    }

    fn record(&self, _: RequestReport) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Requires explicit COCKPIT_LIVE_NATIVE_CONFIG and authorized real-account requests"]
async fn live_native_process_driver() -> Result<(), Box<dyn std::error::Error>> {
    let path = PathBuf::from(
        std::env::var_os("COCKPIT_LIVE_NATIVE_CONFIG").ok_or("Missing live configuration")?,
    );
    let metadata = std::fs::symlink_metadata(&path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 1 << 20 {
        return Err("Invalid live configuration file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("Live configuration must be private".into());
        }
    }
    let config: LiveConfig =
        serde_json::from_slice(&std::fs::read(path)?).map_err(|_| "Invalid live configuration")?;
    if config.accounts.len() != 2 || config.key.len() < 32 {
        return Err("Two explicit accounts and a private acceptance key are required".into());
    }
    let ready = config.ready_path.clone();
    let identity = config.identity_path.clone();
    let host = Arc::new(LiveHost(config));
    let mut handle = start("127.0.0.1:0".parse()?, host, identity, Options::default()).await?;
    std::fs::write(
        ready,
        json!({"port":handle.addr.port(),"pid":std::process::id()}).to_string(),
    )?;
    tokio::task::spawn_blocking(|| {
        let _ = std::io::stdin().read(&mut [0u8; 1]);
    })
    .await?;
    handle.shutdown(Duration::from_secs(10)).await?;
    Ok(())
}
