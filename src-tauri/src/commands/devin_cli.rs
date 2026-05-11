use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::modules::config;

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevinCliAccount {
    pub id: String,
    pub name: String,
    pub email: Option<String>,
    pub tier: Option<String>,
    pub plan: Option<String>,
    pub org_id: Option<String>,
    #[serde(default)]
    pub created_at: i64,
    pub last_used_at: Option<i64>,
    #[serde(default)]
    pub needs_login: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct StoreFile {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    accounts: Vec<DevinCliAccount>,
}

impl Default for StoreFile {
    fn default() -> Self {
        Self {
            version: 1,
            accounts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevinAuthStatus {
    pub logged_in: bool,
    pub email: Option<String>,
    pub name: Option<String>,
    pub tier: Option<String>,
    pub plan: Option<String>,
}

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

const DEVIN_CLI_SUBDIR: &str = "devin-cli";
const ACCOUNTS_FILE: &str = "accounts.json";
const PROFILES_SUBDIR: &str = "profiles";

fn base_dir() -> Result<PathBuf, String> {
    let data_dir = config::get_data_dir()?;
    Ok(data_dir.join(DEVIN_CLI_SUBDIR))
}

fn store_path() -> Result<PathBuf, String> {
    Ok(base_dir()?.join(ACCOUNTS_FILE))
}

fn profiles_dir() -> Result<PathBuf, String> {
    Ok(base_dir()?.join(PROFILES_SUBDIR))
}

fn profile_root(id: &str) -> Result<PathBuf, String> {
    validate_id(id)?;
    Ok(profiles_dir()?.join(id))
}

fn profile_data_home(id: &str) -> Result<PathBuf, String> {
    Ok(profile_root(id)?.join("data"))
}

fn profile_config_home(id: &str) -> Result<PathBuf, String> {
    Ok(profile_root(id)?.join("config"))
}

fn devin_data_dir(id: &str) -> Result<PathBuf, String> {
    Ok(profile_data_home(id)?.join("devin"))
}

fn devin_config_dir(id: &str) -> Result<PathBuf, String> {
    Ok(profile_config_home(id)?.join("devin"))
}

fn credentials_toml_path(id: &str) -> Result<PathBuf, String> {
    Ok(devin_data_dir(id)?.join("credentials.toml"))
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

fn validate_id(id: &str) -> Result<(), String> {
    if id.trim().is_empty() {
        return Err("Account ID must not be empty".to_string());
    }
    if id.contains('/') || id.contains('\\') || id.contains("..") {
        return Err("Account ID contains invalid characters".to_string());
    }
    Ok(())
}

fn validate_account_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim().to_string();
    if trimmed.is_empty() {
        return Err("Account name must not be empty".to_string());
    }
    if trimmed.len() > 64 {
        return Err("Account name is too long (max 64 characters)".to_string());
    }
    if !trimmed
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        return Err(
            "Account name may only contain letters, digits, underscores, and hyphens".to_string(),
        );
    }
    Ok(trimmed)
}

// ---------------------------------------------------------------------------
// Store I/O
// ---------------------------------------------------------------------------

fn load_store() -> Result<StoreFile, String> {
    let path = store_path()?;
    if !path.exists() {
        return Ok(StoreFile::default());
    }
    let content = fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read Devin CLI accounts: {}", e))?;
    if content.trim().is_empty() {
        return Ok(StoreFile::default());
    }
    serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse Devin CLI accounts: {}", e))
}

fn save_store(store: &StoreFile) -> Result<(), String> {
    let dir = base_dir()?;
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create Devin CLI data dir: {}", e))?;
    let json = serde_json::to_string_pretty(store)
        .map_err(|e| format!("Failed to serialize accounts: {}", e))?;
    fs::write(store_path()?, json)
        .map_err(|e| format!("Failed to write accounts file: {}", e))
}

fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

// ---------------------------------------------------------------------------
// Profile directory management
// ---------------------------------------------------------------------------

fn ensure_profile_dirs(id: &str) -> Result<PathBuf, String> {
    let root = profile_root(id)?;
    let data_home = profile_data_home(id)?;
    let config_home = profile_config_home(id)?;
    let devin_data = devin_data_dir(id)?;
    let devin_config = devin_config_dir(id)?;

    for dir in [&root, &data_home, &config_home, &devin_data, &devin_config] {
        fs::create_dir_all(dir)
            .map_err(|e| format!("Failed to create profile directory {}: {}", dir.display(), e))?;
    }

    // Symlink shared CLI state (sessions, logs, workspaces) from the user's
    // normal Devin data directory into the profile, so each account shares
    // the same session history but has isolated credentials.
    link_shared_cli_state(&devin_data)?;

    Ok(root)
}

fn remove_profile_dir(id: &str) -> Result<(), String> {
    let root = profile_root(id)?;
    if root.exists() {
        fs::remove_dir_all(&root)
            .map_err(|e| format!("Failed to remove profile directory: {}", e))?;
    }
    Ok(())
}

/// Symlink `~/.local/share/devin/cli` (or the Windows/macOS equivalent) into
/// the profile's devin data dir so that shared CLI state (sessions, logs,
/// workspaces) is available across all profiles while credentials stay
/// isolated in each profile's own `credentials.toml`.
fn link_shared_cli_state(devin_data: &Path) -> Result<(), String> {
    let shared_cli_dir = resolve_shared_devin_cli_dir();
    let profile_cli_dir = devin_data.join("cli");

    if shared_cli_dir.is_none() {
        // Shared dir does not exist yet — nothing to link.
        return Ok(());
    }
    let shared = shared_cli_dir.unwrap();

    // Create the shared dir if it doesn't exist.
    fs::create_dir_all(&shared)
        .map_err(|e| format!("Failed to create shared Devin CLI dir: {}", e))?;

    if profile_cli_dir.exists() || profile_cli_dir.is_symlink() {
        // Already exists (dir or symlink) — leave it as-is.
        return Ok(());
    }

    // Create symlink: profile_cli_dir → shared_cli_dir
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&shared, &profile_cli_dir)
            .map_err(|e| format!("Failed to create CLI symlink: {}", e))?;
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(&shared, &profile_cli_dir)
            .or_else(|_| {
                // Symlinks on Windows may require admin privileges.
                // Fall back to a directory junction.
                Command::new("cmd")
                    .args([
                        "/C",
                        "mklink",
                        "/J",
                        &profile_cli_dir.to_string_lossy(),
                        &shared.to_string_lossy(),
                    ])
                    .output()
                    .map(|_| ())
                    .map_err(|e2| format!("Failed to create CLI junction: {}", e2))
            })
            .map_err(|e| format!("Failed to create CLI symlink/junction: {}", e))?;
    }

    Ok(())
}

/// Resolve the user's normal Devin CLI data directory.
/// On Unix: `~/.local/share/devin/cli` (or `$XDG_DATA_HOME/devin/cli`)
/// On Windows: `%APPDATA%\devin\cli`
fn resolve_shared_devin_cli_dir() -> Option<PathBuf> {
    let base = if cfg!(target_os = "windows") {
        dirs::data_dir()
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .and_then(|v| {
                let s = v.to_string_lossy().trim().to_string();
                if s.is_empty() {
                    None
                } else {
                    Some(PathBuf::from(s))
                }
            })
            .or_else(dirs::data_dir)
    };
    base.map(|b| b.join("devin").join("cli"))
}

// ---------------------------------------------------------------------------
// Environment builder
// ---------------------------------------------------------------------------

/// Build environment variables for running Devin under an isolated profile.
/// Sets XDG_DATA_HOME and XDG_CONFIG_HOME to the profile's directories so
/// that `devin` reads/writes credentials in the isolated location.
fn build_profile_env(id: &str) -> Result<std::collections::HashMap<String, String>, String> {
    let data_home = profile_data_home(id)?;
    let config_home = profile_config_home(id)?;

    let mut env: std::collections::HashMap<String, String> = std::env::vars().collect();
    env.insert("XDG_DATA_HOME".to_string(), data_home.to_string_lossy().to_string());
    env.insert("XDG_CONFIG_HOME".to_string(), config_home.to_string_lossy().to_string());

    // On Windows, also set APPDATA override so that Devin finds its config
    // in the profile directory rather than the global APPDATA.
    #[cfg(target_os = "windows")]
    {
        env.insert("APPDATA".to_string(), data_home.to_string_lossy().to_string());
    }

    Ok(env)
}

// ---------------------------------------------------------------------------
// Devin binary discovery
// ---------------------------------------------------------------------------

fn which_devin() -> Option<String> {
    let candidates = if cfg!(target_os = "windows") {
        vec!["devin.cmd", "devin.exe", "devin"]
    } else {
        vec!["devin"]
    };

    let paths = std::env::var_os("PATH")?;
    for path_dir in std::env::split_paths(&paths) {
        for candidate in &candidates {
            let full_path = path_dir.join(candidate);
            if full_path.is_file() {
                return Some(full_path.to_string_lossy().to_string());
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Shell quoting helpers
// ---------------------------------------------------------------------------

#[cfg(not(target_os = "windows"))]
fn posix_shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    let needs_quote = value.chars().any(|ch| {
        ch.is_whitespace()
            || matches!(
                ch,
                '\'' | '"' | '$' | '`' | '\\' | '&' | '|' | ';' | '<' | '>' | '(' | ')'
            )
    });
    if !needs_quote {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(target_os = "windows")]
fn windows_cmd_quote(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_string();
    }
    let needs_quote = value
        .chars()
        .any(|ch| ch.is_whitespace() || matches!(ch, '"' | '^' | '&' | '|' | '<' | '>' | '%'));
    if !needs_quote {
        return value.to_string();
    }
    format!("\"{}\"", value.replace('"', "\\\""))
}

#[cfg(target_os = "macos")]
fn escape_applescript(value: &str) -> String {
    value.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`")
}

fn build_devin_command_line(devin_path: &str, args: &[String]) -> String {
    let mut parts = Vec::new();
    #[cfg(target_os = "windows")]
    {
        parts.push(windows_cmd_quote(devin_path));
    }
    #[cfg(not(target_os = "windows"))]
    {
        parts.push(posix_shell_quote(devin_path));
    }
    for arg in args {
        #[cfg(target_os = "windows")]
        {
            parts.push(windows_cmd_quote(arg));
        }
        #[cfg(not(target_os = "windows"))]
        {
            parts.push(posix_shell_quote(arg));
        }
    }
    parts.join(" ")
}

// ---------------------------------------------------------------------------
// Terminal spawning (cross-platform)
// ---------------------------------------------------------------------------

fn spawn_in_terminal(
    command_line: &str,
    env_vars: &std::collections::HashMap<String, String>,
    terminal: Option<&str>,
) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        let app_name = terminal.unwrap_or("Terminal");
        // Build env export statements for the shell.
        let env_exports = build_env_export_line(env_vars);
        let full_command = format!("{}; {}", env_exports, command_line);

        let script = if app_name == "iTerm" {
            format!(
                "tell application \"iTerm\"\n  create window with default profile\n  tell current session of current window\n    write text \"{}\"\n  end tell\nend tell",
                escape_applescript(&full_command)
            )
        } else {
            format!(
                "tell application \"Terminal\"\n  do script \"{}\"\n  activate\nend tell",
                escape_applescript(&full_command)
            )
        };

        let output = Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .map_err(|e| format!("Failed to open terminal ({}): {}", app_name, e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Terminal execution failed: {}", stderr.trim()));
        }
        return Ok(format!("Opened Devin in {}", app_name));
    }

    #[cfg(target_os = "windows")]
    {
        let terminal = terminal.unwrap_or("cmd");
        // Build set commands for env vars.
        let env_set_cmds = build_windows_env_set_cmds(env_vars);
        let full_command = format!("{} & {}", env_set_cmds, command_line);

        if terminal == "PowerShell" || terminal == "powershell" {
            let ps_env = build_powershell_env_cmds(env_vars);
            let full_command = format!("{}; {}", ps_env, command_line);
            Command::new("powershell")
                .args(["-NoExit", "-Command", &full_command])
                .spawn()
                .map_err(|e| format!("Failed to open PowerShell: {}", e))?;
        } else if terminal == "pwsh" {
            let ps_env = build_powershell_env_cmds(env_vars);
            let full_command = format!("{}; {}", ps_env, command_line);
            Command::new("pwsh")
                .args(["-NoExit", "-Command", &full_command])
                .spawn()
                .map_err(|e| format!("Failed to open pwsh: {}", e))?;
        } else if terminal == "wt" {
            Command::new("wt")
                .args([
                    "-p",
                    "Command Prompt",
                    "cmd",
                    "/K",
                    &full_command,
                ])
                .spawn()
                .map_err(|e| format!("Failed to open Windows Terminal: {}", e))?;
        } else {
            Command::new("cmd")
                .args(["/C", "start", "", "cmd", "/K", &full_command])
                .spawn()
                .map_err(|e| format!("Failed to open cmd: {}", e))?;
        }
        return Ok("Opened Devin in terminal".to_string());
    }

    #[cfg(target_os = "linux")]
    {
        let env_exports = build_env_export_line(env_vars);
        let shell_command = format!("{}; {}; exec bash", env_exports, command_line);
        let terminal = terminal.unwrap_or("system");

        let mut cmd = if terminal == "system" || terminal.is_empty() {
            Command::new("x-terminal-emulator")
        } else {
            Command::new(terminal)
        };

        cmd.args(["-e", "bash", "-lc", &shell_command])
            .spawn()
            .or_else(|_| {
                if terminal == "system" || terminal.is_empty() {
                    Command::new("gnome-terminal")
                        .args(["--", "bash", "-lc", &shell_command])
                        .spawn()
                } else {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "Terminal not found",
                    ))
                }
            })
            .or_else(|_| {
                if terminal == "system" || terminal.is_empty() {
                    Command::new("konsole")
                        .args(["-e", "bash", "-lc", &shell_command])
                        .spawn()
                } else {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "Terminal not found",
                    ))
                }
            })
            .or_else(|_| Command::new("sh").args(["-lc", &shell_command]).spawn())
            .map_err(|e| format!("Failed to open terminal: {}", e))?;
        return Ok("Opened Devin in terminal".to_string());
    }

    #[allow(unreachable_code)]
    Ok("Opened".to_string())
}

#[cfg(unix)]
fn build_env_export_line(env_vars: &std::collections::HashMap<String, String>) -> String {
    env_vars
        .iter()
        .map(|(k, v)| format!("export {}={}", posix_shell_quote(k), posix_shell_quote(v)))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(target_os = "windows")]
fn build_windows_env_set_cmds(env_vars: &std::collections::HashMap<String, String>) -> String {
    env_vars
        .iter()
        .map(|(k, v)| format!("set {}={}", k, v))
        .collect::<Vec<_>>()
        .join(" & ")
}

#[cfg(target_os = "windows")]
fn build_powershell_env_cmds(env_vars: &std::collections::HashMap<String, String>) -> String {
    env_vars
        .iter()
        .map(|(k, v)| format!("$env:{}='{}'", k, v.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join("; ")
}

// ---------------------------------------------------------------------------
// Auth status parsing
// ---------------------------------------------------------------------------

fn parse_auth_status(output: &str) -> DevinAuthStatus {
    let not_logged_in = regex::Regex::new(r"(?i)not logged in")
        .map(|re| re.is_match(output))
        .unwrap_or(false);
    let logged_in = !not_logged_in
        && regex::Regex::new(r"(?i)logged in")
            .map(|re| re.is_match(output))
            .unwrap_or(false);

    DevinAuthStatus {
        logged_in,
        email: field_from_output(output, "Email"),
        name: field_from_output(output, "Name"),
        tier: field_from_output(output, "Tier"),
        plan: field_from_output(output, "Plan"),
    }
}

fn field_from_output(output: &str, label: &str) -> Option<String> {
    let pattern = regex::Regex::new(&format!(r"(?im)^\s*{}:\s*(.+)$", label)).ok()?;
    pattern
        .captures(output)
        .and_then(|caps| caps.get(1).map(|m| m.as_str().trim().to_string()))
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// List all Devin CLI accounts
#[tauri::command]
pub fn devin_cli_list_accounts() -> Result<Vec<DevinCliAccount>, String> {
    let store = load_store()?;
    Ok(store.accounts)
}

/// Check if the `devin` CLI is installed
#[tauri::command]
pub fn devin_cli_is_devin_installed() -> Result<bool, String> {
    Ok(which_devin().is_some())
}

/// Add a new Devin CLI account (creates isolated profile directory)
#[tauri::command]
pub fn devin_cli_add_account(name: String) -> Result<DevinCliAccount, String> {
    let trimmed = validate_account_name(&name)?;
    let mut store = load_store()?;

    if store.accounts.iter().any(|a| a.name == trimmed) {
        return Err(format!("Account '{}' already exists", trimmed));
    }

    let id = Uuid::new_v4().to_string();
    let account = DevinCliAccount {
        id: id.clone(),
        name: trimmed,
        email: None,
        tier: None,
        plan: None,
        org_id: None,
        created_at: now_seconds(),
        last_used_at: None,
        needs_login: true,
    };

    // Create the isolated profile directory structure.
    ensure_profile_dirs(&id)?;

    store.accounts.push(account.clone());
    save_store(&store)?;

    Ok(account)
}

/// Remove a Devin CLI account (deletes profile directory and credentials)
#[tauri::command]
pub fn devin_cli_remove_account(id: String) -> Result<DevinCliAccount, String> {
    let mut store = load_store()?;
    let index = store
        .accounts
        .iter()
        .position(|a| a.id == id)
        .ok_or_else(|| format!("Account not found: {}", id))?;

    let removed = store.accounts.remove(index);
    remove_profile_dir(&removed.id)?;
    save_store(&store)?;

    Ok(removed)
}

/// Rename a Devin CLI account
#[tauri::command]
pub fn devin_cli_rename_account(id: String, new_name: String) -> Result<DevinCliAccount, String> {
    let trimmed = validate_account_name(&new_name)?;
    let mut store = load_store()?;

    let index = store
        .accounts
        .iter()
        .position(|a| a.id == id)
        .ok_or_else(|| format!("Account not found: {}", id))?;

    if store
        .accounts
        .iter()
        .any(|a| a.id != id && a.name == trimmed)
    {
        return Err(format!("Account '{}' already exists", trimmed));
    }

    store.accounts[index].name = trimmed;
    let account = store.accounts[index].clone();
    save_store(&store)?;

    Ok(account)
}

/// Run `devin auth login` for a specific account in a terminal
#[tauri::command]
pub fn devin_cli_login_account(
    id: String,
    terminal: Option<String>,
) -> Result<String, String> {
    let store = load_store()?;
    let account = store
        .accounts
        .iter()
        .find(|a| a.id == id)
        .ok_or_else(|| format!("Account not found: {}", id))?;

    let devin_path = which_devin().ok_or(
        "devin CLI not found in PATH. Please install Devin CLI first.",
    )?;

    // Ensure profile dirs exist before login.
    ensure_profile_dirs(&account.id)?;

    let env_vars = build_profile_env(&account.id)?;
    let command_line = build_devin_command_line(&devin_path, &["auth".to_string(), "login".to_string()]);

    spawn_in_terminal(&command_line, &env_vars, terminal.as_deref())
}

/// Run `devin` with a specific account's isolated environment in a terminal
#[tauri::command]
pub fn devin_cli_use_account(
    id: String,
    args: Vec<String>,
    terminal: Option<String>,
) -> Result<String, String> {
    let mut store = load_store()?;
    let index = store
        .accounts
        .iter()
        .position(|a| a.id == id)
        .ok_or_else(|| format!("Account not found: {}", id))?;

    if store.accounts[index].needs_login {
        return Err(format!(
            "Account '{}' needs login first. Use the Login button to authenticate.",
            store.accounts[index].name
        ));
    }

    let devin_path = which_devin().ok_or(
        "devin CLI not found in PATH. Please install Devin CLI first.",
    )?;

    // Ensure profile dirs exist.
    ensure_profile_dirs(&store.accounts[index].id)?;

    // Mark as used.
    store.accounts[index].last_used_at = Some(now_seconds());
    let account_id = store.accounts[index].id.clone();
    let account_name = store.accounts[index].name.clone();
    save_store(&store)?;

    let env_vars = build_profile_env(&account_id)?;

    // Validate args.
    if args.len() > 32 {
        return Err("Too many arguments".to_string());
    }
    for arg in &args {
        if arg.len() > 512 {
            return Err("Argument is too long".to_string());
        }
        if arg.contains('\n') || arg.contains('\r') || arg.contains('\0') {
            return Err("Argument contains unsupported control characters".to_string());
        }
    }

    let command_line = build_devin_command_line(&devin_path, &args);

    spawn_in_terminal(&command_line, &env_vars, terminal.as_deref())
        .map(|msg| format!("{} (account: {})", msg, account_name))
}

/// Check auth status for a specific account by running `devin auth status`
#[tauri::command]
pub async fn devin_cli_check_auth_status(id: String) -> Result<DevinAuthStatus, String> {
    let store = load_store()?;
    let account = store
        .accounts
        .iter()
        .find(|a| a.id == id)
        .ok_or_else(|| format!("Account not found: {}", id))?;

    let devin_path = which_devin().ok_or(
        "devin CLI not found in PATH. Please install Devin CLI first.",
    )?;

    // Ensure profile dirs exist.
    ensure_profile_dirs(&account.id)?;

    let env_vars = build_profile_env(&account.id)?;

    let mut cmd = Command::new(&devin_path);
    cmd.args(["auth", "status"]);
    for (k, v) in &env_vars {
        cmd.env(k, v);
    }

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run 'devin auth status': {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}\n{}", stdout, stderr);

    let status = parse_auth_status(&combined);

    // Update account metadata from auth status.
    if status.logged_in {
        let mut store = load_store()?;
        if let Some(acc) = store.accounts.iter_mut().find(|a| a.id == id) {
            acc.needs_login = false;
            if status.email.is_some() {
                acc.email = status.email.clone();
            }
            if status.tier.is_some() {
                acc.tier = status.tier.clone();
            }
            if status.plan.is_some() {
                acc.plan = status.plan.clone();
            }
            let _ = save_store(&store);
        }
    } else {
        let mut store = load_store()?;
        if let Some(acc) = store.accounts.iter_mut().find(|a| a.id == id) {
            acc.needs_login = true;
            let _ = save_store(&store);
        }
    }

    Ok(status)
}

/// Sync auth metadata for all accounts (checks `devin auth status` for each)
#[tauri::command]
pub async fn devin_cli_sync_all_accounts() -> Result<Vec<DevinCliAccount>, String> {
    let devin_path = which_devin().ok_or(
        "devin CLI not found in PATH. Please install Devin CLI first.",
    )?;

    let mut store = load_store()?;

    for account in &mut store.accounts {
        let env_vars = match build_profile_env(&account.id) {
            Ok(env) => env,
            Err(_) => continue,
        };

        // Ensure profile dirs exist.
        let _ = ensure_profile_dirs(&account.id);

        let mut cmd = Command::new(&devin_path);
        cmd.args(["auth", "status"]);
        for (k, v) in &env_vars {
            cmd.env(k, v);
        }

        match cmd.output() {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = format!("{}\n{}", stdout, stderr);
                let status = parse_auth_status(&combined);

                account.needs_login = !status.logged_in;
                if status.email.is_some() {
                    account.email = status.email.clone();
                }
                if status.tier.is_some() {
                    account.tier = status.tier.clone();
                }
                if status.plan.is_some() {
                    account.plan = status.plan.clone();
                }
            }
            Err(_) => {
                account.needs_login = true;
            }
        }
    }

    let accounts = store.accounts.clone();
    let _ = save_store(&store);

    Ok(accounts)
}
