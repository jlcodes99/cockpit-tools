use crate::models::ssh_server::{SshCodexSyncResult, SshServer};
use crate::modules::ssh_server::{self, SshServerList};

#[tauri::command]
pub fn list_ssh_servers() -> Result<SshServerList, String> {
    ssh_server::list_servers()
}

#[tauri::command]
pub fn upsert_ssh_server(server: SshServer) -> Result<SshServerList, String> {
    ssh_server::upsert_server(server)
}

#[tauri::command]
pub fn delete_ssh_server(server_id: String) -> Result<SshServerList, String> {
    ssh_server::delete_server(&server_id)
}

#[tauri::command]
pub fn select_ssh_server(server_id: Option<String>) -> Result<SshServerList, String> {
    ssh_server::select_server(server_id)
}

#[tauri::command]
pub async fn test_ssh_server_connection(server_id: String) -> Result<String, String> {
    ssh_server::test_connection(&server_id).await
}

#[tauri::command]
pub async fn sync_current_codex_account_to_ssh_server(
    server_id: Option<String>,
) -> Result<SshCodexSyncResult, String> {
    ssh_server::sync_current_account_to_server(server_id).await
}
