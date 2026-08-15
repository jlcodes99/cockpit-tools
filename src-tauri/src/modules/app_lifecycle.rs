use std::io::{Error, ErrorKind};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Mutex, MutexGuard};

static SHUTDOWN_STARTED: AtomicBool = AtomicBool::new(false);
static PROCESS_SPAWN_LOCK: Mutex<()> = Mutex::new(());
static APP_EXIT_STATE: AtomicU8 = AtomicU8::new(APP_EXIT_IDLE);

const APP_EXIT_IDLE: u8 = 0;
const APP_EXIT_CLEANING_UP: u8 = 1;
const APP_EXIT_READY: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppExitDecision {
    StartCleanup,
    WaitForCleanup,
    ExitNow,
}

pub struct ProcessSpawnGuard {
    _guard: MutexGuard<'static, ()>,
}

fn lock_process_spawn() -> MutexGuard<'static, ()> {
    PROCESS_SPAWN_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn is_shutdown_started() -> bool {
    SHUTDOWN_STARTED.load(Ordering::SeqCst)
}

pub fn begin_shutdown() -> bool {
    !SHUTDOWN_STARTED.swap(true, Ordering::SeqCst)
}

pub fn wait_for_in_flight_process_spawns() {
    drop(lock_process_spawn());
}

pub fn request_app_exit_cleanup() -> AppExitDecision {
    begin_shutdown();
    match APP_EXIT_STATE.compare_exchange(
        APP_EXIT_IDLE,
        APP_EXIT_CLEANING_UP,
        Ordering::SeqCst,
        Ordering::SeqCst,
    ) {
        Ok(_) => AppExitDecision::StartCleanup,
        Err(APP_EXIT_CLEANING_UP) => AppExitDecision::WaitForCleanup,
        Err(APP_EXIT_READY) => AppExitDecision::ExitNow,
        Err(_) => AppExitDecision::WaitForCleanup,
    }
}

pub fn finish_app_exit_cleanup(result: Result<(), String>) -> Result<(), String> {
    APP_EXIT_STATE.store(APP_EXIT_READY, Ordering::SeqCst);
    result
}

#[cfg(target_os = "windows")]
fn cancel_system_shutdown() {
    let _spawn_guard = lock_process_spawn();
    if APP_EXIT_STATE.load(Ordering::SeqCst) == APP_EXIT_IDLE {
        SHUTDOWN_STARTED.store(false, Ordering::SeqCst);
    }
}

pub fn acquire_process_spawn_guard(program: &str) -> std::io::Result<ProcessSpawnGuard> {
    let guard = lock_process_spawn();
    if !process_spawn_allowed(is_shutdown_started()) {
        return Err(Error::new(
            ErrorKind::Interrupted,
            format!("系统正在关闭，已取消启动 {}", program),
        ));
    }
    Ok(ProcessSpawnGuard { _guard: guard })
}

#[cfg(target_os = "windows")]
pub fn install_system_shutdown_listener() -> Result<(), String> {
    use std::sync::mpsc::sync_channel;

    let (shutdown_tx, shutdown_rx) = sync_channel::<()>(1);
    std::thread::Builder::new()
        .name("cockpit-system-shutdown-cleanup".to_string())
        .spawn(move || {
            if shutdown_rx.recv().is_ok() {
                crate::modules::logger::log_info(
                    "[Lifecycle] Windows 正在关闭，停止后台注入并禁止创建新子进程",
                );
                crate::modules::codex_app_injection::stop_all();
            }
        })
        .map_err(|error| format!("启动 Windows 关机清理线程失败: {}", error))?;

    std::thread::Builder::new()
        .name("cockpit-system-shutdown-listener".to_string())
        .spawn(move || {
            if let Err(error) = run_windows_shutdown_message_loop(shutdown_tx) {
                crate::modules::logger::log_warn(&format!(
                    "[Lifecycle] Windows 关机监听退出: {}",
                    error
                ));
            }
        })
        .map_err(|error| format!("启动 Windows 关机监听线程失败: {}", error))?;

    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn install_system_shutdown_listener() -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "windows")]
fn run_windows_shutdown_message_loop(
    shutdown_tx: std::sync::mpsc::SyncSender<()>,
) -> Result<(), String> {
    use std::ffi::c_void;
    use std::sync::OnceLock;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW,
        TranslateMessage, MSG, WM_ENDSESSION, WM_QUERYENDSESSION, WNDCLASSW, WS_EX_TOOLWINDOW,
        WS_OVERLAPPED,
    };

    static SHUTDOWN_TX: OnceLock<std::sync::mpsc::SyncSender<()>> = OnceLock::new();

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_QUERYENDSESSION => {
                begin_shutdown();
                LRESULT(1)
            }
            WM_ENDSESSION if wparam.0 != 0 => {
                begin_shutdown();
                if let Some(sender) = SHUTDOWN_TX.get() {
                    let _ = sender.try_send(());
                }
                LRESULT(0)
            }
            WM_ENDSESSION => {
                cancel_system_shutdown();
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
        }
    }

    SHUTDOWN_TX
        .set(shutdown_tx)
        .map_err(|_| "Windows 关机监听已初始化".to_string())?;

    let class_name = format!("CockpitToolsShutdownListener-{}\0", std::process::id())
        .encode_utf16()
        .collect::<Vec<_>>();
    let window_name = "Cockpit Tools Shutdown Listener\0"
        .encode_utf16()
        .collect::<Vec<_>>();

    unsafe {
        let module = GetModuleHandleW(None)
            .map_err(|error| format!("读取 Windows 模块句柄失败: {}", error))?;
        let instance = HINSTANCE(module.0);
        let window_class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        if RegisterClassW(&window_class) == 0 {
            return Err(format!(
                "注册 Windows 关机监听窗口失败: {}",
                std::io::Error::last_os_error()
            ));
        }

        let _window = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            PCWSTR(class_name.as_ptr()),
            PCWSTR(window_name.as_ptr()),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            instance,
            None::<*const c_void>,
        )
        .map_err(|error| format!("创建 Windows 关机监听窗口失败: {}", error))?;

        let mut message = MSG::default();
        loop {
            let result = GetMessageW(&mut message, None, 0, 0);
            if result.0 == -1 {
                return Err(format!(
                    "读取 Windows 关机消息失败: {}",
                    std::io::Error::last_os_error()
                ));
            }
            if !result.as_bool() {
                break;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::AppExitDecision;
    use std::sync::{mpsc, Arc, Barrier, Mutex, MutexGuard};
    use std::time::Duration;

    static LIFECYCLE_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn lock_lifecycle_test() -> MutexGuard<'static, ()> {
        LIFECYCLE_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn reset_lifecycle_state() {
        super::SHUTDOWN_STARTED.store(false, super::Ordering::SeqCst);
        super::APP_EXIT_STATE.store(super::APP_EXIT_IDLE, super::Ordering::SeqCst);
    }

    #[test]
    fn process_spawn_policy_rejects_shutdown_state() {
        assert!(super::process_spawn_allowed(false));
        assert!(!super::process_spawn_allowed(true));
    }

    #[test]
    fn begin_shutdown_does_not_wait_for_in_flight_process_spawn() {
        let _test_guard = lock_lifecycle_test();
        reset_lifecycle_state();

        let (guard_ready_tx, guard_ready_rx) = mpsc::channel();
        let (release_guard_tx, release_guard_rx) = mpsc::channel();
        let guard_thread = std::thread::spawn(move || {
            let _guard = super::acquire_process_spawn_guard("lifecycle-test")
                .expect("test process spawn should be allowed");
            guard_ready_tx
                .send(())
                .expect("test should observe acquired spawn guard");
            release_guard_rx
                .recv()
                .expect("test should release spawn guard");
        });
        guard_ready_rx
            .recv()
            .expect("spawn guard thread should become ready");

        let (shutdown_result_tx, shutdown_result_rx) = mpsc::channel();
        let shutdown_thread = std::thread::spawn(move || {
            shutdown_result_tx
                .send(super::begin_shutdown())
                .expect("test should observe shutdown result");
        });

        let shutdown_result = shutdown_result_rx.recv_timeout(Duration::from_secs(2));
        let (wait_done_tx, wait_done_rx) = mpsc::channel();
        let wait_thread = std::thread::spawn(move || {
            super::wait_for_in_flight_process_spawns();
            wait_done_tx
                .send(())
                .expect("test should observe spawn quiescence");
        });
        assert!(
            wait_done_rx
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "cleanup waiter must stay behind the in-flight process spawn"
        );
        release_guard_tx
            .send(())
            .expect("test should unblock spawn guard thread");
        guard_thread.join().expect("spawn guard thread should join");
        wait_done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("cleanup waiter should finish after spawn guard release");
        wait_thread
            .join()
            .expect("cleanup waiter thread should join");
        shutdown_thread
            .join()
            .expect("shutdown request thread should join");
        reset_lifecycle_state();

        assert_eq!(
            shutdown_result,
            Ok(true),
            "shutdown initiation must not block the UI event thread"
        );
    }

    #[test]
    fn successful_app_exit_cleanup_allows_the_next_exit_request() {
        let _test_guard = lock_lifecycle_test();
        reset_lifecycle_state();

        assert_eq!(
            super::request_app_exit_cleanup(),
            AppExitDecision::StartCleanup
        );
        assert!(super::is_shutdown_started());
        assert_eq!(super::finish_app_exit_cleanup(Ok(())), Ok(()));
        assert_eq!(super::request_app_exit_cleanup(), AppExitDecision::ExitNow);

        reset_lifecycle_state();
    }

    #[test]
    fn failed_app_exit_cleanup_still_releases_repeated_exit_requests() {
        let _test_guard = lock_lifecycle_test();
        reset_lifecycle_state();

        assert_eq!(
            super::request_app_exit_cleanup(),
            AppExitDecision::StartCleanup
        );
        assert_eq!(
            super::request_app_exit_cleanup(),
            AppExitDecision::WaitForCleanup
        );
        assert_eq!(
            super::finish_app_exit_cleanup(Err("sidecar shutdown failed".to_string())),
            Err("sidecar shutdown failed".to_string())
        );
        assert_eq!(super::request_app_exit_cleanup(), AppExitDecision::ExitNow);

        reset_lifecycle_state();
    }

    #[test]
    fn concurrent_exit_requests_start_exactly_one_cleanup() {
        let _test_guard = lock_lifecycle_test();
        reset_lifecycle_state();

        const REQUEST_COUNT: usize = 8;
        let barrier = Arc::new(Barrier::new(REQUEST_COUNT));
        let requests = (0..REQUEST_COUNT)
            .map(|_| {
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    super::request_app_exit_cleanup()
                })
            })
            .collect::<Vec<_>>();
        let decisions = requests
            .into_iter()
            .map(|thread| thread.join().expect("exit request thread should join"))
            .collect::<Vec<_>>();

        assert_eq!(
            decisions
                .iter()
                .filter(|decision| **decision == AppExitDecision::StartCleanup)
                .count(),
            1
        );
        assert_eq!(
            decisions
                .iter()
                .filter(|decision| **decision == AppExitDecision::WaitForCleanup)
                .count(),
            REQUEST_COUNT - 1
        );
        assert!(!decisions.contains(&AppExitDecision::ExitNow));

        assert_eq!(super::finish_app_exit_cleanup(Ok(())), Ok(()));
        reset_lifecycle_state();
    }
}

fn process_spawn_allowed(shutdown_started: bool) -> bool {
    !shutdown_started
}
