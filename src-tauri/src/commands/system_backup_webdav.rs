// System commands：Local backup archives, retention and WebDAV synchronization。
// 通过 include! 保持原 commands::system 作用域和 Tauri command 路径。
fn get_app_auto_launch_enabled(app: &tauri::AppHandle) -> Result<bool, String> {
    app.autolaunch()
        .is_enabled()
        .map_err(|err| format!("读取应用自启动状态失败: {}", err))
}

fn apply_app_auto_launch_enabled(app: &tauri::AppHandle, enabled: bool) -> Result<(), String> {
    if enabled {
        app.autolaunch()
            .enable()
            .map_err(|err| format!("启用应用自启动失败: {}", err))
    } else {
        app.autolaunch()
            .disable()
            .map_err(|err| format!("停用应用自启动失败: {}", err))
    }
}

fn sanitize_ui_scale(raw: f64) -> f64 {
    if !raw.is_finite() {
        return DEFAULT_UI_SCALE;
    }
    raw.clamp(MIN_UI_SCALE, MAX_UI_SCALE)
}

fn resolve_downloads_dir() -> Result<PathBuf, String> {
    if let Some(dir) = dirs::download_dir() {
        return Ok(dir);
    }
    if let Some(home) = dirs::home_dir() {
        return Ok(home.join("Downloads"));
    }
    Err("无法获取下载目录".to_string())
}

fn get_auto_backup_dir_path() -> Result<PathBuf, String> {
    modules::backup_storage::get_backup_root_dir()
}

fn ensure_auto_backup_dir_path() -> Result<PathBuf, String> {
    let dir = get_auto_backup_dir_path()?;
    if !dir.exists() {
        fs::create_dir_all(&dir).map_err(|err| format!("创建自动备份目录失败: {}", err))?;
    }
    Ok(dir)
}

fn build_auto_backup_settings(config: &UserConfig) -> Result<AutoBackupSettings, String> {
    let (include_accounts, include_config) = config::normalize_auto_backup_selection(
        config.auto_backup_include_accounts,
        config.auto_backup_include_config,
    );
    Ok(AutoBackupSettings {
        enabled: config.auto_backup_enabled,
        include_accounts,
        include_config,
        retention_days: config::sanitize_auto_backup_retention_days(
            config.auto_backup_retention_days,
        ),
        last_backup_at: config.auto_backup_last_backup_at.clone(),
        directory_path: get_auto_backup_dir_path()?.to_string_lossy().to_string(),
    })
}

fn build_webdav_sync_settings(config: &UserConfig) -> WebdavSyncSettings {
    let url = modules::webdav_sync::normalize_base_url(&config.webdav_sync_url)
        .unwrap_or_else(|_| config::default_webdav_sync_url());
    let remote_dir = modules::webdav_sync::normalize_remote_dir(&config.webdav_sync_remote_dir)
        .unwrap_or_else(|_| config::default_webdav_sync_remote_dir());

    WebdavSyncSettings {
        enabled: config.webdav_sync_enabled,
        url,
        username: config.webdav_sync_username.clone(),
        has_password: !config.webdav_sync_password.is_empty(),
        remote_dir,
        last_upload_at: config.webdav_sync_last_upload_at.clone(),
        last_upload_file_name: config.webdav_sync_last_upload_file_name.clone(),
        last_download_at: config.webdav_sync_last_download_at.clone(),
        last_download_file_name: config.webdav_sync_last_download_file_name.clone(),
        retention_days: config.webdav_sync_retention_days,
    }
}

fn resolve_webdav_password_update(
    current_password: &str,
    password: Option<String>,
    clear_password: Option<bool>,
) -> String {
    if clear_password.unwrap_or(false) {
        return String::new();
    }
    password
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| current_password.to_string())
}

fn validate_webdav_sync_config(
    enabled: bool,
    url: &str,
    username: &str,
    password: &str,
    remote_dir: &str,
) -> Result<(String, String, String), String> {
    let normalized_url = modules::webdav_sync::normalize_base_url(url)?;
    let normalized_remote_dir = modules::webdav_sync::normalize_remote_dir(remote_dir)?;
    let normalized_username = username.trim().to_string();

    if enabled {
        if normalized_username.is_empty() {
            return Err("启用 WebDAV 同步时账号不能为空".to_string());
        }
        if password.is_empty() {
            return Err("启用 WebDAV 同步时应用密码不能为空".to_string());
        }
    }

    Ok((normalized_url, normalized_username, normalized_remote_dir))
}

fn sanitize_auto_backup_file_name(file_name: &str) -> Result<String, String> {
    let trimmed = file_name.trim();
    if trimmed.is_empty() {
        return Err("备份文件名不能为空".to_string());
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err("备份文件名不合法".to_string());
    }
    if !trimmed.ends_with(".json") && !trimmed.ends_with(".zip") {
        return Err("自动备份文件必须为 JSON 或 ZIP".to_string());
    }
    Ok(trimmed.to_string())
}

fn resolve_auto_backup_file_path(file_name: &str) -> Result<PathBuf, String> {
    let safe_name = sanitize_auto_backup_file_name(file_name)?;
    Ok(get_auto_backup_dir_path()?.join(safe_name))
}

fn auto_backup_archive_file_name(file_name: &str) -> Option<String> {
    file_name
        .strip_suffix(".json")
        .map(|stem| format!("{}.zip", stem))
}

fn auto_backup_json_file_name(file_name: &str) -> Option<String> {
    file_name
        .strip_suffix(".zip")
        .map(|stem| format!("{}.json", stem))
}

fn read_u16_le(bytes: &[u8], offset: usize) -> Option<u16> {
    let slice = bytes.get(offset..offset + 2)?;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Option<u32> {
    let slice = bytes.get(offset..offset + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn push_u16_le(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32_le(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn push_zip_entry(
    out: &mut Vec<u8>,
    central: &mut Vec<(String, u32, u32, u32)>,
    name: &str,
    content: &[u8],
) -> Result<(), String> {
    let name_bytes = name.as_bytes();
    let name_len =
        u16::try_from(name_bytes.len()).map_err(|_| format!("ZIP 条目名过长: {}", name))?;
    let size = u32::try_from(content.len()).map_err(|_| format!("ZIP 条目过大: {}", name))?;
    let offset = u32::try_from(out.len()).map_err(|_| "ZIP 文件过大".to_string())?;
    let crc = crc32(content);
    let dos_time = 0u16;
    let dos_date = 33u16;

    push_u32_le(out, 0x0403_4b50);
    push_u16_le(out, 20);
    push_u16_le(out, 0);
    push_u16_le(out, 0);
    push_u16_le(out, dos_time);
    push_u16_le(out, dos_date);
    push_u32_le(out, crc);
    push_u32_le(out, size);
    push_u32_le(out, size);
    push_u16_le(out, name_len);
    push_u16_le(out, 0);
    out.extend_from_slice(name_bytes);
    out.extend_from_slice(content);

    central.push((name.to_string(), crc, size, offset));
    Ok(())
}

fn build_stored_zip(entries: Vec<(String, Vec<u8>)>) -> Result<Vec<u8>, String> {
    if entries.is_empty() {
        return Err("ZIP 条目不能为空".to_string());
    }

    let mut out = Vec::new();
    let mut central_entries = Vec::new();
    for (name, content) in entries {
        push_zip_entry(&mut out, &mut central_entries, &name, &content)?;
    }

    let central_offset = u32::try_from(out.len()).map_err(|_| "ZIP 文件过大".to_string())?;
    let dos_time = 0u16;
    let dos_date = 33u16;

    for (name, crc, size, offset) in &central_entries {
        let name_bytes = name.as_bytes();
        let name_len =
            u16::try_from(name_bytes.len()).map_err(|_| format!("ZIP 条目名过长: {}", name))?;
        push_u32_le(&mut out, 0x0201_4b50);
        push_u16_le(&mut out, 20);
        push_u16_le(&mut out, 20);
        push_u16_le(&mut out, 0);
        push_u16_le(&mut out, 0);
        push_u16_le(&mut out, dos_time);
        push_u16_le(&mut out, dos_date);
        push_u32_le(&mut out, *crc);
        push_u32_le(&mut out, *size);
        push_u32_le(&mut out, *size);
        push_u16_le(&mut out, name_len);
        push_u16_le(&mut out, 0);
        push_u16_le(&mut out, 0);
        push_u16_le(&mut out, 0);
        push_u16_le(&mut out, 0);
        push_u32_le(&mut out, 0);
        push_u32_le(&mut out, *offset);
        out.extend_from_slice(name_bytes);
    }

    let central_size = u32::try_from(out.len())
        .ok()
        .and_then(|len| len.checked_sub(central_offset))
        .ok_or_else(|| "ZIP 中央目录过大".to_string())?;
    let entry_count =
        u16::try_from(central_entries.len()).map_err(|_| "ZIP 条目过多".to_string())?;

    push_u32_le(&mut out, 0x0605_4b50);
    push_u16_le(&mut out, 0);
    push_u16_le(&mut out, 0);
    push_u16_le(&mut out, entry_count);
    push_u16_le(&mut out, entry_count);
    push_u32_le(&mut out, central_size);
    push_u32_le(&mut out, central_offset);
    push_u16_le(&mut out, 0);

    Ok(out)
}

fn backup_json_from_zip_bytes(bytes: &[u8]) -> Result<String, String> {
    let mut offset = 0usize;
    while offset + 30 <= bytes.len() {
        let Some(signature) = read_u32_le(bytes, offset) else {
            break;
        };
        if signature != 0x0403_4b50 {
            break;
        }
        let compression =
            read_u16_le(bytes, offset + 8).ok_or_else(|| "ZIP 本地文件头不完整".to_string())?;
        let compressed_size =
            read_u32_le(bytes, offset + 18).ok_or_else(|| "ZIP 条目大小缺失".to_string())? as usize;
        let name_len = read_u16_le(bytes, offset + 26)
            .ok_or_else(|| "ZIP 条目名长度缺失".to_string())? as usize;
        let extra_len =
            read_u16_le(bytes, offset + 28).ok_or_else(|| "ZIP 扩展长度缺失".to_string())? as usize;
        let name_start = offset + 30;
        let name_end = name_start + name_len;
        let data_start = name_end + extra_len;
        let data_end = data_start + compressed_size;
        if data_end > bytes.len() {
            return Err("ZIP 条目内容不完整".to_string());
        }
        let name = String::from_utf8_lossy(&bytes[name_start..name_end]);
        if name == "backup.json" {
            if compression != 0 {
                return Err("暂不支持压缩过的 ZIP 备份条目".to_string());
            }
            return String::from_utf8(bytes[data_start..data_end].to_vec())
                .map_err(|_| "ZIP 备份中的 backup.json 不是 UTF-8".to_string());
        }
        offset = data_end;
    }

    Err("ZIP 备份中未找到 backup.json".to_string())
}

fn backup_json_from_path(path: &Path) -> Result<String, String> {
    match path.extension().and_then(|item| item.to_str()) {
        Some("json") => match fs::read_to_string(path) {
            Ok(content) => {
                if serde_json::from_str::<serde_json::Value>(&content).is_ok() {
                    return Ok(content);
                }
                if let Some(file_name) = path.file_name().and_then(|name| name.to_str()) {
                    if let Some(archive_name) = auto_backup_archive_file_name(file_name) {
                        let archive_path = path.with_file_name(archive_name);
                        if archive_path.exists() {
                            return backup_json_from_path(&archive_path);
                        }
                    }
                }
                Ok(content)
            }
            Err(err) => {
                if let Some(file_name) = path.file_name().and_then(|name| name.to_str()) {
                    if let Some(archive_name) = auto_backup_archive_file_name(file_name) {
                        let archive_path = path.with_file_name(archive_name);
                        if archive_path.exists() {
                            return backup_json_from_path(&archive_path);
                        }
                    }
                }
                Err(format!("读取自动备份文件失败: {}", err))
            }
        },
        Some("zip") => {
            let bytes = fs::read(path).map_err(|err| format!("读取自动备份压缩包失败: {}", err))?;
            backup_json_from_zip_bytes(&bytes)
        }
        _ => Err("不支持的自动备份文件类型".to_string()),
    }
}

fn collect_auto_backup_platforms_from_value(
    value: &serde_json::Value,
) -> Vec<AutoBackupPlatformEntry> {
    let accounts = value
        .get("accounts")
        .filter(|item| item.is_object())
        .unwrap_or(value);
    let Some(platforms) = accounts.get("platforms").and_then(|item| item.as_object()) else {
        return Vec::new();
    };

    let mut result = Vec::new();
    for (platform, payload) in platforms {
        let exported_data = payload
            .get("exported_data")
            .or_else(|| payload.get("data"))
            .or_else(|| payload.get("accounts"));
        let account_count = payload
            .get("account_count")
            .and_then(|item| item.as_u64())
            .or_else(|| {
                exported_data
                    .and_then(|item| item.as_array())
                    .map(|items| items.len() as u64)
            })
            .unwrap_or(0);
        if account_count == 0 {
            continue;
        }
        result.push(AutoBackupPlatformEntry {
            platform: platform.clone(),
            account_count,
        });
    }
    result.sort_by(|left, right| left.platform.cmp(&right.platform));
    result
}

fn collect_auto_backup_platforms(json_content: &str) -> Vec<AutoBackupPlatformEntry> {
    serde_json::from_str::<serde_json::Value>(json_content)
        .ok()
        .map(|value| collect_auto_backup_platforms_from_value(&value))
        .unwrap_or_default()
}

fn build_auto_backup_zip_bytes(file_name: &str, content: &str) -> Result<Vec<u8>, String> {
    let root = serde_json::from_str::<serde_json::Value>(content)
        .map_err(|err| format!("自动备份 JSON 解析失败，无法生成 ZIP: {}", err))?;
    let platforms = collect_auto_backup_platforms_from_value(&root);
    let manifest = serde_json::json!({
        "schema": "cockpit-tools.auto-backup-archive",
        "version": 1,
        "source_file_name": file_name,
        "platforms": &platforms,
        "sections": root.get("sections").cloned().unwrap_or(serde_json::Value::Null),
        "exported_at": root.get("exported_at").cloned().unwrap_or(serde_json::Value::Null),
    });

    let mut entries = vec![
        ("backup.json".to_string(), content.as_bytes().to_vec()),
        (
            "manifest.json".to_string(),
            serde_json::to_vec_pretty(&manifest)
                .map_err(|err| format!("序列化 ZIP 清单失败: {}", err))?,
        ),
    ];

    if let Some(accounts) = root.get("accounts").filter(|item| item.is_object()) {
        if let Some(platforms_map) = accounts.get("platforms").and_then(|item| item.as_object()) {
            for platform in &platforms {
                if let Some(payload) = platforms_map.get(&platform.platform) {
                    let exported_data = payload
                        .get("exported_data")
                        .or_else(|| payload.get("data"))
                        .or_else(|| payload.get("accounts"))
                        .cloned()
                        .unwrap_or_else(|| serde_json::json!([]));
                    entries.push((
                        format!("accounts/{}.json", platform.platform),
                        serde_json::to_vec_pretty(&exported_data)
                            .map_err(|err| format!("序列化平台备份失败: {}", err))?,
                    ));
                }
            }
        }
    } else if let Some(platforms_map) = root.get("platforms").and_then(|item| item.as_object()) {
        for platform in &platforms {
            if let Some(payload) = platforms_map.get(&platform.platform) {
                let exported_data = payload
                    .get("exported_data")
                    .or_else(|| payload.get("data"))
                    .or_else(|| payload.get("accounts"))
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!([]));
                entries.push((
                    format!("accounts/{}.json", platform.platform),
                    serde_json::to_vec_pretty(&exported_data)
                        .map_err(|err| format!("序列化平台备份失败: {}", err))?,
                ));
            }
        }
    }

    build_stored_zip(entries)
}

fn system_time_to_unix_ms(value: SystemTime) -> Option<i64> {
    value
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
}

fn open_path_in_system(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| format!("打开目录失败: {}", e))?;
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(path)
            .spawn()
            .map_err(|e| format!("打开目录失败: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| format!("打开目录失败: {}", e))?;
    }

    Ok(())
}

#[tauri::command]
pub async fn open_data_folder() -> Result<(), String> {
    let path = modules::account::get_data_dir()?;
    open_path_in_system(path.as_path())
}

#[tauri::command]
pub fn open_local_path(path: String) -> Result<(), String> {
    let p = std::path::PathBuf::from(path.trim());
    if !p.exists() {
        return Err(format!("路径不存在: {}", p.display()));
    }
    open_path_in_system(p.as_path())
}

#[tauri::command]
pub async fn windows_elevated_close_processes(pids: Vec<u32>) -> Result<u32, String> {
    tauri::async_runtime::spawn_blocking(move || {
        modules::windows_operation::elevated_close_supported_processes(&pids)
    })
    .await
    .map_err(|error| format!("WINDOWS_ELEVATION_TASK_FAILED: {}", error))?
}

/// 保存文本文件
#[tauri::command]
pub async fn save_text_file(path: String, content: String) -> Result<(), String> {
    modules::atomic_write::write_string_atomic(std::path::Path::new(&path), &content)
}

/// 获取下载目录
#[tauri::command]
pub fn get_downloads_dir() -> Result<String, String> {
    Ok(resolve_downloads_dir()?.to_string_lossy().to_string())
}

#[tauri::command]
pub fn get_auto_backup_settings() -> Result<AutoBackupSettings, String> {
    let config = config::get_user_config();
    build_auto_backup_settings(&config)
}

#[tauri::command]
pub fn save_auto_backup_settings(
    enabled: bool,
    include_accounts: bool,
    include_config: bool,
    retention_days: i32,
) -> Result<AutoBackupSettings, String> {
    let (next_include_accounts, next_include_config) =
        config::normalize_auto_backup_selection(include_accounts, include_config);
    let next_retention_days = config::sanitize_auto_backup_retention_days(retention_days);
    let new_config = config::patch_user_config(move |current| {
        current.auto_backup_enabled = enabled;
        current.auto_backup_include_accounts = next_include_accounts;
        current.auto_backup_include_config = next_include_config;
        current.auto_backup_retention_days = next_retention_days;
        Ok(())
    })?;
    build_auto_backup_settings(&new_config)
}

#[tauri::command]
pub fn update_auto_backup_last_run(
    last_backup_at: Option<String>,
) -> Result<AutoBackupSettings, String> {
    let normalized_last_backup_at = last_backup_at.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });
    let new_config = config::patch_user_config(move |current| {
        current.auto_backup_last_backup_at = normalized_last_backup_at;
        Ok(())
    })?;
    build_auto_backup_settings(&new_config)
}

#[tauri::command]
pub fn write_auto_backup_file(file_name: String, content: String) -> Result<String, String> {
    modules::backup_storage::ensure_backup_write_available()?;
    let safe_name = sanitize_auto_backup_file_name(&file_name)?;
    if !safe_name.ends_with(".json") {
        return Err("自动备份主文件必须为 JSON".to_string());
    }
    let dir = ensure_auto_backup_dir_path()?;
    let path = dir.join(&safe_name);
    crate::modules::atomic_write::write_string_atomic(&path, &content)
        .map_err(|err| format!("写入自动备份文件失败: {}", err))?;

    if let Some(archive_name) = auto_backup_archive_file_name(&safe_name) {
        let archive_path = dir.join(archive_name);
        let zip_bytes = build_auto_backup_zip_bytes(&safe_name, &content)?;
        crate::modules::atomic_write::write_bytes_atomic(&archive_path, &zip_bytes)
            .map_err(|err| format!("写入自动备份压缩包失败: {}", err))?;
    }

    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn read_auto_backup_file(file_name: String) -> Result<String, String> {
    let path = resolve_auto_backup_file_path(&file_name)?;
    backup_json_from_path(&path)
}

#[tauri::command]
pub fn copy_auto_backup_file(file_name: String, target_path: String) -> Result<String, String> {
    let source_path = resolve_auto_backup_file_path(&file_name)?;
    if !source_path.exists() {
        return Err("备份文件不存在".to_string());
    }
    let target = PathBuf::from(target_path.trim());
    if target.as_os_str().is_empty() {
        return Err("目标路径不能为空".to_string());
    }
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| format!("创建下载目录失败: {}", err))?;
        }
    }
    fs::copy(&source_path, &target).map_err(|err| format!("复制备份文件失败: {}", err))?;
    Ok(target.to_string_lossy().to_string())
}

#[tauri::command]
pub fn list_auto_backup_files() -> Result<Vec<AutoBackupFileEntry>, String> {
    let dir = get_auto_backup_dir_path()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    let entries = fs::read_dir(&dir).map_err(|err| format!("读取自动备份目录失败: {}", err))?;
    let mut json_stems = std::collections::HashSet::new();

    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| format!("读取自动备份文件失败: {}", err))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        paths.push(path);
    }

    for path in &paths {
        if let Some(stem) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".json"))
        {
            json_stems.insert(stem.to_string());
        }
    }

    for path in paths {
        let file_name = match path.file_name().and_then(|name| name.to_str()) {
            Some(name) if name.ends_with(".json") || name.ends_with(".zip") => name.to_string(),
            _ => continue,
        };
        if file_name.ends_with(".zip") {
            if let Some(json_name) = auto_backup_json_file_name(&file_name) {
                if json_stems.contains(json_name.trim_end_matches(".json")) {
                    continue;
                }
            }
        }
        let metadata =
            fs::metadata(&path).map_err(|err| format!("读取备份文件信息失败: {}", err))?;
        let archive_name = if file_name.ends_with(".json") {
            auto_backup_archive_file_name(&file_name)
        } else {
            None
        };
        let archive_path = archive_name
            .as_ref()
            .map(|name| dir.join(name))
            .filter(|path| path.exists());
        let archive_metadata = archive_path
            .as_ref()
            .and_then(|path| fs::metadata(path).ok());
        let json_content = backup_json_from_path(&path).ok();
        files.push(AutoBackupFileEntry {
            file_name,
            path: path.to_string_lossy().to_string(),
            file_kind: path
                .extension()
                .and_then(|item| item.to_str())
                .unwrap_or("json")
                .to_string(),
            size_bytes: metadata.len(),
            modified_at_ms: metadata.modified().ok().and_then(system_time_to_unix_ms),
            archive_file_name: archive_path
                .as_ref()
                .and_then(|path| path.file_name().and_then(|name| name.to_str()))
                .map(|name| name.to_string()),
            archive_path: archive_path
                .as_ref()
                .map(|path| path.to_string_lossy().to_string()),
            archive_size_bytes: archive_metadata.map(|metadata| metadata.len()),
            platforms: json_content
                .as_deref()
                .map(collect_auto_backup_platforms)
                .unwrap_or_default(),
        });
    }

    files.sort_by(|left, right| {
        right
            .modified_at_ms
            .unwrap_or_default()
            .cmp(&left.modified_at_ms.unwrap_or_default())
            .then_with(|| right.file_name.cmp(&left.file_name))
    });

    Ok(files)
}

#[tauri::command]
pub fn delete_auto_backup_file(file_name: String) -> Result<(), String> {
    let path = resolve_auto_backup_file_path(&file_name)?;
    if !path.exists() {
        return Ok(());
    }
    fs::remove_file(&path).map_err(|err| format!("删除自动备份文件失败: {}", err))?;
    if file_name.ends_with(".json") {
        if let Some(archive_name) = auto_backup_archive_file_name(&file_name) {
            let archive_path = resolve_auto_backup_file_path(&archive_name)?;
            if archive_path.exists() {
                fs::remove_file(&archive_path)
                    .map_err(|err| format!("删除自动备份压缩包失败: {}", err))?;
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub fn cleanup_auto_backup_files(retention_days: i32) -> Result<Vec<String>, String> {
    let dir = get_auto_backup_dir_path()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let normalized_retention_days = config::sanitize_auto_backup_retention_days(retention_days);
    let now = SystemTime::now();
    let cutoff = now
        .checked_sub(Duration::from_secs(
            normalized_retention_days as u64 * 24 * 60 * 60,
        ))
        .unwrap_or(now);

    let mut deleted = Vec::new();
    let entries = fs::read_dir(&dir).map_err(|err| format!("读取自动备份目录失败: {}", err))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("读取自动备份文件失败: {}", err))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = match path.file_name().and_then(|name| name.to_str()) {
            Some(name) if name.ends_with(".json") || name.ends_with(".zip") => name.to_string(),
            _ => continue,
        };
        let metadata =
            fs::metadata(&path).map_err(|err| format!("读取备份文件信息失败: {}", err))?;
        let modified = match metadata.modified() {
            Ok(value) => value,
            Err(_) => continue,
        };
        if modified >= cutoff {
            continue;
        }
        fs::remove_file(&path).map_err(|err| format!("清理过期备份失败: {}", err))?;
        deleted.push(file_name);
    }

    deleted.sort();
    Ok(deleted)
}

#[tauri::command]
pub fn open_auto_backup_dir() -> Result<(), String> {
    let path = ensure_auto_backup_dir_path()?;
    open_path_in_system(path.as_path())
}

/// 获取定时备份与行为备份的空间占用明细。
#[tauri::command]
pub fn get_backup_usage() -> Result<modules::backup_storage::BackupUsageSummary, String> {
    modules::backup_storage::get_backup_usage()
}

/// 修改本地备份根目录；迁移选项由前端在确认后传入。
#[tauri::command]
pub async fn preview_backup_directory_change(
    directory: String,
) -> Result<modules::backup_storage::BackupDirectoryMigrationPreview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        modules::backup_storage::preview_backup_root_dir_change(&directory)
    })
    .await
    .map_err(|error| format!("扫描待迁移备份失败: {}", error))?
}

/// 修改本地备份根目录，并通过事件上报迁移进度。
#[tauri::command]
pub async fn change_backup_directory(
    app: tauri::AppHandle,
    directory: String,
    migrate_existing: bool,
    migration_id: String,
) -> Result<modules::backup_storage::BackupDirectoryChangeResult, String> {
    tauri::async_runtime::spawn_blocking(move || {
        modules::backup_storage::change_backup_root_dir_with_progress(
            &directory,
            migrate_existing,
            &migration_id,
            |progress| {
                let _ = app.emit("backup-directory-migration-progress", progress);
            },
        )
    })
    .await
    .map_err(|error| format!("备份目录迁移任务失败: {}", error))?
}

/// 请求取消仍处于扫描或复制阶段的备份目录迁移。
#[tauri::command]
pub fn cancel_backup_directory_change(migration_id: String) -> Result<bool, String> {
    modules::backup_storage::cancel_backup_root_dir_change(&migration_id)
}

/// 清理行为快照，只保留每个来源/实例/类型的最新一份。
#[tauri::command]
pub fn cleanup_behavior_backups() -> Result<modules::backup_storage::BackupCleanupResult, String> {
    modules::backup_storage::cleanup_behavior_backups()
}

#[tauri::command]
pub fn get_webdav_sync_settings() -> Result<WebdavSyncSettings, String> {
    let config = config::get_user_config();
    Ok(build_webdav_sync_settings(&config))
}

#[tauri::command]
pub fn save_webdav_sync_settings(
    enabled: bool,
    url: String,
    username: String,
    password: Option<String>,
    clear_password: Option<bool>,
    remote_dir: String,
    retention_days: i32,
) -> Result<WebdavSyncSettings, String> {
    let new_config = config::patch_user_config(move |current| {
        let next_password =
            resolve_webdav_password_update(&current.webdav_sync_password, password, clear_password);
        let (next_url, next_username, next_remote_dir) =
            validate_webdav_sync_config(enabled, &url, &username, &next_password, &remote_dir)?;

        current.webdav_sync_enabled = enabled;
        current.webdav_sync_url = next_url;
        current.webdav_sync_username = next_username;
        current.webdav_sync_password = next_password;
        current.webdav_sync_remote_dir = next_remote_dir;
        current.webdav_sync_retention_days =
            config::sanitize_webdav_sync_retention_days(retention_days);
        Ok(())
    })?;
    Ok(build_webdav_sync_settings(&new_config))
}

#[tauri::command]
pub async fn test_webdav_sync_connection(
    url: String,
    username: String,
    password: Option<String>,
    clear_password: Option<bool>,
    remote_dir: String,
) -> Result<modules::webdav_sync::WebdavTestResult, String> {
    let current = config::get_user_config();
    let next_password =
        resolve_webdav_password_update(&current.webdav_sync_password, password, clear_password);
    let connection =
        modules::webdav_sync::connection_from_parts(&url, &username, &next_password, &remote_dir)?;
    modules::webdav_sync::test_connection(&connection).await
}

#[tauri::command]
pub async fn upload_auto_backup_to_webdav(
    file_name: String,
) -> Result<modules::webdav_sync::WebdavUploadResult, String> {
    let config = config::get_user_config();
    if !config.webdav_sync_enabled {
        return Err("WebDAV 同步未启用".to_string());
    }

    let connection = modules::webdav_sync::connection_from_config(&config)?;
    let safe_name = sanitize_auto_backup_file_name(&file_name)?;
    if !safe_name.ends_with(".json") {
        return Err("WebDAV 同步入口文件必须为 JSON 备份".to_string());
    }

    let archive_name = auto_backup_archive_file_name(&safe_name)
        .ok_or_else(|| "无法获取对应的压缩包名称".to_string())?;
    let archive_path = resolve_auto_backup_file_path(&archive_name)?;
    if !archive_path.exists() {
        return Err("本地备份压缩包不存在".to_string());
    }

    let archive_bytes =
        fs::read(&archive_path).map_err(|err| format!("读取本地备份压缩包失败: {}", err))?;

    let sync_client = modules::webdav_sync::WebdavSyncClient::new(&connection)?;

    let mut uploaded_files = Vec::new();
    uploaded_files.push(
        sync_client
            .upload_backup_bytes(&archive_name, archive_bytes)
            .await?,
    );

    let deleted_files = sync_client
        .cleanup_remote_backups(config::sanitize_webdav_sync_retention_days(
            config.webdav_sync_retention_days,
        ))
        .await?;
    let uploaded_at = chrono::Utc::now().to_rfc3339();
    let remote_dir = connection.remote_dir.clone();

    config::patch_user_config(|current| {
        current.webdav_sync_last_upload_at = Some(uploaded_at.clone());
        current.webdav_sync_last_upload_file_name = Some(archive_name.clone());
        Ok(())
    })?;

    Ok(modules::webdav_sync::WebdavUploadResult {
        uploaded_files,
        deleted_files,
        uploaded_at,
        remote_dir,
    })
}

#[tauri::command]
pub async fn list_webdav_backup_files(
) -> Result<Vec<modules::webdav_sync::WebdavBackupFileEntry>, String> {
    let config = config::get_user_config();
    let connection = modules::webdav_sync::connection_from_config(&config)?;
    modules::webdav_sync::list_remote_backups(&connection).await
}

#[tauri::command]
pub async fn read_webdav_backup_file(file_name: String) -> Result<String, String> {
    let config = config::get_user_config();
    let safe_name = sanitize_auto_backup_file_name(&file_name)?;
    let connection = modules::webdav_sync::connection_from_config(&config)?;

    let downloaded_at = chrono::Utc::now().to_rfc3339();
    let content = if safe_name.ends_with(".zip") {
        let bytes = modules::webdav_sync::read_remote_backup_bytes(&connection, &safe_name).await?;
        backup_json_from_zip_bytes(&bytes)?
    } else if safe_name.ends_with(".json") {
        modules::webdav_sync::read_remote_backup(&connection, &safe_name).await?
    } else {
        return Err("不支持的备份文件格式".to_string());
    };

    config::patch_user_config(move |current| {
        current.webdav_sync_last_download_at = Some(downloaded_at);
        current.webdav_sync_last_download_file_name = Some(safe_name);
        Ok(())
    })?;
    Ok(content)
}

#[tauri::command]
pub async fn delete_webdav_backup_file(file_name: String) -> Result<(), String> {
    let config = config::get_user_config();
    let safe_name = sanitize_auto_backup_file_name(&file_name)?;
    let connection = modules::webdav_sync::connection_from_config(&config)?;
    modules::webdav_sync::delete_remote_backup(&connection, &safe_name).await
}
