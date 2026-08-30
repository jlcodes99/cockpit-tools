//! Codex SSH account projection sync (vertical slice of #1404).

use crate::modules::atomic_write;
use crate::modules::codex_account;
use serde::{Deserialize, Serialize};
use std::fs;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

const STORE_FILE: &str = "codex_ssh_servers.json";
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSshServer {
    pub id: String,
    pub name: String,
    pub host: String,
    pub user: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// Remote directory that should receive auth.json (e.g. ~/.codex)
    pub remote_codex_dir: String,
}

fn default_port() -> u16 {
    22
}

/// Validate and return a shell-safe remote directory path for ssh/scp.
/// Rejects shell metacharacters; allows `~`, letters, digits, `.`, `_`, `-`, `/`.
pub fn sanitize_remote_codex_dir(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    let path = if trimmed.is_empty() {
        "~/.codex"
    } else {
        trimmed
    };
    if path.len() > 512 {
        return Err("远端 Codex 目录过长".to_string());
    }
    // Disallow shell metacharacters and whitespace to prevent injection into `ssh … sh -c`.
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '~');
    if !path.chars().all(allowed) {
        return Err("远端 Codex 目录含非法字符（仅允许字母数字、~、/、.、_、-）".to_string());
    }
    if path.contains("..") {
        return Err("远端 Codex 目录不允许包含 ..".to_string());
    }
    Ok(path.to_string())
}

/// Single-quote for remote shell (POSIX): wrap in '…' and escape embedded ' as '\''.
pub fn shell_single_quote(path: &str) -> String {
    let mut out = String::from("'");
    for ch in path.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct CodexSshStore {
    #[serde(default)]
    servers: Vec<CodexSshServer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selected_id: Option<String>,
}

fn store_path() -> Result<PathBuf, String> {
    let dir = dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .ok_or("无法定位应用数据目录")?;
    Ok(dir.join("cockpit-tools").join(STORE_FILE))
}

fn load_store() -> Result<CodexSshStore, String> {
    let path = store_path()?;
    if !path.exists() {
        return Ok(CodexSshStore::default());
    }
    let raw = fs::read_to_string(&path).map_err(|e| format!("读取 SSH 配置失败: {e}"))?;
    if raw.trim().is_empty() {
        return Ok(CodexSshStore::default());
    }
    serde_json::from_str(&raw).map_err(|e| format!("解析 SSH 配置失败: {e}"))
}

fn save_store(store: &CodexSshStore) -> Result<(), String> {
    let path = store_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let raw = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    atomic_write::write_string_atomic(&path, &raw)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSshListResult {
    pub servers: Vec<CodexSshServer>,
    pub selected_id: Option<String>,
}

pub fn list_servers() -> Result<(Vec<CodexSshServer>, Option<String>), String> {
    let store = load_store()?;
    Ok((store.servers, store.selected_id))
}

fn validate_ssh_user_or_host(kind: &str, value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{kind} 不能为空"));
    }
    if trimmed.len() > 253 {
        return Err(format!("{kind} 过长"));
    }
    if trimmed
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || c == '@')
    {
        return Err(format!("{kind} 含非法空白、控制字符或 @"));
    }
    Ok(trimmed.to_string())
}

pub fn upsert_server(server: CodexSshServer) -> Result<CodexSshServer, String> {
    let host = validate_ssh_user_or_host("host", &server.host)?;
    let user = validate_ssh_user_or_host("user", &server.user)?;
    let remote_codex_dir = sanitize_remote_codex_dir(&server.remote_codex_dir)?;
    let name = {
        let n = server.name.trim();
        if n.is_empty() {
            format!("{user}@{host}")
        } else {
            n.to_string()
        }
    };
    let port = if server.port == 0 { 22 } else { server.port };
    let server = CodexSshServer {
        id: server.id,
        name,
        host,
        user,
        port,
        remote_codex_dir,
    };
    let mut store = load_store()?;
    if let Some(existing) = store.servers.iter_mut().find(|s| s.id == server.id) {
        *existing = server.clone();
    } else {
        store.servers.push(server.clone());
    }
    if store.selected_id.is_none() {
        store.selected_id = Some(server.id.clone());
    }
    save_store(&store)?;
    Ok(server)
}

pub fn delete_server(id: &str) -> Result<(), String> {
    let mut store = load_store()?;
    store.servers.retain(|s| s.id != id);
    if store.selected_id.as_deref() == Some(id) {
        store.selected_id = store.servers.first().map(|s| s.id.clone());
    }
    save_store(&store)
}

pub fn select_server(id: &str) -> Result<(), String> {
    let mut store = load_store()?;
    if !store.servers.iter().any(|s| s.id == id) {
        return Err(format!("未找到 SSH 服务器: {id}"));
    }
    store.selected_id = Some(id.to_string());
    save_store(&store)
}

fn ssh_base_args(server: &CodexSshServer) -> Vec<String> {
    vec![
        "-p".to_string(),
        server.port.to_string(),
        "-o".to_string(),
        "BatchMode=yes".to_string(),
        "-o".to_string(),
        "ConnectTimeout=8".to_string(),
        format!("{}@{}", server.user.trim(), server.host.trim()),
    ]
}

fn run_hidden(cmd: &str, args: &[String]) -> Result<String, String> {
    let mut command = Command::new(cmd);
    command.args(args);
    #[cfg(target_os = "windows")]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command
        .output()
        .map_err(|e| format!("执行 {cmd} 失败: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "{cmd} 失败 (code={:?}): {}",
            output.status.code(),
            stderr.trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn test_connection(id: &str) -> Result<String, String> {
    let store = load_store()?;
    let server = store
        .servers
        .iter()
        .find(|s| s.id == id)
        .ok_or_else(|| format!("未找到 SSH 服务器: {id}"))?;
    let mut args = ssh_base_args(server);
    args.push("echo".to_string());
    args.push("cockpit-ssh-ok".to_string());
    let out = run_hidden("ssh", &args)?;
    if !out.contains("cockpit-ssh-ok") {
        return Err(format!("意外输出: {out}"));
    }
    Ok("连接成功".to_string())
}

/// Copy local ~/.codex/auth.json for the active account projection to the remote host.
pub fn sync_current_account(id: &str) -> Result<String, String> {
    let store = load_store()?;
    let server = store
        .servers
        .iter()
        .find(|s| s.id == id)
        .ok_or_else(|| format!("未找到 SSH 服务器: {id}"))?
        .clone();

    let account = codex_account::get_current_account()
        .ok_or_else(|| "当前没有激活的 Codex 账号".to_string())?;

    let home = dirs::home_dir().ok_or("无法获取 Home")?;
    let local_auth = home.join(".codex").join("auth.json");
    if !local_auth.exists() {
        return Err(format!("本地 {} 不存在，请先切号", local_auth.display()));
    }

    let remote_dir = sanitize_remote_codex_dir(&server.remote_codex_dir)?;
    let remote_dir_quoted = shell_single_quote(&remote_dir);

    let mut mkdir_args = ssh_base_args(&server);
    mkdir_args.push(format!("mkdir -p {remote_dir_quoted}"));
    let _ = run_hidden("ssh", &mkdir_args)?;

    let remote_root = format!(
        "{}@{}:{}",
        server.user.trim(),
        server.host.trim(),
        remote_dir.trim_end_matches('/')
    );

    let scp_file = |local: &std::path::Path, remote_name: &str| -> Result<(), String> {
        if !local.exists() {
            return Ok(());
        }
        let remote_target = format!("{remote_root}/{remote_name}");
        let scp_args = vec![
            "-P".to_string(),
            server.port.to_string(),
            "-o".to_string(),
            "BatchMode=yes".to_string(),
            "-o".to_string(),
            "ConnectTimeout=8".to_string(),
            local.display().to_string(),
            remote_target,
        ];
        run_hidden("scp", &scp_args)?;
        Ok(())
    };

    scp_file(&local_auth, "auth.json")?;
    // #1404: also project config.toml when present (best-effort).
    let local_config = home.join(".codex").join("config.toml");
    let mut synced = vec!["auth.json".to_string()];
    if local_config.exists() {
        scp_file(&local_config, "config.toml")?;
        synced.push("config.toml".to_string());
    }

    // Best-effort remote size check for auth.json (not full SHA when scp is used).
    let remote_auth = format!("{}/auth.json", remote_dir.trim_end_matches('/'));
    let remote_auth_quoted = shell_single_quote(&remote_auth);
    let mut stat_args = ssh_base_args(&server);
    stat_args.push(format!("wc -c < {remote_auth_quoted}"));
    match run_hidden("ssh", &stat_args) {
        Ok(remote_len) => {
            let local_len = fs::metadata(&local_auth).map(|m| m.len()).unwrap_or(0);
            let remote_len = remote_len.trim().parse::<u64>().unwrap_or(0);
            if remote_len > 0 && local_len > 0 && remote_len != local_len {
                return Err(format!(
                    "远端 auth.json 大小校验失败: local={local_len}, remote={remote_len}"
                ));
            }
        }
        Err(e) => {
            // Non-fatal: some hosts restrict wc; sync already completed.
            let _ = e;
        }
    }

    // Best-effort remote Codex app-server reload (#1404) so Desktop picks up new auth.
    let reload_cmd = format!(
        "command -v pkill >/dev/null 2>&1 && pkill -f 'codex.*app-server' || true; command -v codex >/dev/null 2>&1 && (codex app-server --help >/dev/null 2>&1 || true)"
    );
    let mut reload_args = ssh_base_args(&server);
    reload_args.push(reload_cmd);
    let reload_note = match run_hidden("ssh", &reload_args) {
        Ok(_) => "；已尝试重载远端 app-server".to_string(),
        Err(e) => format!("；远端重载跳过: {e}"),
    };

    Ok(format!(
        "已同步账号 {} 的 {} 到 {}@{}:{}{}",
        account.email,
        synced.join("+"),
        server.user,
        server.host,
        remote_dir,
        reload_note
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_port_is_22() {
        assert_eq!(default_port(), 22);
    }

    #[test]
    fn ssh_base_args_include_batch_mode() {
        let s = CodexSshServer {
            id: "1".into(),
            name: "t".into(),
            host: "example.com".into(),
            user: "u".into(),
            port: 2222,
            remote_codex_dir: "~/.codex".into(),
        };
        let args = ssh_base_args(&s);
        assert!(args.iter().any(|a| a == "BatchMode=yes"));
        assert!(args.iter().any(|a| a == "2222"));
        assert!(args.iter().any(|a| a == "u@example.com"));
    }

    #[test]
    fn validate_ssh_user_or_host_rejects_at_and_whitespace() {
        assert!(validate_ssh_user_or_host("host", "example.com").is_ok());
        assert_eq!(
            validate_ssh_user_or_host("host", "  example.com  ").as_deref(),
            Ok("example.com")
        );
        assert!(validate_ssh_user_or_host("user", "u@h").is_err());
        assert!(validate_ssh_user_or_host("host", "bad host").is_err());
        assert!(validate_ssh_user_or_host("host", "").is_err());
    }

    #[test]
    fn sanitize_remote_codex_dir_rejects_metacharacters() {
        assert_eq!(sanitize_remote_codex_dir("").unwrap(), "~/.codex");
        assert_eq!(
            sanitize_remote_codex_dir("~/workspace/.codex").unwrap(),
            "~/workspace/.codex"
        );
        assert!(sanitize_remote_codex_dir("~/x;curl evil").is_err());
        assert!(sanitize_remote_codex_dir("~/x && rm -rf /").is_err());
        assert!(sanitize_remote_codex_dir("~/../etc").is_err());
        assert_eq!(shell_single_quote("a'b"), "'a'\\''b'");
        let mkdir = format!("mkdir -p {}", shell_single_quote("~/.codex"));
        assert_eq!(mkdir, "mkdir -p '~/.codex'");
    }
}
