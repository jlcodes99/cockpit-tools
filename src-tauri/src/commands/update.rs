use crate::modules::linux_updater::{self, UpdateRuntimeInfo};
use crate::modules::logger;
use crate::modules::update_checker::{self, ReleaseHistoryItem, UpdateSettings, VersionJumpInfo};
use std::time::Instant;

/// Check if we should check for updates (based on interval settings)
#[tauri::command]
pub fn should_check_updates() -> Result<bool, String> {
    // #1104: respect external-network kill switch for auto update probes.
    if !crate::modules::config::get_user_config().external_network_enabled {
        return Ok(false);
    }
    let settings = update_checker::load_update_settings()?;
    Ok(update_checker::should_check_for_updates(&settings))
}

/// Update the last check time
#[tauri::command]
pub fn update_last_check_time() -> Result<(), String> {
    let started = Instant::now();
    let result = update_checker::update_last_check_time();
    match &result {
        Ok(_) => logger::log_info(&format!(
            "[StartupPerf][UpdaterCommand] update_last_check_time completed in {}ms",
            started.elapsed().as_millis()
        )),
        Err(err) => logger::log_error(&format!(
            "[StartupPerf][UpdaterCommand] update_last_check_time failed in {}ms: {}",
            started.elapsed().as_millis(),
            err
        )),
    }
    result
}

/// Get update settings
#[tauri::command]
pub fn get_update_settings() -> Result<UpdateSettings, String> {
    let started = Instant::now();
    let result = update_checker::load_update_settings();
    match &result {
        Ok(settings) => logger::log_info(&format!(
            "[StartupPerf][UpdaterCommand] get_update_settings completed in {}ms: auto_check={}, auto_install={}, last_check_time={}",
            started.elapsed().as_millis(),
            settings.auto_check,
            settings.auto_install,
            settings.last_check_time
        )),
        Err(err) => logger::log_error(&format!(
            "[StartupPerf][UpdaterCommand] get_update_settings failed in {}ms: {}",
            started.elapsed().as_millis(),
            err
        )),
    }
    result
}

/// Patch only the updater fields changed by the caller.
#[tauri::command]
pub fn patch_update_settings(
    auto_check: Option<bool>,
    check_interval_hours: Option<u64>,
    auto_install: Option<bool>,
    last_run_version: Option<String>,
    remind_on_update: Option<bool>,
    skipped_version: Option<String>,
) -> Result<UpdateSettings, String> {
    update_checker::patch_update_settings(move |settings| {
        if let Some(value) = auto_check {
            settings.auto_check = value;
        }
        if let Some(value) = check_interval_hours {
            settings.check_interval_hours = value;
        }
        if let Some(value) = auto_install {
            settings.auto_install = value;
        }
        if let Some(value) = last_run_version {
            settings.last_run_version = value;
        }
        if let Some(value) = remind_on_update {
            settings.remind_on_update = value;
        }
        if let Some(value) = skipped_version {
            settings.skipped_version = value;
        }
    })
}

/// Save release notes for a downloaded/pending update
#[tauri::command]
pub fn save_pending_update_notes(
    version: String,
    release_notes: String,
    release_notes_zh: String,
) -> Result<(), String> {
    update_checker::save_pending_update_notes(version, release_notes, release_notes_zh)
}

/// Check if a version jump occurred (for post-update changelog display)
#[tauri::command]
pub fn check_version_jump() -> Result<Option<VersionJumpInfo>, String> {
    let started = Instant::now();
    let result = update_checker::check_version_jump();
    match &result {
        Ok(Some(info)) => logger::log_info(&format!(
            "[StartupPerf][UpdaterCommand] check_version_jump hit in {}ms: {} -> {}",
            started.elapsed().as_millis(),
            info.previous_version,
            info.current_version
        )),
        Ok(None) => logger::log_info(&format!(
            "[StartupPerf][UpdaterCommand] check_version_jump completed in {}ms: no jump",
            started.elapsed().as_millis()
        )),
        Err(err) => logger::log_error(&format!(
            "[StartupPerf][UpdaterCommand] check_version_jump failed in {}ms: {}",
            started.elapsed().as_millis(),
            err
        )),
    }
    result
}

/// Read release history from changelog files
#[tauri::command]
pub fn get_release_history(
    locale: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<ReleaseHistoryItem>, String> {
    update_checker::get_release_history(locale.as_deref(), limit)
}

/// Write updater lifecycle logs from frontend into app.log
#[tauri::command]
pub fn update_log(level: String, message: String) -> Result<(), String> {
    let level = level.trim().to_lowercase();
    let message = message.trim();
    if message.is_empty() {
        return Ok(());
    }

    let text = format!("[Updater] {}", message);
    match level.as_str() {
        "error" => logger::log_error(&text),
        "warn" | "warning" => logger::log_warn(&text),
        _ => logger::log_info(&text),
    }

    Ok(())
}

#[tauri::command]
pub fn get_update_runtime_info() -> Result<UpdateRuntimeInfo, String> {
    Ok(linux_updater::get_update_runtime_info())
}

#[tauri::command]
pub async fn install_linux_update(
    app: tauri::AppHandle,
    expected_version: Option<String>,
) -> Result<(), String> {
    linux_updater::install_linux_update(app, expected_version).await
}
