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

pub fn default_user_data_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or("无法获取 Home 目录")?;
        return Ok(home.join("Library/Application Support/Antigravity IDE"));
    }

    #[cfg(target_os = "windows")]
    {
        let roaming_dir = roaming_app_data_dir()?;
        return Ok(roaming_dir.join("Antigravity IDE"));
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().ok_or("无法获取 Home 目录")?;
        return Ok(home.join(".config/Antigravity IDE"));
    }

    #[allow(unreachable_code)]
    Err("无法确定 Antigravity IDE 默认目录".to_string())
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
