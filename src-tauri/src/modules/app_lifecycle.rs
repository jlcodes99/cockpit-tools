use std::io::{Error, ErrorKind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

static SHUTDOWN_STARTED: AtomicBool = AtomicBool::new(false);
static PROCESS_SPAWN_LOCK: Mutex<()> = Mutex::new(());

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
    let _spawn_guard = lock_process_spawn();
    !SHUTDOWN_STARTED.swap(true, Ordering::SeqCst)
}

#[cfg(target_os = "windows")]
fn cancel_system_shutdown() {
    let _spawn_guard = lock_process_spawn();
    SHUTDOWN_STARTED.store(false, Ordering::SeqCst);
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
    #[test]
    fn process_spawn_policy_rejects_shutdown_state() {
        assert!(super::process_spawn_allowed(false));
        assert!(!super::process_spawn_allowed(true));
    }
}

fn process_spawn_allowed(shutdown_started: bool) -> bool {
    !shutdown_started
}
