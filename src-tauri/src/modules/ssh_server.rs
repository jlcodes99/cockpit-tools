use crate::models::codex::CodexAccount;
use crate::models::ssh_server::{
    SshAuthConfig, SshCodexStateRepairStatus, SshCodexSyncResult, SshCodexSyncStatus, SshServer,
    SshServerStore,
};
use crate::modules::{account, atomic_write, codex_account, logger};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::task::JoinSet;
use tokio::time::timeout;
use uuid::Uuid;

const SSH_SERVERS_FILE: &str = "ssh_servers.json";
const STORE_VERSION: &str = "1";
/// TCP/SSH 握手超时（传给 OpenSSH ConnectTimeout）
const CONNECTION_TIMEOUT_SECS: u64 = 12;
/// 测连整段命令墙钟超时
const TEST_COMMAND_TIMEOUT_SECS: u64 = 20;
/// 读写同步脚本墙钟超时
const SYNC_TIMEOUT_SECS: u64 = 45;
/// 远端 SQLite 在线备份可能明显慢于凭据写入。
const STATE_REPAIR_TIMEOUT_SECS: u64 = 120;
/// 远端 app-server pause/resume/reload 墙钟超时。
const APP_SERVER_RELOAD_TIMEOUT_SECS: u64 = 45;
/// 只读列出远端会话的墙钟超时。
const SESSION_LIST_TIMEOUT_SECS: u64 = 20;
const DEFAULT_MODEL_PROVIDER_ID: &str = "openai";
#[cfg(test)]
const STATE_REPAIR_OUTPUT_PREFIX: &str = "__COCKPIT_CODEX_STATE_REPAIR__";
const SYNC_TRANSACTION_OUTPUT_PREFIX: &str = "__COCKPIT_CODEX_SYNC_TRANSACTION__";
const STAGING_OUTPUT_PREFIX: &str = "__COCKPIT_CODEX_SYNC_STAGING__";
const SESSION_LIST_OUTPUT_PREFIX: &str = "__COCKPIT_CODEX_SESSION_LIST__";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshServerList {
    pub selected_server_id: Option<String>,
    pub servers: Vec<SshServer>,
}

#[derive(Debug, Clone)]
pub struct SshCodexSessionSnapshot {
    pub server_id: String,
    pub server_name: String,
    pub session_id: String,
    pub title: String,
    pub cwd: String,
    pub updated_at: Option<i64>,
}

#[derive(Debug, Default)]
pub struct CodexSwitchSshSyncOutcome {
    pub results: Vec<SshCodexSyncResult>,
    pub reachable_server_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RemoteSessionRow {
    id: String,
    title: String,
    cwd: String,
    updated_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QuiescedAppServer {
    status: String,
    pids: Vec<String>,
    listener_pids: Vec<String>,
    watchdog_pid: Option<String>,
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
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    if stdin_payload.is_some() {
        command.stdin(Stdio::piped());
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
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
        // 尽快关闭 stdin，避免远端 sh -s 一直等 EOF
        drop(stdin);
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

fn remote_session_list_script() -> &'static str {
    r#"set -eu
codex_home_encoded="$1"
python_bin=""
for candidate in python3 python; do
  if command -v "$candidate" >/dev/null 2>&1; then
    python_bin="$candidate"
    break
  fi
done
if [ -z "$python_bin" ]; then
  printf '__COCKPIT_CODEX_SESSION_LIST__[]\n'
  exit 0
fi

"$python_bin" - "$codex_home_encoded" <<'__COCKPIT_CODEX_SESSION_LIST_PY__'
import base64
import json
import sqlite3
import sys
from pathlib import Path

OUTPUT_PREFIX = "__COCKPIT_CODEX_SESSION_LIST__"
root = Path(base64.b64decode(sys.argv[1]).decode("utf-8")).expanduser()
db_path = root / "state_5.sqlite"
if not db_path.exists():
    print(OUTPUT_PREFIX + "[]")
    raise SystemExit(0)

connection = sqlite3.connect(db_path.resolve().as_uri() + "?mode=ro", uri=True)
columns = {str(row[1]) for row in connection.execute("PRAGMA table_info(threads)")}
if not {"id", "cwd"}.issubset(columns):
    print(OUTPUT_PREFIX + "[]")
    raise SystemExit(0)

title_terms = []
for column in ("title", "first_user_message", "preview"):
    if column in columns:
        title_terms.append("NULLIF(" + column + ", '')")
title_terms.append("id")
title_expr = "COALESCE(" + ", ".join(title_terms) + ")"

updated_terms = []
for column in ("updated_at", "recency_at", "created_at"):
    if column in columns:
        updated_terms.append(column)
for column in ("updated_at_ms", "recency_at_ms", "created_at_ms"):
    if column in columns:
        updated_terms.append("CAST(" + column + " / 1000 AS INTEGER)")
if not updated_terms:
    updated_expr = "NULL"
elif len(updated_terms) == 1:
    updated_expr = updated_terms[0]
else:
    updated_expr = "COALESCE(" + ", ".join(updated_terms) + ")"

where_terms = []
if "archived" in columns:
    where_terms.append("COALESCE(archived, 0) = 0")
if "has_user_event" in columns:
    where_terms.append("COALESCE(has_user_event, 0) = 1")
if "first_user_message" in columns:
    where_terms.append("COALESCE(first_user_message, '') <> ''")
if "thread_source" in columns:
    where_terms.append("COALESCE(thread_source, 'user') = 'user'")
where_sql = " WHERE " + " AND ".join(where_terms) if where_terms else ""

query = (
    "SELECT id, " + title_expr + " AS title, cwd, " + updated_expr
    + " AS updated_at FROM threads" + where_sql + " ORDER BY updated_at DESC"
)
rows = [
    {
        "id": str(row[0] or ""),
        "title": str(row[1] or row[0] or ""),
        "cwd": str(row[2] or ""),
        "updated_at": int(row[3]) if row[3] is not None else None,
    }
    for row in connection.execute(query)
]
connection.close()
print(OUTPUT_PREFIX + json.dumps(rows, separators=(",", ":"), ensure_ascii=True))
__COCKPIT_CODEX_SESSION_LIST_PY__
"#
}

fn parse_remote_session_list_output(
    server: &SshServer,
    output: &str,
) -> Result<Vec<SshCodexSessionSnapshot>, String> {
    let payload = output
        .lines()
        .find_map(|line| line.trim().strip_prefix(SESSION_LIST_OUTPUT_PREFIX))
        .ok_or_else(|| "ssh_remote_session_list_failed: missing session list result".to_string())?;
    let rows: Vec<RemoteSessionRow> = serde_json::from_str(payload).map_err(|error| {
        format!(
            "ssh_remote_session_list_failed: invalid session list result: {}",
            sanitize_error(error)
        )
    })?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let session_id = row.id.trim().to_string();
            if session_id.is_empty() {
                return None;
            }
            let title = if row.title.trim().is_empty() {
                session_id.clone()
            } else {
                row.title.trim().to_string()
            };
            let cwd = if row.cwd.trim().is_empty() {
                server.codex_home.clone()
            } else {
                row.cwd.trim().to_string()
            };
            Some(SshCodexSessionSnapshot {
                server_id: server.id.clone(),
                server_name: server.name.clone(),
                session_id,
                title,
                cwd,
                updated_at: row.updated_at,
            })
        })
        .collect())
}

async fn list_sessions_from_server(
    server: &SshServer,
) -> Result<Vec<SshCodexSessionSnapshot>, String> {
    let codex_home_encoded = STANDARD.encode(server.codex_home.as_bytes());
    let output = run_ssh(
        server,
        SESSION_LIST_TIMEOUT_SECS,
        &["sh", "-s", "--", &codex_home_encoded],
        Some(remote_session_list_script().to_string()),
    )
    .await
    .map_err(|error| format!("ssh_remote_session_list_failed: {}", sanitize_error(error)))?;
    parse_remote_session_list_output(server, &output)
}

pub async fn list_codex_sessions_from_servers() -> Result<Vec<SshCodexSessionSnapshot>, String> {
    let store = load_store()?;
    let mut tasks = JoinSet::new();
    for server in store.servers {
        tasks.spawn(async move {
            let result = list_sessions_from_server(&server).await;
            (server.name.clone(), result)
        });
    }

    let mut sessions = Vec::new();
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok((_, Ok(mut remote_sessions))) => sessions.append(&mut remote_sessions),
            Ok((server_name, Err(error))) => {
                eprintln!(
                    "[SSH Session List] skipped unavailable server {}: {}",
                    server_name,
                    sanitize_error(error)
                );
            }
            Err(error) => {
                eprintln!(
                    "[SSH Session List] worker failed: {}",
                    sanitize_error(error)
                );
            }
        }
    }
    Ok(sessions)
}

pub async fn test_connection(server_id: &str) -> Result<String, String> {
    let store = load_store()?;
    let server = store
        .servers
        .iter()
        .find(|server| server.id == server_id)
        .cloned()
        .ok_or_else(|| format!("SSH server not found: {}", server_id))?;
    probe_server_connection(&server).await?;
    Ok("cockpit-tools-ssh-ok".to_string())
}

async fn probe_server_connection(server: &SshServer) -> Result<(), String> {
    validate_server(server)?;
    let output = run_ssh(
        server,
        TEST_COMMAND_TIMEOUT_SECS,
        &["printf", "cockpit-tools-ssh-ok"],
        None,
    )
    .await?;
    if output.trim() == "cockpit-tools-ssh-ok" {
        Ok(())
    } else {
        Err("ssh_connection_failed: unexpected SSH test output".to_string())
    }
}

async fn read_remote_config_toml(server: &SshServer) -> Result<Option<String>, String> {
    let script = r#"set -eu
codex_home=$1
case "$codex_home" in
  "~") codex_home="$HOME" ;;
  "~/"*) codex_home="$HOME/${codex_home#~/}" ;;
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

fn validate_model_provider_config(config_toml: Option<&str>) -> Result<String, String> {
    let content = config_toml.unwrap_or_default();
    let doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(content)
        .map_err(|error| {
            format!(
                "ssh_remote_model_provider_invalid: config.toml parse failed: {}",
                sanitize_error(error)
            )
        })?;
    let provider = match doc.get("model_provider") {
        Some(item) => item
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                "ssh_remote_model_provider_invalid: model_provider must be a non-empty string"
                    .to_string()
            })?,
        None => DEFAULT_MODEL_PROVIDER_ID,
    };
    let provider_is_defined = provider == DEFAULT_MODEL_PROVIDER_ID
        || doc
            .get("model_providers")
            .and_then(|item| item.as_table())
            .is_some_and(|providers| providers.contains_key(provider));
    if !provider_is_defined {
        return Err(format!(
            "ssh_remote_model_provider_invalid: Model provider {} not found",
            provider
        ));
    }
    Ok(provider.to_string())
}

fn projection_bundle_model_provider(
    bundle: &codex_account::CodexAccountProjectionBundle,
) -> Result<String, String> {
    let config = bundle
        .files
        .iter()
        .find(|file| file.relative_path == "config.toml")
        .ok_or_else(|| {
            "ssh_remote_model_provider_invalid: staged config.toml is missing".to_string()
        })?;
    validate_model_provider_config(Some(&config.content))
}

async fn stage_remote_bundle(
    server: &SshServer,
    bundle: &codex_account::CodexAccountProjectionBundle,
    run_id: &str,
) -> Result<String, String> {
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
    let manifest = serde_json::json!({
        "version": 1,
        "run_id": run_id,
        "files": bundle.files.iter().map(|file| serde_json::json!({
            "relative_path": file.relative_path,
            "mode": file.mode,
            "sha256": file.sha256,
        })).collect::<Vec<_>>(),
    });
    let manifest_encoded =
        STANDARD.encode(serde_json::to_vec(&manifest).map_err(|error| {
            format!("codex_bundle_failed: serialize staging manifest: {}", error)
        })?);
    let staging_prefix = STAGING_OUTPUT_PREFIX;
    let script = format!(
        r#"set -eu
codex_home=$1
run_id=$2
manifest_encoded=$3
case "$codex_home" in
  "~") codex_home="$HOME" ;;
  "~/"*) codex_home="$HOME/${{codex_home#~/}}" ;;
esac
case "$run_id" in
  *[!A-Za-z0-9-]*) printf 'invalid staging run id\n' >&2; exit 4 ;;
esac
mkdir -p "$codex_home"
chmod 700 "$codex_home" 2>/dev/null || true
staging_root="$codex_home/.cps-codex-sync/staging"
staging_dir="$staging_root/$run_id"
mkdir -p "$staging_root"
chmod 700 "$codex_home/.cps-codex-sync" "$staging_root" 2>/dev/null || true
[ ! -e "$staging_dir" ] || {{ printf 'staging directory already exists\n' >&2; exit 5; }}
mkdir "$staging_dir"
chmod 700 "$staging_dir" 2>/dev/null || true
complete=0
cleanup() {{ [ "$complete" -eq 1 ] || rm -rf "$staging_dir"; }}
trap cleanup EXIT INT TERM
cat <<'__COCKPIT_CODEX_PAYLOAD__' | while IFS='	' read -r rel mode expected encoded; do
{payload}__COCKPIT_CODEX_PAYLOAD__
  [ -n "$rel" ] || continue
  case "$rel" in
    auth.json|config.toml|.cockpit_codex_auth.json) ;;
    *) printf 'invalid relative path: %s\n' "$rel" >&2; exit 4 ;;
  esac
  target="$staging_dir/$rel"
  if ! printf '%s' "$encoded" | base64 -d > "$target" 2>/dev/null; then
    printf '%s' "$encoded" | base64 -D > "$target"
  fi
  chmod "$mode" "$target" 2>/dev/null || true
  actual="$(sha256sum "$target" 2>/dev/null | awk '{{print $1}}' || shasum -a 256 "$target" | awk '{{print $1}}')"
  if [ "$actual" != "$expected" ]; then
    printf 'hash mismatch for %s\n' "$rel" >&2
    exit 5
  fi
  printf '%s\t%s\n' "$rel" "$actual"
done
if ! printf '%s' "$manifest_encoded" | base64 -d > "$staging_dir/manifest.json" 2>/dev/null; then
  printf '%s' "$manifest_encoded" | base64 -D > "$staging_dir/manifest.json"
fi
chmod 600 "$staging_dir/manifest.json" 2>/dev/null || true
complete=1
printf '%s%s\n' '{staging_prefix}' "$staging_dir"
"#
    );
    let output = run_ssh(
        server,
        SYNC_TIMEOUT_SECS,
        &[
            "sh",
            "-s",
            "--",
            &server.codex_home,
            run_id,
            &manifest_encoded,
        ],
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
    output
        .lines()
        .find_map(|line| line.strip_prefix(STAGING_OUTPUT_PREFIX))
        .map(str::to_string)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| "ssh_remote_verify_failed: missing staging path".to_string())
}

#[cfg(test)]
fn remote_state_repair_script() -> &'static str {
    r#"set -eu
codex_home=$1
provider_encoded=$2
case "$codex_home" in
  "~") codex_home="$HOME" ;;
  "~/"*) codex_home="$HOME/${codex_home#~/}" ;;
esac
db="$codex_home/state_5.sqlite"
if [ ! -f "$db" ]; then
  printf '%s\n' '__COCKPIT_CODEX_STATE_REPAIR__{"database_found":false,"backup_path":null,"provider_schema_supported":false,"visibility_schema_supported":false,"rollout_schema_supported":false,"provider_rows_to_repair":0,"visibility_rows_to_repair":0,"rollout_files_to_repair":0,"rows_repaired":0,"rollout_files_repaired":0,"provider_rows_remaining":0,"visibility_rows_remaining":0,"rollout_files_remaining":0,"quick_check":null}'
  exit 0
fi

python_bin=''
for candidate in python3 python; do
  if command -v "$candidate" >/dev/null 2>&1; then
    python_bin=$candidate
    break
  fi
done
if [ -z "$python_bin" ]; then
  printf 'python3 or python is required to safely back up and repair %s\n' "$db" >&2
  exit 6
fi
if ! model_provider="$(printf '%s' "$provider_encoded" | base64 -d 2>/dev/null)"; then
  model_provider="$(printf '%s' "$provider_encoded" | base64 -D)"
fi

"$python_bin" - "$codex_home" "$model_provider" <<'__COCKPIT_CODEX_STATE_PY__'
import datetime
import json
import os
import shutil
import sqlite3
import sys
from pathlib import Path

OUTPUT_PREFIX = "__COCKPIT_CODEX_STATE_REPAIR__"
root = Path(sys.argv[1]).expanduser().resolve()
model_provider = sys.argv[2]
db_path = root / "state_5.sqlite"


def quick_check(connection):
    return "; ".join(str(row[0]) for row in connection.execute("PRAGMA quick_check"))


def resolve_managed_rollout(raw_path):
    path = Path(str(raw_path))
    if not path.is_absolute():
        path = root / path
    resolved = path.resolve()
    try:
        resolved.relative_to(root)
    except ValueError:
        return None
    return resolved


def prepare_rollout_rewrite(path):
    content = path.read_bytes()
    newline_index = content.find(b"\n")
    if newline_index < 0:
        first_line = content
        separator = b""
        rest = b""
    else:
        first_line = content[:newline_index]
        if first_line.endswith(b"\r"):
            first_line = first_line[:-1]
            separator = b"\r\n"
        else:
            separator = b"\n"
        rest = content[newline_index + 1:]
    try:
        record = json.loads(first_line.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        return None
    if (
        not isinstance(record, dict)
        or record.get("type") != "session_meta"
        or not isinstance(record.get("payload"), dict)
    ):
        return None
    payload = record["payload"]
    if payload.get("model_provider") == model_provider:
        return None
    payload["model_provider"] = model_provider
    updated_first_line = json.dumps(
        record, separators=(",", ":"), ensure_ascii=False
    ).encode("utf-8")
    return updated_first_line + separator + rest


def read_rollout_provider(path):
    with path.open("rb") as source:
        first_line = source.readline().rstrip(b"\r\n")
    try:
        record = json.loads(first_line.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        return None
    if (
        not isinstance(record, dict)
        or record.get("type") != "session_meta"
        or not isinstance(record.get("payload"), dict)
    ):
        return None
    return record["payload"].get("model_provider")


def write_atomic(path, content, metadata_path):
    metadata = metadata_path.stat()
    temporary = path.parent / (
        "." + path.name + ".cockpit-ssh-sync-" + str(os.getpid()) + ".tmp"
    )
    try:
        with temporary.open("xb") as output:
            output.write(content)
            output.flush()
            os.fsync(output.fileno())
        os.chmod(temporary, metadata.st_mode & 0o7777)
        os.utime(
            temporary,
            ns=(metadata.st_atime_ns, metadata.st_mtime_ns),
            follow_symlinks=False,
        )
        os.replace(temporary, path)
        directory_fd = os.open(str(path.parent), os.O_RDONLY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
    finally:
        if temporary.exists():
            temporary.unlink()


connection = sqlite3.connect(str(db_path), timeout=10.0, isolation_level=None)
connection.execute("PRAGMA busy_timeout = 10000")
initial_check = quick_check(connection)
if initial_check != "ok":
    raise RuntimeError("state_5.sqlite quick_check failed before repair: " + initial_check)

stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%d-%H%M%S-%f")
backup_dir = root / ("recovery-backup-" + stamp + "-ssh-sync")
backup_dir.mkdir(mode=0o700, parents=False, exist_ok=False)
os.chmod(backup_dir, 0o700)
backup_path = backup_dir / "state_5.sqlite"
backup_connection = sqlite3.connect(str(backup_path))
try:
    connection.backup(backup_connection)
finally:
    backup_connection.close()
os.chmod(backup_path, 0o600)

columns = {str(row[1]) for row in connection.execute("PRAGMA table_info(threads)")}
provider_supported = "model_provider" in columns
visibility_supported = {"first_user_message", "has_user_event"}.issubset(columns)
rollout_supported = "rollout_path" in columns
has_thread_source = "thread_source" in columns
if not provider_supported or not visibility_supported:
    raise RuntimeError(
        "unsupported threads schema: model_provider={}, visibility={}".format(
            provider_supported, visibility_supported
        )
    )

provider_where = "COALESCE(model_provider, '') <> ?"
visibility_terms = ["COALESCE(has_user_event, 0) <> 1"]
if has_thread_source:
    visibility_terms.append("COALESCE(thread_source, '') = ''")
visibility_where = (
    "COALESCE(first_user_message, '') <> '' AND ("
    + " OR ".join(visibility_terms)
    + ")"
)

provider_rows_to_repair = connection.execute(
    "SELECT COUNT(*) FROM threads WHERE " + provider_where,
    (model_provider,),
).fetchone()[0]
visibility_rows_to_repair = connection.execute(
    "SELECT COUNT(*) FROM threads WHERE " + visibility_where
).fetchone()[0]

rollout_changes = []
seen_rollouts = set()
if rollout_supported:
    referenced_rollouts = connection.execute(
        "SELECT rollout_path FROM threads "
        "WHERE rollout_path IS NOT NULL AND rollout_path <> ''"
    ).fetchall()
    for (raw_path,) in referenced_rollouts:
        rollout_path = resolve_managed_rollout(raw_path)
        if (
            rollout_path is None
            or rollout_path in seen_rollouts
            or not rollout_path.is_file()
        ):
            continue
        seen_rollouts.add(rollout_path)
        updated_content = prepare_rollout_rewrite(rollout_path)
        if updated_content is None:
            continue
        relative_path = rollout_path.relative_to(root)
        rollout_backup_path = backup_dir / "rollouts" / relative_path
        rollout_changes.append(
            (rollout_path, rollout_backup_path, updated_content)
        )

rollout_files_to_repair = len(rollout_changes)
for rollout_path, rollout_backup_path, _ in rollout_changes:
    rollout_backup_path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    shutil.copy2(rollout_path, rollout_backup_path)

assignments = [
    "model_provider = ?",
    "has_user_event = CASE WHEN COALESCE(first_user_message, '') <> '' "
    "THEN 1 ELSE has_user_event END",
]
if has_thread_source:
    assignments.append(
        "thread_source = CASE WHEN COALESCE(thread_source, '') = '' "
        "AND COALESCE(first_user_message, '') <> '' THEN 'user' ELSE thread_source END"
    )
parameters = [model_provider, model_provider]

changed_rollouts = []
try:
    for rollout_path, rollout_backup_path, updated_content in rollout_changes:
        changed_rollouts.append((rollout_path, rollout_backup_path))
        write_atomic(rollout_path, updated_content, rollout_backup_path)

    rollout_files_remaining = 0
    for rollout_path, _, _ in rollout_changes:
        if read_rollout_provider(rollout_path) != model_provider:
            rollout_files_remaining += 1
    if rollout_files_remaining != 0:
        raise RuntimeError(
            "rollout verification failed: remaining={}".format(
                rollout_files_remaining
            )
        )

    connection.execute("BEGIN IMMEDIATE")
    try:
        cursor = connection.execute(
            "UPDATE threads SET "
            + ", ".join(assignments)
            + " WHERE "
            + provider_where
            + " OR ("
            + visibility_where
            + ")",
            tuple(parameters),
        )
        rows_repaired = max(cursor.rowcount, 0)
        provider_rows_remaining = connection.execute(
            "SELECT COUNT(*) FROM threads WHERE " + provider_where,
            (model_provider,),
        ).fetchone()[0]
        visibility_rows_remaining = connection.execute(
            "SELECT COUNT(*) FROM threads WHERE " + visibility_where
        ).fetchone()[0]
        final_check = quick_check(connection)
        if provider_rows_remaining != 0 or visibility_rows_remaining != 0:
            raise RuntimeError(
                "row verification failed: provider_remaining={}, visibility_remaining={}".format(
                    provider_rows_remaining, visibility_rows_remaining
                )
            )
        if final_check != "ok":
            raise RuntimeError(
                "state_5.sqlite quick_check failed after repair: " + final_check
            )
        connection.execute("COMMIT")
    except Exception:
        connection.execute("ROLLBACK")
        raise
except Exception:
    for rollout_path, rollout_backup_path in reversed(changed_rollouts):
        write_atomic(rollout_path, rollout_backup_path.read_bytes(), rollout_backup_path)
    raise
finally:
    connection.close()

result = {
    "database_found": True,
    "backup_path": str(backup_path),
    "provider_schema_supported": provider_supported,
    "visibility_schema_supported": visibility_supported,
    "rollout_schema_supported": rollout_supported,
    "provider_rows_to_repair": provider_rows_to_repair,
    "visibility_rows_to_repair": visibility_rows_to_repair,
    "rollout_files_to_repair": rollout_files_to_repair,
    "rows_repaired": rows_repaired,
    "rollout_files_repaired": len(changed_rollouts),
    "provider_rows_remaining": provider_rows_remaining,
    "visibility_rows_remaining": visibility_rows_remaining,
    "rollout_files_remaining": rollout_files_remaining,
    "quick_check": final_check,
}
print(OUTPUT_PREFIX + json.dumps(result, separators=(",", ":"), ensure_ascii=True))
__COCKPIT_CODEX_STATE_PY__
"#
}

#[cfg(test)]
fn parse_remote_state_repair_output(output: &str) -> Result<SshCodexStateRepairStatus, String> {
    let payload = output
        .lines()
        .find_map(|line| line.trim().strip_prefix(STATE_REPAIR_OUTPUT_PREFIX))
        .ok_or_else(|| "ssh_remote_state_repair_failed: missing state repair result".to_string())?;
    let result: SshCodexStateRepairStatus = serde_json::from_str(payload).map_err(|error| {
        format!(
            "ssh_remote_state_repair_failed: invalid state repair result: {}",
            error
        )
    })?;
    if result.database_found {
        if result.backup_path.as_deref().is_none_or(str::is_empty) {
            return Err(
                "ssh_remote_state_repair_failed: state database backup was not reported"
                    .to_string(),
            );
        }
        if !result.provider_schema_supported || !result.visibility_schema_supported {
            return Err(
                "ssh_remote_state_repair_failed: unsupported remote threads schema".to_string(),
            );
        }
        if result.provider_rows_remaining != 0 || result.visibility_rows_remaining != 0 {
            return Err(format!(
                "ssh_remote_state_repair_failed: row verification failed: provider_remaining={}, visibility_remaining={}",
                result.provider_rows_remaining, result.visibility_rows_remaining
            ));
        }
        if result.rollout_files_remaining != 0
            || result.rollout_files_repaired != result.rollout_files_to_repair
        {
            return Err(format!(
                "ssh_remote_state_repair_failed: rollout verification failed: repaired={}, expected={}, remaining={}",
                result.rollout_files_repaired,
                result.rollout_files_to_repair,
                result.rollout_files_remaining
            ));
        }
        if result.quick_check.as_deref() != Some("ok") {
            return Err(format!(
                "ssh_remote_state_repair_failed: quick_check returned {}",
                result.quick_check.as_deref().unwrap_or("no result")
            ));
        }
    }
    Ok(result)
}

fn remote_sync_transaction_script() -> String {
    let mut script = r#"set -eu
codex_home=$1
staging_dir=$2
provider_encoded=$3
case "$codex_home" in
  "~") codex_home="$HOME" ;;
  "~/"*) codex_home="$HOME/${codex_home#~/}" ;;
esac
python_bin=''
for candidate in python3 python; do
  if command -v "$candidate" >/dev/null 2>&1; then
    python_bin=$candidate
    break
  fi
done
if [ -z "$python_bin" ]; then
  printf 'python3 or python is required for transactional Codex sync\n' >&2
  exit 6
fi
if ! model_provider="$(printf '%s' "$provider_encoded" | base64 -d 2>/dev/null)"; then
  model_provider="$(printf '%s' "$provider_encoded" | base64 -D)"
fi
"$python_bin" - "$codex_home" "$staging_dir" "$model_provider" <<'__COCKPIT_CODEX_SYNC_PY__'
"#
    .to_string();
    script.push_str(include_str!("../../scripts/codex_ssh_sync.py"));
    script.push_str("\n__COCKPIT_CODEX_SYNC_PY__\n");
    script
}

fn parse_remote_sync_transaction_output(output: &str) -> Result<SshCodexStateRepairStatus, String> {
    let payload = output
        .lines()
        .find_map(|line| line.trim().strip_prefix(SYNC_TRANSACTION_OUTPUT_PREFIX))
        .ok_or_else(|| "ssh_remote_transaction_failed: missing transaction result".to_string())?;
    serde_json::from_str(payload).map_err(|error| {
        format!(
            "ssh_remote_transaction_failed: invalid transaction result: {}",
            sanitize_error(error)
        )
    })
}

async fn execute_remote_sync_transaction(
    server: &SshServer,
    staging_dir: &str,
    model_provider: &str,
) -> Result<SshCodexStateRepairStatus, String> {
    let provider_encoded = STANDARD.encode(model_provider.as_bytes());
    let output = run_ssh(
        server,
        STATE_REPAIR_TIMEOUT_SECS,
        &[
            "sh",
            "-s",
            "--",
            &server.codex_home,
            staging_dir,
            &provider_encoded,
        ],
        Some(remote_sync_transaction_script()),
    )
    .await
    .map_err(|error| format!("ssh_remote_transaction_failed: {}", sanitize_error(error)))?;
    parse_remote_sync_transaction_output(&output)
}

async fn cleanup_remote_staging(server: &SshServer, staging_dir: &str) {
    let script = r#"set -eu
codex_home=$1
staging_dir=$2
case "$codex_home" in
  "~") codex_home="$HOME" ;;
  "~/"*) codex_home="$HOME/${codex_home#~/}" ;;
esac
staging_root="$codex_home/.cps-codex-sync/staging"
case "$staging_dir" in
  "$staging_root"/*) rm -rf "$staging_dir" ;;
esac
"#;
    if let Err(error) = run_ssh(
        server,
        SYNC_TIMEOUT_SECS,
        &["sh", "-s", "--", &server.codex_home, staging_dir],
        Some(script.to_string()),
    )
    .await
    {
        logger::log_warn(&format!(
            "[Codex SSH] 清理远端暂存目录失败: server_id={}, error={}",
            server.id,
            sanitize_error(error)
        ));
    }
}

fn quiesce_app_server_script() -> &'static str {
    r#"set +e
test_marker=${1:-}
watchdog_delay=${2:-240}
case "$watchdog_delay" in
  ''|*[!0-9]*) watchdog_delay=240 ;;
esac
processes="$(ps -u "$(id -u)" -o pid= -o args= 2>/dev/null || true)"
listener_pids="$(printf '%s\n' "$processes" | awk -v marker="$test_marker" '
{ executable = $2; sub(/^.*\//, "", executable) }
(executable == "codex" && index($0, "app-server --listen") > 0) &&
  (marker == "" || index($0, marker) > 0) { print $1 }
' || true)"
proxy_pids="$(printf '%s\n' "$processes" | awk -v marker="$test_marker" '
{ executable = $2; sub(/^.*\//, "", executable) }
(executable == "codex" && index($0, "app-server proxy") > 0) &&
  (marker == "" || index($0, marker) > 0) { print $1 }
' || true)"
pids="$(printf '%s\n%s\n' "$listener_pids" "$proxy_pids" | awk 'NF && !seen[$1]++ { print $1 }')"
pids="$(printf '%s\n' "$pids" | tr -s '[:space:]' ' ' | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')"
listener_pids="$(printf '%s\n' "$listener_pids" | tr -s '[:space:]' ' ' | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')"
if [ -z "$pids" ]; then
  printf 'no-app-server\n'
  exit 0
fi
paused=''
for pid in $pids; do
  case "$pid" in ''|*[!0-9]*) continue ;; esac
  if kill -STOP "$pid" 2>/dev/null; then
    paused="$paused $pid"
  fi
done
paused="$(printf '%s\n' "$paused" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')"
if [ -z "$paused" ]; then
  printf 'pause-failed\n'
  exit 0
fi

# The detached watchdog guarantees SIGCONT even if Cockpit or SSH disappears mid-transaction.
# shellcheck disable=SC2086
nohup sh -c '
delay=$1
shift
sleep "$delay"
for pid in "$@"; do kill -CONT "$pid" 2>/dev/null || true; done
' cps-app-server-watchdog "$watchdog_delay" $paused </dev/null >/dev/null 2>&1 &
watchdog_pid=$!
if ! kill -0 "$watchdog_pid" 2>/dev/null; then
  for pid in $paused; do kill -CONT "$pid" 2>/dev/null || true; done
  printf 'pause-failed\n'
  exit 0
fi
paused_csv=''
for pid in $paused; do
  if [ -z "$paused_csv" ]; then paused_csv=$pid; else paused_csv="$paused_csv,$pid"; fi
done
listener_csv=''
for pid in $listener_pids; do
  if [ -z "$listener_csv" ]; then listener_csv=$pid; else listener_csv="$listener_csv,$pid"; fi
done
printf 'app-server-paused:%s:%s:%s\n' "$watchdog_pid" "$paused_csv" "$listener_csv"
"#
}

fn parse_process_ids(raw: &str, allow_empty: bool) -> Result<Vec<String>, String> {
    let pids = raw
        .split(',')
        .map(str::trim)
        .filter(|pid| !pid.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if (!allow_empty && pids.is_empty())
        || pids
            .iter()
            .any(|pid| !pid.bytes().all(|value| value.is_ascii_digit()) || pid == "0")
    {
        return Err(format!("invalid process token: {}", sanitize_error(raw)));
    }
    Ok(pids)
}

fn parse_quiesced_app_server(output: &str) -> Result<QuiescedAppServer, String> {
    let response = output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    if response == "no-app-server" {
        return Ok(QuiescedAppServer {
            status: response.to_string(),
            pids: Vec::new(),
            listener_pids: Vec::new(),
            watchdog_pid: None,
        });
    }
    let Some(raw_token) = response.strip_prefix("app-server-paused:") else {
        return Err(format!(
            "ssh_remote_app_server_quiesce_failed: unexpected response: {}",
            sanitize_error(response)
        ));
    };
    let mut fields = raw_token.splitn(3, ':');
    let watchdog_pid = fields.next().unwrap_or("");
    let raw_pids = fields.next().unwrap_or("");
    let raw_listener_pids = fields.next().unwrap_or("");
    let pids = parse_process_ids(raw_pids, false)
        .map_err(|error| format!("ssh_remote_app_server_quiesce_failed: {error}"))?;
    let listener_pids = parse_process_ids(raw_listener_pids, true)
        .map_err(|error| format!("ssh_remote_app_server_quiesce_failed: {error}"))?;
    if !watchdog_pid.bytes().all(|value| value.is_ascii_digit())
        || watchdog_pid == "0"
        || listener_pids.iter().any(|pid| !pids.contains(pid))
    {
        return Err(format!(
            "ssh_remote_app_server_quiesce_failed: invalid process token: {}",
            sanitize_error(raw_token)
        ));
    }
    Ok(QuiescedAppServer {
        status: "app-server-paused".to_string(),
        pids,
        listener_pids,
        watchdog_pid: Some(watchdog_pid.to_string()),
    })
}

async fn quiesce_remote_codex_app_server(server: &SshServer) -> Result<QuiescedAppServer, String> {
    let output = run_ssh(
        server,
        APP_SERVER_RELOAD_TIMEOUT_SECS,
        &["sh", "-s"],
        Some(quiesce_app_server_script().to_string()),
    )
    .await
    .map_err(|error| {
        format!(
            "ssh_remote_app_server_quiesce_failed: {}",
            sanitize_error(error)
        )
    })?;
    parse_quiesced_app_server(&output)
}

fn restore_app_server_script() -> &'static str {
    r#"set +e
watchdog_pid=${1:-}
[ "$#" -gt 0 ] && shift
if [ "$#" -eq 0 ]; then
  printf 'not-required\n'
  exit 0
fi
failed=0
for pid in "$@"; do
  case "$pid" in ''|*[!0-9]*) failed=1; continue ;; esac
  if ! kill -CONT "$pid" 2>/dev/null; then failed=1; fi
done
case "$watchdog_pid" in
  ''|*[!0-9]*) ;;
  *)
    watchdog_args="$(ps -p "$watchdog_pid" -o args= 2>/dev/null || true)"
    case "$watchdog_args" in
      *cps-app-server-watchdog*) kill -TERM "$watchdog_pid" 2>/dev/null || true ;;
    esac
    ;;
esac
if [ "$failed" -ne 0 ]; then
  printf 'restore-failed\n'
else
  printf 'app-server-resumed\n'
fi
"#
}

async fn restore_remote_codex_app_server(
    server: &SshServer,
    quiesced: &QuiescedAppServer,
) -> Result<String, String> {
    let watchdog_pid = quiesced.watchdog_pid.as_deref().unwrap_or("");
    let mut remote_args = vec!["sh", "-s", "--", watchdog_pid];
    remote_args.extend(quiesced.pids.iter().map(String::as_str));
    let output = run_ssh(
        server,
        APP_SERVER_RELOAD_TIMEOUT_SECS,
        &remote_args,
        Some(restore_app_server_script().to_string()),
    )
    .await
    .map_err(|error| {
        format!(
            "ssh_remote_app_server_restore_failed: {}",
            sanitize_error(error)
        )
    })?;
    let status = output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    if status == "app-server-resumed" {
        Ok(status.to_string())
    } else {
        Err(format!(
            "ssh_remote_app_server_restore_failed: {}",
            sanitize_error(status)
        ))
    }
}

fn reload_app_server_script() -> &'static str {
    r#"set +e
codex_home=$1
test_marker=${2:-}
shift 2
[ "$test_marker" = "__cps_no_test_marker__" ] && test_marker=''
case "$codex_home" in
  "~") codex_home="$HOME" ;;
  "~/"*) codex_home="$HOME/${codex_home#~/}" ;;
esac
export CODEX_HOME="$codex_home"
PATH="${CODEX_INSTALL_DIR:-$HOME/.local/bin}:$PATH"
export PATH
old_listener_pids="$*"
if [ -z "$old_listener_pids" ]; then
  printf 'not-required\n'
  exit 0
fi
if ! command -v codex >/dev/null 2>&1; then
  printf 'reload-unavailable\n'
  exit 0
fi

listener_pids() {
  ps -u "$(id -u)" -o pid= -o args= 2>/dev/null | awk -v marker="$test_marker" '
  { executable = $2; sub(/^.*\//, "", executable) }
  (executable == "codex" && index($0, "app-server --listen") > 0) &&
    (marker == "" || index($0, marker) > 0) { print $1 }
  ' | tr -s '[:space:]' ' ' | sed 's/^[[:space:]]*//;s/[[:space:]]*$//'
}

is_listener_pid() {
  candidate=$1
  case "$candidate" in ''|*[!0-9]*) return 1 ;; esac
  kill -0 "$candidate" 2>/dev/null || return 1
  candidate_args="$(ps -p "$candidate" -o args= 2>/dev/null || true)"
  candidate_executable=${candidate_args%% *}
  [ "${candidate_executable##*/}" = "codex" ] || return 1
  case "$candidate_args" in
    *codex*"app-server --listen"*)
      [ -z "$test_marker" ] || case "$candidate_args" in *"$test_marker"*) return 0 ;; *) return 1 ;; esac
      return 0
      ;;
    *) return 1 ;;
  esac
}

has_fresh_listener() {
  current="$(listener_pids)"
  [ -n "$current" ] || return 1
  for candidate in $current; do
    old=0
    for previous in $old_listener_pids; do
      [ "$candidate" = "$previous" ] && old=1
    done
    if [ "$old" -eq 0 ] && is_listener_pid "$candidate"; then return 0; fi
  done
  return 1
}

if command -v timeout >/dev/null 2>&1; then
  timeout 12 codex app-server daemon restart >/dev/null 2>&1
  restart_rc=$?
else
  codex app-server daemon restart >/dev/null 2>&1
  restart_rc=$?
fi
if [ "$restart_rc" -eq 0 ]; then
  tries=0
  while [ "$tries" -lt 50 ]; do
    if has_fresh_listener && codex app-server daemon version >/dev/null 2>&1; then
      printf 'daemon-restarted\n'
      exit 0
    fi
    sleep 0.1
    tries=$((tries + 1))
  done
fi

# A listener started directly by Codex App owns the control socket but cannot be
# restarted by `daemon restart`. Stop only the verified old listeners, then let
# the supported daemon command take ownership.
for pid in $old_listener_pids; do
  if is_listener_pid "$pid"; then kill -TERM "$pid" 2>/dev/null || true; fi
done
tries=0
while [ "$tries" -lt 50 ]; do
  alive=0
  for pid in $old_listener_pids; do
    if is_listener_pid "$pid"; then alive=1; fi
  done
  [ "$alive" -eq 0 ] && break
  sleep 0.1
  tries=$((tries + 1))
done
for pid in $old_listener_pids; do
  if is_listener_pid "$pid"; then kill -KILL "$pid" 2>/dev/null || true; fi
done

if command -v timeout >/dev/null 2>&1; then
  timeout 15 codex app-server daemon start >/dev/null 2>&1
  start_rc=$?
else
  codex app-server daemon start >/dev/null 2>&1
  start_rc=$?
fi
if [ "$start_rc" -ne 0 ]; then
  printf 'reload-failed\n'
  exit 0
fi
tries=0
while [ "$tries" -lt 50 ]; do
  if has_fresh_listener && codex app-server daemon version >/dev/null 2>&1; then
    printf 'daemon-started-after-takeover\n'
    exit 0
  fi
  sleep 0.1
  tries=$((tries + 1))
done
printf 'reload-failed\n'
"#
}

async fn reload_remote_codex_app_server(
    server: &SshServer,
    listener_pids: &[String],
) -> Result<String, String> {
    if listener_pids.is_empty() {
        return Ok("not-required".to_string());
    }
    // OpenSSH joins remote argv into a shell command, so an empty positional
    // argument is not preserved reliably. Use an explicit no-marker sentinel.
    let mut remote_args = vec![
        "sh",
        "-s",
        "--",
        &server.codex_home,
        "__cps_no_test_marker__",
    ];
    remote_args.extend(listener_pids.iter().map(String::as_str));
    let output = run_ssh(
        server,
        APP_SERVER_RELOAD_TIMEOUT_SECS,
        &remote_args,
        Some(reload_app_server_script().to_string()),
    )
    .await
    .map_err(|error| {
        format!(
            "ssh_remote_app_server_reload_failed: {}",
            sanitize_error(error)
        )
    })?;
    let status = output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    if matches!(status, "daemon-restarted" | "daemon-started-after-takeover") {
        Ok("daemon-restarted".to_string())
    } else {
        Err(format!(
            "ssh_remote_app_server_reload_failed: {}",
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
        bundle_verified: status.bundle_verified,
        model_provider: status.model_provider,
        model_provider_verified: status.model_provider_verified,
        state_repair: status.state_repair,
        app_server_reload_status: status.app_server_reload_status,
        app_server_quiesce_status: status.app_server_quiesce_status,
        app_server_restore_status: status.app_server_restore_status,
        verified: status.verified,
        error: status.error,
        synced_at: status.synced_at,
    }
}

fn initial_sync_status(account: &CodexAccount) -> SshCodexSyncStatus {
    SshCodexSyncStatus {
        account_id: account.id.clone(),
        account_email: account.email.clone(),
        token_generation: account.token_generation,
        bundle_hash: String::new(),
        bundle_verified: false,
        model_provider: None,
        model_provider_verified: false,
        state_repair: None,
        app_server_reload_status: None,
        app_server_quiesce_status: None,
        app_server_restore_status: None,
        synced_at: now_timestamp(),
        verified: false,
        error: None,
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
    let mut status = initial_sync_status(account);
    let sync_attempt = async {
        validate_server(&server)?;
        let existing_config = read_remote_config_toml(&server).await?;
        let bundle =
            codex_account::build_projection_bundle_for_remote(account, existing_config.as_deref())
                .map_err(|e| format!("codex_bundle_failed: {}", sanitize_error(e)))?;
        status.account_id = bundle.account_id.clone();
        status.account_email = bundle.account_email.clone();
        status.token_generation = bundle.token_generation;
        status.bundle_hash = bundle.bundle_hash.clone();

        let model_provider = projection_bundle_model_provider(&bundle)?;
        status.model_provider = Some(model_provider.clone());
        let run_id = Uuid::new_v4().to_string();
        let staging_dir = stage_remote_bundle(&server, &bundle, &run_id).await?;

        let quiesced = match quiesce_remote_codex_app_server(&server).await {
            Ok(value) => value,
            Err(error) => {
                cleanup_remote_staging(&server, &staging_dir).await;
                return Err(error);
            }
        };
        status.app_server_quiesce_status = Some(quiesced.status.clone());

        let transaction =
            execute_remote_sync_transaction(&server, &staging_dir, &model_provider).await;
        cleanup_remote_staging(&server, &staging_dir).await;

        let restore = if !quiesced.pids.is_empty() {
            restore_remote_codex_app_server(&server, &quiesced).await
        } else {
            Ok("not-required".to_string())
        };
        match &restore {
            Ok(value) => {
                status.app_server_restore_status = Some(value.clone());
            }
            Err(error) => {
                status.app_server_restore_status = Some("restore-failed".to_string());
                logger::log_warn(&format!(
                    "[Codex SSH] 恢复远端 app-server 失败: server_id={}, error={}",
                    server.id,
                    sanitize_error(error)
                ));
            }
        }

        let transaction = transaction?;
        let transaction_success = transaction.success;
        let transaction_error = transaction.error.clone();
        let transaction_error_stage = transaction.error_stage.clone();
        status.state_repair = Some(transaction);
        if !transaction_success {
            let mut error = format!(
                "ssh_remote_transaction_failed at {}: {}",
                transaction_error_stage.as_deref().unwrap_or("unknown"),
                transaction_error
                    .as_deref()
                    .unwrap_or("unknown transaction error")
            );
            if let Err(restore_error) = restore {
                error.push_str("; ");
                error.push_str(&restore_error);
            }
            return Err(error);
        }
        if let Err(restore_error) = restore {
            return Err(restore_error);
        }

        let reload = reload_remote_codex_app_server(&server, &quiesced.listener_pids).await;
        match &reload {
            Ok(value) => status.app_server_reload_status = Some(value.clone()),
            Err(error) => {
                status.app_server_reload_status = Some("reload-failed".to_string());
                logger::log_warn(&format!(
                    "[Codex SSH] 重载远端 app-server 失败: server_id={}, error={}",
                    server.id,
                    sanitize_error(error)
                ));
            }
        }
        if let Err(reload_error) = reload {
            return Err(reload_error);
        }

        status.bundle_verified = true;
        status.model_provider_verified = true;
        Ok::<(), String>(())
    }
    .await;

    match sync_attempt {
        Ok(()) => status.verified = true,
        Err(error) => status.error = Some(sanitize_error(error)),
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

fn persist_preflight_failure(
    server: &SshServer,
    account: &CodexAccount,
    error: String,
) -> SshCodexSyncResult {
    let mut status = initial_sync_status(account);
    status.error = Some(format!("ssh_preflight_failed: {}", sanitize_error(error)));
    match persist_sync_status(&server.id, status.clone()) {
        Ok(result) => result,
        Err(persist_error) => {
            logger::log_warn(&format!(
                "[Codex SSH] 保存探活失败状态失败: server_id={}, error={}",
                server.id, persist_error
            ));
            result_from_status(server, status)
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

fn servers_enabled_for_codex_switch(store: SshServerStore) -> Vec<SshServer> {
    let selected_id = store.selected_server_id;
    let mut servers = store
        .servers
        .into_iter()
        .filter(|server| server.sync_on_codex_switch)
        .collect::<Vec<_>>();
    servers.sort_by_key(|server| {
        (
            selected_id.as_deref() != Some(server.id.as_str()),
            server.name.clone(),
        )
    });
    servers
}

pub async fn sync_servers_after_codex_switch(account: &CodexAccount) -> CodexSwitchSshSyncOutcome {
    let store = match load_store() {
        Ok(store) => store,
        Err(error) => {
            logger::log_warn(&format!("[Codex SSH] 读取 SSH 服务器配置失败: {}", error));
            return CodexSwitchSshSyncOutcome::default();
        }
    };

    let servers = servers_enabled_for_codex_switch(store);
    let mut preflight_results = Vec::with_capacity(servers.len());
    let mut reachable_server_ids = Vec::with_capacity(servers.len());
    for server in servers {
        let preflight_error = match probe_server_connection(&server).await {
            Ok(()) => {
                logger::log_info(&format!(
                    "[Codex SSH] 切号前探活成功: server_id={}, server_name={}",
                    server.id, server.name
                ));
                reachable_server_ids.push(server.id.clone());
                None
            }
            Err(error) => {
                logger::log_warn(&format!(
                    "[Codex SSH] 切号前探活失败: server_id={}, server_name={}, error={}",
                    server.id,
                    server.name,
                    sanitize_error(&error)
                ));
                Some(error)
            }
        };
        preflight_results.push((server, preflight_error));
    }

    // Keep probing separate from the heavier sync so launch context reflects reachability,
    // even when a later auth/config/state verification step fails.
    let mut results = Vec::with_capacity(preflight_results.len());
    for (server, preflight_error) in preflight_results {
        if let Some(error) = preflight_error {
            results.push(persist_preflight_failure(&server, account, error));
        } else {
            results.push(sync_account_to_server(server, account).await);
        }
    }
    CodexSwitchSshSyncOutcome {
        results,
        reachable_server_ids,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::io::Write;
    use std::path::PathBuf;
    #[cfg(unix)]
    use std::process::Command as StdCommand;
    #[cfg(unix)]
    use std::process::Output as StdOutput;

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
    fn codex_switch_includes_all_enabled_servers_with_selected_first() {
        let mut server_a = valid_server();
        server_a.id = "server-a".to_string();
        server_a.name = "Alpha".to_string();

        let mut server_b = valid_server();
        server_b.id = "server-b".to_string();
        server_b.name = "Beta".to_string();

        let mut disabled = valid_server();
        disabled.id = "server-disabled".to_string();
        disabled.name = "Disabled".to_string();
        disabled.sync_on_codex_switch = false;

        let servers = servers_enabled_for_codex_switch(SshServerStore {
            version: STORE_VERSION.to_string(),
            selected_server_id: Some(server_b.id.clone()),
            servers: vec![server_a, disabled, server_b],
        });
        let ids = servers
            .iter()
            .map(|server| server.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["server-b", "server-a"]);
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
    fn app_server_scripts_quiesce_and_restore_only_when_needed() {
        let quiesce = quiesce_app_server_script();
        assert!(quiesce.contains("kill -STOP"));
        assert!(quiesce.contains("app-server-paused:"));
        assert!(quiesce.contains("no-app-server"));
        assert!(quiesce.contains("nohup sh -c"));
        assert!(quiesce.contains("</dev/null >/dev/null 2>&1 &"));
        assert!(quiesce.contains("watchdog_delay=${2:-240}"));
        assert!(quiesce.contains("index($0, \"app-server --listen\")"));
        assert!(quiesce.contains("listener_pids"));
        assert!(!quiesce.contains("kill -TERM"));
        assert!(!quiesce.contains("kill -KILL"));
        assert!(!quiesce.contains("daemon restart"));

        let restore = restore_app_server_script();
        assert!(restore.contains("kill -CONT"));
        assert!(restore.contains("app-server-resumed"));
        assert!(restore.contains("restore-failed"));
        assert!(restore.contains("cps-app-server-watchdog"));
        assert!(!restore.contains("codex app-server"));

        let reload = reload_app_server_script();
        assert!(reload.contains("codex app-server daemon restart"));
        assert!(reload.contains("codex app-server daemon start"));
        assert!(reload.contains("daemon-started-after-takeover"));
        assert!(reload.contains("has_fresh_listener"));
        assert!(reload.contains("__cps_no_test_marker__"));
        assert!(reload.contains("kill -TERM"));
        assert!(reload.contains("kill -KILL"));
    }

    #[test]
    fn app_server_pause_token_is_strictly_validated() {
        assert_eq!(
            parse_quiesced_app_server("app-server-paused:99:123,456:123\n").unwrap(),
            QuiescedAppServer {
                status: "app-server-paused".to_string(),
                pids: vec!["123".to_string(), "456".to_string()],
                listener_pids: vec!["123".to_string()],
                watchdog_pid: Some("99".to_string()),
            }
        );
        assert_eq!(
            parse_quiesced_app_server("no-app-server\n").unwrap(),
            QuiescedAppServer {
                status: "no-app-server".to_string(),
                pids: Vec::new(),
                listener_pids: Vec::new(),
                watchdog_pid: None,
            }
        );
        assert!(parse_quiesced_app_server("app-server-paused:99:12,bad:12").is_err());
        assert!(parse_quiesced_app_server("app-server-paused:99:12:13").is_err());
        assert!(parse_quiesced_app_server("app-server-paused:bad:12:12").is_err());
        assert!(parse_quiesced_app_server("pause-failed").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn app_server_scripts_pause_and_resume_the_same_process() {
        fn run_script(script: &str, args: &[&str]) -> std::process::Output {
            let mut child = StdCommand::new("sh")
                .arg("-s")
                .arg("--")
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn shell script");
            child
                .stdin
                .take()
                .expect("script stdin")
                .write_all(script.as_bytes())
                .expect("write shell script");
            child.wait_with_output().expect("wait for shell script")
        }

        let marker = format!("cps-pause-test-{}", std::process::id());
        let root = std::env::temp_dir().join(format!("cps-pause-test-{}", Uuid::new_v4()));
        let listener_executable = fake_listener_executable(&root);
        let mut app_server = spawn_fake_listener(&marker, &listener_executable);
        let pid = app_server.id().to_string();
        std::thread::sleep(Duration::from_millis(100));

        let result = (|| -> Result<(), String> {
            let quiesce = run_script(quiesce_app_server_script(), &[&marker, "3"]);
            if !quiesce.status.success() {
                return Err(String::from_utf8_lossy(&quiesce.stderr).to_string());
            }
            let token = parse_quiesced_app_server(&String::from_utf8_lossy(&quiesce.stdout))?;
            if token.pids != vec![pid.clone()] {
                return Err(format!("unexpected paused pids: {:?}", token.pids));
            }
            if token.listener_pids != vec![pid.clone()] {
                return Err(format!(
                    "unexpected listener pids: {:?}",
                    token.listener_pids
                ));
            }
            let paused_state = StdCommand::new("ps")
                .args(["-o", "state=", "-p", &pid])
                .output()
                .map_err(|error| error.to_string())?;
            if !String::from_utf8_lossy(&paused_state.stdout)
                .trim()
                .starts_with('T')
            {
                return Err("fake app-server was not stopped".to_string());
            }

            let watchdog_pid = token
                .watchdog_pid
                .as_deref()
                .ok_or_else(|| "missing watchdog pid".to_string())?;
            let restore = run_script(restore_app_server_script(), &[watchdog_pid, &pid]);
            if !restore.status.success()
                || String::from_utf8_lossy(&restore.stdout).trim() != "app-server-resumed"
            {
                return Err(format!(
                    "restore failed: {}",
                    String::from_utf8_lossy(&restore.stderr)
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
            let resumed_state = StdCommand::new("ps")
                .args(["-o", "state=", "-p", &pid])
                .output()
                .map_err(|error| error.to_string())?;
            if String::from_utf8_lossy(&resumed_state.stdout)
                .trim()
                .starts_with('T')
            {
                return Err("fake app-server remained stopped".to_string());
            }
            Ok(())
        })();

        let _ = StdCommand::new("kill").args(["-CONT", &pid]).status();
        let _ = app_server.kill();
        let _ = app_server.wait();
        let _ = std::fs::remove_dir_all(&root);
        result.expect("pause and resume lifecycle");
    }

    #[cfg(unix)]
    fn run_script_with_env(script: &str, args: &[&str], env: &[(&str, &str)]) -> StdOutput {
        let mut command = StdCommand::new("sh");
        command
            .arg("-s")
            .arg("--")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in env {
            command.env(key, value);
        }
        let mut child = command.spawn().expect("spawn shell script");
        child
            .stdin
            .take()
            .expect("script stdin")
            .write_all(script.as_bytes())
            .expect("write shell script");
        child.wait_with_output().expect("wait for shell script")
    }

    #[cfg(unix)]
    fn fake_listener_executable(root: &std::path::Path) -> PathBuf {
        use std::os::unix::fs::symlink;

        let bin_dir = root.join("listener-bin");
        std::fs::create_dir_all(&bin_dir).expect("create fake listener bin directory");
        let executable = bin_dir.join("codex");
        symlink("/bin/sh", &executable).expect("link fake codex listener executable");
        executable
    }

    #[cfg(unix)]
    fn spawn_fake_listener(
        marker: &str,
        listener_executable: &std::path::Path,
    ) -> std::process::Child {
        let process_name =
            format!("codex -c features.code_mode_host=true app-server --listen unix:// {marker}");
        StdCommand::new(listener_executable)
            .arg("-c")
            .arg("while :; do sleep 1; done")
            .arg(process_name)
            .spawn()
            .expect("spawn fake listener")
    }

    #[cfg(unix)]
    fn write_fake_codex_cli(bin_dir: &std::path::Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        std::fs::create_dir_all(bin_dir).expect("create fake bin directory");
        let path = bin_dir.join("codex");
        std::fs::write(
            &path,
            r#"#!/bin/sh
set +e
command_name="${1:-} ${2:-} ${3:-}"
spawn_listener() {
  "$FAKE_CODEX_LISTENER" -c 'while :; do sleep 1; done' "codex -c features.code_mode_host=true app-server --listen unix:// $FAKE_CODEX_MARKER" </dev/null >/dev/null 2>&1 &
  printf '%s\n' "$!" >"$FAKE_CODEX_STATE/new_pid"
}
case "$command_name" in
  "app-server daemon restart")
    if [ "$FAKE_CODEX_MODE" != "managed" ]; then exit 1; fi
    old_pid="$(cat "$FAKE_CODEX_STATE/old_pid")"
    kill -TERM "$old_pid" 2>/dev/null || true
    spawn_listener
    exit 0
    ;;
  "app-server daemon start")
    spawn_listener
    exit 0
    ;;
  "app-server daemon version")
    new_pid="$(cat "$FAKE_CODEX_STATE/new_pid" 2>/dev/null)"
    kill -0 "$new_pid" 2>/dev/null
    exit $?
    ;;
esac
exit 1
"#,
        )
        .expect("write fake codex CLI");
        let mut permissions = std::fs::metadata(&path)
            .expect("fake codex metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).expect("make fake codex executable");
        path
    }

    #[cfg(unix)]
    fn assert_reload_lifecycle(mode: &str, expected_status: &str) {
        let marker = format!("cps-reload-{mode}-{}", Uuid::new_v4());
        let root = std::env::temp_dir().join(format!("cps-reload-test-{}", Uuid::new_v4()));
        let state_dir = root.join("state");
        let bin_dir = root.join("bin");
        std::fs::create_dir_all(&state_dir).expect("create fake state directory");
        write_fake_codex_cli(&bin_dir);
        let listener_executable = fake_listener_executable(&root);

        let mut old_listener = spawn_fake_listener(&marker, &listener_executable);
        let old_pid = old_listener.id().to_string();
        std::fs::write(state_dir.join("old_pid"), &old_pid).expect("write old listener pid");
        std::thread::sleep(Duration::from_millis(100));

        let original_path = std::env::var("PATH").unwrap_or_default();
        let test_path = format!("{}:{original_path}", bin_dir.display());
        let state = state_dir.to_string_lossy().into_owned();
        let listener = listener_executable.to_string_lossy().into_owned();
        let output = run_script_with_env(
            reload_app_server_script(),
            &["~/.codex", &marker, &old_pid],
            &[
                ("PATH", &test_path),
                ("FAKE_CODEX_MODE", mode),
                ("FAKE_CODEX_MARKER", &marker),
                ("FAKE_CODEX_STATE", &state),
                ("FAKE_CODEX_LISTENER", &listener),
            ],
        );

        let result = (|| -> Result<String, String> {
            if !output.status.success() {
                return Err(String::from_utf8_lossy(&output.stderr).to_string());
            }
            let status = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if status != expected_status {
                return Err(format!(
                    "unexpected reload status {status:?}: {}",
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            let new_pid = std::fs::read_to_string(state_dir.join("new_pid"))
                .map_err(|error| error.to_string())?
                .trim()
                .to_string();
            if new_pid == old_pid {
                return Err("reload reused the old listener pid".to_string());
            }
            let new_args = StdCommand::new("ps")
                .args(["-p", &new_pid, "-o", "args="])
                .output()
                .map_err(|error| error.to_string())?;
            if !String::from_utf8_lossy(&new_args.stdout).contains(&marker) {
                return Err("fresh listener was not running".to_string());
            }
            Ok(new_pid)
        })();

        let _ = old_listener.kill();
        let _ = old_listener.wait();
        if let Ok(new_pid) = &result {
            let _ = StdCommand::new("kill").args(["-TERM", new_pid]).status();
        }
        let _ = std::fs::remove_dir_all(&root);
        result.expect("reload app-server lifecycle");
    }

    #[cfg(unix)]
    #[test]
    fn app_server_reload_restarts_a_managed_listener() {
        assert_reload_lifecycle("managed", "daemon-restarted");
    }

    #[cfg(unix)]
    #[test]
    fn app_server_reload_takes_over_an_unmanaged_listener() {
        assert_reload_lifecycle("unmanaged", "daemon-started-after-takeover");
    }

    #[test]
    fn transaction_script_embeds_reconciliation_and_rollback_guards() {
        let script = remote_sync_transaction_script();
        assert!(script.contains("archived_sessions"));
        assert!(script.contains("rollout_paths_repaired"));
        assert!(script.contains("user_events_recovered"));
        assert!(script.contains("orphan_threads_recovered"));
        assert!(script.contains("verify_rollout_fingerprints"));
        assert!(script.contains("restore_backup"));
        assert!(script.contains("connection.backup(backup_connection)"));
        assert!(script.contains(SYNC_TRANSACTION_OUTPUT_PREFIX));
    }

    #[test]
    fn model_provider_validation_accepts_default_and_defined_provider() {
        assert_eq!(
            validate_model_provider_config(None).expect("default provider"),
            DEFAULT_MODEL_PROVIDER_ID
        );
        let config = r#"model_provider = "relay"

[model_providers.relay]
name = "Relay"
base_url = "https://example.com/v1"
"#;
        assert_eq!(
            validate_model_provider_config(Some(config)).expect("defined provider"),
            "relay"
        );
    }

    #[test]
    fn model_provider_validation_rejects_missing_definition() {
        let error =
            validate_model_provider_config(Some(r#"model_provider = "codex_local_access""#))
                .expect_err("missing provider definition must fail");
        assert!(error.contains("Model provider codex_local_access not found"));
    }

    #[test]
    fn state_repair_output_requires_verified_rows_and_integrity() {
        let valid = format!(
            "noise\n{}{}",
            STATE_REPAIR_OUTPUT_PREFIX,
            r#"{"database_found":true,"backup_path":"/tmp/backup/state_5.sqlite","provider_schema_supported":true,"visibility_schema_supported":true,"rollout_schema_supported":true,"provider_rows_to_repair":2,"visibility_rows_to_repair":1,"rollout_files_to_repair":2,"rows_repaired":2,"rollout_files_repaired":2,"provider_rows_remaining":0,"visibility_rows_remaining":0,"rollout_files_remaining":0,"quick_check":"ok"}"#
        );
        let parsed = parse_remote_state_repair_output(&valid).expect("valid repair output");
        assert_eq!(parsed.rows_repaired, 2);

        let invalid = valid.replace(
            "\"visibility_rows_remaining\":0",
            "\"visibility_rows_remaining\":1",
        );
        assert!(parse_remote_state_repair_output(&invalid)
            .expect_err("remaining rows must fail")
            .contains("row verification failed"));

        let invalid_rollout = valid.replace(
            "\"rollout_files_remaining\":0",
            "\"rollout_files_remaining\":1",
        );
        assert!(parse_remote_state_repair_output(&invalid_rollout)
            .expect_err("remaining rollout must fail")
            .contains("rollout verification failed"));
    }

    #[test]
    fn state_repair_script_uses_online_backup_and_transaction() {
        let script = remote_state_repair_script();
        assert!(script.contains("connection.backup(backup_connection)"));
        assert!(script.contains("BEGIN IMMEDIATE"));
        assert!(script.contains("PRAGMA quick_check"));
        assert!(script.contains("shutil.copy2(rollout_path, rollout_backup_path)"));
        assert!(script.contains("os.replace(temporary, path)"));
        assert!(!script.contains("cp \"$db\""));
    }

    #[cfg(unix)]
    #[test]
    fn session_list_script_returns_only_visible_user_threads() {
        use rusqlite::Connection;
        use std::io::Write;
        use std::process::{Command as StdCommand, Stdio as StdStdio};

        let temp_dir = std::env::temp_dir().join(format!(
            "cockpit ssh session list {}; metacharacters",
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&temp_dir).expect("create temporary Codex home");
        let connection =
            Connection::open(temp_dir.join("state_5.sqlite")).expect("create state database");
        connection
            .execute_batch(
                r#"
                CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    title TEXT,
                    cwd TEXT,
                    updated_at INTEGER,
                    archived INTEGER,
                    first_user_message TEXT,
                    has_user_event INTEGER,
                    thread_source TEXT
                );
                INSERT INTO threads VALUES ('visible', 'Visible task', '/repo', 42, 0, 'hello', 1, 'user');
                INSERT INTO threads VALUES ('archived', 'Archived task', '/repo', 41, 1, 'hello', 1, 'user');
                INSERT INTO threads VALUES ('subagent', 'Child task', '/repo', 40, 0, '', 0, 'subagent');
                "#,
            )
            .expect("seed state database");

        let temp_dir_encoded =
            STANDARD.encode(temp_dir.to_str().expect("utf-8 temporary path").as_bytes());
        let mut child = StdCommand::new("sh")
            .args(["-s", "--", &temp_dir_encoded])
            .stdin(StdStdio::piped())
            .stdout(StdStdio::piped())
            .stderr(StdStdio::piped())
            .spawn()
            .expect("spawn session list script");
        child
            .stdin
            .as_mut()
            .expect("session list script stdin")
            .write_all(remote_session_list_script().as_bytes())
            .expect("write session list script");
        let output = child
            .wait_with_output()
            .expect("wait for session list script");
        assert!(
            output.status.success(),
            "session list failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).expect("utf-8 session list output");
        let sessions =
            parse_remote_session_list_output(&valid_server(), &stdout).expect("parse sessions");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "visible");
        assert_eq!(sessions[0].title, "Visible task");
        assert_eq!(sessions[0].cwd, "/repo");
        assert_eq!(sessions[0].updated_at, Some(42));
        assert_eq!(sessions[0].server_name, "Dev");
        assert!(remote_session_list_script().contains("?mode=ro"));
        assert!(remote_session_list_script().contains("uri=True"));

        drop(connection);
        std::fs::remove_dir_all(&temp_dir).expect("remove temporary Codex home");
    }

    #[cfg(unix)]
    #[test]
    fn state_repair_script_aligns_database_and_rollout_providers() {
        use rusqlite::Connection;
        use std::io::Write;
        use std::process::{Command as StdCommand, Stdio as StdStdio};

        let temp_dir =
            std::env::temp_dir().join(format!("cockpit-ssh-state-repair-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).expect("create temporary Codex home");
        let rollout_dir = temp_dir.join("sessions").join("2026").join("07").join("16");
        std::fs::create_dir_all(&rollout_dir).expect("create rollout directory");
        let user_rollout = rollout_dir.join("rollout-user-needs-repair.jsonl");
        let child_rollout = rollout_dir.join("rollout-child-thread.jsonl");
        let ok_rollout = rollout_dir.join("rollout-user-ok.jsonl");
        std::fs::write(
            &user_rollout,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"user-needs-repair\",\"model_provider\":\"old\"}}\n{\"type\":\"event_msg\",\"payload\":{}}\n",
        )
        .expect("write user rollout");
        std::fs::write(
            &child_rollout,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"child-thread\",\"model_provider\":\"old\"}}\n",
        )
        .expect("write child rollout");
        std::fs::write(
            &ok_rollout,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"user-ok\",\"model_provider\":\"openai\"}}\n",
        )
        .expect("write matching rollout");
        let relative_rollout = |path: &std::path::Path| {
            path.strip_prefix(&temp_dir)
                .expect("rollout is inside Codex home")
                .to_string_lossy()
                .to_string()
        };
        let db_path = temp_dir.join("state_5.sqlite");
        let connection = Connection::open(&db_path).expect("create state database");
        connection
            .execute_batch(
                r#"
                PRAGMA journal_mode = WAL;
                CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT,
                    model_provider TEXT,
                    first_user_message TEXT,
                    has_user_event INTEGER,
                    thread_source TEXT
                );
                "#,
            )
            .expect("create state database schema");
        connection
            .execute(
                "INSERT INTO threads VALUES ('user-needs-repair', ?1, 'old', 'hello', 0, '')",
                [relative_rollout(&user_rollout)],
            )
            .expect("insert user thread");
        connection
            .execute(
                "INSERT INTO threads VALUES ('child-thread', ?1, 'old', '', 0, 'subagent')",
                [relative_rollout(&child_rollout)],
            )
            .expect("insert child thread");
        connection
            .execute(
                "INSERT INTO threads VALUES ('user-ok', ?1, 'openai', 'ready', 1, 'user')",
                [relative_rollout(&ok_rollout)],
            )
            .expect("insert matching thread");

        let mut child = StdCommand::new("sh")
            .args([
                "-s",
                "--",
                temp_dir.to_str().expect("utf-8 temporary path"),
                &STANDARD.encode(DEFAULT_MODEL_PROVIDER_ID.as_bytes()),
            ])
            .stdin(StdStdio::piped())
            .stdout(StdStdio::piped())
            .stderr(StdStdio::piped())
            .spawn()
            .expect("spawn repair script");
        child
            .stdin
            .as_mut()
            .expect("repair script stdin")
            .write_all(remote_state_repair_script().as_bytes())
            .expect("write repair script");
        let output = child.wait_with_output().expect("wait for repair script");
        assert!(
            output.status.success(),
            "repair failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).expect("utf-8 repair output");
        let result = parse_remote_state_repair_output(&stdout).expect("parse repair result");
        assert_eq!(result.provider_rows_to_repair, 2);
        assert_eq!(result.visibility_rows_to_repair, 1);
        assert!(result.rollout_schema_supported);
        assert_eq!(result.rollout_files_to_repair, 2);
        assert_eq!(result.rollout_files_repaired, 2);
        assert_eq!(result.rollout_files_remaining, 0);
        assert_eq!(result.rows_repaired, 2);
        assert_eq!(result.quick_check.as_deref(), Some("ok"));

        assert!(std::fs::read_to_string(&user_rollout)
            .expect("read repaired user rollout")
            .contains("\"model_provider\":\"openai\""));
        assert!(std::fs::read_to_string(&child_rollout)
            .expect("read repaired child rollout")
            .contains("\"model_provider\":\"openai\""));
        assert!(std::fs::read_to_string(&ok_rollout)
            .expect("read matching rollout")
            .contains("\"model_provider\":\"openai\""));

        let repaired: (String, i64, String) = connection
            .query_row(
                "SELECT model_provider, has_user_event, thread_source FROM threads WHERE id = 'user-needs-repair'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read repaired user thread");
        assert_eq!(repaired, ("openai".to_string(), 1, "user".to_string()));
        let child_thread: (String, i64, String) = connection
            .query_row(
                "SELECT model_provider, has_user_event, thread_source FROM threads WHERE id = 'child-thread'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read child thread");
        assert_eq!(
            child_thread,
            ("openai".to_string(), 0, "subagent".to_string())
        );

        let backup_path = std::path::PathBuf::from(result.backup_path.expect("backup path"));
        let backup_root = backup_path.parent().expect("backup directory");
        let user_rollout_backup = backup_root
            .join("rollouts")
            .join(relative_rollout(&user_rollout));
        assert!(std::fs::read_to_string(user_rollout_backup)
            .expect("read rollout backup")
            .contains("\"model_provider\":\"old\""));
        let backup = Connection::open(backup_path).expect("open online backup");
        let backup_state: (String, i64, String) = backup
            .query_row(
                "SELECT model_provider, has_user_event, thread_source FROM threads WHERE id = 'user-needs-repair'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .expect("read backup state");
        assert_eq!(backup_state, ("old".to_string(), 0, String::new()));

        drop(backup);
        drop(connection);
        std::fs::remove_dir_all(&temp_dir).expect("remove temporary Codex home");
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
    async fn live_ssh_lists_configured_sessions() {
        if std::env::var("COCKPIT_LIVE_SSH_SESSION_LIST")
            .ok()
            .as_deref()
            != Some("1")
        {
            eprintln!("set COCKPIT_LIVE_SSH_SESSION_LIST=1 to run the live SSH session list test");
            return;
        }

        let expected_session_id = std::env::var("COCKPIT_LIVE_SSH_EXPECTED_SESSION_ID")
            .expect("set COCKPIT_LIVE_SSH_EXPECTED_SESSION_ID to a non-sensitive test session");

        let sessions = list_codex_sessions_from_servers()
            .await
            .expect("configured SSH session list should load");
        assert!(
            sessions
                .iter()
                .any(|session| session.session_id == expected_session_id),
            "expected configured test session in the remote list"
        );
    }

    #[tokio::test]
    #[ignore]
    async fn live_ssh_syncs_selected_configured_server() {
        if std::env::var("COCKPIT_LIVE_SSH_CONFIGURED_SYNC")
            .ok()
            .as_deref()
            != Some("1")
        {
            eprintln!(
                "set COCKPIT_LIVE_SSH_CONFIGURED_SYNC=1 to sync the selected configured server"
            );
            return;
        }

        let result = sync_current_account_to_server(None)
            .await
            .expect("configured SSH sync should return a result");
        assert!(
            result.verified,
            "configured SSH sync should verify remote state: {:?}",
            result.error
        );
        let repair = result
            .state_repair
            .expect("configured SSH sync should report state repair");
        assert_eq!(repair.provider_rows_remaining, 0);
        assert_eq!(repair.visibility_rows_remaining, 0);
        assert_eq!(repair.rollout_files_remaining, 0);
        assert_eq!(
            repair.rollout_files_repaired,
            repair.rollout_files_to_repair
        );
        assert_eq!(repair.quick_check.as_deref(), Some("ok"));
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
