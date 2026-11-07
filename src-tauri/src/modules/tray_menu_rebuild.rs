// Tray refresh scheduling and native menu application.
#[cfg(target_os = "linux")]
static APPLIED_TRAY_MENU_SNAPSHOT: std::sync::Mutex<Option<TrayMenuSnapshot>> =
    std::sync::Mutex::new(None);

#[cfg(any(test, target_os = "linux"))]
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

fn rebuild_tray_menu_now<R: Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<Option<TrayMenuRebuildTiming>, String> {
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
        let snapshot = collect_tray_menu_snapshot();
        let data_ms = data_started.elapsed().as_millis();
        let generation = next_tray_menu_apply_generation(&TRAY_MENU_APPLY_GENERATION);
        let app_handle = app.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        app.run_on_main_thread(move || {
            if is_stale_tray_menu_apply(&TRAY_MENU_APPLY_GENERATION, generation) {
                let _ = tx.send(Ok(None));
                return;
            }
            let apply_started = std::time::Instant::now();
            let result = (|| {
                let Some(tray) = app_handle.tray_by_id(TRAY_ID) else {
                    return Ok(false);
                };
                let apply = || {
                    let menu = build_tray_menu_from_snapshot(&app_handle, &snapshot)
                        .map_err(|e| e.to_string())?;
                    tray.set_menu(Some(menu)).map_err(|e| e.to_string())
                };
                #[cfg(target_os = "linux")]
                return apply_changed_tray_menu(&APPLIED_TRAY_MENU_SNAPSHOT, &snapshot, apply);
                #[cfg(not(target_os = "linux"))]
                {
                    apply()?;
                    Ok(true)
                }
            })();
            let apply_ms = apply_started.elapsed().as_millis();
            let _ = tx.send(result.map(|changed| changed.then_some(apply_ms)));
        })
        .map_err(|e| e.to_string())?;

        let applied = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .map_err(|error| {
                let _ = TRAY_MENU_APPLY_GENERATION.compare_exchange(
                    generation,
                    generation.wrapping_add(1),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
                format!("等待托盘菜单提交失败: {}", error)
            })?;
        match applied {
            Ok(Some(apply_ms)) => Ok(Some(TrayMenuRebuildTiming { data_ms, apply_ms })),
            Ok(None) => Ok(None),
            Err(err) => Err(err),
        }
    }
}
