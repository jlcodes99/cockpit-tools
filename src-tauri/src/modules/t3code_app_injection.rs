//! Quota and usage capsule for the T3 Code composer over CDP.

use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::Emitter;
use tokio::time::timeout;

use super::app_cdp::{self, evaluate_target, is_safe_cdp_websocket_url, CdpTarget};
use crate::models::t3code::{T3CodeProviderRuntimeView, T3CodeProviderWorkspaceView};
use crate::modules::{app_lifecycle, codex_app_injection, config, i18n, t3code_provider_bindings};
use codex_app_injection::{dialog_data, InjectionDialogPoller};

#[path = "t3code_app_injection_usage.rs"]
mod thread_usage;
use thread_usage::Conversation;

const INJECTION_INTERVAL: Duration = Duration::from_secs(2);
const WORKSPACE_INTERVAL: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct T3CodeAppInjectionLaunch {
    pub args: Vec<String>,
    pub port: Option<u16>,
}

struct InjectionRuntime {
    task: tauri::async_runtime::JoinHandle<()>,
    port: u16,
    binding_changed: Arc<AtomicBool>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ObservedClerkAccount {
    pub signed_in: bool,
    pub user_id: Option<String>,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub session_id: Option<String>,
    pub expires_at: Option<i64>,
    pub plan_type: Option<String>,
    pub t3chat_linked: Option<bool>,
    pub relay_verified_at: Option<i64>,
    pub relay_failed_at: Option<i64>,
    pub tunnels_used: Option<f64>,
    pub tunnels_limit: Option<f64>,
    pub force_refresh_handled: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EvaluationResult {
    active_instance_id: Option<String>,
    model: Option<String>,
    conversation: Option<Conversation>,
    dialog_query: Option<Value>,
    gateway_active: bool,
    refresh_token: Option<String>,
    refresh_account_id: Option<String>,
    #[serde(default)]
    clerk_account: Option<ObservedClerkAccount>,
    #[serde(default)]
    provider_status: Vec<T3CodeProviderRuntimeView>,
}

type ProviderStatus = (Instant, Vec<T3CodeProviderRuntimeView>);

fn provider_statuses() -> &'static Mutex<HashMap<String, ProviderStatus>> {
    static STATUS: OnceLock<Mutex<HashMap<String, ProviderStatus>>> = OnceLock::new();
    STATUS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn forget_provider_status(instance_id: &str, provider_id: &str) {
    if let Ok(mut statuses) = provider_statuses().lock() {
        if let Some((_, rows)) = statuses.get_mut(instance_id) {
            rows.retain(|status| status.instance_id != provider_id);
        }
    }
}

fn remember_observed_provider_status(instance_id: &str, observed: &[T3CodeProviderRuntimeView]) {
    if let Ok(mut statuses) = provider_statuses().lock() {
        let mut rows = observed.to_vec();
        if let Some((_, existing)) = statuses.get(instance_id) {
            for row in &mut rows {
                if let Some(newer) = existing.iter().find(|previous| {
                    previous.instance_id == row.instance_id
                        && previous.checked_at.as_deref() > row.checked_at.as_deref()
                }) {
                    *row = newer.clone();
                }
            }
        }
        statuses.insert(instance_id.to_owned(), (Instant::now(), rows));
    }
}

pub(crate) fn observed_provider_status(instance_id: &str) -> Vec<T3CodeProviderRuntimeView> {
    let Ok(statuses) = provider_statuses().lock() else {
        return Vec::new();
    };
    statuses
        .get(instance_id)
        .filter(|(at, _)| at.elapsed() < Duration::from_secs(12))
        .map(|(_, rows)| {
            rows.iter()
                .cloned()
                .map(|mut row| {
                    if row.source.is_none() {
                        row.source = Some("live".to_string());
                    }
                    row
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn has_active_connection(instance_id: &str) -> bool {
    provider_statuses().lock().ok().is_some_and(|statuses| {
        statuses
            .get(instance_id)
            .is_some_and(|(at, _)| at.elapsed() < Duration::from_secs(12))
    })
}

fn runtimes() -> &'static Mutex<HashMap<String, InjectionRuntime>> {
    static RUNTIMES: OnceLock<Mutex<HashMap<String, InjectionRuntime>>> = OnceLock::new();
    RUNTIMES.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn notify_provider_binding_changed(instance_id: &str) {
    if let Ok(map) = runtimes().lock() {
        if let Some(runtime) = map.get(instance_id) {
            runtime.binding_changed.store(true, Ordering::Release);
        }
    }
    if let Some(app) = crate::get_app_handle() {
        let _ = app.emit("t3code-provider-binding-changed", instance_id);
    }
}

fn clerk_force_refresh_tokens() -> &'static Mutex<HashMap<String, String>> {
    static TOKENS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    TOKENS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn request_clerk_refresh(instance_id: &str) -> Option<String> {
    let is_running = runtimes()
        .lock()
        .ok()
        .is_some_and(|map| map.contains_key(instance_id));
    if !is_running {
        return None;
    }
    let mut tokens = clerk_force_refresh_tokens().lock().ok()?;
    Some(
        tokens
            .entry(instance_id.to_owned())
            .or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone(),
    )
}

fn reserve_cdp_port() -> Result<u16, String> {
    TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("分配 T3 Code CDP 端口失败: {}", error))?
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| format!("读取 T3 Code CDP 端口失败: {}", error))
}

fn is_debug_arg(value: &str, name: &str) -> bool {
    value == name || value.starts_with(&format!("{}=", name))
}

pub fn enabled_for_app() -> bool {
    config::get_user_config().t3code_app_ui_injection_enabled
}

pub fn build_launch_args(
    existing: &[String],
    enabled: bool,
) -> Result<T3CodeAppInjectionLaunch, String> {
    if !enabled {
        return Ok(T3CodeAppInjectionLaunch {
            args: existing.to_vec(),
            port: None,
        });
    }

    let mut args = Vec::with_capacity(existing.len() + 2);
    let mut skip_next = false;
    for value in existing {
        if skip_next {
            skip_next = false;
            continue;
        }
        if is_debug_arg(value, "--remote-debugging-port")
            || is_debug_arg(value, "--remote-debugging-address")
        {
            if value == "--remote-debugging-port" || value == "--remote-debugging-address" {
                skip_next = true;
            }
            continue;
        }
        args.push(value.clone());
    }

    let port = reserve_cdp_port()?;
    args.push("--remote-debugging-address=127.0.0.1".to_string());
    args.push(format!("--remote-debugging-port={}", port));
    Ok(T3CodeAppInjectionLaunch {
        args,
        port: Some(port),
    })
}

pub fn stop_for_instance(instance_id: &str) {
    crate::modules::t3code_account::forget_observed_clerk_state(Some(instance_id));
    if let Ok(mut map) = runtimes().lock() {
        if let Some(runtime) = map.remove(instance_id) {
            runtime.task.abort();
        }
    }
    if let Ok(mut statuses) = provider_statuses().lock() {
        statuses.remove(instance_id);
    }
    if let Ok(mut tokens) = clerk_force_refresh_tokens().lock() {
        tokens.remove(instance_id);
    }
}

pub fn stop_all() {
    crate::modules::t3code_account::forget_observed_clerk_state(None);
    if let Ok(mut map) = runtimes().lock() {
        for (_, runtime) in map.drain() {
            runtime.task.abort();
        }
    }
    if let Ok(mut statuses) = provider_statuses().lock() {
        statuses.clear();
    }
    if let Ok(mut tokens) = clerk_force_refresh_tokens().lock() {
        tokens.clear();
    }
}

pub fn start_for_launched_instance(instance_id: &str, state_dir: &Path, port: u16) {
    if !enabled_for_app() {
        stop_for_instance(instance_id);
        return;
    }
    stop_for_instance(instance_id);
    let id = instance_id.to_string();
    let dir = state_dir.to_path_buf();
    let binding_changed = Arc::new(AtomicBool::new(false));
    let binding_signal = Arc::clone(&binding_changed);
    let task = tauri::async_runtime::spawn(async move {
        run_injection_loop(id, dir, port, binding_signal).await;
    });
    if let Ok(mut map) = runtimes().lock() {
        map.insert(
            instance_id.to_string(),
            InjectionRuntime {
                task,
                port,
                binding_changed,
            },
        );
    }
}

fn is_t3code_app_target(target: &CdpTarget) -> bool {
    if target.target_type.as_deref() != Some("page") {
        return false;
    }
    let url = target.url.as_deref().unwrap_or_default();
    if url.starts_with("devtools://") || url.starts_with("chrome-extension://") {
        return false;
    }
    let title = target
        .title
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    url.contains("localhost")
        || url.contains("127.0.0.1")
        || url.starts_with("t3")
        || title.contains("t3 code")
        || title.contains("t3code")
}

async fn query_targets(client: &Client, port: u16) -> Vec<CdpTarget> {
    app_cdp::query_targets(client, port, is_t3code_app_target)
        .await
        .unwrap_or_default()
}

/// Obtain an RPC ticket through the official desktop preload. The bearer stays in T3 Code.
pub(super) async fn provider_control_url(
    instance_id: &str,
    environment_id: &str,
) -> Option<reqwest::Url> {
    let port = runtimes().lock().ok()?.get(instance_id)?.port;
    let client = Client::builder().no_proxy().build().ok()?;
    let targets = query_targets(&client, port).await;
    let script = format!(
        "({})({})",
        include_str!("t3code_provider_control_ticket.js"),
        json!(environment_id)
    );
    for target in targets {
        let Some(raw) = evaluate_target::<String>(&target, &script, true).await else {
            continue;
        };
        let Ok(url) = reqwest::Url::parse(&raw) else {
            continue;
        };
        if url
            .port()
            .is_some_and(|port| is_safe_cdp_websocket_url(&raw, port))
            && url.path() == "/ws"
        {
            return Some(url);
        }
    }
    None
}

pub(super) async fn native_client_settings(field: Option<&str>, value: &Value) -> Option<Value> {
    let port = runtimes().lock().ok()?.get("__default__")?.port;
    let client = Client::builder().no_proxy().build().ok()?;
    let targets = query_targets(&client, port).await;
    let script = format!(
        "({})({})",
        include_str!("t3code_native_settings.js"),
        json!({ "field": field, "value": value }),
    );
    // One desktop window owns the saved client preferences. Never repeat a
    // write after a failed or ambiguous result in that window.
    let target = targets.first()?;
    evaluate_target::<Value>(target, &script, true)
        .await
        .filter(Value::is_object)
}

fn build_composer_injection_script(state: Value, locale: &str) -> String {
    let labels = json!({
        "title": i18n::translate(locale, "common.codexUsage.title", &[]),
        "close": i18n::translate(locale, "common.close", &[]),
        "unknownPlan": i18n::translate(locale, "common.unknown", &[]),
        "noQuota": i18n::translate(locale, "t3code.quotaQuery.empty", &[]),
        "identityPending": i18n::translate(locale, "t3code.providerWorkspace.runtimePending", &[]),
        "accountMismatch": i18n::translate(locale, "t3code.overview.accountMismatch", &[]),
        "credits": i18n::translate(locale, "accounts.credits.label", &[]),
        "speed": i18n::translate(locale, "common.codexUsage.speed", &[]),
        "responseSpeed": i18n::translate(locale, "common.codexUsage.responseSpeed", &[]),
        "overallSpeed": i18n::translate(locale, "common.codexUsage.overallSpeedWithThinking", &[]),
        "streamingSpeed": i18n::translate(locale, "common.codexUsage.streamingSpeed", &[]),
        "context": i18n::translate(locale, "common.codexUsage.contextShort", &[]),
        "token": i18n::translate(locale, "common.codexUsage.token", &[]),
        "noData": i18n::translate(locale, "common.noData", &[]),
        "resetsAt": i18n::translate(locale, "common.apiBudget.resetsAt", &[]),
        "gatewayName": i18n::translate(locale, "common.apiBudget.gatewayName", &[]),
        "unlimited": i18n::translate(locale, "common.apiBudget.unmeteredShort", &[]),
        "weekly": i18n::translate(locale, "settings.general.codexAppUiInjectionWeeklyLabel", &[]),
        "fiveHour": i18n::translate(locale, "settings.general.codexAppUiInjectionFiveHourLabel", &[]),
        "missing": i18n::translate(locale, "settings.general.codexAppUiInjectionMissingLabel", &[]),
        "plan": i18n::translate(locale, "floatingCard.plan", &[]),
        "accountPool": i18n::translate(locale, "settings.general.codexAppUiInjectionPoolLabel", &[]),
        "balance": i18n::translate(locale, "codex.modelProviders.usage.fields.balance", &[]),
        "quotaOverview": i18n::translate(locale, "codex.modelProviders.usage.title", &[]),
        "official": i18n::translate(locale, "chatgptPlan.officialFlow.badge", &[]),
        "grokEmpty": i18n::translate(locale, "grok.quotaQuery.empty", &[]),
    });
    let usage_labels = codex_app_injection::usage_dialog_labels(locale);
    let plan_badge_tokens = include_str!("../../../src/styles/plan-badge-tokens.css").replacen(
        ":root",
        "[data-cockpit-t3-details]",
        1,
    );
    let styles = json!([
        include_str!("codex_app_injection_footer.css"),
        include_str!("codex_app_injection_dialog.css"),
        plan_badge_tokens,
        include_str!("t3code_app_injection_badges.css"),
    ]);
    let usage_dialog = include_str!("codex_app_injection_dialog.js");
    let composer_js = include_str!("t3code_app_injection_composer.js");
    let selection_js = include_str!("t3code_app_injection_selection.js");
    let clerk_js = include_str!("t3code_app_injection_clerk.js");
    let context_usage = include_str!("composer_context_usage.js");
    format!("(() => {{
        const state = {state}, labels = {labels}, usageLabels = {usage_labels}, sharedStyles = {styles};
        const quotaSelection = {selection_js};
        const clerkSync = {clerk_js};
        const formatContextUsage = {context_usage};
        const createUsageDialog = ({{
            accountCount, availableAccountCount, abnormalAccountCount, cooldownAccountCount,
            plans = [], providerQuotas = [], grokLines = [], balanceLines = [],
        }} = {{}}) => {{
            const missingLabel = labels.missing, closeLabel = labels.close, officialLabel = labels.official;
            const accountPoolLabel = labels.accountPool, planLabel = labels.plan, weeklyLabel = labels.weekly;
            const fiveHourLabel = labels.fiveHour, balanceLabel = labels.balance, quotaOverviewLabel = labels.quotaOverview;
            const quotaEmptyLabel = labels.noQuota, grokEmptyLabel = labels.grokEmpty;
            return (
                {usage_dialog}
            );
        }};
        return ({composer_js})();
    }})()")
}

type WorkspaceRead = (
    Option<T3CodeProviderWorkspaceView>,
    Option<String>,
    Option<String>,
);

fn load_workspace(instance_id: String, state_dir: PathBuf) -> WorkspaceRead {
    let workspace = t3code_provider_bindings::get_provider_workspace(Some(&instance_id)).ok();
    let environment_id = std::fs::read_to_string(state_dir.join("environment-id"))
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let badge_style = crate::modules::ui_preferences::load_ui_preferences()
        .ok()
        .and_then(|prefs| prefs.values.get("agtools.codex_plan_badge_style").cloned());
    (workspace, environment_id, badge_style)
}

async fn read_workspace_after_refresh(
    instance_id: String,
    state_dir: PathBuf,
    refresh: Option<(String, String)>,
) -> (WorkspaceRead, bool) {
    let Some((provider_id, account_id)) = refresh else {
        let view = tokio::task::spawn_blocking(move || load_workspace(instance_id, state_dir))
            .await
            .unwrap_or_default();
        return (view, false);
    };
    let target_id = instance_id.clone();
    let success = matches!(
        timeout(Duration::from_secs(25), async move {
            let workspace = tokio::task::spawn_blocking(move || {
                t3code_provider_bindings::get_provider_workspace(Some(&target_id))
            })
            .await
            .map_err(|error| error.to_string())??;
            let provider = workspace
                .provider_instances
                .iter()
                .find(|provider| {
                    provider.instance_id == provider_id
                        && !provider.bound_is_api_service
                        && provider.bound_account_id.as_deref() == Some(account_id.as_str())
                })
                .ok_or_else(|| "T3 Code provider binding changed".to_owned())?;
            t3code_provider_bindings::refresh_bound_account_quota(&provider.driver, &account_id)
                .await
        })
        .await,
        Ok(Ok(()))
    );
    let view = tokio::task::spawn_blocking(move || load_workspace(instance_id, state_dir))
        .await
        .unwrap_or_default();
    let success = success && view.0.is_some();
    (view, success)
}

#[derive(Debug, PartialEq, Eq, Clone)]
struct GatewaySelection {
    instance_id: String,
    home: PathBuf,
    account_id: String,
}

fn gateway_selection(
    workspace: &T3CodeProviderWorkspaceView,
    instance_id: &str,
) -> Option<GatewaySelection> {
    let instance = workspace
        .provider_instances
        .iter()
        .find(|instance| instance.instance_id == instance_id && instance.bound_is_api_service)?;
    let home = instance
        .home_path
        .as_deref()
        .or(instance.shadow_home_path.as_deref())?;
    Some(GatewaySelection {
        instance_id: instance_id.to_owned(),
        home: PathBuf::from(home),
        account_id: instance.bound_account_id.clone()?,
    })
}

#[derive(Debug, PartialEq, Eq, Clone)]
struct ThreadSelection {
    conversation: Conversation,
    gateway: Option<GatewaySelection>,
    model: Option<String>,
}

struct ThreadSnapshot {
    selection: ThreadSelection,
    usage: thread_usage::ThreadUsage,
    speed: Option<Value>,
}

async fn run_injection_loop(
    instance_id: String,
    state_dir: PathBuf,
    port: u16,
    binding_changed: Arc<AtomicBool>,
) {
    let client = Client::new();
    let mut workspace: Option<T3CodeProviderWorkspaceView> = None;
    let mut workspace_read = None::<tokio::task::JoinHandle<(WorkspaceRead, bool)>>;
    let mut workspace_refresh = None;
    let mut discard_workspace_read = false;
    let mut next_workspace_read = Instant::now();
    let mut gateway_data = HashMap::<String, (GatewaySelection, Value)>::new();
    let mut gateway_at = HashMap::<String, (GatewaySelection, String, Instant)>::new();
    let mut gateway_task =
        None::<tokio::task::JoinHandle<(GatewaySelection, Option<String>, bool, Option<Value>)>>;
    let mut dialogs = HashMap::<PathBuf, InjectionDialogPoller>::new();
    let mut thread_task =
        None::<tokio::task::JoinHandle<(ThreadSelection, rusqlite::Result<ThreadSnapshot>)>>;
    let mut thread_snapshot: Option<ThreadSnapshot> = None;
    let mut thread_at = None::<(ThreadSelection, Instant)>;
    let mut environment_id = None;
    let mut startup_model_refresh_scheduled = false;
    let mut codex_plan_badge_style = None;
    let mut refresh_result = None::<(String, bool)>;
    let mut previous: Option<EvaluationResult> = None;
    let mut last_clerk_sync: Option<ObservedClerkAccount> = None;
    let mut clerk_sync_task =
        None::<tokio::task::JoinHandle<(ObservedClerkAccount, Result<bool, String>)>>;

    loop {
        if app_lifecycle::is_shutdown_started() || !enabled_for_app() {
            tokio::time::sleep(INJECTION_INTERVAL).await;
            continue;
        }
        let targets = query_targets(&client, port).await;
        if targets.is_empty() {
            crate::modules::t3code_account::forget_observed_clerk_state(Some(&instance_id));
            last_clerk_sync = None;
            tokio::time::sleep(INJECTION_INTERVAL).await;
            continue;
        }
        let locale = config::get_user_config().language;

        if clerk_sync_task
            .as_ref()
            .is_some_and(|job| job.is_finished())
        {
            if let Ok((synced, changed)) = clerk_sync_task.take().unwrap().await {
                last_clerk_sync = Some(synced);
                if let Err(error) = &changed {
                    crate::modules::logger::log_warn(&format!(
                        "[T3 Code] 同步 T3 Connect 账号失败: {error}"
                    ));
                }
                if matches!(changed, Ok(true)) {
                    if let Some(app) = crate::get_app_handle() {
                        let _ = app.emit("t3code-accounts-updated", &instance_id);
                    }
                }
            }
        }

        if binding_changed.swap(false, Ordering::AcqRel) {
            if workspace_refresh.is_some() {
                // Keep the explicit quota refresh and discard its pre-switch workspace snapshot.
                discard_workspace_read = true;
            } else if let Some(read) = workspace_read.take() {
                read.abort();
            }
            next_workspace_read = Instant::now();
        }

        if workspace_read.as_ref().is_some_and(|job| job.is_finished()) {
            let completed = workspace_read.take().unwrap().await;
            let success = completed.as_ref().is_ok_and(|(_, success)| *success);
            if let Some(token) = workspace_refresh.take() {
                refresh_result = Some((token, success));
            }
            let discard = std::mem::take(&mut discard_workspace_read);
            match completed {
                Ok(((updated, environment, style), _)) if !discard => {
                    environment_id = environment;
                    codex_plan_badge_style = style;
                    if updated.is_some() {
                        workspace = updated;
                    }
                }
                _ => {}
            }
        }
        let selected = previous.as_ref();
        let refresh_token = selected
            .and_then(|result| result.refresh_token.as_ref())
            .filter(|token| {
                token.len() <= 128
                    && !token.is_empty()
                    && !refresh_result
                        .as_ref()
                        .is_some_and(|(handled, _)| handled == *token)
            });
        let direct_refresh =
            refresh_token.filter(|_| selected.is_some_and(|result| !result.gateway_active));
        if workspace_read.is_none()
            && (Instant::now() >= next_workspace_read || direct_refresh.is_some())
        {
            let (id, dir) = (instance_id.clone(), state_dir.clone());
            let refresh = direct_refresh.and_then(|_| {
                let selection = selected?;
                Some((
                    selection.active_instance_id.clone()?,
                    selection.refresh_account_id.clone()?,
                ))
            });
            workspace_refresh = direct_refresh.cloned();
            next_workspace_read = Instant::now() + WORKSPACE_INTERVAL;
            workspace_read = Some(tokio::spawn(read_workspace_after_refresh(id, dir, refresh)));
        }

        let current_gateway = |id: &str| {
            workspace
                .as_ref()
                .and_then(|view| gateway_selection(view, id))
        };
        if gateway_task.as_ref().is_some_and(|task| task.is_finished()) {
            if let Ok((choice, token, refreshed, usage)) = gateway_task.take().unwrap().await {
                if current_gateway(&choice.instance_id).as_ref() == Some(&choice) {
                    let has_usage = usage.is_some();
                    if let Some(usage) = usage {
                        gateway_data.insert(choice.instance_id.clone(), (choice, usage));
                    }
                    if let Some(token) = token {
                        refresh_result = Some((token, refreshed && has_usage));
                    }
                }
            }
        }
        gateway_data.retain(|id, (choice, _)| current_gateway(id).as_ref() == Some(choice));
        gateway_at.retain(|id, (choice, _, _)| current_gateway(id).as_ref() == Some(choice));

        let gateway =
            selected.and_then(|result| current_gateway(result.active_instance_id.as_deref()?));
        let active_gateway = gateway
            .as_ref()
            .filter(|_| selected.is_some_and(|result| result.gateway_active));
        let selected_model = selected
            .and_then(|result| result.model.as_deref())
            .unwrap_or_default();
        if let Some(choice) = gateway.as_ref().filter(|choice| {
            gateway_task.is_none()
                && (active_gateway.is_some() && refresh_token.is_some()
                    || gateway_at
                        .get(&choice.instance_id)
                        .is_none_or(|(prior, model, at)| {
                            prior != *choice
                                || model != selected_model
                                || at.elapsed() >= WORKSPACE_INTERVAL
                        }))
        }) {
            gateway_at.insert(
                choice.instance_id.clone(),
                (choice.clone(), selected_model.to_owned(), Instant::now()),
            );
            let (client, choice, model, locale) = (
                client.clone(),
                choice.clone(),
                selected_model.to_owned(),
                locale.clone(),
            );
            let token = refresh_token.filter(|_| active_gateway.is_some()).cloned();
            gateway_task = Some(tokio::spawn(async move {
                let refreshed = if token.is_some() {
                    codex_app_injection::refresh_t3code_gateway_quota().await
                } else {
                    true
                };
                let mut usage = codex_app_injection::t3code_gateway_usage(
                    &client,
                    choice.home.clone(),
                    Some(&model),
                    &locale,
                )
                .await;
                if let Some(usage) = usage.as_mut() {
                    usage["model"] = json!(model);
                }
                (choice, token, refreshed, usage)
            }));
        }

        let thread = selected
            .and_then(|result| result.conversation.as_ref())
            .filter(|conversation| {
                Some(&conversation.environment_id) == environment_id.as_ref()
                    && conversation.thread_id.len() <= 128
                    && !conversation.thread_id.is_empty()
                    && Some(conversation.provider_instance_id.as_str())
                        == selected.and_then(|result| result.active_instance_id.as_deref())
            })
            .map(|conversation| ThreadSelection {
                conversation: conversation.clone(),
                gateway: active_gateway.cloned(),
                model: selected.and_then(|result| result.model.clone()),
            });
        if thread_task.as_ref().is_some_and(|task| task.is_finished()) {
            if let Ok((choice, result)) = thread_task.take().unwrap().await {
                if thread.as_ref() == Some(&choice) {
                    match result {
                        Ok(snapshot) => thread_snapshot = Some(snapshot),
                        Err(error) => crate::modules::logger::log_warn(&format!(
                            "[T3 Code Usage] Thread projection unavailable: {error}"
                        )),
                    }
                }
            }
        }
        if let Some(choice) = thread.as_ref().filter(|choice| {
            thread_task.is_none()
                && thread_at.as_ref().is_none_or(|(prior, at)| {
                    prior != *choice || at.elapsed() >= WORKSPACE_INTERVAL / 2
                })
        }) {
            let (choice, dir) = (choice.clone(), state_dir.clone());
            thread_at = Some((choice.clone(), Instant::now()));
            thread_task = Some(tokio::task::spawn_blocking(move || {
                let snapshot = thread_usage::read_thread_usage(
                    &dir,
                    &choice.conversation,
                    choice.model.as_deref(),
                )
                .map(|usage| {
                    let speed = choice
                        .gateway
                        .as_ref()
                        .zip(usage.native_thread_id.as_deref())
                        .and_then(|(gateway, id)| {
                            codex_app_injection::t3code_gateway_speed(
                                &gateway.home,
                                id,
                                choice.model.clone(),
                            )
                        });
                    ThreadSnapshot {
                        selection: choice.clone(),
                        usage,
                        speed,
                    }
                });
                (choice, snapshot)
            }));
        }
        let snapshot = thread_snapshot
            .as_ref()
            .filter(|snapshot| Some(&snapshot.selection) == thread.as_ref());
        let mut dialog_data = Value::Null;
        for (path, poller) in &mut dialogs {
            if active_gateway.is_none_or(|choice| &choice.home != path) {
                poller.request(vec![]);
                poller.collect();
            }
        }
        if let Some(choice) = active_gateway {
            if let Some((_, usage)) = gateway_data.get(&choice.instance_id) {
                let native_thread =
                    snapshot.and_then(|snapshot| snapshot.usage.native_thread_id.as_deref());
                let query = selected
                    .and_then(|result| result.dialog_query.as_ref())
                    .and_then(dialog_data::parse_query)
                    .filter(|query| {
                        query.binding.as_deref() == usage["binding"].as_str()
                            && query.thread_id.as_deref() == native_thread
                    });
                let poller = dialogs
                    .entry(choice.home.clone())
                    .or_insert_with(|| InjectionDialogPoller::new(&choice.home));
                poller.request(query.into_iter().collect());
                dialog_data = poller.collect();
            }
        }

        let gateways: Vec<_> = gateway_data
            .values()
            .map(|(choice, usage)| json!({"instanceId": choice.instance_id, "usage": usage}))
            .collect();
        let clerk_force_refresh = clerk_force_refresh_tokens()
            .lock()
            .ok()
            .and_then(|map| map.get(&instance_id).cloned());
        let script = build_composer_injection_script(
            json!({
                "workspace": workspace.as_ref().map(|view| json!({"providerInstances": view.provider_instances})),
                "environmentId": environment_id,
                "codexPlanBadgeStyle": codex_plan_badge_style,
                "gateways": gateways,
                "thread": snapshot.map(|snapshot| json!({
                    "conversation": snapshot.selection.conversation,
                    "usage": snapshot.usage,
                    "speed": snapshot.speed,
                    "model": snapshot.selection.model,
                })),
                "dialog": dialog_data,
                "refresh": refresh_result.as_ref().map(|(token, success)| json!({
                    "token": token, "success": success,
                })),
                "clerkForceRefresh": clerk_force_refresh,
            }),
            &locale,
        );

        let results = futures_util::future::join_all(
            targets
                .iter()
                .map(|target| evaluate_target::<EvaluationResult>(target, &script, false)),
        )
        .await;
        if let Some(clerk_observed) = results
            .iter()
            .flatten()
            .filter_map(|result| result.clerk_account.clone())
            .max_by_key(|account| {
                (
                    account.signed_in,
                    account.tunnels_limit.is_some(),
                    account.email.is_some(),
                )
            })
        {
            if let Ok(mut tokens) = clerk_force_refresh_tokens().lock() {
                if tokens.get(&instance_id) == clerk_observed.force_refresh_handled.as_ref() {
                    tokens.remove(&instance_id);
                }
            }
            crate::modules::t3code_account::remember_observed_clerk_state(
                &instance_id,
                &state_dir,
                &clerk_observed,
            );
            if !clerk_observed.signed_in {
                last_clerk_sync = None;
            } else if clerk_sync_task.is_none() && last_clerk_sync.as_ref() != Some(&clerk_observed)
            {
                let dir = state_dir.clone();
                let observed_clone = clerk_observed.clone();
                clerk_sync_task = Some(tokio::task::spawn_blocking(move || {
                    let changed = crate::modules::t3code_account::sync_observed_clerk_account(
                        &dir,
                        &observed_clone,
                    );
                    (observed_clone, changed)
                }));
            }
        } else if results.iter().any(|result| result.is_some()) {
            crate::modules::t3code_account::forget_observed_clerk_state(Some(&instance_id));
            last_clerk_sync = None;
        }
        if let Some(status) = results
            .iter()
            .flatten()
            .map(|result| &result.provider_status)
            .filter(|status| !status.is_empty())
            .max_by_key(|status| status.len())
        {
            remember_observed_provider_status(&instance_id, status);
            if let Some(workspace) = workspace
                .as_ref()
                .filter(|_| !startup_model_refresh_scheduled)
            {
                startup_model_refresh_scheduled = true;
                for provider in &workspace.provider_instances {
                    // Antigravity's normal startup check only resolves the executable.
                    if provider.driver == t3code_provider_bindings::DRIVER_ANTIGRAVITY
                        && provider.enabled
                        && provider.bound_account_id.is_some()
                        && !t3code_provider_bindings::live::model_refresh_pending(
                            &state_dir,
                            &provider.instance_id,
                        )
                        && !status.iter().any(|row| {
                            row.instance_id == provider.instance_id
                                && row.auth_status.as_deref() == Some("authenticated")
                                && row.model_count > 0
                        })
                    {
                        t3code_provider_bindings::live::schedule_model_refresh(
                            &instance_id,
                            &state_dir,
                            &provider.instance_id,
                        );
                    }
                }
            }
        }
        previous = results
            .into_iter()
            .flatten()
            .filter(|result| result.active_instance_id.is_some())
            .max_by_key(|result| result.dialog_query.is_some() || result.refresh_token.is_some());
        tokio::time::sleep(INJECTION_INTERVAL).await;
    }
}
