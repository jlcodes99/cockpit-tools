use crate::models::codex::CodexAccount;
use crate::models::ssh_server::{
    SshAuthConfig, SshCodexSyncResult, SshCodexSyncStatus, SshServer, SshServerStore,
};
use crate::modules::{account, atomic_write, codex_account, logger};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;
use uuid::Uuid;

const SSH_SERVERS_FILE: &str = "ssh_servers.json";
const STORE_VERSION: &str = "1";
const CONNECTION_TIMEOUT_SECS: u64 = 10;
const SYNC_TIMEOUT_SECS: u64 = 30;
const APP_SERVER_RELOAD_TIMEOUT_SECS: u64 = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshServerList {
    pub selected_server_id: Option<String>,
    pub servers: Vec<SshServer>,
}

fn now_timestamp() -> i64 {
    chrono::Utc::now().timestamp()
}

fn store_path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(SSH_SERVERS_FILE))
}

fn default_codex_home() -> String {
    "~/.codex".to_string()
}

fn contains_control_separator(value: &str) -> bool {
    value.contains('\n') || value.contains('\r') || value.contains('\0')
}

fn normalize_text(value: &str) -> String {
    value.trim().to_string()
}

fn sanitize_error(error: impl ToString) -> String {
    let mut value = error.to_string();
    for marker in [
        "OPENAI_API_KEY",
        "access_token",
        "refresh_token",
        "id_token",
    ] {
        value = value.replace(marker, "[redacted]");
    }
    value
}

fn validate_server(server: &SshServer) -> Result<(), String> {
    if server.name.trim().is_empty() {
        return Err("SSH server name is required".to_string());
    }
    for (label, value) in [
        ("host", server.host.as_str()),
        ("username", server.username.as_str()),
        ("codex_home", server.codex_home.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(format!("SSH server {} is required", label));
        }
        if contains_control_separator(value) {
            return Err(format!(
                "SSH server {} contains unsupported characters",
                label
            ));
        }
    }
    if server.port == 0 {
        return Err("SSH server port must be between 1 and 65535".to_string());
    }
    match &server.auth {
        SshAuthConfig::Agent => {}
        SshAuthConfig::PrivateKeyFile { path } => {
            if path.trim().is_empty() {
                return Err("SSH private key path is required".to_string());
            }
            if contains_control_separator(path) {
                return Err("SSH private key path contains unsupported characters".to_string());
            }
        }
    }
    Ok(())
}

fn normalize_server(
    mut server: SshServer,
    existing: Option<&SshServer>,
) -> Result<SshServer, String> {
    let now = now_timestamp();
    if server.id.trim().is_empty() {
        server.id = Uuid::new_v4().to_string();
    } else {
        server.id = normalize_text(&server.id);
    }
    server.name = normalize_text(&server.name);
    server.host = normalize_text(&server.host);
    server.username = normalize_text(&server.username);
    server.codex_home = normalize_text(&server.codex_home);
    if server.codex_home.is_empty() {
        server.codex_home = default_codex_home();
    }
    if server.port == 0 {
        server.port = 22;
    }
    if server.created_at <= 0 {
        server.created_at = existing.map(|item| item.created_at).unwrap_or(now);
    }
    server.updated_at = now;
    if let Some(existing) = existing {
        if server.last_sync.is_none() {
            server.last_sync = existing.last_sync.clone();
        }
    }
    validate_server(&server)?;
    Ok(server)
}

pub fn load_store() -> Result<SshServerStore, String> {
    let path = store_path()?;
    if !path.exists() {
        return Ok(SshServerStore::default());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read SSH servers store: {}", e))?;
    let mut store: SshServerStore = atomic_write::parse_json_with_auto_restore(&path, &content)
        .map_err(|e| format!("Failed to parse SSH servers store: {}", e))?;
    if store.version.trim().is_empty() {
        store.version = STORE_VERSION.to_string();
    }
    if let Some(selected_id) = store.selected_server_id.clone() {
        if !store.servers.iter().any(|server| server.id == selected_id) {
            store.selected_server_id = None;
        }
    }
    Ok(store)
}

fn save_store(store: &SshServerStore) -> Result<(), String> {
    let path = store_path()?;
    let content = serde_json::to_string_pretty(store)
        .map_err(|e| format!("Failed to serialize SSH servers store: {}", e))?;
    atomic_write::write_string_atomic(&path, &content)
}

pub fn list_servers() -> Result<SshServerList, String> {
    let store = load_store()?;
    Ok(SshServerList {
        selected_server_id: store.selected_server_id,
        servers: store.servers,
    })
}

pub fn upsert_server(server: SshServer) -> Result<SshServerList, String> {
    let mut store = load_store()?;
    store.version = STORE_VERSION.to_string();
    let existing_index = store.servers.iter().position(|item| item.id == server.id);
    let existing = existing_index.and_then(|index| store.servers.get(index));
    let server = normalize_server(server, existing)?;
    if let Some(index) = existing_index {
        store.servers[index] = server;
    } else {
        store.servers.push(server);
    }
    save_store(&store)?;
    list_servers()
}

pub fn delete_server(server_id: &str) -> Result<SshServerList, String> {
    let mut store = load_store()?;
    let server_id = server_id.trim();
    store.servers.retain(|server| server.id != server_id);
    if store.selected_server_id.as_deref() == Some(server_id) {
        store.selected_server_id = None;
    }
    save_store(&store)?;
    list_servers()
}

pub fn select_server(server_id: Option<String>) -> Result<SshServerList, String> {
    let mut store = load_store()?;
    let selected = server_id.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });
    if let Some(selected_id) = selected.as_deref() {
        if !store.servers.iter().any(|server| server.id == selected_id) {
            return Err(format!("SSH server not found: {}", selected_id));
        }
    }
    store.selected_server_id = selected;
    save_store(&store)?;
    list_servers()
}

fn selected_server_from_store(store: &SshServerStore) -> Option<SshServer> {
    let selected_id = store.selected_server_id.as_deref()?;
    store
        .servers
        .iter()
        .find(|server| server.id == selected_id)
        .cloned()
}

fn build_ssh_args(server: &SshServer, connect_timeout_secs: u64) -> Vec<String> {
    let mut args = vec![
        "-p".to_string(),
        server.port.to_string(),
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        format!("ConnectTimeout={}", connect_timeout_secs),
    ];
    if let SshAuthConfig::PrivateKeyFile { path } = &server.auth {
        args.push("-i".to_string());
        args.push(path.clone());
    }
    args.push(format!("{}@{}", server.username, server.host));
    args
}

async fn run_ssh(
    server: &SshServer,
    timeout_secs: u64,
    remote_args: &[&str],
    stdin_payload: Option<String>,
) -> Result<String, String> {
    let mut command = Command::new("ssh");
    command.args(build_ssh_args(server, timeout_secs));
    command.args(remote_args);
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    if stdin_payload.is_some() {
        command.stdin(Stdio::piped());
    }

    let mut child = command
        .spawn()
        .map_err(|e| format!("ssh_binary_missing: {}", e))?;
    if let Some(payload) = stdin_payload {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "ssh_connection_failed: stdin unavailable".to_string())?;
        stdin
            .write_all(payload.as_bytes())
            .await
            .map_err(|e| format!("ssh_connection_failed: {}", e))?;
    }

    let output = timeout(Duration::from_secs(timeout_secs), child.wait_with_output())
        .await
        .map_err(|_| "ssh_connection_failed: SSH command timed out".to_string())?
        .map_err(|e| format!("ssh_connection_failed: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let category = if stderr.to_ascii_lowercase().contains("permission denied") {
            "ssh_auth_failed"
        } else {
            "ssh_connection_failed"
        };
        return Err(format!(
            "{}: {}",
            category,
            sanitize_error(if stderr.is_empty() {
                format!("exit status {}", output.status)
            } else {
                stderr
            })
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub async fn test_connection(server_id: &str) -> Result<String, String> {
    let store = load_store()?;
    let server = store
        .servers
        .iter()
        .find(|server| server.id == server_id)
        .cloned()
        .ok_or_else(|| format!("SSH server not found: {}", server_id))?;
    let output = run_ssh(
        &server,
        CONNECTION_TIMEOUT_SECS,
        &["printf", "cockpit-tools-ssh-ok"],
        None,
    )
    .await?;
    if output.trim() == "cockpit-tools-ssh-ok" {
        Ok(output)
    } else {
        Err("ssh_connection_failed: unexpected SSH test output".to_string())
    }
}

async fn read_remote_config_toml(server: &SshServer) -> Result<Option<String>, String> {
    let script = r#"set -eu
codex_home=$1
case "$codex_home" in
  "~") codex_home="$HOME" ;;
  "~/"*) codex_home="$HOME/${{codex_home#~/}}" ;;
esac
target="$codex_home/config.toml"
if [ -f "$target" ]; then
  printf '__COCKPIT_EXISTS__\n'
  cat "$target"
elif [ -e "$target" ]; then
  printf 'config.toml is not a regular file\n' >&2
  exit 3
else
  printf '__COCKPIT_MISSING__\n'
fi
"#;
    let output = run_ssh(
        server,
        SYNC_TIMEOUT_SECS,
        &["sh", "-s", "--", &server.codex_home],
        Some(script.to_string()),
    )
    .await
    .map_err(|e| format!("ssh_remote_read_failed: {}", sanitize_error(e)))?;
    if let Some(rest) = output.strip_prefix("__COCKPIT_EXISTS__\n") {
        return Ok(Some(rest.to_string()));
    }
    if output.trim() == "__COCKPIT_MISSING__" {
        return Ok(None);
    }
    Err("ssh_remote_read_failed: unexpected remote read response".to_string())
}

async fn upload_and_verify_bundle(
    server: &SshServer,
    bundle: &codex_account::CodexAccountProjectionBundle,
) -> Result<(), String> {
    let mut payload = String::new();
    for file in &bundle.files {
        payload.push_str(&format!(
            "{}\t{:o}\t{}\t{}\n",
            file.relative_path,
            file.mode,
            file.sha256,
            STANDARD.encode(file.content.as_bytes())
        ));
    }
    let script = format!(
        r#"set -eu
codex_home=$1
case "$codex_home" in
  "~") codex_home="$HOME" ;;
  "~/"*) codex_home="$HOME/${{codex_home#~/}}" ;;
esac
mkdir -p "$codex_home"
chmod 700 "$codex_home" 2>/dev/null || true
tmp_dir="$codex_home/.cockpit-codex-sync.$$"
rm -rf "$tmp_dir"
mkdir -p "$tmp_dir"
cleanup() {{ rm -rf "$tmp_dir"; }}
trap cleanup EXIT INT TERM
cat <<'__COCKPIT_CODEX_PAYLOAD__' | while IFS='	' read -r rel mode expected encoded; do
{payload}__COCKPIT_CODEX_PAYLOAD__
  [ -n "$rel" ] || continue
  case "$rel" in
    auth.json|config.toml|.cockpit_codex_auth.json) ;;
    *) printf 'invalid relative path: %s\n' "$rel" >&2; exit 4 ;;
  esac
  tmp="$tmp_dir/$rel"
  target="$codex_home/$rel"
  if ! printf '%s' "$encoded" | base64 -d > "$tmp" 2>/dev/null; then
    printf '%s' "$encoded" | base64 -D > "$tmp"
  fi
  chmod "$mode" "$tmp" 2>/dev/null || true
  mv "$tmp" "$target"
  chmod "$mode" "$target" 2>/dev/null || true
  actual="$(sha256sum "$target" 2>/dev/null | awk '{{print $1}}' || shasum -a 256 "$target" | awk '{{print $1}}')"
  if [ "$actual" != "$expected" ]; then
    printf 'hash mismatch for %s\n' "$rel" >&2
    exit 5
  fi
  printf '%s\t%s\n' "$rel" "$actual"
done
"#
    );
    let output = run_ssh(
        server,
        SYNC_TIMEOUT_SECS,
        &["sh", "-s", "--", &server.codex_home],
        Some(script),
    )
    .await
    .map_err(|e| format!("ssh_remote_write_failed: {}", sanitize_error(e)))?;

    for file in &bundle.files {
        let verified = output
            .lines()
            .any(|line| line == format!("{}\t{}", file.relative_path, file.sha256));
        if !verified {
            return Err(format!(
                "ssh_remote_verify_failed: missing verification for {}",
                file.relative_path
            ));
        }
    }
    Ok(())
}

fn reload_app_server_script() -> &'static str {
    r#"set -eu
if command -v codex >/dev/null 2>&1 && codex app-server daemon restart >/dev/null 2>&1; then
  printf 'daemon-restarted\n'
  exit 0
fi

pids="$(ps -u "$(id -u)" -o pid= -o args= | awk '
/codex app-server --listen/ || /codex app-server proxy/ { print $1 }
')"
if [ -z "$pids" ]; then
  printf 'no-app-server\n'
  exit 0
fi

kill -TERM $pids 2>/dev/null || true
sleep 1
for pid in $pids; do
  if kill -0 "$pid" 2>/dev/null; then
    kill -KILL "$pid" 2>/dev/null || true
  fi
done
printf 'app-server-terminated\n'
"#
}

async fn reload_remote_codex_app_server(server: &SshServer) -> Result<(), String> {
    let output = run_ssh(
        server,
        APP_SERVER_RELOAD_TIMEOUT_SECS,
        &["sh", "-s"],
        Some(reload_app_server_script().to_string()),
    )
    .await
    .map_err(|e| format!("ssh_remote_app_server_reload_failed: {}", sanitize_error(e)))?;

    let status = output.trim();
    if matches!(
        status,
        "daemon-restarted" | "app-server-terminated" | "no-app-server"
    ) {
        Ok(())
    } else {
        Err(format!(
            "ssh_remote_app_server_reload_failed: unexpected reload response: {}",
            sanitize_error(status)
        ))
    }
}

fn result_from_status(server: &SshServer, status: SshCodexSyncStatus) -> SshCodexSyncResult {
    SshCodexSyncResult {
        server_id: server.id.clone(),
        server_name: server.name.clone(),
        account_id: status.account_id,
        account_email: status.account_email,
        token_generation: status.token_generation,
        bundle_hash: status.bundle_hash,
        verified: status.verified,
        error: status.error,
        synced_at: status.synced_at,
    }
}

fn persist_sync_status(
    server_id: &str,
    status: SshCodexSyncStatus,
) -> Result<SshCodexSyncResult, String> {
    let mut store = load_store()?;
    let index = store
        .servers
        .iter()
        .position(|server| server.id == server_id)
        .ok_or_else(|| format!("SSH server not found: {}", server_id))?;
    store.servers[index].last_sync = Some(status.clone());
    store.servers[index].updated_at = now_timestamp();
    let result = result_from_status(&store.servers[index], status);
    save_store(&store)?;
    Ok(result)
}

async fn sync_account_to_server(server: SshServer, account: &CodexAccount) -> SshCodexSyncResult {
    let synced_at = now_timestamp();
    let sync_attempt = async {
        validate_server(&server)?;
        let existing_config = read_remote_config_toml(&server).await?;
        let bundle =
            codex_account::build_projection_bundle_for_remote(account, existing_config.as_deref())
                .map_err(|e| format!("codex_bundle_failed: {}", sanitize_error(e)))?;
        upload_and_verify_bundle(&server, &bundle).await?;
        reload_remote_codex_app_server(&server).await?;
        Ok::<_, String>(bundle)
    }
    .await;

    let status = match sync_attempt {
        Ok(bundle) => SshCodexSyncStatus {
            account_id: bundle.account_id,
            account_email: bundle.account_email,
            token_generation: bundle.token_generation,
            bundle_hash: bundle.bundle_hash,
            synced_at,
            verified: true,
            error: None,
        },
        Err(error) => SshCodexSyncStatus {
            account_id: account.id.clone(),
            account_email: account.email.clone(),
            token_generation: account.token_generation,
            bundle_hash: String::new(),
            synced_at,
            verified: false,
            error: Some(sanitize_error(error)),
        },
    };

    match persist_sync_status(&server.id, status.clone()) {
        Ok(result) => result,
        Err(error) => {
            logger::log_warn(&format!(
                "[Codex SSH] 保存同步状态失败: server_id={}, error={}",
                server.id, error
            ));
            result_from_status(&server, status)
        }
    }
}

pub async fn sync_current_account_to_server(
    server_id: Option<String>,
) -> Result<SshCodexSyncResult, String> {
    let account = codex_account::get_current_account()
        .ok_or_else(|| "codex_bundle_failed: no current Codex account".to_string())?;
    let store = load_store()?;
    let server = if let Some(server_id) = server_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        store
            .servers
            .iter()
            .find(|server| server.id == server_id)
            .cloned()
            .ok_or_else(|| format!("SSH server not found: {}", server_id))?
    } else {
        selected_server_from_store(&store)
            .ok_or_else(|| "ssh_not_configured: no selected SSH server".to_string())?
    };
    Ok(sync_account_to_server(server, &account).await)
}

pub async fn sync_selected_server_after_codex_switch(
    account: &CodexAccount,
) -> Option<SshCodexSyncResult> {
    let store = match load_store() {
        Ok(store) => store,
        Err(error) => {
            logger::log_warn(&format!("[Codex SSH] 读取 SSH 服务器配置失败: {}", error));
            return None;
        }
    };
    let Some(server) = selected_server_from_store(&store) else {
        return None;
    };
    if !server.sync_on_codex_switch {
        return None;
    }
    Some(sync_account_to_server(server, account).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct StoreBackup {
        path: PathBuf,
        original: Option<Vec<u8>>,
    }

    impl StoreBackup {
        fn capture() -> Self {
            let path = store_path().expect("resolve ssh server store path");
            let original = std::fs::read(&path).ok();
            Self { path, original }
        }
    }

    impl Drop for StoreBackup {
        fn drop(&mut self) {
            if let Some(original) = self.original.as_ref() {
                if let Some(parent) = self.path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&self.path, original);
            } else if self.path.exists() {
                let _ = std::fs::remove_file(&self.path);
            }
        }
    }

    fn valid_server() -> SshServer {
        SshServer {
            id: "server-1".to_string(),
            name: "Dev".to_string(),
            host: "example.com".to_string(),
            port: 22,
            username: "alice".to_string(),
            codex_home: "~/.codex".to_string(),
            auth: SshAuthConfig::Agent,
            sync_on_codex_switch: true,
            created_at: 1,
            updated_at: 1,
            last_sync: None,
        }
    }

    #[test]
    fn validation_rejects_empty_host() {
        let mut server = valid_server();
        server.host.clear();
        assert!(validate_server(&server).is_err());
    }

    #[test]
    fn validation_rejects_private_key_without_path() {
        let mut server = valid_server();
        server.auth = SshAuthConfig::PrivateKeyFile {
            path: String::new(),
        };
        assert!(validate_server(&server).is_err());
    }

    #[test]
    fn ssh_args_include_batch_mode_without_disabling_host_key_checks() {
        let server = valid_server();
        let args = build_ssh_args(&server, 10);
        assert!(args.contains(&"BatchMode=yes".to_string()));
        assert!(!args
            .iter()
            .any(|arg| arg.contains("StrictHostKeyChecking=no")));
    }

    #[test]
    fn app_server_reload_script_restarts_or_terminates_codex_app_server() {
        let script = reload_app_server_script();
        assert!(script.contains("codex app-server daemon restart"));
        assert!(script.contains("codex app-server --listen"));
        assert!(script.contains("codex app-server proxy"));
        assert!(!script.contains("pkill"));
    }

    #[tokio::test]
    #[ignore]
    async fn live_ssh_own_syncs_current_codex_account() {
        if std::env::var("COCKPIT_LIVE_SSH_OWN_SYNC").ok().as_deref() != Some("1") {
            eprintln!("set COCKPIT_LIVE_SSH_OWN_SYNC=1 to run the live own SSH sync test");
            return;
        }

        let current = codex_account::get_current_account()
            .expect("a current Codex account is required for live SSH sync");
        let _backup = StoreBackup::capture();
        let now = now_timestamp();
        let server = SshServer {
            id: "live-ssh-own".to_string(),
            name: "own".to_string(),
            host: "own".to_string(),
            port: 22,
            username: "ubuntu".to_string(),
            codex_home: "~/.codex".to_string(),
            auth: SshAuthConfig::Agent,
            sync_on_codex_switch: true,
            created_at: now,
            updated_at: now,
            last_sync: None,
        };
        let store = SshServerStore {
            version: STORE_VERSION.to_string(),
            selected_server_id: Some(server.id.clone()),
            servers: vec![server.clone()],
        };
        save_store(&store).expect("write live SSH server store");

        test_connection(&server.id)
            .await
            .expect("live SSH connection test should pass");
        let result = sync_current_account_to_server(Some(server.id.clone()))
            .await
            .expect("live SSH sync should return a result");

        assert!(
            result.verified,
            "live SSH sync should verify remote hashes: {:?}",
            result.error
        );
        assert_eq!(result.account_id, current.id);
        assert_eq!(result.account_email, current.email);
        assert_eq!(result.token_generation, current.token_generation);
    }
}
