//! Optional desktop enhancements, scoped to a launched Codex profile and its CDP port.
//! Slow credential reads and all network work stay outside the launch transaction.

mod fast;
mod browser_preload;

use crate::modules::{app_lifecycle, codex_account, config, logger};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::watch;
use tokio::task::JoinHandle;

const POLL: Duration = Duration::from_secs(3);
const RETRY: Duration = Duration::from_secs(30);

fn runtimes() -> &'static Mutex<HashMap<PathBuf, watch::Sender<bool>>> {
    static RUNTIMES: OnceLock<Mutex<HashMap<PathBuf, watch::Sender<bool>>>> = OnceLock::new();
    RUNTIMES.get_or_init(Default::default)
}

pub fn enabled() -> bool {
    let cfg = config::get_user_config();
    cfg.codex_app_native_fast_enabled || cfg.codex_app_api_quota_compat_enabled
}

pub fn start_for_profile(
    profile: PathBuf,
    instance: String,
    port: u16,
    fresh_launch: bool,
    api_service: bool,
) {
    stop_for_profile(&profile);
    if !enabled() && !api_service {
        return;
    }
    let (stop, receiver) = watch::channel(false);
    if let Ok(mut active) = runtimes().lock() {
        active.insert(profile.clone(), stop);
    }
    let startup = fresh_launch.then(|| Arc::new(fast::StartupReload::new()));
    tauri::async_runtime::spawn(run_profile(
        profile,
        instance,
        port,
        startup,
        api_service,
        receiver,
    ));
}

pub fn stop_for_profile(profile: &Path) {
    if let Ok(mut active) = runtimes().lock() {
        if let Some(stop) = active.remove(profile) {
            let _ = stop.send(true);
        }
    }
}

pub fn stop_all() {
    if let Ok(mut active) = runtimes().lock() {
        for (_, stop) in active.drain() {
            let _ = stop.send(true);
        }
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
struct Profile {
    native_fast: bool,
    api_key_auth_verified: bool,
    quota_compat: bool,
}

impl Profile {
    fn active(&self) -> bool {
        self.native_fast || self.quota_compat
    }
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn profile_from_values(
    doc: &toml_edit::Document,
    auth: &Value,
    cfg: &config::UserConfig,
) -> Profile {
    let selected = doc
        .get("profile")
        .and_then(|item| item.as_str())
        .and_then(|name| doc.get("profiles")?.get(name));
    let provider_id = selected
        .and_then(|profile| profile.get("model_provider"))
        .or_else(|| doc.get("model_provider"))
        .and_then(|item| item.as_str())
        .unwrap_or("openai");
    let provider = doc
        .get("model_providers")
        .and_then(|providers| providers.get(provider_id));
    let requires_auth = provider
        .and_then(|provider| provider.get("requires_openai_auth"))
        .and_then(|item| item.as_bool());
    let oauth = nonempty(auth.pointer("/tokens/id_token").and_then(Value::as_str)).is_some()
        && nonempty(auth.pointer("/tokens/access_token").and_then(Value::as_str)).is_some();
    let api_auth = nonempty(auth.get("OPENAI_API_KEY").and_then(Value::as_str));
    let bearer = provider
        .and_then(|provider| provider.get("experimental_bearer_token"))
        .and_then(|item| nonempty(item.as_str()));
    let api_key_auth_verified =
        !oauth && (requires_auth == Some(false) || api_auth.is_some() || bearer.is_some());
    let base = provider
        .and_then(|provider| provider.get("base_url"))
        .and_then(|item| item.as_str())
        .or_else(|| doc.get("openai_base_url").and_then(|item| item.as_str()))
        .or_else(|| (provider_id == "openai").then_some("https://api.openai.com/v1"));
    let custom_api = provider_id != "openai"
        && bearer.is_some()
        && bearer != auth.pointer("/tokens/access_token").and_then(Value::as_str);
    // Quota compatibility still requires a real configured API route. No audio
    // endpoint or transcription credential transport is prepared here.
    let valid_api_base = base.is_some_and(|base| {
        url::Url::parse(base).is_ok_and(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
        })
    });
    Profile {
        native_fast: cfg.codex_app_native_fast_enabled && api_key_auth_verified,
        api_key_auth_verified,
        quota_compat: cfg.codex_app_api_quota_compat_enabled
            && requires_auth == Some(true)
            && oauth
            && custom_api
            && valid_api_base,
    }
}

fn read_profile(profile: &Path) -> Result<Profile, ()> {
    let cfg = config::get_user_config();
    let text = std::fs::read_to_string(profile.join("config.toml")).map_err(|_| ())?;
    let doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(&text)
        .map_err(|_| ())?;
    // Respect file/keyring/auto selection on both macOS and Windows.
    let auth = codex_account::read_configured_codex_auth_value(profile)
        .map_err(|_| ())?
        .unwrap_or(Value::Null);
    Ok(profile_from_values(&doc, &auth, &cfg))
}

#[derive(Deserialize)]
struct Target {
    #[serde(rename = "type")]
    kind: String,
    url: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    socket: Option<String>,
}

fn safe_socket(value: &str, port: u16) -> bool {
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    url.scheme() == "ws"
        && url.host_str() == Some("127.0.0.1")
        && url.port() == Some(port)
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.path().starts_with("/devtools/page/")
}

async fn targets(
    client: &reqwest::Client,
    port: u16,
    include_pending: bool,
) -> Option<HashSet<String>> {
    let response = client
        .get(format!("http://127.0.0.1:{port}/json/list"))
        .send()
        .await
        .ok()?;
    let values = response
        .error_for_status()
        .ok()?
        .json::<Vec<Target>>()
        .await
        .ok()?;
    Some(
        values
            .into_iter()
            .filter(|target| {
                matches!(target.kind.as_str(), "page" | "webview")
                    && (target.url.starts_with("app://-/")
                        || include_pending
                            && (target.url.is_empty() || target.url == "about:blank"))
            })
            .filter_map(|target| target.socket.filter(|socket| safe_socket(socket, port)))
            .collect(),
    )
}

struct Child {
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}
impl Child {
    fn stop(self) {
        let _ = self.stop.send(true);
    }
}

// APIService capability denial must not wait on, or be revoked by, a slow
// keyring/config read. Optional Fast/quota adaptations fail closed meanwhile.
fn profile_or_api_fallback(profile: Option<Profile>, api_service: bool) -> Option<Profile> {
    profile.or_else(|| api_service.then(Profile::default))
}

fn trusted_preload_source(profile: Option<&Profile>, api_service: bool, port: u16) -> Option<String> {
    profile.filter(|profile| profile.active() || api_service).map(|profile| {
        fast::browser_preload_source(profile.native_fast, profile.quota_compat,
            profile.api_key_auth_verified, api_service, &format!("preload-{port}"))
    })
}

// Browser pre-registration also protects restored macOS conversations from
// needing a reload. Only Windows requires the page worker to stay reload-free;
// macOS retains its existing guarded fallback if browser attachment is late.
fn startup_preload_policy(platform: &str, fresh_launch: bool) -> (bool, bool) {
    (
        fresh_launch && matches!(platform, "windows" | "macos"),
        fresh_launch && platform == "windows",
    )
}

async fn run_profile(
    profile_dir: PathBuf,
    instance: String,
    port: u16,
    startup: Option<Arc<fast::StartupReload>>,
    api_service: bool,
    mut stop: watch::Receiver<bool>,
) {
    // The launch transaction starts this worker before starting the process.
    // Browser discovery must race neither a blocking keyring read nor a second
    // copy of that read. No target is paused while the trusted snapshot loads.
    let (browser_preload_enabled, preload_first) =
        startup_preload_policy(std::env::consts::OS, startup.is_some());
    let Ok(client) = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .redirect(reqwest::redirect::Policy::none())
        .build()
    else {
        return;
    };
    let preload_source = if browser_preload_enabled {
        let (source, receiver) = watch::channel(None);
        let preload_stop = stop.clone();
        let launch = startup.clone().expect("preload requires an explicit launch");
        tauri::async_runtime::spawn(async move {
            if let Err(error) = browser_preload::run(port, receiver, preload_stop, launch.remaining()).await {
                logger::log_warn(&format!("[Codex enhancements] browser preload ended: {error}"));
            }
        });
        Some(source)
    } else { None };
    let mut children: HashMap<String, Child> = HashMap::new();
    let mut retry_at: HashMap<String, Instant> = HashMap::new();
    let mut current = profile_or_api_fallback(None, api_service);
    let mut read: Option<JoinHandle<Result<Profile, ()>>> = None;
    let mut read_started = Instant::now();
    let mut read_timed_out = false;
    let mut initial_snapshot_ready = !browser_preload_enabled;
    let mut current_verified = false;
    // New launches discover the first completed credential read promptly, before
    // the user starts editing. Later refreshes retain the ordinary polling rate.
    let mut initial_polling = startup.is_some();
    let mut tick = tokio::time::interval(if initial_polling {
        Duration::from_millis(100)
    } else {
        POLL
    });
    loop {
        tokio::select! { biased;
            _ = stop.changed() => break,
            _ = tick.tick() => {}
        }
        if *stop.borrow() || app_lifecycle::is_shutdown_started() {
            break;
        }
        if read.as_ref().is_some_and(JoinHandle::is_finished) {
            let verified = read.take().unwrap().await.ok().and_then(Result::ok);
            initial_snapshot_ready = true;
            current_verified = verified.is_some();
            let next = profile_or_api_fallback(verified, api_service);
            if initial_polling {
                initial_polling = false;
                tick = tokio::time::interval(POLL);
                tick.reset();
            }
            if next != current {
                for (_, child) in children.drain() {
                    child.stop();
                }
                retry_at.clear();
                current = next;
            }
        } else if read.is_some() && !read_timed_out && read_started.elapsed() > Duration::from_secs(10) {
            // A stalled keyring probe remains single-flight; revoke active adapters
            // while waiting rather than launching more blocking credential reads.
            if let Some(sender) = &preload_source { sender.send_replace(None); }
            read_timed_out = true;
            initial_snapshot_ready = true;
            current_verified = false;
            if initial_polling {
                initial_polling = false;
                tick = tokio::time::interval(POLL);
                tick.reset();
            }
            logger::log_warn("[Codex enhancements] profile snapshot timed out; optional adaptations suspended");
            let safe = profile_or_api_fallback(None, api_service);
            if current != safe {
                for (_, child) in children.drain() {
                    child.stop();
                }
                current = safe;
            }
        }
        if read.is_none() {
            let path = profile_dir.clone();
            let sender = preload_source.clone();
            read_started = Instant::now();
            read_timed_out = false;
            read = Some(tokio::task::spawn_blocking(move || {
                let result = read_profile(&path);
                // Publish immediately from the one existing read worker, not
                // after page discovery's network timeout or the next poll.
                // An API binding without a verified snapshot publishes nothing.
                if let Some(sender) = sender {
                    sender.send_replace(trusted_preload_source(result.as_ref().ok(), api_service, port));
                }
                result
            }));
        }
        // An unverified per-page fallback must not race the browser preloader
        // and install a disabled-feature module into the first document.
        if !initial_snapshot_ready { continue; }
        let Some(profile) = current.as_ref().filter(|profile| profile.active() || api_service) else {
            continue;
        };
        let allow_blank = preload_first
            || (cfg!(target_os = "windows")
                && startup.is_some()
                && profile.native_fast
                && profile.api_key_auth_verified
                && !profile.quota_compat);
        let Some(available) = targets(&client, port, allow_blank).await else {
            continue;
        };
        let ended: Vec<_> = children
            .iter()
            .filter(|(key, child)| {
                child.task.is_finished() || !available.iter().any(|socket| key.ends_with(socket))
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in ended {
            if let Some(child) = children.remove(&key) {
                child.stop();
            }
            retry_at.insert(key, Instant::now() + RETRY);
        }
        for socket in available {
            let key = format!("controls:{socket}");
            if children.contains_key(&key)
                || retry_at.get(&key).is_some_and(|at| *at > Instant::now())
            {
                continue;
            }
            let (child_stop, receiver) = watch::channel(false);
            let profile = profile.clone();
            let instance = instance.clone();
            let startup = current_verified.then(|| startup.clone()).flatten();
            let child_preload_first = preload_first && current_verified;
            let draft_profile = profile_dir.clone();
            let task = tokio::spawn(async move {
                let result = fast::run(
                    socket,
                    draft_profile,
                    profile.native_fast,
                    profile.quota_compat,
                    profile.api_key_auth_verified,
                    api_service,
                    child_preload_first,
                    startup,
                    receiver,
                )
                .await;
                if result.is_err() {
                    logger::log_warn(&format!("[Codex enhancements] setup_failed: instance={instance}, component=controls"));
                }
            });
            children.insert(key, Child { stop: child_stop, task });
        }
    }
    for (_, child) in children {
        child.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn enhancement_scope_requires_actual_auth_and_separate_api_route() {
        let mut cfg = config::UserConfig::default();
        cfg.codex_app_native_fast_enabled = true;
        cfg.codex_app_api_quota_compat_enabled = true;
        let doc: toml_edit::Document = "model_provider='relay'\n[model_providers.relay]\nbase_url='http://127.0.0.1:8000/v1'\nexperimental_bearer_token='local-key'\nrequires_openai_auth=true\n".parse().unwrap();
        let oauth = json!({"tokens":{"id_token":"id", "access_token":"access"}});
        let profile = profile_from_values(&doc, &oauth, &cfg);
        assert!(!profile.native_fast);
        assert!(!profile.api_key_auth_verified);
        assert!(profile.quota_compat);
        let api = json!({"auth_mode":"apikey","OPENAI_API_KEY":"api-key"});
        let profile = profile_from_values(&doc, &api, &cfg);
        assert!(profile.native_fast);
        assert!(profile.api_key_auth_verified);
        assert!(!profile.quota_compat);
        assert!(!profile_from_values(&toml_edit::Document::new(), &oauth, &cfg).quota_compat);
        let same_token: toml_edit::Document = doc
            .to_string()
            .replace("local-key", "access")
            .parse()
            .unwrap();
        assert!(!profile_from_values(&same_token, &oauth, &cfg).quota_compat);
    }

    #[test]
    fn pure_api_verification_uses_profile_evidence_and_oauth_takes_precedence() {
        let cfg = config::UserConfig::default();
        let blank = toml_edit::Document::new();
        assert!(!profile_from_values(&blank, &Value::Null, &cfg).api_key_auth_verified);
        assert!(
            !profile_from_values(&blank, &json!({"OPENAI_API_KEY":"  "}), &cfg)
                .api_key_auth_verified
        );
        assert!(
            profile_from_values(&blank, &json!({"OPENAI_API_KEY":"api-key"}), &cfg)
                .api_key_auth_verified
        );
        let no_oauth: toml_edit::Document =
            "model_provider='local'\n[model_providers.local]\nrequires_openai_auth=false\n"
                .parse()
                .unwrap();
        assert!(profile_from_values(&no_oauth, &Value::Null, &cfg).api_key_auth_verified);
        let bearer: toml_edit::Document = "model_provider='relay'\n[model_providers.relay]\nexperimental_bearer_token='provider-key'\n".parse().unwrap();
        let profile = profile_from_values(&bearer, &Value::Null, &cfg);
        assert!(profile.api_key_auth_verified);
        assert!(profile.native_fast);
        let oauth =
            json!({"OPENAI_API_KEY":"api-key","tokens":{"id_token":"id","access_token":"access"}});
        let profile = profile_from_values(&bearer, &oauth, &cfg);
        assert!(!profile.api_key_auth_verified);
        assert!(!profile.native_fast);
    }

    #[test]
    fn cdp_target_must_stay_on_the_launched_instances_loopback_port() {
        assert!(safe_socket("ws://127.0.0.1:9333/devtools/page/one", 9333));
        for url in [
            "ws://127.0.0.1:9334/devtools/page/one",
            "ws://evil.test:9333/devtools/page/one",
            "ws://user@127.0.0.1:9333/devtools/page/one",
            "ws://127.0.0.1:9333/devtools/browser/one",
        ] {
            assert!(!safe_socket(url, 9333));
        }
    }

    #[test]
    fn quota_route_does_not_accept_embedded_credentials_or_url_queries() {
        let mut cfg = config::UserConfig::default();
        cfg.codex_app_api_quota_compat_enabled = true;
        let auth = json!({"tokens":{"id_token":"id", "access_token":"access"}});
        for base in [
            "https://user:secret@example.test/v1",
            "https://example.test/v1?key=secret",
            "file:///tmp/audio",
        ] {
            let doc: toml_edit::Document = format!("model_provider='relay'\n[model_providers.relay]\nbase_url='{base}'\nexperimental_bearer_token='api-key'\nrequires_openai_auth=true\n").parse().unwrap();
            assert!(!profile_from_values(&doc, &auth, &cfg).quota_compat);
        }
    }
    #[test]
    fn api_service_banner_scope_survives_missing_or_stalled_profile_reads() {
        let safe = profile_or_api_fallback(None, true).unwrap();
        assert!(!safe.active());
        assert!(!safe.api_key_auth_verified);
        assert!(profile_or_api_fallback(None, false).is_none());
        let configured = Profile { native_fast: true, ..Profile::default() };
        assert!(profile_or_api_fallback(Some(configured), true).unwrap().native_fast);
    }

    #[test]
    fn browser_preload_requires_completed_profile_evidence_even_for_api_service() {
        assert!(trusted_preload_source(None, true, 9333).is_none());
        assert!(trusted_preload_source(None, false, 9333).is_none());
        assert!(trusted_preload_source(Some(&Profile::default()), false, 9333).is_none());
        let verified = Profile { native_fast:true, api_key_auth_verified:true, quota_compat:false };
        let script = trusted_preload_source(Some(&verified), true, 9333).unwrap();
        assert!(script.contains("\"nativeFast\":true"));
        assert!(script.contains("\"apiKeyAuthVerified\":true"));
        assert!(script.contains("\"quotaCompat\":false"));
        assert!(script.contains("\"apiService\":true"));
        assert!(script.contains("__cockpitStartupPreloadStarted"));
        assert!(script.contains("document-already-started"));
    }

    #[test]
    fn browser_preload_platform_policy_preserves_mac_guarded_fallback_and_windows_no_reload() {
        for (platform, expected_fresh) in [
            ("windows", (true, true)),
            ("macos", (true, false)),
            ("linux", (false, false)),
        ] {
            assert_eq!(startup_preload_policy(platform, true), expected_fresh, "{platform}");
            assert_eq!(startup_preload_policy(platform, false), (false, false),
                "reattaching an existing {platform} process is never a fresh launch");
        }
    }

}
