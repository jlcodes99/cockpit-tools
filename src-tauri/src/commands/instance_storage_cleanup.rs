use crate::modules::instance_storage_cleanup::{self, CleanupResult, OrphanInstanceDir};

#[tauri::command]
pub async fn scan_orphan_instance_dirs() -> Result<Vec<OrphanInstanceDir>, String> {
    tauri::async_runtime::spawn_blocking(instance_storage_cleanup::scan_orphan_instance_dirs)
        .await
        .map_err(|error| format!("Instance storage scan failed: {}", error))?
}

#[tauri::command]
pub async fn delete_orphan_instance_dirs(paths: Vec<String>) -> Result<CleanupResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        instance_storage_cleanup::delete_orphan_instance_dirs(paths)
    })
    .await
    .map_err(|error| format!("Instance storage cleanup failed: {}", error))?
}
