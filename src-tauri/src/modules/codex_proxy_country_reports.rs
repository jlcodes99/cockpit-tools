// Live country evidence; never log proxy URLs, credentials or IP addresses.
use sha2::{Digest, Sha256};
use std::sync::LazyLock;
use tauri::Emitter;
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CountryCheck {
    pub check_id: String,
    pub account_id: String,
    pub scope: String,
    pub expected_country: Option<String>,
    pub actual_country: Option<String>,
    pub status: String,
    pub checked_at: i64,
    pub elapsed_ms: u64,
    pub source: String,
    pub reason: Option<String>,
    pub stale: bool,
}
#[derive(Clone)]
struct RecordedCheck {
    result: CountryCheck,
    home: PathBuf,
    signature: String,
}
static CHECKS: LazyLock<Mutex<BTreeMap<String, Vec<RecordedCheck>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));
pub(crate) fn invalidate_report(account: &str) {
    if let Ok(key) = policy_key(account) {
        if let Ok(mut checks) = CHECKS.lock() {
            checks.remove(&key);
        }
    }
}
fn configuration_signature(account: &str, home: &Path) -> Result<String, String> {
    let data = super::account::get_data_dir().map_err(|_| STORAGE)?;
    let paths = vec![
        home.join(".env"),
        data.join("codex-proxy-env.json"),
        data.join("codex-proxy-country.json"),
        data.join("codex-proxy-sources.json"),
        data.join("codex-client-proxies.json"),
    ];
    let mut digest = Sha256::new();
    digest.update(
        serde_json::to_vec(&super::config::get_codex_proxy_direct_hosts()).map_err(|_| STORAGE)?,
    );
    for path in paths {
        digest.update(path.to_string_lossy().as_bytes());
        match std::fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => digest.update([0]),
            Ok(m) if m.is_file() && !m.file_type().is_symlink() && m.len() <= 4 * 1024 * 1024 => {
                digest.update(std::fs::read(path).map_err(|_| STORAGE)?);
            }
            _ => return Err(STORAGE.into()),
        }
    }
    if let Some(account) = super::codex_account::load_account(account) {
        digest.update(
            serde_json::to_vec(&(
                account.egress_proxy_url,
                account.egress_proxy_disabled,
                account.egress_proxy_client_defaults,
            ))
            .map_err(|_| STORAGE)?,
        );
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn record_check(
    app: Option<&tauri::AppHandle>,
    result: &CountryCheck,
    home: &Path,
    signature: &str,
) {
    let account_hash = format!("{:x}", Sha256::digest(result.account_id.as_bytes()));
    let line = format!("[Codex Country Check] check_id={} account={} scope={} status={} source={} expected={} actual={} elapsed_ms={} reason={}",
        result.check_id, &account_hash[..12], result.scope, result.status, result.source,
        result.expected_country.as_deref().unwrap_or("-"), result.actual_country.as_deref().unwrap_or("-"),
        result.elapsed_ms, result.reason.as_deref().unwrap_or("-"));
    if result.status == "blocked" {
        super::logger::log_warn(&line);
    } else {
        super::logger::log_info(&line);
    }
    if let Ok(mut checks) = CHECKS.lock() {
        if checks.len() >= 128 && !checks.contains_key(&result.account_id) {
            checks.clear();
        }
        let records = checks.entry(result.account_id.clone()).or_default();
        if let Some(item) = records
            .iter_mut()
            .find(|item| item.result.check_id == result.check_id)
        {
            *item = RecordedCheck {
                result: result.clone(),
                home: home.into(),
                signature: signature.into(),
            };
        } else {
            records.push(RecordedCheck {
                result: result.clone(),
                home: home.into(),
                signature: signature.into(),
            });
            if records.len() > 12 {
                records.remove(0);
            }
        }
    }
    if let Some(app) = app {
        let _ = app.emit("codex:country-check", result);
    }
}
struct CountryCheckGuard {
    app: Option<tauri::AppHandle>,
    result: CountryCheck,
    home: PathBuf,
    signature: String,
    started: std::time::Instant,
    completed: bool,
}
impl Drop for CountryCheckGuard {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        self.result.status = "blocked".into();
        self.result.reason = Some(UNKNOWN.into());
        self.result.elapsed_ms = self.started.elapsed().as_millis().min(u64::MAX as u128) as u64;
        record_check(self.app.as_ref(), &self.result, &self.home, &self.signature);
    }
}
pub fn recent_checks(account: &str) -> Result<Vec<CountryCheck>, String> {
    let key = policy_key(account)?;
    let items = CHECKS
        .lock()
        .map_err(|_| STORAGE)?
        .get(&key)
        .cloned()
        .unwrap_or_default();
    Ok(items
        .into_iter()
        .map(|item| {
            let mut result = item.result;
            result.stale = configuration_signature(&key, &item.home)
                .map_or(true, |current| current != item.signature);
            result
        })
        .collect())
}
async fn run_check(
    app: Option<&tauri::AppHandle>,
    bound: Option<&str>,
    home: &Path,
    scope: &str,
    prepared_proxy: Option<&CodexLaunchProxy>,
) -> Result<Option<CountryCheck>, String> {
    if !cfg!(windows) {
        return Ok(None);
    }
    let account = policy_key(bound.unwrap_or("__client__"))?;
    let expected = policy(&account)?.expected_country;
    if expected.is_none() && scope == "launch" {
        super::logger::log_info("[Codex Country Check] status=skipped reason=disabled");
        return Ok(None);
    }
    let signature = configuration_signature(&account, home)?;
    let started = std::time::Instant::now();
    let mut result = CountryCheck {
        check_id: uuid::Uuid::new_v4().to_string(),
        account_id: account.clone(),
        scope: scope.into(),
        expected_country: expected.clone(),
        actual_country: None,
        status: "checking".into(),
        checked_at: chrono::Utc::now().timestamp_millis(),
        elapsed_ms: 0,
        source: "unknown".into(),
        reason: None,
        stale: false,
    };
    record_check(app, &result, home, &signature);
    let mut guard = CountryCheckGuard {
        app: app.cloned(),
        result: result.clone(),
        home: home.into(),
        signature: signature.clone(),
        started,
        completed: false,
    };
    let checked = async {
        let resolved;
        let proxy = match prepared_proxy {
            Some(proxy) => proxy,
            None => {
                resolved =
                    super::codex_instance::resolve_egress_proxy_for_bind_account(bound).await?;
                &resolved
            }
        };
        let (route, bypass, source) = route_for_check(&account, home, proxy)?;
        result.source = source.into();
        guard.result = result.clone();
        let client = probe_client(route.as_deref(), bypass.as_deref())?;
        let actual =
            tokio::time::timeout(Duration::from_secs(15), query_country(&client, ENDPOINT))
                .await
                .map_err(|_| UNKNOWN)??;
        result.actual_country = Some(actual.clone());
        if policy(&account)?.expected_country != expected
            || configuration_signature(&account, home)? != signature
        {
            return Err("PROXY_COUNTRY_CHANGED".into());
        }
        if let Some(expected) = expected.as_deref() {
            compare_country(expected, &actual)?;
        }
        Ok::<(), String>(())
    }
    .await;
    result.elapsed_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    result.status = if checked.is_ok() { "passed" } else { "blocked" }.into();
    result.reason = checked.as_ref().err().map(|error| {
        if error.starts_with("PROXY_COUNTRY_")
            && error.len() < 96
            && error
                .bytes()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_' || c == b':')
        {
            error.clone()
        } else {
            "PROXY_COUNTRY_ROUTE_UNKNOWN".into()
        }
    });
    guard.completed = true;
    record_check(app, &result, home, &signature);
    checked.map(|_| Some(result))
}
pub async fn check_now(
    app: &tauri::AppHandle,
    account: &str,
    home: &Path,
) -> Result<CountryCheck, String> {
    run_check(Some(app), Some(account), home, "manual", None)
        .await?
        .ok_or_else(|| UNKNOWN.into())
}
