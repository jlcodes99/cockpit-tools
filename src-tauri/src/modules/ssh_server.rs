use crate::models::codex::CodexAccount;
use crate::models::ssh_server::{
    SshAuthConfig, SshCodexSyncResult, SshCodexSyncStatus, SshServer, SshServerStore,
};
use crate::modules::{account, codex_account, logger};
use base64::{engine::general_purpose::STANDARD, Engine};
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Duration;
use tokio::process::Command;
use uuid::Uuid;

#[path = "ssh_server_store.rs"]
mod store;
#[path = "ssh_server_batch.rs"]
mod batch;
pub use crate::models::ssh_server::SshServerList;

#[path = "ssh_server_process.rs"]
mod process;

pub(crate) use process::output as command_output;

pub(crate) fn has_auto_sync_targets() -> bool {
    load_store().is_ok_and(|state| !store::switch_targets(&state).is_empty())
}

const SSH_SERVERS_FILE: &str = "ssh_servers.json";
/// TCP/SSH 握手超时（传给 OpenSSH ConnectTimeout）
const CONNECTION_TIMEOUT_SECS: u64 = 12;
/// 测连整段命令墙钟超时
const TEST_COMMAND_TIMEOUT_SECS: u64 = 20;
/// 读写同步脚本墙钟超时
const SYNC_TIMEOUT_SECS: u64 = 45;
static ACCOUNT_SYNC_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

fn ensure_current_sync_account(account: &CodexAccount) -> Result<(), String> {
    if codex_account::get_current_account().is_some_and(|current| {
        current.id == account.id && current.token_generation == account.token_generation
    }) {
        Ok(())
    } else {
        Err("ssh_sync_superseded: a newer Codex account is active".into())
    }
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
        value = redact_secret_values(&value, marker);
    }
    value
}

fn redact_secret_values(value: &str, marker: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut remaining = value;
    while let Some(index) = remaining.find(marker) {
        let (before, matched_and_after) = remaining.split_at(index);
        output.push_str(before);
        output.push_str(marker);

        let after_marker = &matched_and_after[marker.len()..];
        let Some((delimiter_end, quote)) = secret_value_start(after_marker) else {
            remaining = after_marker;
            continue;
        };
        output.push_str(&after_marker[..delimiter_end]);

        let value_start = delimiter_end;
        let value_end = secret_value_end(&after_marker[value_start..], quote);
        output.push_str("[redacted]");
        remaining = &after_marker[value_start + value_end..];
    }
    output.push_str(remaining);
    output
}

fn secret_value_start(value: &str) -> Option<(usize, Option<char>)> {
    let mut chars = value.char_indices().peekable();
    let mut end = 0;
    while let Some((index, ch)) = chars.peek().copied() {
        if ch.is_whitespace() || ch == '"' || ch == '\'' {
            end = index + ch.len_utf8();
            chars.next();
        } else {
            break;
        }
    }
    let (_, delimiter) = chars.next()?;
    if delimiter != '=' && delimiter != ':' {
        return None;
    }
    end += delimiter.len_utf8();
    while let Some((index, ch)) = chars.peek().copied() {
        if ch.is_whitespace() {
            end = index + ch.len_utf8();
            chars.next();
        } else {
            break;
        }
    }
    if let Some((index, quote @ ('"' | '\''))) = chars.peek().copied() {
        return Some((index + quote.len_utf8(), Some(quote)));
    }
    Some((end, None))
}

fn secret_value_end(value: &str, quote: Option<char>) -> usize {
    match quote {
        Some(quote) => value.find(quote).unwrap_or(value.len()),
        None => value
            .find(|ch: char| ch.is_whitespace() || ch == ',' || ch == ';' || ch == '}')
            .unwrap_or(value.len()),
    }
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
    store::load(&store_path()?)
}

#[cfg(test)]
fn save_store(store: &SshServerStore) -> Result<(), String> {
    let path = store_path()?;
    let content = serde_json::to_string_pretty(store)
        .map_err(|e| format!("Failed to serialize SSH servers store: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(&path, &content)
}

pub fn list_servers() -> Result<SshServerList, String> {
    Ok(load_store()?.into())
}

pub fn upsert_server(server: SshServer) -> Result<SshServerList, String> {
    store::update(&store_path()?, |store| {
        let existing_index = store.servers.iter().position(|item| item.id == server.id);
        let existing = existing_index.and_then(|index| store.servers.get(index));
        let server = normalize_server(server, existing)?;
        // Saving another auto-sync server adds it without replacing prior targets.
        store.selected_server_ids.retain(|id| id != &server.id);
        if server.sync_on_codex_switch { store.selected_server_ids.push(server.id.clone()); }
        if let Some(index) = existing_index { store.servers[index] = server; }
        else { store.servers.push(server); }
        Ok(())
    })
}

pub fn delete_server(server_id: &str) -> Result<SshServerList, String> {
    let server_id = server_id.trim();
    store::update(&store_path()?, |store| {
        store.servers.retain(|server| server.id != server_id);
        Ok(())
    })
}

pub fn select_servers(server_ids: Vec<String>) -> Result<SshServerList, String> {
    store::select(&store_path()?, server_ids)
}

pub fn set_server_selected(server_id: &str, selected: bool) -> Result<SshServerList, String> {
    store::set_selected(&store_path()?, server_id, selected)
}

/// OpenSSH 参数：非交互、握手超时与私钥 IdentitiesOnly，避免 agent 里一堆 key 拖慢/超时。
fn build_ssh_args(server: &SshServer, connect_timeout_secs: u64) -> Vec<String> {
    let connect_timeout = connect_timeout_secs.clamp(3, 30);
    let mut args = vec![
        "-p".to_string(),
        server.port.to_string(),
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        "NumberOfPasswordPrompts=0".to_string(),
        "-o".to_string(),
        format!("ConnectTimeout={}", connect_timeout),
        "-o".to_string(),
        "ServerAliveInterval=5".to_string(),
        "-o".to_string(),
        "ServerAliveCountMax=2".to_string(),
    ];
    if let SshAuthConfig::PrivateKeyFile { path } = &server.auth {
        // 与手动 `ssh -o IdentitiesOnly=yes -i key` 对齐：只用指定私钥，不试 agent 其它身份
        args.push("-o".to_string());
        args.push("IdentitiesOnly=yes".to_string());
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
    // 握手超时与整段墙钟分开：ConnectTimeout 用连接上限，命令本身可更长
    let connect_timeout = CONNECTION_TIMEOUT_SECS.min(timeout_secs);
    let mut command = Command::new("ssh");
    command.args(build_ssh_args(server, connect_timeout));
    command.args(remote_args);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let output = process::output(command, Duration::from_secs(timeout_secs), stdin_payload).await?;

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
        TEST_COMMAND_TIMEOUT_SECS,
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
    let list = store::update(&store_path()?, |store| {
        let server = store.servers.iter_mut().find(|server| server.id == server_id)
            .ok_or_else(|| format!("SSH server not found: {server_id}"))?;
        server.last_sync = Some(status.clone());
        server.updated_at = now_timestamp();
        Ok(())
    })?;
    let server = list.servers.iter().find(|server| server.id == server_id)
        .ok_or_else(|| format!("SSH server not found: {server_id}"))?;
    Ok(result_from_status(server, status))
}

async fn sync_account_to_server(server: SshServer, account: &CodexAccount, wait_for_desktop: bool) -> SshCodexSyncResult {
    let synced_at = now_timestamp();
    let sync_attempt = async {
        ensure_current_sync_account(account)?;
        validate_server(&server)?;
        let existing_config = read_remote_config_toml(&server).await?;
        ensure_current_sync_account(account)?;
        let bundle =
            codex_account::build_projection_bundle_for_remote(account, existing_config.as_deref())
                .map_err(|e| format!("codex_bundle_failed: {}", sanitize_error(e)))?;
        // 鉴权落盘 + 校验是硬条件
        upload_and_verify_bundle(&server, &bundle).await?;
        ensure_current_sync_account(account)?;
        let refresh_error = match crate::modules::codex_app_injection::refresh_ssh_connection(
            &server, account, wait_for_desktop,
        ).await {
            Ok(true) => {
                logger::log_info(&format!("[Codex SSH] 客户端连接已重连并确认账号: server_id={}", server.id));
                None
            }
            Ok(false) => None, // No running Desktop connection to refresh.
            Err(error) => Some(format!("ssh_connection_refresh_failed: {}", sanitize_error(error))),
        };
        Ok::<_, String>((bundle, refresh_error))
    }
    .await;

    let status = match sync_attempt {
        Ok((bundle, refresh_error)) => SshCodexSyncStatus {
            account_id: bundle.account_id,
            account_email: bundle.account_email,
            token_generation: bundle.token_generation,
            bundle_hash: bundle.bundle_hash,
            synced_at,
            verified: true,
            error: refresh_error,
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

    // A switch may finish while SSH is in flight. The next serialized worker
    // will write the current credentials; never publish the older status.
    if ensure_current_sync_account(account).is_err() {
        return result_from_status(&server, status);
    }
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
    let _guard = ACCOUNT_SYNC_LOCK.lock().await;
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
        if store.selected_server_ids.len() != 1 {
            return Err("ssh_target_required: specify one SSH server for manual sync".into());
        }
        store.servers.iter().find(|s| store.selected_server_ids.contains(&s.id)).cloned()
            .ok_or_else(|| "ssh_not_configured: no selected SSH server".to_string())?
    };
    Ok(sync_account_to_server(server, &account, false).await)
}

pub async fn sync_selected_servers_after_codex_switch(
    account: &CodexAccount,
    wait_for_desktop: bool,
    mut publish: impl FnMut(SshCodexSyncResult),
) {
    let _guard = ACCOUNT_SYNC_LOCK.lock().await;
    if ensure_current_sync_account(account).is_err() { return; }
    let store = match load_store() {
        Ok(store) => store,
        Err(error) => {
            logger::log_warn(&format!("[Codex SSH] 读取 SSH 服务器配置失败: {}", error));
            return;
        }
    };
    let result = batch::run(store::switch_targets(&store),
        || ensure_current_sync_account(account).is_ok(),
        |server| load_store().map(|latest|
            store::switch_targets(&latest).iter().any(|target| target.id == server.id)),
        |server| sync_account_to_server(server, account, wait_for_desktop), &mut publish).await;
    if let Err((server, error)) = result {
        logger::log_warn(&format!("[Codex SSH] 无法回读同步目标，停止当前队列: {}", sanitize_error(&error)));
        publish(result_from_status(&server, SshCodexSyncStatus {
            account_id: account.id.clone(), account_email: account.email.clone(),
            token_generation: account.token_generation, bundle_hash: String::new(),
            synced_at: now_timestamp(), verified: false, error: Some(sanitize_error(error)),
        }));
    }
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
        assert!(args.contains(&"ConnectTimeout=10".to_string()));
        assert!(!args
            .iter()
            .any(|arg| arg.contains("StrictHostKeyChecking=no")));
        // agent 模式不强制 IdentitiesOnly
        assert!(!args.iter().any(|arg| arg == "IdentitiesOnly=yes"));
    }

    #[test]
    fn ssh_args_use_identities_only_for_private_key() {
        let mut server = valid_server();
        server.auth = SshAuthConfig::PrivateKeyFile {
            path: "/tmp/id_test".to_string(),
        };
        let args = build_ssh_args(&server, 12);
        assert!(args
            .windows(2)
            .any(|w| w[0] == "-o" && w[1] == "IdentitiesOnly=yes"));
        assert!(args
            .windows(2)
            .any(|w| w[0] == "-i" && w[1] == "/tmp/id_test"));
    }

    #[test]
    fn sanitize_error_redacts_secret_values() {
        let error = r#"access_token=abc123 refresh_token: 'def456' {"id_token":"ghi789","OPENAI_API_KEY":"sk-test"}"#;
        let sanitized = sanitize_error(error);
        assert!(sanitized.contains("access_token=[redacted]"));
        assert!(sanitized.contains("refresh_token: '[redacted]'"));
        assert!(sanitized.contains(r#""id_token":"[redacted]""#));
        assert!(sanitized.contains(r#""OPENAI_API_KEY":"[redacted]""#));
        assert!(!sanitized.contains("abc123"));
        assert!(!sanitized.contains("def456"));
        assert!(!sanitized.contains("ghi789"));
        assert!(!sanitized.contains("sk-test"));
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
            version: "2".to_string(),
            selected_server_ids: vec![server.id.clone()],
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
