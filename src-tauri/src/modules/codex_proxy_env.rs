//! Account-specific proxy assignments, staged outside CODEX_HOME until launch.
use super::{atomic_write, codex_proxy_dotenv, codex_proxy_launch_guard, secure_account_storage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path, sync::Mutex};
const STORAGE: &str = "PROXY_ENV_STORAGE";
const INVALID: &str = "PROXY_ENV_INVALID";
static WRITES: Mutex<()> = Mutex::new(());
pub const NAMES: [&str; 4] = ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY"];

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvView {
    pub document: String,
    pub source: String,
    pub revision: String,
    pub recognition_failed: bool,
}
#[derive(Default, Serialize, Deserialize)]
struct Store {
    accounts: BTreeMap<String, String>,
}
fn path() -> Result<std::path::PathBuf, String> {
    Ok(super::account::get_data_dir()
        .map_err(|_| STORAGE)?
        .join("codex-proxy-env.json"))
}
fn read() -> Result<Store, String> {
    let path = path()?;
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Store::default()),
        Ok(m) if m.is_file() && !m.file_type().is_symlink() && m.len() <= 4 * 1024 * 1024 => (),
        _ => return Err(STORAGE.into()),
    }
    let raw = std::fs::read_to_string(path).map_err(|_| STORAGE)?;
    let store: Store =
        secure_account_storage::deserialize_encrypted_snapshot(&raw).map_err(|_| STORAGE)?;
    if store.accounts.len() > 128 || store.accounts.values().any(|s| s.len() > 16384) {
        return Err(STORAGE.into());
    }
    Ok(store)
}
pub(crate) fn saved_document(account: &str) -> Result<Option<String>, String> {
    let _lock = WRITES.lock().map_err(|_| STORAGE)?;
    Ok(read()?
        .accounts
        .get(&codex_proxy_launch_guard::policy_key(account)?)
        .cloned())
}
fn revision(value: &Option<String>) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).unwrap_or_default())
    )
}
pub(crate) fn format_variables(variables: &BTreeMap<String, String>) -> String {
    NAMES
        .iter()
        .filter_map(|name| {
            variables
                .get(&name.to_ascii_lowercase())
                .map(|value| format!("{name}='{value}'\n"))
        })
        .collect()
}
pub(crate) fn normalize_document(document: &str) -> Result<String, String> {
    if document.len() > 16384 {
        return Err(INVALID.into());
    }
    let mut seen = std::collections::BTreeSet::new();
    for line in document.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let name = line
            .strip_prefix("export ")
            .unwrap_or(line)
            .split_once('=')
            .ok_or(INVALID)?
            .0
            .trim()
            .to_ascii_uppercase();
        if !NAMES.contains(&name.as_str()) || !seen.insert(name) {
            return Err(INVALID.into());
        }
    }
    let variables = codex_proxy_dotenv::parse_proxy_variables(document).map_err(|_| INVALID)?;
    for (name, value) in &variables {
        if value.len() > 4096 || value.contains(['\'', '"']) {
            return Err(INVALID.into());
        }
        if name == "no_proxy" || value.is_empty() {
            continue;
        }
        let url = url::Url::parse(value).map_err(|_| INVALID)?;
        if !matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h")
            || url.host_str().is_none()
            || !matches!(url.path(), "" | "/")
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(INVALID.into());
        }
    }
    Ok(format_variables(&variables))
}
pub(crate) fn save(account: &str, document: &str, previous_revision: &str) -> Result<(), String> {
    let document = normalize_document(document)?;
    {
        let _lock = WRITES.lock().map_err(|_| STORAGE)?;
        let mut store = read()?;
        let key = codex_proxy_launch_guard::policy_key(account)?;
        if revision(&store.accounts.get(&key).cloned()) != previous_revision {
            return Err("PROXY_ENV_CHANGED".into());
        }
        if store.accounts.len() >= 128 && !store.accounts.contains_key(&key) {
            return Err(STORAGE.into());
        }
        store.accounts.insert(key, document);
        let raw = secure_account_storage::serialize_account_file("codex-proxy-env", &store)
            .map_err(|_| STORAGE)?;
        atomic_write::write_secret_string_atomic(&path()?, &raw).map_err(|_| STORAGE)?;
    }
    codex_proxy_launch_guard::invalidate_report(account);
    Ok(())
}
pub(crate) fn view(account: &str, home: &Path) -> Result<EnvView, String> {
    let saved = saved_document(account)?;
    let rev = revision(&saved);
    if let Some(document) = saved {
        let document = normalize_document(&document).map_err(|_| STORAGE)?;
        return Ok(EnvView {
            document,
            source: "saved".into(),
            revision: rev,
            recognition_failed: false,
        });
    }
    let original = match codex_proxy_dotenv::preview_proxy_variables(
        home,
        account,
        &super::process::CodexLaunchProxy::ClientDefaults,
    ) {
        Ok(original) => original,
        Err(_) => {
            return Ok(EnvView {
                document: String::new(),
                source: "unknown".into(),
                revision: rev,
                recognition_failed: true,
            })
        }
    };
    if !original.is_empty() {
        return Ok(EnvView {
            document: format_variables(&original),
            source: "env".into(),
            revision: rev,
            recognition_failed: false,
        });
    }
    let environment = super::process::proxy_environment_for_check(
        &super::process::CodexLaunchProxy::ClientDefaults,
    );
    let (document, source) = if environment.values().any(|v| !v.is_empty()) {
        (format_variables(&environment), "environment")
    } else if let Some(url) =
        super::codex_local_access::system_proxy_url_for_target("https://chatgpt.com/cdn-cgi/trace")
    {
        (
            format!("HTTP_PROXY='{url}'\nHTTPS_PROXY='{url}'\n"),
            "system",
        )
    } else {
        (String::new(), "none")
    };
    Ok(EnvView {
        document,
        source: source.into(),
        revision: rev,
        recognition_failed: false,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proxy_env_accepts_four_proxy_keys_and_rejects_unrelated_or_ambiguous_content() {
        let doc = normalize_document("http_proxy=http://127.0.0.1:7890\nHTTPS_PROXY='socks5h://127.0.0.1:1080'\nALL_PROXY=socks5://127.0.0.1:1080\nNO_PROXY=localhost,127.0.0.1\n").unwrap();
        assert!(doc.contains("HTTP_PROXY='http://127.0.0.1:7890'"));
        assert_eq!(
            codex_proxy_dotenv::parse_proxy_variables(&doc)
                .unwrap()
                .len(),
            4
        );
        for invalid in [
            "OPENAI_API_KEY=secret",
            "HTTPS_PROXY=${OTHER}",
            "HTTP_PROXY=not-a-url",
            "HTTP_PROXY=file:///tmp/proxy",
            "HTTPS_PROXY=http://localhost/?secret=1",
            "HTTP_PROXY=http://a\nhttp_proxy=http://b",
        ] {
            assert_eq!(normalize_document(invalid).unwrap_err(), INVALID);
        }
        assert_eq!(normalize_document("# optional proxy keys\n").unwrap(), "");
    }
    #[test]
    fn saved_proxy_env_is_encrypted_staged_per_account_and_revision_checked() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!("proxy-env-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("home")).unwrap();
        let keys = ["COCKPIT_TOOLS_TEST_DATA_DIR", "COCKPIT_TOOLS_DATA_DIR"];
        let previous = keys.map(std::env::var_os);
        struct Cleanup(std::path::PathBuf, [Option<std::ffi::OsString>; 2]);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                for (key, value) in ["COCKPIT_TOOLS_TEST_DATA_DIR", "COCKPIT_TOOLS_DATA_DIR"]
                    .into_iter()
                    .zip(&self.1)
                {
                    match value {
                        Some(value) => std::env::set_var(key, value),
                        None => std::env::remove_var(key),
                    }
                }
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(root.clone(), previous);
        for key in keys {
            std::env::set_var(key, root.join("app"));
        }
        let home = root.join("home");
        let original = "HTTPS_PROXY=http://original.invalid:7890\nOTHER=preserved\n";
        std::fs::write(home.join(".env"), original).unwrap();
        let initial = view("a", &home).unwrap();
        save(
            "a",
            "HTTP_PROXY=http://account-a.invalid:7890\nHTTPS_PROXY=http://account-a.invalid:7890",
            &initial.revision,
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(home.join(".env")).unwrap(),
            original
        );
        assert_eq!(view("a", &home).unwrap().source, "saved");
        assert_eq!(view("b", &home).unwrap().source, "env");
        assert_eq!(
            save("a", "", &initial.revision).unwrap_err(),
            "PROXY_ENV_CHANGED"
        );
        assert!(!std::fs::read_to_string(path().unwrap())
            .unwrap()
            .contains("account-a.invalid"));
        if cfg!(windows) {
            codex_proxy_dotenv::apply(
                &home,
                "a",
                &super::super::process::CodexLaunchProxy::ClientDefaults,
            )
            .unwrap();
            codex_proxy_dotenv::verify(
                &home,
                "a",
                &super::super::process::CodexLaunchProxy::ClientDefaults,
            )
            .unwrap();
            codex_proxy_dotenv::apply(
                &home,
                "b",
                &super::super::process::CodexLaunchProxy::ClientDefaults,
            )
            .unwrap();
            assert_eq!(
                std::fs::read_to_string(home.join(".env")).unwrap(),
                original
            );
        }
    }
}
