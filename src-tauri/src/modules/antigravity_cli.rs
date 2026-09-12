use crate::models::Account;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntigravityCliStatus {
    pub installed: bool,
    pub executable_path: Option<String>,
    pub version: Option<String>,
    pub auth_backend: String,
    pub current_account_id: Option<String>,
    pub current_email: Option<String>,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AntigravityCliRunOptions {
    pub cwd: Option<String>,
    pub args: Option<Vec<String>>,
    pub terminal: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AntigravityCliRunResult {
    pub pid: Option<u32>,
    pub executable_path: String,
    pub account_id: String,
    pub email: String,
}

fn find_in_path(binary_name: &str) -> Option<PathBuf> {
    if let Some(paths) = std::env::var_os("PATH") {
        for path in std::env::split_paths(&paths) {
            let candidate = path.join(binary_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

pub fn detect_agy_binary() -> Option<PathBuf> {
    if let Some(path) = find_in_path("agy") {
        return Some(path);
    }

    if let Some(home) = dirs::home_dir() {
        let candidates = [
            home.join(".local/bin/agy"),
            home.join(".gemini/antigravity-cli/bin/agy"),
            home.join(".antigravity/antigravity-cli/bin/agy"),
            PathBuf::from("/opt/homebrew/bin/agy"),
            PathBuf::from("/usr/local/bin/agy"),
            PathBuf::from("/usr/bin/agy"),
        ];
        for candidate in candidates {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(path) = find_in_path("agy.exe") {
            return Some(path);
        }
        if let Some(local_app_data) = dirs::data_local_dir() {
            let win_candidate = local_app_data.join("Programs\\antigravity-cli\\agy.exe");
            if win_candidate.is_file() {
                return Some(win_candidate);
            }
        }
        if let Some(home) = dirs::home_dir() {
            let win_candidate = home.join(".local\\bin\\agy.exe");
            if win_candidate.is_file() {
                return Some(win_candidate);
            }
        }
    }

    None
}

pub fn detect_agy_version(bin: &Path) -> Option<String> {
    let output = Command::new(bin).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if !trimmed.is_empty() {
        Some(trimmed.to_string())
    } else {
        None
    }
}

pub fn get_antigravity_cli_status() -> Result<AntigravityCliStatus, String> {
    let bin_path = detect_agy_binary();
    let version = bin_path.as_ref().and_then(|p| detect_agy_version(p));
    let installed = bin_path.is_some();
    let auth_backend = crate::modules::antigravity_credential::detect_auth_backend();

    let current_account_id =
        crate::modules::provider_current_state::get_current_account_id("antigravity_cli")
            .ok()
            .flatten();

    let current_email = if let Some(id) = current_account_id.as_deref() {
        crate::modules::load_account(id).ok().map(|a| a.email)
    } else {
        None
    };

    let diagnostic = if !installed {
        Some("CLI_NOT_INSTALLED: 未在系统 PATH 或标准路径中检测到 agy 可执行文件".to_string())
    } else if auth_backend == "unknown" {
        Some("AUTH_BACKEND_UNAVAILABLE: 未检测到支持的系统凭据存储后端".to_string())
    } else {
        None
    };

    Ok(AntigravityCliStatus {
        installed,
        executable_path: bin_path.map(|p| p.to_string_lossy().to_string()),
        version,
        auth_backend,
        current_account_id,
        current_email,
        diagnostic,
    })
}

/// 操作系统级跨进程文件锁，用于串行化 Cockpit GUI 与 cockpit-cli 等多独立进程对系统凭据库的并发操作。
pub struct CrossProcessSwitchLock {
    file: std::fs::File,
    _path: PathBuf,
}

#[cfg(unix)]
impl CrossProcessSwitchLock {
    pub async fn acquire() -> Result<Self, String> {
        use std::os::unix::io::AsRawFd;
        let data_dir = crate::modules::account::get_data_dir()
            .map_err(|e| format!("获取数据目录失败: {}", e))?;
        let lock_path = data_dir.join("antigravity_cli_switch.lock");

        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|e| format!("无法打开跨进程锁文件 ({}): {}", lock_path.display(), e))?;

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(15);
        let fd = file.as_raw_fd();

        loop {
            let res = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
            if res == 0 {
                return Ok(Self {
                    file,
                    _path: lock_path,
                });
            }
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() != Some(libc::EWOULDBLOCK) && err.raw_os_error() != Some(libc::EAGAIN) {
                return Err(format!("获取跨进程文件锁异常: {}", err));
            }
            if start.elapsed() >= timeout {
                return Err(
                    "SWITCH_LOCK_TIMEOUT: 等待其他进程释放 Antigravity CLI 凭据切换锁超时 (15s)".to_string(),
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }
}

#[cfg(unix)]
impl Drop for CrossProcessSwitchLock {
    fn drop(&mut self) {
        use std::os::unix::io::AsRawFd;
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[cfg(windows)]
impl CrossProcessSwitchLock {
    pub async fn acquire() -> Result<Self, String> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Storage::FileSystem::{
            LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
        };

        let data_dir = crate::modules::account::get_data_dir()
            .map_err(|e| format!("获取数据目录失败: {}", e))?;
        let lock_path = data_dir.join("antigravity_cli_switch.lock");

        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|e| format!("无法打开跨进程锁文件 ({}): {}", lock_path.display(), e))?;

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(15);
        let handle = windows::Win32::Foundation::HANDLE(file.as_raw_handle());

        loop {
            let mut overlapped = unsafe { std::mem::zeroed() };
            let success = unsafe {
                LockFileEx(
                    handle,
                    LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                    0,
                    1,
                    0,
                    &mut overlapped,
                )
            };
            if success.is_ok() {
                return Ok(Self {
                    file,
                    _path: lock_path,
                });
            }
            if start.elapsed() >= timeout {
                return Err(
                    "SWITCH_LOCK_TIMEOUT: 等待其他进程释放 Antigravity CLI 凭据切换锁超时 (15s)".to_string(),
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }
}

#[cfg(windows)]
impl Drop for CrossProcessSwitchLock {
    fn drop(&mut self) {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Storage::FileSystem::UnlockFileEx;
        let handle = windows::Win32::Foundation::HANDLE(self.file.as_raw_handle());
        let mut overlapped = unsafe { std::mem::zeroed() };
        let _ = unsafe { UnlockFileEx(handle, 0, 1, 0, &mut overlapped) };
    }
}

#[cfg(not(any(unix, windows)))]
impl CrossProcessSwitchLock {
    pub async fn acquire() -> Result<Self, String> {
        let data_dir = crate::modules::account::get_data_dir()
            .map_err(|e| format!("获取数据目录失败: {}", e))?;
        let lock_path = data_dir.join("antigravity_cli_switch.lock");
        let file = std::fs::File::create(&lock_path)
            .map_err(|e| format!("无法打开锁文件: {}", e))?;
        Ok(Self { file, _path: lock_path })
    }
}

static ANTIGRAVITY_CLI_SWITCH_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

pub async fn switch_account_transaction(account_id: &str) -> Result<Account, String> {
    // 1. 本进程内协程串行化
    let _switch_guard = ANTIGRAVITY_CLI_SWITCH_LOCK.lock().await;

    // 2. 操作系统级跨进程文件互斥锁（覆盖 Cockpit GUI 与 cockpit-cli 独立进程）
    let _cross_process_guard = CrossProcessSwitchLock::acquire().await?;

    // 未知/不支持的凭据后端必须 fail-closed
    let auth_backend = crate::modules::antigravity_credential::detect_auth_backend();
    if auth_backend == "unknown" {
        return Err("AUTH_BACKEND_UNAVAILABLE: 未检测到可用的系统凭据库，无法在当前系统执行 CLI 账号切换".to_string());
    }

    crate::modules::logger::log_info(&format!(
        "[Antigravity CLI] 开始切换 CLI 账号事务: {}",
        account_id
    ));

    // 1. 加载并准备目标账号（执行必要的 Token 刷新与重试逻辑）
    let mut account = crate::modules::account::prepare_account_for_injection(account_id).await?;
    crate::modules::logger::log_info(&format!(
        "[Antigravity CLI] 准备注入账号凭据: {} (ID: {})",
        account.email, account.id
    ));

    // 2. 快照当前 CLI 凭据
    let snapshot = crate::modules::antigravity_credential::snapshot_antigravity_system_credential()
        .map_err(|e| format!("AUTH_BACKEND_UNAVAILABLE: 读取凭据快照失败: {}", e))?;

    // 3. 写入目标凭据
    if let Err(write_err) =
        crate::modules::antigravity_credential::write_antigravity_system_credential(&account)
    {
        crate::modules::logger::log_error(&format!(
            "[Antigravity CLI] 写入目标凭据失败，正在回滚: {}",
            write_err
        ));
        let rollback_result = crate::modules::antigravity_credential::restore_antigravity_system_credential(
            snapshot.as_deref(),
        );
        if let Err(rollback_err) = rollback_result {
            crate::modules::logger::log_error(&format!(
                "[Antigravity CLI] 严重故障：凭据回滚失败: {}",
                rollback_err
            ));
            return Err(format!(
                "CREDENTIAL_ROLLBACK_FAILED: 凭据写入失败 ({}) 且回滚快照失败 ({})",
                write_err, rollback_err
            ));
        }
        return Err(format!("CREDENTIAL_WRITE_FAILED: {}", write_err));
    }

    // 4. 回读并校验目标凭据
    if let Err(verify_err) =
        crate::modules::antigravity_credential::verify_antigravity_system_credential(&account)
    {
        crate::modules::logger::log_error(&format!(
            "[Antigravity CLI] 凭据校验未通过，正在回滚: {}",
            verify_err
        ));
        let rollback_result = crate::modules::antigravity_credential::restore_antigravity_system_credential(
            snapshot.as_deref(),
        );
        if let Err(rollback_err) = rollback_result {
            crate::modules::logger::log_error(&format!(
                "[Antigravity CLI] 严重故障：校验失败后回滚失败: {}",
                rollback_err
            ));
            return Err(format!(
                "CREDENTIAL_ROLLBACK_FAILED: 凭据校验失败 ({}) 且回滚快照失败 ({})",
                verify_err, rollback_err
            ));
        }
        return Err(format!("CREDENTIAL_VERIFY_FAILED: {}", verify_err));
    }

    // 5. 凭据校验成功后，持久化 antigravity_cli 绑定
    if let Err(e) =
        crate::modules::provider_current_state::set_current_account_id("antigravity_cli", Some(account_id))
    {
        crate::modules::logger::log_warn(&format!(
            "[Antigravity CLI] 更新当前账号映射失败: {}",
            e
        ));
    }
    account.update_last_used();
    let _ = crate::modules::save_account(&account);

    crate::modules::logger::log_info(&format!(
        "[Antigravity CLI] CLI 账号切换事务顺利完成: {}",
        account.email
    ));

    Ok(account)
}

fn posix_quote(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', "'\\''"))
}

fn escape_applescript(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn spawn_fresh_agy_process(
    agy_path: &Path,
    args: &[String],
    cwd: Option<&Path>,
    terminal_preference: Option<&str>,
) -> Result<Option<u32>, String> {
    let pref = terminal_preference.unwrap_or("").trim();

    #[cfg(target_os = "macos")]
    {
        // 如果明确指定 direct，则不通过终端应用包装
        if pref.eq_ignore_ascii_case("direct") {
            let mut cmd = Command::new(agy_path);
            cmd.args(args);
            if let Some(c) = cwd {
                cmd.current_dir(c);
            }
            let child = cmd
                .spawn()
                .map_err(|e| format!("启动 agy 进程失败: {}", e))?;
            return Ok(Some(child.id()));
        }

        // 默认或指定 Terminal 时，通过 macOS Terminal.app 打开交互窗口
        let quoted_bin = posix_quote(&agy_path.to_string_lossy());
        let quoted_args: Vec<String> = args.iter().map(|a| posix_quote(a)).collect();
        let full_exec = if quoted_args.is_empty() {
            quoted_bin
        } else {
            format!("{} {}", quoted_bin, quoted_args.join(" "))
        };

        let shell_command = if let Some(dir) = cwd {
            format!("cd {} && exec {}", posix_quote(&dir.to_string_lossy()), full_exec)
        } else {
            format!("exec {}", full_exec)
        };

        let script = format!(
            "tell application \"Terminal\"\nactivate\ndo script \"{}\"\nend tell",
            escape_applescript(&shell_command)
        );

        let output = Command::new("osascript")
            .args(["-e", &script])
            .output()
            .map_err(|e| format!("执行 osascript 启动 Terminal.app 失败: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Terminal.app 启动失败: {}", stderr.trim()));
        }

        return Ok(None);
    }

    #[cfg(target_os = "windows")]
    {
        if pref.eq_ignore_ascii_case("direct") {
            let mut cmd = Command::new(agy_path);
            cmd.args(args);
            if let Some(c) = cwd {
                cmd.current_dir(c);
            }
            let child = cmd
                .spawn()
                .map_err(|e| format!("启动 agy 进程失败: {}", e))?;
            return Ok(Some(child.id()));
        }

        // Windows 终端启动
        let mut cmd = Command::new("cmd.exe");
        cmd.arg("/c").arg("start").arg("cmd.exe").arg("/k");
        cmd.arg(agy_path);
        cmd.args(args);
        if let Some(c) = cwd {
            cmd.current_dir(c);
        }
        let child = cmd
            .spawn()
            .map_err(|e| format!("启动 Windows 终端失败: {}", e))?;
        return Ok(Some(child.id()));
    }

    #[cfg(target_os = "linux")]
    {
        if pref.eq_ignore_ascii_case("direct") {
            let mut cmd = Command::new(agy_path);
            cmd.args(args);
            if let Some(c) = cwd {
                cmd.current_dir(c);
            }
            let child = cmd
                .spawn()
                .map_err(|e| format!("启动 agy 进程失败: {}", e))?;
            return Ok(Some(child.id()));
        }

        let quoted_bin = posix_quote(&agy_path.to_string_lossy());
        let quoted_args: Vec<String> = args.iter().map(|a| posix_quote(a)).collect();
        let full_exec = if quoted_args.is_empty() {
            quoted_bin
        } else {
            format!("{} {}", quoted_bin, quoted_args.join(" "))
        };

        let shell_command = if let Some(dir) = cwd {
            format!("cd {} && {}; exec bash", posix_quote(&dir.to_string_lossy()), full_exec)
        } else {
            format!("{}; exec bash", full_exec)
        };

        let mut cmd = Command::new("x-terminal-emulator");
        cmd.args(["-e", "bash", "-lc", &shell_command]);
        let child = cmd
            .spawn()
            .map_err(|e| format!("启动 Linux 终端失败: {}", e))?;
        return Ok(Some(child.id()));
    }
}

pub async fn run_antigravity_cli(
    account_id: &str,
    options: Option<AntigravityCliRunOptions>,
) -> Result<AntigravityCliRunResult, String> {
    // 1. 先执行凭据切换事务（如失败，立即终止，绝不启动进程）
    let switched_account = switch_account_transaction(account_id).await?;

    // 2. 解析 agy 可执行文件
    let agy_path = match detect_agy_binary() {
        Some(p) => p,
        None => {
            return Err("CLI_NOT_INSTALLED: 未在系统中找到 agy 命令，请确保已安装 Antigravity CLI 并将其加入系统 PATH。".to_string());
        }
    };

    // 3. 启动进程。注意：此时凭据切换已成功，启动进程失败时不回滚凭据！
    let options = options.unwrap_or_default();
    let args = options.args.unwrap_or_default();
    let cwd = options.cwd.as_deref().map(Path::new);

    match spawn_fresh_agy_process(&agy_path, &args, cwd, options.terminal.as_deref()) {
        Ok(pid) => Ok(AntigravityCliRunResult {
            pid,
            executable_path: agy_path.to_string_lossy().to_string(),
            account_id: switched_account.id,
            email: switched_account.email,
        }),
        Err(err) => {
            crate::modules::logger::log_warn(&format!(
                "[Antigravity CLI] 账号已成功切换为 {}，但进程启动失败: {}",
                switched_account.email, err
            ));
            Err(format!(
                "CLI_LAUNCH_FAILED: 凭据已切换为 {}，但启动 agy 失败: {}",
                switched_account.email, err
            ))
        }
    }
}
