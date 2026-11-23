// Tray refresh scheduling and native menu application.
#[cfg(any(target_os = "linux", target_os = "windows"))]
static APPLIED_TRAY_MENU_SNAPSHOT: std::sync::Mutex<Option<TrayMenuSnapshot>> =
    std::sync::Mutex::new(None);

#[cfg(any(test, target_os = "linux", target_os = "windows"))]
fn apply_changed_tray_menu(
    cache: &std::sync::Mutex<Option<TrayMenuSnapshot>>,
    snapshot: &TrayMenuSnapshot,
    apply: impl FnOnce() -> Result<(), String>,
) -> Result<bool, String> {
    if cache
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
        == Some(snapshot)
    {
        return Ok(false);
    }
    // Native menu work runs outside the cache lock. Only successful application
    // may suppress the next refresh; failed updates remain retryable.
    apply()?;
    *cache.lock().unwrap_or_else(|error| error.into_inner()) = Some(snapshot.clone());
    Ok(true)
}
pub fn update_tray_menu<R: Runtime>(app: &tauri::AppHandle<R>) -> Result<(), String> {
    if !crate::modules::config::get_user_config().show_tray_icon {
        return Ok(());
    }
    TRAY_MENU_REQUESTS.fetch_add(1, Ordering::AcqRel);
    if TRAY_MENU_REBUILD_SCHEDULED.swap(true, Ordering::AcqRel) {
        // A worker is already going to pick this request up.
        return Ok(());
    }
    spawn_tray_menu_rebuild_worker(app.clone());
    Ok(())
}

fn spawn_tray_menu_rebuild_worker<R: Runtime>(app: tauri::AppHandle<R>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(TRAY_MENU_COALESCE_WINDOW);
        let collapsed = TRAY_MENU_REQUESTS.swap(0, Ordering::AcqRel);
        let started = std::time::Instant::now();
        match rebuild_tray_menu_now(&app) {
            Ok(Some(timing)) => {
                logger::log_info(&format!(
                    "[Tray] 托盘菜单已更新: 合并请求={}, 数据耗时={}ms, 提交耗时={}ms, 总耗时={}ms",
                    collapsed,
                    timing.data_ms,
                    timing.apply_ms,
                    started.elapsed().as_millis()
                ));
            }
            Ok(None) => {
                logger::log_info(&format!(
                    "[Tray] 托盘菜单跳过未变化或过期快照: 合并请求={}, 耗时={}ms",
                    collapsed,
                    started.elapsed().as_millis()
                ));
            }
            Err(err) => {
                logger::log_warn(&format!("[Tray] 托盘菜单重建失败: {}", err));
            }
        }

        // Release the slot, then re-check: a request that landed between the
        // swap above and this store would otherwise never be served.
        TRAY_MENU_REBUILD_SCHEDULED.store(false, Ordering::Release);
        if TRAY_MENU_REQUESTS.load(Ordering::Acquire) == 0 {
            return;
        }
        if TRAY_MENU_REBUILD_SCHEDULED.swap(true, Ordering::AcqRel) {
            // Someone else claimed the slot and will do the work.
            return;
        }
    });
}

struct TrayMenuRebuildTiming {
    data_ms: u128,
    apply_ms: u128,
}

#[cfg(not(target_os = "macos"))]
enum TrayMenuApply {
    Skipped,
    Applied(u128),
    #[cfg(target_os = "windows")]
    MenuOpen,
}

#[cfg(target_os = "windows")]
fn windows_popup_menu_open() -> Result<bool, String> {
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetGUIThreadInfo, GUITHREADINFO, GUI_POPUPMENUMODE,
    };

    let mut info = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetGUIThreadInfo(GetCurrentThreadId(), &mut info) }
        .map_err(|err| format!("读取 Windows 托盘菜单状态失败: {err}"))?;
    Ok(info.flags.contains(GUI_POPUPMENUMODE))
}

fn rebuild_tray_menu_now<R: Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<Option<TrayMenuRebuildTiming>, String> {
    if !crate::modules::config::get_user_config().show_tray_icon {
        return Ok(None);
    }
    #[cfg(target_os = "macos")]
    {
        crate::modules::macos_native_menu::update_status_item(app)?;
        if !MACOS_TRAY_SKIP_LOGGED.swap(true, Ordering::Relaxed) {
            logger::log_info("[Tray] macOS 原生菜单模式，已更新菜单栏状态");
        }
        Ok(Some(TrayMenuRebuildTiming {
            data_ms: 0,
            apply_ms: 0,
        }))
    }

    #[cfg(not(target_os = "macos"))]
    {
        let data_started = std::time::Instant::now();
        let snapshot = std::sync::Arc::new(collect_tray_menu_snapshot());
        let data_ms = data_started.elapsed().as_millis();
        let generation = next_tray_menu_apply_generation(&TRAY_MENU_APPLY_GENERATION);
        loop {
            match submit_tray_menu_snapshot(app, snapshot.clone(), generation)? {
                TrayMenuApply::Applied(apply_ms) => {
                    return Ok(Some(TrayMenuRebuildTiming { data_ms, apply_ms }));
                }
                TrayMenuApply::Skipped => return Ok(None),
                #[cfg(target_os = "windows")]
                TrayMenuApply::MenuOpen => {
                    // Keep the snapshot on the refresh worker while Windows tracks the menu.
                    std::thread::sleep(TRAY_MENU_COALESCE_WINDOW);
                    if TRAY_MENU_REQUESTS.load(Ordering::Acquire) != 0 {
                        return Ok(None);
                    }
                }
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn submit_tray_menu_snapshot<R: Runtime>(
    app: &tauri::AppHandle<R>,
    snapshot: std::sync::Arc<TrayMenuSnapshot>,
    generation: usize,
) -> Result<TrayMenuApply, String> {
    let app_handle = app.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let result = (|| {
            if is_stale_tray_menu_apply(&TRAY_MENU_APPLY_GENERATION, generation)
                || !crate::modules::config::get_user_config().show_tray_icon
            {
                return Ok(TrayMenuApply::Skipped);
            }
            let Some(tray) = app_handle.tray_by_id(TRAY_ID) else {
                return Ok(TrayMenuApply::Skipped);
            };
            #[cfg(target_os = "windows")]
            if windows_popup_menu_open()? {
                // Replacing the HMENU while TrackPopupMenu is active cancels the selection.
                return Ok(TrayMenuApply::MenuOpen);
            }
            let applied_at = std::time::Instant::now();
            let apply = || {
                let menu = build_tray_menu_from_snapshot(&app_handle, &snapshot)
                    .map_err(|e| e.to_string())?;
                tray.set_menu(Some(menu)).map_err(|e| e.to_string())
            };
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            let changed = apply_changed_tray_menu(&APPLIED_TRAY_MENU_SNAPSHOT, &snapshot, apply)?;
            #[cfg(not(any(target_os = "linux", target_os = "windows")))]
            let changed = {
                apply()?;
                true
            };
            Ok(if changed {
                TrayMenuApply::Applied(applied_at.elapsed().as_millis())
            } else {
                TrayMenuApply::Skipped
            })
        })();
        let _ = tx.send(result);
    })
    .map_err(|e| e.to_string())?;

    rx.recv_timeout(std::time::Duration::from_secs(5))
        .map_err(|error| {
            let _ = TRAY_MENU_APPLY_GENERATION.compare_exchange(
                generation,
                generation.wrapping_add(1),
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            format!("等待托盘菜单提交失败: {}", error)
        })?
}
