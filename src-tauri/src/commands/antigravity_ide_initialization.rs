use std::path::{Path, PathBuf};

use crate::modules::{account, instance, process};

use super::DEFAULT_INSTANCE_ID;

struct IdeProfile {
    directory: PathBuf,
    extra_args: String,
    last_pid: Option<u32>,
    bind_account: bool,
    bind_account_id: Option<String>,
}

fn load_profile(instance_id: &str) -> Result<IdeProfile, String> {
    if instance_id == DEFAULT_INSTANCE_ID {
        let settings = instance::load_default_settings()?;
        return Ok(IdeProfile {
            directory: instance::get_default_user_data_dir()?,
            extra_args: settings.extra_args,
            last_pid: settings.last_pid,
            bind_account: settings.bind_account_id.is_some(),
            bind_account_id: settings.bind_account_id,
        });
    }
    let profile = instance::load_instance_store()?
        .instances
        .into_iter()
        .find(|profile| profile.id == instance_id)
        .ok_or("实例不存在")?;
    Ok(IdeProfile {
        directory: PathBuf::from(profile.user_data_dir),
        extra_args: profile.extra_args,
        last_pid: profile.last_pid,
        bind_account: profile.bind_account_id.is_some(),
        bind_account_id: profile.bind_account_id,
    })
}

fn needs_initialization(
    shared_credentials: bool,
    profile: &Path,
    copy_source: Option<&Path>,
) -> Result<bool, String> {
    if shared_credentials {
        return Ok(false);
    }
    instance::ide_profile_needs_initialization(profile, copy_source)
}

#[tauri::command]
pub(crate) async fn antigravity_ide_needs_initialization(
    instance_id: String,
) -> Result<bool, String> {
    let probe = tokio::task::spawn_blocking(move || {
        let profile = load_profile(&instance_id)?;
        let copy_source = (profile.bind_account && instance_id != DEFAULT_INSTANCE_ID)
            .then(instance::get_default_user_data_dir)
            .transpose()?;
        needs_initialization(
            crate::commands::system::antigravity_ide_uses_shared_credentials(),
            &profile.directory,
            copy_source.as_deref(),
        )
    });
    tokio::time::timeout(std::time::Duration::from_secs(3), probe)
        .await
        .map_err(|_| "ANTIGRAVITY_IDE_INITIALIZATION_CHECK_TIMEOUT".to_string())?
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn initialize_antigravity_ide(instance_id: String) -> Result<(), String> {
    let permit = account::try_begin_antigravity_switch(&account::ANTIGRAVITY_SWITCH_GATE)?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let profile = load_profile(&instance_id)?;
        let default = instance_id == DEFAULT_INSTANCE_ID;
        let directory = profile.directory.to_string_lossy();
        let target_dir = (!default).then_some(directory.as_ref());
        if let Some(pid) = process::resolve_antigravity_pid(profile.last_pid, target_dir) {
            let _ = process::focus_antigravity_instance(Some(pid), target_dir);
            return Ok(());
        }
        let egress_proxy_url = profile
            .bind_account_id
            .as_deref()
            .and_then(|id| account::load_account(id).ok())
            .and_then(|entry| entry.egress_proxy_url);
        let args = process::parse_extra_args(&profile.extra_args);
        let pid = process::start_antigravity_with_args_proxy(
            target_dir.unwrap_or(""),
            &args,
            egress_proxy_url.as_deref(),
        )?;
        if default {
            instance::update_default_pid(Some(pid))?;
        } else {
            instance::update_instance_after_start(&instance_id, pid)?;
        }
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn antigravity_ide_initialization_waits_for_the_client_database_schema() {
        let profile = tempfile::tempdir().unwrap();
        let global_storage = profile.path().join("User/globalStorage");
        std::fs::create_dir_all(&global_storage).unwrap();
        let path = global_storage.join("state.vscdb");

        assert!(needs_initialization(false, profile.path(), None).unwrap());
        assert!(!path.exists());
        assert!(!needs_initialization(true, profile.path(), None).unwrap());

        let db = rusqlite::Connection::open(&path).unwrap();
        assert!(needs_initialization(false, profile.path(), None).unwrap());
        db.execute("CREATE TABLE ItemTable (key TEXT UNIQUE, value TEXT)", [])
            .unwrap();
        assert!(!needs_initialization(false, profile.path(), None).unwrap());

        let blank = tempfile::tempdir().unwrap();
        assert!(needs_initialization(false, blank.path(), None).unwrap());
        assert!(!needs_initialization(false, blank.path(), Some(profile.path())).unwrap());
        assert!(instance::require_ide_switch_database(blank.path(), None).is_err());
        assert!(instance::require_ide_switch_database(blank.path(), Some(profile.path())).is_ok());
    }
}
