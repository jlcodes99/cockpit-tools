//! WorkBuddy AI 授权浏览器：以「独立 profile + 无痕/InPrivate」方式打开授权 URL。
//!
//! 设计要点（不依赖真实浏览器即可单测纯函数）：
//! - 不使用 shell、不对进程名做模糊匹配；只启动我们亲手 spawn 的这一个子进程。
//! - 不在启动失败时回退到系统默认/普通浏览器；找不到 Chrome/Edge 即报错。
//! - 每次调用都生成唯一 uuid 目录作为 `--user-data-dir`，`create_dir` 而非复用。
//! - `--user-data-dir=PATH` 作为单个参数传递，路径含空格也安全。
//! - 退出（用户关闭 / `is_active` 失活 / 超过最长存活时间）时只删除“本次创建的
//!   唯一目录”，绝不扫描全局临时目录；删除失败只记录固定告警（不含 URL）。

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use crate::modules::logger::log_warn;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static SHUTDOWN: AtomicBool = AtomicBool::new(false);
static ACTIVE_BROWSERS: AtomicUsize = AtomicUsize::new(0);

struct BrowserGuard;
impl Drop for BrowserGuard {
    fn drop(&mut self) {
        ACTIVE_BROWSERS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Give owned browser monitors time to close/clean before normal app exit.
pub fn shutdown() {
    SHUTDOWN.store(true, Ordering::SeqCst);
    let start = Instant::now();
    while ACTIVE_BROWSERS.load(Ordering::SeqCst) > 0 && start.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(50));
    }
    if ACTIVE_BROWSERS.load(Ordering::SeqCst) > 0 {
        log_warn("[WorkBuddyAuthBrowser] 退出清理超时，临时 profile 可能保留");
    }
}

/// 极短时间检查：浏览器若在此窗口内退出，视为启动失败并回报调用者。
const QUICK_EXIT_CHECK_MS: u64 = 150;
/// 生命周期监控轮询间隔。
const WAKE_INTERVAL_MS: u64 = 500;
/// 浏览器最长存活时间，超时后强制结束（仅此子进程）。
const MAX_LIFETIME_SECS: u64 = 600;

/// 一个候选浏览器的可执行文件候选路径集合与对应的无痕参数。
struct BrowserCandidate {
    /// 无痕参数：Chrome 为 `--incognito`，Edge 为 `--inprivate`。
    private_flag: &'static str,
    /// 可执行文件候选（Windows/macOS 为绝对路径；Linux 为需经 PATH 查找的裸文件名）。
    candidates: Vec<PathBuf>,
}

/// 返回当前平台下所有候选浏览器（按优先级排序）。
#[cfg(target_os = "windows")]
fn candidate_browsers() -> Vec<BrowserCandidate> {
    let entries: &[(Option<String>, &str, &str)] = &[
        (
            std::env::var("ProgramFiles").ok(),
            "Google\\Chrome\\Application\\chrome.exe",
            "--incognito",
        ),
        (
            std::env::var("ProgramFiles(x86)").ok(),
            "Google\\Chrome\\Application\\chrome.exe",
            "--incognito",
        ),
        (
            std::env::var("LOCALAPPDATA").ok(),
            "Google\\Chrome\\Application\\chrome.exe",
            "--incognito",
        ),
        (
            std::env::var("ProgramFiles").ok(),
            "Microsoft\\Edge\\Application\\msedge.exe",
            "--inprivate",
        ),
        (
            std::env::var("ProgramFiles(x86)").ok(),
            "Microsoft\\Edge\\Application\\msedge.exe",
            "--inprivate",
        ),
        (
            std::env::var("LOCALAPPDATA").ok(),
            "Microsoft\\Edge\\Application\\msedge.exe",
            "--inprivate",
        ),
    ];

    let mut list = Vec::new();
    for &(ref base, relative, flag) in entries {
        if let Some(base) = base {
            // 使用 join 拼接常见安装路径。
            let path = PathBuf::from(base).join(relative);
            list.push(BrowserCandidate {
                private_flag: flag,
                candidates: vec![path],
            });
        }
    }
    list
}

#[cfg(target_os = "macos")]
fn candidate_browsers() -> Vec<BrowserCandidate> {
    vec![
        BrowserCandidate {
            private_flag: "--incognito",
            candidates: vec![PathBuf::from(
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            )],
        },
        BrowserCandidate {
            private_flag: "--inprivate",
            candidates: vec![PathBuf::from(
                "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            )],
        },
    ]
}

#[cfg(target_os = "linux")]
fn candidate_browsers() -> Vec<BrowserCandidate> {
    vec![
        BrowserCandidate {
            private_flag: "--incognito",
            candidates: vec![PathBuf::from("google-chrome")],
        },
        BrowserCandidate {
            private_flag: "--incognito",
            candidates: vec![PathBuf::from("google-chrome-stable")],
        },
        BrowserCandidate {
            private_flag: "--incognito",
            candidates: vec![PathBuf::from("chromium")],
        },
        BrowserCandidate {
            private_flag: "--incognito",
            candidates: vec![PathBuf::from("chromium-browser")],
        },
        BrowserCandidate {
            private_flag: "--inprivate",
            candidates: vec![PathBuf::from("microsoft-edge")],
        },
        BrowserCandidate {
            private_flag: "--inprivate",
            candidates: vec![PathBuf::from("microsoft-edge-stable")],
        },
    ]
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn candidate_browsers() -> Vec<BrowserCandidate> {
    Vec::new()
}

/// 解析单个候选可执行文件：绝对/相对路径直接检查存在性；裸文件名则经 PATH 查找。
/// 不启动进程、不做模糊匹配。
fn resolve_executable(candidate: &Path) -> Option<PathBuf> {
    if candidate.is_absolute() && candidate.is_file() {
        return Some(candidate.to_path_buf());
    }

    // 仅当是裸文件名（不含任何目录分量）时，才通过 PATH 查找可执行文件。
    let is_bare = candidate
        .file_name()
        .map_or(false, |name| name == candidate.as_os_str());
    if is_bare {
        if let Ok(paths) = std::env::var("PATH") {
            for dir in std::env::split_paths(&paths) {
                let probe = dir.join(candidate);
                if dir.is_absolute() && probe.is_file() {
                    return Some(probe);
                }
            }
        }
    }
    None
}

/// 在候选列表中找到第一个存在的浏览器可执行文件，返回（路径, 无痕参数）。
fn find_browser() -> Option<(PathBuf, &'static str)> {
    for spec in candidate_browsers() {
        for candidate in &spec.candidates {
            if let Some(executable) = resolve_executable(candidate) {
                return Some((executable, spec.private_flag));
            }
        }
    }
    None
}

/// 生成唯一 profile 目录名（每次调用都不同）。纯函数，便于测试参数隔离。
pub(crate) fn make_profile_dir_name() -> String {
    format!("workbuddy-auth-{}", uuid::Uuid::new_v4().simple())
}

/// 在系统临时目录下创建本次唯一 profile 目录（unix 下权限 0700，不覆盖已有目录）。
fn create_profile_dir() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(make_profile_dir_name());

    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700);
        builder
            .create(&dir)
            .map_err(|e| format!("创建授权浏览器 profile 目录失败: {}", e))?;
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir(&dir).map_err(|e| format!("创建授权浏览器 profile 目录失败: {}", e))?;
    }

    Ok(dir)
}

/// 构建启动参数（纯函数，不触碰真实浏览器）。
/// `--user-data-dir=PATH` 为单个参数，路径含空格也安全；URL 作为 `--new-window` 的下
/// 一个独立参数传递。
pub(crate) fn build_browser_args(
    private_flag: &str,
    profile_dir: &Path,
    auth_url: &str,
) -> Vec<OsString> {
    let mut profile_arg = OsString::from("--user-data-dir=");
    profile_arg.push(profile_dir.as_os_str());
    vec![
        profile_arg,
        OsString::from(private_flag),
        OsString::from("--no-first-run"),
        OsString::from("--no-default-browser-check"),
        OsString::from("--disable-background-mode"),
        OsString::from("--disable-sync"),
        OsString::from("--new-window"),
        OsString::from(auth_url),
    ]
}

/// 仅删除“本次创建的”唯一目录。删除失败只记录固定告警（不含 URL），不做全局清理。
fn cleanup_profile_dir(profile_dir: &Path) {
    if let Err(_e) = std::fs::remove_dir_all(profile_dir) {
        // 固定消息，刻意不含 auth_url；明确说明该目录被保留。
        log_warn(&format!(
            "[WorkBuddyAuthBrowser] 临时授权浏览器 profile 目录无法删除，已保留: {}",
            profile_dir.display()
        ));
    }
}

/// 打开一个独立的无痕授权浏览器窗口。
///
/// - 每次使用独立 profile 目录 + 无痕/InPrivate 模式，绝不以普通浏览器回退。
/// - `is_active` 返回 `false` 时仅结束我们启动的这个子进程（不会误杀用户其它浏览器）。
/// - 返回 `Ok(())` 表示浏览器已成功启动并持续运行；`Err(String)` 表示启动/快速退出失败，
///   且已清理本次创建的目录。
pub fn open_fresh(
    auth_url: &str,
    is_active: impl Fn() -> bool + Send + 'static,
) -> Result<(), String> {
    ACTIVE_BROWSERS.fetch_add(1, Ordering::SeqCst);
    let guard = BrowserGuard;
    if SHUTDOWN.load(Ordering::SeqCst) || !is_active() {
        return Err("授权会话已结束，请重新添加账号".to_string());
    }
    let profile_dir = create_profile_dir()?;

    let (executable, private_flag) = match find_browser() {
        Some(found) => found,
        None => {
            cleanup_profile_dir(&profile_dir);
            return Err("未找到可用的 Chrome / Edge 浏览器，无法打开独立授权窗口".to_string());
        }
    };

    let args: Vec<OsString> = build_browser_args(private_flag, &profile_dir, auth_url);

    let mut cmd = std::process::Command::new(executable);
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            cleanup_profile_dir(&profile_dir);
            return Err(format!("启动授权浏览器失败: {}", e));
        }
    };

    let browser_name = if private_flag == "--incognito" {
        "chrome"
    } else {
        "edge"
    };
    crate::modules::logger::log_info(&format!(
        "[WorkBuddyAuthBrowser] 已打开独立授权窗口: browser={}",
        browser_name
    ));

    // 把子进程与生命周期交给专用后台线程监控，调用方在此短等启动结果后返回。
    let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();

    std::thread::spawn(move || {
        let _guard = guard;
        // 短检查：浏览器若在极短时间内退出，视为启动失败，回报调用者。
        std::thread::sleep(Duration::from_millis(QUICK_EXIT_CHECK_MS));
        match child.try_wait() {
            Ok(Some(status)) => {
                cleanup_profile_dir(&profile_dir);
                let _ = tx.send(Err(format!("授权浏览器快速退出: {}", status)));
                return;
            }
            Ok(None) => {
                let _ = tx.send(Ok(()));
            }
            Err(e) => {
                if child.kill().is_ok() && child.wait().is_ok() {
                    cleanup_profile_dir(&profile_dir);
                } else {
                    log_warn("[WorkBuddyAuthBrowser] 无法确认授权浏览器已退出，保留临时 profile");
                }
                let _ = tx.send(Err(format!("授权浏览器状态查询失败: {}", e)));
                return;
            }
        }

        // 生命周期监控：子进程退出 / 失活 / 超时 时清理并结束线程。
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => {
                    cleanup_profile_dir(&profile_dir);
                    return;
                }
                Ok(None) => {}
                Err(_) => {
                    if child.kill().is_ok() && child.wait().is_ok() {
                        cleanup_profile_dir(&profile_dir);
                    } else {
                        log_warn("[WorkBuddyAuthBrowser] 无法确认授权浏览器已退出，保留临时 profile");
                    }
                    return;
                }
            }

            if SHUTDOWN.load(Ordering::SeqCst) || !is_active() {
                // 仅结束我们启动的这个子进程，不会误杀用户现有浏览器。
                if child.kill().is_ok() && child.wait().is_ok() {
                    cleanup_profile_dir(&profile_dir);
                } else {
                    log_warn("[WorkBuddyAuthBrowser] 无法确认授权浏览器已退出，保留临时 profile");
                }
                return;
            }

            if start.elapsed() >= Duration::from_secs(MAX_LIFETIME_SECS) {
                if child.kill().is_ok() && child.wait().is_ok() {
                    cleanup_profile_dir(&profile_dir);
                } else {
                    log_warn("[WorkBuddyAuthBrowser] 无法确认授权浏览器已退出，保留临时 profile");
                }
                return;
            }

            std::thread::sleep(Duration::from_millis(WAKE_INTERVAL_MS));
        }
    });

    let result = rx
        .recv()
        .map_err(|_| "授权浏览器监控线程异常".to_string())?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_dir_name_is_unique_per_call() {
        let a = make_profile_dir_name();
        let b = make_profile_dir_name();
        assert_ne!(a, b, "每次调用应生成不同的 profile 目录名");
        assert!(a.starts_with("workbuddy-auth-"), "目录名前缀应稳定");
    }

    #[test]
    fn browser_args_isolate_profile_per_call() {
        let url = "https://example.com/auth";
        let dir_a = std::env::temp_dir().join(make_profile_dir_name());
        let dir_b = std::env::temp_dir().join(make_profile_dir_name());
        let args_a = build_browser_args("--incognito", &dir_a, url);
        let args_b = build_browser_args("--incognito", &dir_b, url);

        let ud_a = args_a
            .iter()
            .find(|a| a.to_string_lossy().starts_with("--user-data-dir="))
            .expect("应包含 --user-data-dir 参数");
        let ud_b = args_b
            .iter()
            .find(|a| a.to_string_lossy().starts_with("--user-data-dir="))
            .expect("应包含 --user-data-dir 参数");

        assert_ne!(ud_a, ud_b, "两次调用应使用不同的 user-data-dir（参数隔离）");
    }

    #[test]
    fn user_data_dir_is_single_space_safe_arg() {
        // 路径含空格时，--user-data-dir=PATH 必须是单个参数，路径本身不得作为独立参数出现。
        let dir = Path::new("/tmp/a b/workbuddy-auth-1234");
        let args = build_browser_args("--inprivate", dir, "https://x.com/c?d=e");

        let path_standalone = OsString::from("/tmp/a b/workbuddy-auth-1234");
        assert!(
            !args.contains(&path_standalone),
            "--user-data-dir 路径不应作为独立参数出现"
        );

        let ud = args
            .iter()
            .find(|a| a.to_string_lossy().starts_with("--user-data-dir="))
            .expect("应包含 --user-data-dir 参数");
        assert_eq!(
            ud,
            &OsString::from("--user-data-dir=/tmp/a b/workbuddy-auth-1234"),
            "--user-data-dir=PATH 应为单个、空格安全的参数"
        );
    }

    #[test]
    fn browser_args_contain_required_flags() {
        let dir = std::env::temp_dir().join("x");
        let args = build_browser_args("--incognito", &dir, "https://e.com");
        let joined: Vec<String> = args.iter().map(|a| a.to_string_lossy().into_owned()).collect();

        assert!(joined.contains(&"--no-first-run".to_string()));
        assert!(joined.contains(&"--no-default-browser-check".to_string()));
        assert!(joined.contains(&"--disable-background-mode".to_string()));
        assert!(joined.contains(&"--disable-sync".to_string()));
        assert!(joined.contains(&"--new-window".to_string()));
        assert!(joined.contains(&"https://e.com".to_string()));
        assert!(joined.contains(&"--incognito".to_string()));
        assert!(joined
            .iter()
            .any(|a| a.starts_with("--user-data-dir=")));
    }

    #[test]
    fn edge_uses_inprivate_flag() {
        let dir = std::env::temp_dir().join("x");
        let args = build_browser_args("--inprivate", &dir, "https://e.com");
        let joined: Vec<String> = args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(joined.contains(&"--inprivate".to_string()));
        assert!(!joined.contains(&"--incognito".to_string()));
    }
}
