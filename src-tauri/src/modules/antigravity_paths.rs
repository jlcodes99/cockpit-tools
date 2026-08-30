#[cfg(target_os = "windows")]
use std::path::Path;
use std::path::PathBuf;

#[cfg(target_os = "windows")]
fn roaming_app_data_dir() -> Result<PathBuf, String> {
    use std::ffi::c_void;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{
        FOLDERID_RoamingAppData, SHGetKnownFolderPath, KF_FLAG_DEFAULT,
    };

    unsafe {
        let raw =
            SHGetKnownFolderPath(&FOLDERID_RoamingAppData, KF_FLAG_DEFAULT, HANDLE::default())
                .map_err(|e| format!("无法获取 Roaming AppData 目录: {}", e))?;
        let path = raw
            .to_string()
            .map_err(|e| format!("无法解析 Roaming AppData 路径: {}", e));
        CoTaskMemFree(Some(raw.as_ptr().cast::<c_void>()));
        path.map(PathBuf::from)
    }
}

/// Windows 下可能同时存在 `Antigravity IDE` 与 `Antigravity` 目录。
#[cfg(target_os = "windows")]
fn windows_user_data_candidates(roaming_dir: &Path) -> Vec<PathBuf> {
    vec![
        roaming_dir.join("Antigravity IDE"),
        roaming_dir.join("Antigravity"),
    ]
}

/// 优先选择「User/globalStorage/state.vscdb 真实存在」的候选目录，避免误选空壳安装路径。
pub fn prefer_user_data_dir_with_state_db(candidates: &[PathBuf], fallback: PathBuf) -> PathBuf {
    for candidate in candidates {
        let state_db = candidate
            .join("User")
            .join("globalStorage")
            .join("state.vscdb");
        if state_db.is_file() {
            return candidate.clone();
        }
    }
    for candidate in candidates {
        if candidate.exists() {
            return candidate.clone();
        }
    }
    fallback
}

pub fn default_user_data_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or("无法获取 Home 目录")?;
        return Ok(home.join("Library/Application Support/Antigravity IDE"));
    }

    #[cfg(target_os = "windows")]
    {
        let roaming_dir = roaming_app_data_dir()?;
        let candidates = windows_user_data_candidates(&roaming_dir);
        let fallback = candidates
            .first()
            .cloned()
            .unwrap_or_else(|| roaming_dir.join("Antigravity IDE"));
        return Ok(prefer_user_data_dir_with_state_db(&candidates, fallback));
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().ok_or("无法获取 Home 目录")?;
        return Ok(home.join(".config/Antigravity IDE"));
    }

    #[allow(unreachable_code)]
    Err("无法确定 Antigravity IDE 默认目录".to_string())
}

pub fn legacy_default_user_data_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or("无法获取 Home 目录")?;
        return Ok(home.join("Library/Application Support/Antigravity"));
    }

    #[cfg(target_os = "windows")]
    {
        let roaming_dir = roaming_app_data_dir()?;
        return Ok(roaming_dir.join("Antigravity"));
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().ok_or("无法获取 Home 目录")?;
        return Ok(home.join(".config/Antigravity"));
    }

    #[allow(unreachable_code)]
    Err("无法确定 Antigravity 默认目录".to_string())
}

pub fn managed_instances_root_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
        return Ok(home.join(".antigravity_cockpit/instances/antigravity"));
    }

    #[cfg(target_os = "windows")]
    {
        let roaming_dir = roaming_app_data_dir()?;
        return Ok(roaming_dir.join(".antigravity_cockpit\\instances\\antigravity"));
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
        return Ok(home.join(".antigravity_cockpit/instances/antigravity"));
    }

    #[allow(unreachable_code)]
    Err("无法确定默认实例目录".to_string())
}

pub fn legacy_managed_instances_root_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
        return Ok(home.join(".antigravity_cockpit/instances/antigravity-legacy"));
    }

    #[cfg(target_os = "windows")]
    {
        let roaming_dir = roaming_app_data_dir()?;
        return Ok(roaming_dir.join(".antigravity_cockpit\\instances\\antigravity-legacy"));
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
        return Ok(home.join(".antigravity_cockpit/instances/antigravity-legacy"));
    }

    #[allow(unreachable_code)]
    Err("无法确定 Antigravity 默认实例目录".to_string())
}

pub fn global_storage_dir() -> Result<PathBuf, String> {
    Ok(default_user_data_dir()?.join("User").join("globalStorage"))
}

pub fn state_db_path() -> Result<PathBuf, String> {
    Ok(global_storage_dir()?.join("state.vscdb"))
}

pub fn storage_json_path() -> Result<PathBuf, String> {
    Ok(global_storage_dir()?.join("storage.json"))
}

pub fn machine_id_path() -> Result<PathBuf, String> {
    Ok(default_user_data_dir()?.join("machineid"))
}

pub fn legacy_global_storage_dir() -> Result<PathBuf, String> {
    Ok(legacy_default_user_data_dir()?
        .join("User")
        .join("globalStorage"))
}

pub fn legacy_state_db_path() -> Result<PathBuf, String> {
    Ok(legacy_global_storage_dir()?.join("state.vscdb"))
}

#[cfg(test)]
mod tests {
    use super::prefer_user_data_dir_with_state_db;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn prefers_candidate_that_contains_state_vscdb() {
        let root = std::env::temp_dir().join(format!(
            "ag-path-prefer-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let empty = root.join("empty");
        let with_db = root.join("with-db");
        fs::create_dir_all(empty.join("User").join("globalStorage")).unwrap();
        fs::create_dir_all(with_db.join("User").join("globalStorage")).unwrap();
        fs::write(
            with_db
                .join("User")
                .join("globalStorage")
                .join("state.vscdb"),
            b"db",
        )
        .unwrap();

        let picked = prefer_user_data_dir_with_state_db(
            &[empty.clone(), with_db.clone()],
            PathBuf::from("fallback"),
        );
        assert_eq!(picked, with_db);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn falls_back_to_existing_dir_when_no_state_db() {
        let root = std::env::temp_dir().join(format!(
            "ag-path-fallback-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let first = root.join("first");
        let second = root.join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        let picked =
            prefer_user_data_dir_with_state_db(&[first.clone(), second.clone()], second.clone());
        assert_eq!(picked, first);
        let _ = fs::remove_dir_all(root);
    }
}
