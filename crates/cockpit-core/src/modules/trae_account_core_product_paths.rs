// cockpit-core Trae 账号：Product paths, platform metadata and auth client discovery。
// 通过 include! 保持原模块作用域和平台调用路径。
pub fn get_default_trae_data_dir_for_platform(
    platform: TraePlatformKind,
) -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
        return Ok(home
            .join("Library/Application Support")
            .join(platform.app_support_dir_name()));
    }

    #[cfg(target_os = "windows")]
    {
        let appdata =
            std::env::var("APPDATA").map_err(|_| "无法获取 APPDATA 环境变量".to_string())?;
        return Ok(PathBuf::from(appdata).join(platform.app_support_dir_name()));
    }

    #[cfg(target_os = "linux")]
    {
        let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
        return Ok(home.join(".config").join(platform.app_support_dir_name()));
    }

    #[allow(unreachable_code)]
    Err("Trae 仅支持 macOS、Windows 和 Linux".to_string())
}

pub fn get_default_trae_data_dir() -> Result<PathBuf, String> {
    get_default_trae_data_dir_for_platform(TraePlatformKind::Trae)
}

pub fn get_default_trae_storage_path_for_platform(
    platform: TraePlatformKind,
) -> Result<PathBuf, String> {
    Ok(get_default_trae_data_dir_for_platform(platform)?
        .join("User")
        .join("globalStorage")
        .join("storage.json"))
}

pub fn get_default_trae_storage_path() -> Result<PathBuf, String> {
    get_default_trae_storage_path_for_platform(TraePlatformKind::Trae)
}

fn read_storage_json(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Err(format!("Trae storage.json 不存在: {}", path.display()));
    }

    let content = fs::read_to_string(path)
        .map_err(|e| format!("读取 Trae storage.json 失败({}): {}", path.display(), e))?;
    if content.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }

    serde_json::from_str::<Value>(&content)
        .map_err(|e| format!("解析 Trae storage.json 失败({}): {}", path.display(), e))
}

fn write_storage_json(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 Trae 目录失败: {}", e))?;
    }
    let content = serde_json::to_string_pretty(value)
        .map_err(|e| format!("序列化 Trae storage.json 失败: {}", e))?;
    crate::modules::atomic_write::write_string_atomic(path, &content)
        .map_err(|e| format!("写入 Trae storage.json 失败: {}", e))
}

fn to_json_string_value(value: &Value) -> Result<Value, String> {
    let text =
        serde_json::to_string(value).map_err(|e| format!("序列化 Trae 存储键值失败: {}", e))?;
    Ok(Value::String(text))
}

fn to_icube_cipher_string_value(value: &Value) -> Result<Value, String> {
    let plaintext =
        serde_json::to_string(value).map_err(|e| format!("序列化 Trae 存储键值失败: {}", e))?;
    let encrypted = byte_crypto_encrypt_v1(plaintext.as_bytes())?;
    Ok(Value::String(BASE64_STANDARD.encode(encrypted)))
}

fn pick_string_multi(roots: &[Option<&Value>], paths: &[&[&str]]) -> Option<String> {
    for root in roots {
        if let Some(value) = pick_string(*root, paths) {
            return Some(value);
        }
    }
    None
}

fn pick_i64_multi(roots: &[Option<&Value>], paths: &[&[&str]]) -> Option<i64> {
    for root in roots {
        if let Some(value) = pick_i64(*root, paths) {
            return Some(value);
        }
    }
    None
}

fn json_value_to_non_empty_string(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return normalize_non_empty(Some(text));
    }
    if let Some(num) = value.as_i64() {
        return Some(num.to_string());
    }
    if let Some(num) = value.as_u64() {
        return Some(num.to_string());
    }
    None
}

fn parse_json_file(path: &Path) -> Option<Value> {
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str::<Value>(&content).ok()
}

fn is_probable_executable_path(path: &Path) -> bool {
    if path.is_file() {
        return true;
    }
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("exe"))
        .unwrap_or(false)
}

fn build_trae_product_file_candidates(base_path: &Path) -> Vec<PathBuf> {
    let mut app_roots: Vec<PathBuf> = Vec::new();
    let base_path_string = base_path.to_string_lossy().to_string();

    if let Some(app_idx) = base_path_string.find(".app") {
        app_roots.push(PathBuf::from(&base_path_string[..app_idx + 4]));
    }
    if base_path.is_dir() {
        app_roots.push(base_path.to_path_buf());
    }
    if is_probable_executable_path(base_path) {
        if let Some(parent) = base_path.parent() {
            app_roots.push(parent.to_path_buf());
        }
    }
    if app_roots.is_empty() {
        app_roots.push(base_path.to_path_buf());
    }

    let mut candidates = Vec::new();
    for root in app_roots {
        candidates.extend([
            root.join("Contents")
                .join("Resources")
                .join("app")
                .join("product.json"),
            root.join("Contents")
                .join("Resources")
                .join("app")
                .join("package.json"),
            root.join("resources").join("app").join("product.json"),
            root.join("resources").join("app").join("package.json"),
            root.join("product.json"),
            root.join("package.json"),
        ]);
    }
    candidates
}

#[cfg(target_os = "windows")]
fn trae_product_exe_names(platform: TraePlatformKind) -> &'static [&'static str] {
    match platform {
        TraePlatformKind::Trae => &["Trae.exe"],
        TraePlatformKind::TraeSolo => &["TRAE SOLO.exe", "Trae.exe", "Electron.exe"],
        TraePlatformKind::TraeCn => &["Trae CN.exe", "Trae.exe", "Electron.exe"],
        TraePlatformKind::TraeSoloCn => &["TRAE SOLO CN.exe", "Trae.exe", "Electron.exe"],
    }
}

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[cfg(target_os = "windows")]
fn windows_cmd_output_utf16(args: &[&str]) -> Option<std::process::Output> {
    use std::os::windows::process::CommandExt;

    let mut command = std::process::Command::new("cmd");
    command.args(args);
    command.creation_flags(CREATE_NO_WINDOW);
    command.output().ok()
}

#[cfg(target_os = "windows")]
fn decode_utf16le(bytes: &[u8]) -> String {
    let words: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();
    String::from_utf16_lossy(&words)
}

#[cfg(target_os = "windows")]
fn registry_line_value(line: &str) -> Option<String> {
    let pos = line.find("REG_")?;
    let after = &line[pos..];
    let value_start = after.find(char::is_whitespace)?;
    let value = after[value_start..].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[cfg(target_os = "windows")]
fn reg_query_value(key: &str, value_name: &str) -> Option<String> {
    let cmd = format!("reg query \"{}\" /v \"{}\"", key, value_name);
    let output = windows_cmd_output_utf16(&["/u", "/c", cmd.as_str()])?;
    if !output.status.success() {
        return None;
    }

    let stdout = decode_utf16le(output.stdout.as_slice());
    let value_name_lower = value_name.to_ascii_lowercase();
    stdout.lines().find_map(|line| {
        let trimmed = line.trim();
        if trimmed
            .to_ascii_lowercase()
            .starts_with(value_name_lower.as_str())
        {
            registry_line_value(trimmed)
        } else {
            None
        }
    })
}

#[cfg(target_os = "windows")]
fn normalize_windows_uninstall_display_name(value: &str) -> String {
    let mut normalized = value
        .trim()
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if let Some(stripped) = normalized.strip_suffix(" (user)") {
        normalized = stripped.to_string();
    }
    normalized
}

#[cfg(target_os = "windows")]
fn windows_uninstall_display_names(platform: TraePlatformKind) -> &'static [&'static str] {
    match platform {
        TraePlatformKind::Trae => &["Trae"],
        TraePlatformKind::TraeSolo => &["TRAE SOLO"],
        TraePlatformKind::TraeCn => &["Trae CN"],
        TraePlatformKind::TraeSoloCn => &["TRAE SOLO CN", "TRAE Work CN"],
    }
}

#[cfg(target_os = "windows")]
fn windows_uninstall_display_name_matches(platform: TraePlatformKind, display_name: &str) -> bool {
    let display_name = normalize_windows_uninstall_display_name(display_name);
    windows_uninstall_display_names(platform)
        .iter()
        .any(|expected| normalize_windows_uninstall_display_name(expected) == display_name)
}

#[cfg(target_os = "windows")]
fn normalize_windows_registry_path(value: &str) -> Option<PathBuf> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let path = if let Some(rest) = trimmed.strip_prefix('"') {
        rest.split('"').next().unwrap_or(rest).trim()
    } else if let Some(pos) = trimmed.to_ascii_lowercase().find(".exe") {
        &trimmed[..pos + 4]
    } else {
        trimmed.split(',').next().unwrap_or(trimmed).trim()
    };

    let path = path.trim_matches('"').trim();
    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

#[cfg(target_os = "windows")]
fn push_windows_install_dir_candidates(
    candidates: &mut Vec<PathBuf>,
    install_dir: &str,
    platform: TraePlatformKind,
) {
    let Some(root) = normalize_windows_registry_path(install_dir) else {
        return;
    };
    for exe_name in trae_product_exe_names(platform) {
        candidates.push(root.join(exe_name));
    }
    candidates.push(root);
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_trae_install_base_paths(platform: TraePlatformKind) -> Vec<PathBuf> {
    let uninstall_roots = [
        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        "HKLM\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        "HKLM\\Software\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
    ];
    let mut matched_keys = Vec::new();

    for root in uninstall_roots {
        let cmd = format!("reg query \"{}\" /s /v DisplayName", root);
        let Some(output) = windows_cmd_output_utf16(&["/u", "/c", cmd.as_str()]) else {
            continue;
        };
        if !output.status.success() {
            continue;
        }

        let stdout = decode_utf16le(output.stdout.as_slice());
        let mut current_key: Option<String> = None;
        for line in stdout.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("HKEY_") {
                current_key = Some(trimmed.to_string());
                continue;
            }
            if !trimmed.to_ascii_lowercase().starts_with("displayname") {
                continue;
            }
            let Some(display_name) = registry_line_value(trimmed) else {
                continue;
            };
            if windows_uninstall_display_name_matches(platform, display_name.as_str()) {
                if let Some(key) = current_key.as_ref() {
                    matched_keys.push(key.clone());
                }
            }
        }
    }

    let mut candidates = Vec::new();
    for key in matched_keys {
        if let Some(display_icon) = reg_query_value(key.as_str(), "DisplayIcon") {
            if let Some(exe_path) = normalize_windows_registry_path(display_icon.as_str()) {
                if let Some(parent) = exe_path.parent() {
                    candidates.push(parent.to_path_buf());
                }
                candidates.push(exe_path);
            }
        }
        if let Some(install_location) = reg_query_value(key.as_str(), "InstallLocation") {
            push_windows_install_dir_candidates(
                &mut candidates,
                install_location.as_str(),
                platform,
            );
        }
    }

    let mut dedup = BTreeMap::new();
    for candidate in candidates {
        dedup
            .entry(candidate.to_string_lossy().to_string())
            .or_insert(candidate);
    }
    dedup.into_values().collect()
}

#[cfg(target_os = "windows")]
fn normalize_windows_scan_root(raw: &str) -> Option<PathBuf> {
    let mut value = raw
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim()
        .to_string();
    if value.is_empty() {
        return None;
    }
    if value.len() == 2 && value.as_bytes().get(1) == Some(&b':') {
        value.push('\\');
    }
    let path = PathBuf::from(value);
    if path.is_dir() {
        Some(path)
    } else {
        None
    }
}

#[cfg(target_os = "windows")]
fn is_windows_drive_root(path: &Path) -> bool {
    let value = path.to_string_lossy().replace('/', "\\");
    let trimmed = value.trim_end_matches('\\');
    trimmed.len() == 2
        && trimmed.as_bytes().get(1) == Some(&b':')
        && trimmed.as_bytes()[0].is_ascii_alphabetic()
}

#[cfg(target_os = "windows")]
fn expand_windows_scan_roots_for_trae(roots: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut expanded = Vec::new();
    for root in roots {
        if is_windows_drive_root(&root) {
            expanded.push(root.join("Program Files"));
            expanded.push(root.join("Program Files (x86)"));
            let users_dir = root.join("Users");
            if let Ok(entries) = fs::read_dir(users_dir) {
                for entry in entries.flatten() {
                    expanded.push(entry.path().join("AppData").join("Local").join("Programs"));
                }
            }
        } else {
            expanded.push(root);
        }
    }

    let mut dedup = BTreeMap::new();
    for root in expanded {
        if root.is_dir() {
            dedup
                .entry(root.to_string_lossy().to_ascii_lowercase())
                .or_insert(root);
        }
    }
    dedup.into_values().collect()
}

#[cfg(target_os = "windows")]
pub(crate) fn windows_trae_scan_root_base_paths(platform: TraePlatformKind) -> Vec<PathBuf> {
    let scan_roots = trae_configured_app_scan_roots(platform);
    if scan_roots.trim().is_empty() {
        return Vec::new();
    }

    let roots = scan_roots
        .split(|ch| matches!(ch, '\n' | '\r' | ';' | ','))
        .filter_map(normalize_windows_scan_root)
        .collect::<Vec<_>>();
    let app_dir = platform.app_support_dir_name();
    let mut candidates = Vec::new();
    for root in expand_windows_scan_roots_for_trae(roots) {
        let root_is_app_dir = root
            .file_name()
            .and_then(|value| value.to_str())
            .map(|value| value.eq_ignore_ascii_case(app_dir))
            .unwrap_or(false);
        if root_is_app_dir {
            for exe_name in trae_product_exe_names(platform) {
                candidates.push(root.join(exe_name));
            }
            candidates.push(root.clone());
        }

        let install_dir = root.join(app_dir);
        for exe_name in trae_product_exe_names(platform) {
            candidates.push(install_dir.join(exe_name));
        }
        candidates.push(install_dir);
    }

    let mut dedup = BTreeMap::new();
    for candidate in candidates {
        dedup
            .entry(candidate.to_string_lossy().to_ascii_lowercase())
            .or_insert(candidate);
    }
    dedup.into_values().collect()
}

#[cfg(target_os = "linux")]
fn trae_product_linux_base_paths(platform: TraePlatformKind) -> &'static [&'static str] {
    match platform {
        TraePlatformKind::Trae => &[
            "/usr/bin/trae",
            "/usr/local/bin/trae",
            "/opt/trae/trae",
            "/opt/Trae",
        ],
        TraePlatformKind::TraeSolo => &[
            "/usr/bin/trae-solo",
            "/usr/local/bin/trae-solo",
            "/opt/trae-solo/trae-solo",
            "/opt/TRAE SOLO",
        ],
        TraePlatformKind::TraeCn => &[
            "/usr/bin/trae-cn",
            "/usr/local/bin/trae-cn",
            "/opt/trae-cn/trae-cn",
            "/opt/Trae CN",
        ],
        TraePlatformKind::TraeSoloCn => &[
            "/usr/bin/trae-solo-cn",
            "/usr/local/bin/trae-solo-cn",
            "/opt/trae-solo-cn/trae-solo-cn",
            "/opt/TRAE SOLO CN",
        ],
    }
}

fn trae_product_base_paths(platform: TraePlatformKind) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let configured_path = trae_configured_app_path(platform).trim().to_string();
    if !configured_path.is_empty() {
        candidates.push(PathBuf::from(configured_path));
    }

    #[cfg(target_os = "macos")]
    {
        let app_root = PathBuf::from("/Applications").join(platform.macos_app_name());
        candidates.push(app_root.clone());
        candidates.push(app_root.join("Contents"));
    }

    #[cfg(target_os = "windows")]
    {
        candidates.extend(windows_trae_install_base_paths(platform));
        candidates.extend(windows_trae_scan_root_base_paths(platform));

        let app_dir = platform.app_support_dir_name();
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            let programs_dir = PathBuf::from(&local_app_data)
                .join("Programs")
                .join(app_dir);
            for exe_name in trae_product_exe_names(platform) {
                candidates.push(programs_dir.join(exe_name));
            }
            candidates.push(programs_dir);
        }
        if let Ok(program_files) = std::env::var("ProgramFiles") {
            let install_dir = PathBuf::from(&program_files).join(app_dir);
            for exe_name in trae_product_exe_names(platform) {
                candidates.push(install_dir.join(exe_name));
            }
            candidates.push(install_dir);
        }
    }

    #[cfg(target_os = "linux")]
    {
        for candidate in trae_product_linux_base_paths(platform) {
            candidates.push(PathBuf::from(candidate));
        }
    }

    candidates
}

fn product_auth_config_group(platform: TraePlatformKind) -> &'static str {
    if platform.is_solo() {
        "SOLO"
    } else {
        "TRAE"
    }
}

fn read_product_auth_client_id(root: &Value, platform: TraePlatformKind) -> Option<String> {
    let group = product_auth_config_group(platform);
    let entries = root
        .get("iCubeApp")?
        .get("authConfig")?
        .get(group)?
        .as_object()?;
    let app_type = pick_string(Some(root), &[&["quality"]]).map(|value| value.to_lowercase());

    if let Some(quality) = app_type.and_then(|value| normalize_non_empty(Some(value.as_str()))) {
        if let Some(client_id) = entries
            .get(quality.as_str())
            .and_then(json_value_to_non_empty_string)
        {
            return Some(client_id);
        }
    }

    if let Some(client_id) = entries
        .get("stable")
        .and_then(json_value_to_non_empty_string)
    {
        return Some(client_id);
    }

    entries.values().find_map(json_value_to_non_empty_string)
}

fn detect_product_auth_client_id(platform: TraePlatformKind) -> Option<String> {
    for base_path in trae_product_base_paths(platform) {
        for candidate in build_trae_product_file_candidates(base_path.as_path()) {
            let Some(root) = parse_json_file(candidate.as_path()) else {
                continue;
            };
            if let Some(client_id) = read_product_auth_client_id(&root, platform) {
                return Some(client_id);
            }
        }
    }
    None
}

fn resolve_auth_client_id_from_roots(
    roots: &[Option<&Value>],
    platform: TraePlatformKind,
) -> String {
    let paths: &[&[&str]] = &[
        &["authClientId"],
        &["clientId"],
        &["ClientID"],
        &["platform", "authClientId"],
        &["platform", "clientId"],
        &["exchangeResponse", "ClientID"],
        &["exchangeResponse", "Result", "ClientID"],
        &["Result", "ClientID"],
        &["data", "ClientID"],
    ];
    let fallback = platform.auth_client_id();
    let mut first: Option<String> = None;

    for root in roots {
        for path in paths {
            let Some(candidate) = pick_string(*root, &[*path])
                .and_then(|value| normalize_non_empty(Some(value.as_str())))
            else {
                continue;
            };

            if first.is_none() {
                first = Some(candidate.clone());
            }
            if candidate != fallback {
                return candidate;
            }
        }
    }

    if let Some(product_client_id) = detect_product_auth_client_id(platform) {
        if product_client_id != fallback {
            return product_client_id;
        }
        if first.is_none() {
            first = Some(product_client_id);
        }
    }

    first.unwrap_or_else(|| fallback.to_string())
}

fn platform_metadata_value(platform: TraePlatformKind) -> Value {
    serde_json::json!({
        "platformId": platform.provider_key(),
        "platformName": platform.display_name(),
        "authClientId": platform.auth_client_id(),
        "authDomain": platform.auth_domain(),
    })
}

fn insert_platform_metadata(obj: &mut Map<String, Value>, platform: TraePlatformKind) {
    obj.insert(
        "platformId".to_string(),
        Value::String(platform.provider_key().to_string()),
    );
    obj.insert(
        "platformName".to_string(),
        Value::String(platform.display_name().to_string()),
    );
    obj.insert(
        "authClientId".to_string(),
        Value::String(platform.auth_client_id().to_string()),
    );
    obj.insert(
        "authDomain".to_string(),
        Value::String(platform.auth_domain().to_string()),
    );
}

fn with_platform_metadata(raw: Option<Value>, platform: TraePlatformKind) -> Value {
    let mut obj = match raw {
        Some(Value::Object(value)) => value,
        Some(value) => {
            let mut wrapped = Map::new();
            wrapped.insert("raw".to_string(), value);
            wrapped
        }
        None => Map::new(),
    };
    insert_platform_metadata(&mut obj, platform);
    Value::Object(obj)
}

fn attach_platform_metadata_to_payload(
    payload: &mut TraeImportPayload,
    platform: TraePlatformKind,
) {
    payload.trae_auth_raw = Some(with_platform_metadata(
        payload.trae_auth_raw.take(),
        platform,
    ));

    let mut server_obj = match payload.trae_server_raw.take() {
        Some(Value::Object(value)) => value,
        Some(value) => {
            let mut wrapped = Map::new();
            wrapped.insert("raw".to_string(), value);
            wrapped
        }
        None => Map::new(),
    };
    server_obj.insert("platform".to_string(), platform_metadata_value(platform));
    payload.trae_server_raw = Some(Value::Object(server_obj));
}

fn resolve_platform_from_roots(roots: &[Option<&Value>]) -> TraePlatformKind {
    if let Some(platform_id) = pick_string_multi(
        roots,
        &[
            &["platformId"],
            &["platform_id"],
            &["platform"],
            &["platform", "platformId"],
            &["platform", "platform_id"],
        ],
    ) {
        if let Ok(platform) = TraePlatformKind::parse(Some(platform_id.as_str())) {
            return platform;
        }
    }

    let client_id = pick_string_multi(
        roots,
        &[
            &["authClientId"],
            &["clientId"],
            &["ClientID"],
            &["exchangeResponse", "ClientID"],
            &["exchangeResponse", "Result", "ClientID"],
        ],
    )
    .map(|value| value.trim().to_string());
    let is_solo = client_id
        .as_deref()
        .map(|value| value == TRAE_SOLO_AUTH_CLIENT_ID)
        .unwrap_or(false);

    let domain_hint = pick_string_multi(
        roots,
        &[
            &["authDomain"],
            &["loginHost"],
            &["apiHost"],
            &["host"],
            &["callbackQuery", "host"],
            &["platform", "authDomain"],
        ],
    )
    .map(|value| value.to_ascii_lowercase())
    .unwrap_or_default();
    let provider_hint = pick_string_multi(
        roots,
        &[
            &["providerCode"],
            &["packageType"],
            &["platform", "providerCode"],
            &["platform", "packageType"],
        ],
    )
    .map(|value| value.to_ascii_lowercase())
    .unwrap_or_default();
    let is_cn = domain_hint.contains("trae.cn")
        || domain_hint.contains("trae.com.cn")
        || provider_hint == "cn"
        || provider_hint.ends_with("_cn");

    match (is_solo, is_cn) {
        (true, true) => TraePlatformKind::TraeSoloCn,
        (true, false) => TraePlatformKind::TraeSolo,
        (false, true) => TraePlatformKind::TraeCn,
        (false, false) => TraePlatformKind::Trae,
    }
}

fn resolve_account_platform_kind(account: &TraeAccount) -> TraePlatformKind {
    let profile_root = profile_payload_root(account.trae_profile_raw.as_ref());
    let roots = [
        account.trae_auth_raw.as_ref(),
        profile_root,
        account.trae_server_raw.as_ref(),
        account.trae_entitlement_raw.as_ref(),
        account.trae_usage_raw.as_ref(),
    ];
    resolve_platform_from_roots(&roots)
}

fn profile_payload_root(profile_raw: Option<&Value>) -> Option<&Value> {
    let root = profile_raw?;
    root.get("Result")
        .or_else(|| root.get("result"))
        .or_else(|| root.get("data"))
        .or(Some(root))
}

fn to_unix_millis(raw: i64) -> Option<i64> {
    if raw <= 0 {
        return None;
    }
    if raw > 10_000_000_000 {
        return Some(raw);
    }
    raw.checked_mul(1000)
}

fn normalize_iso_from_i64(raw: i64) -> Option<String> {
    let millis = to_unix_millis(raw)?;
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(millis)
        .map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

fn normalize_iso_from_text(raw: Option<&str>) -> Option<String> {
    let normalized = normalize_non_empty(raw)?;
    let trimmed = normalized.trim();
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(trimmed) {
        return Some(
            parsed
                .with_timezone(&chrono::Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        );
    }
    if let Ok(parsed) = trimmed.parse::<i64>() {
        return normalize_iso_from_i64(parsed);
    }
    None
}

fn normalize_iso_from_value(raw: Option<&Value>) -> Option<String> {
    let value = raw?;
    if let Some(text) = value.as_str() {
        return normalize_iso_from_text(Some(text));
    }
    if let Some(number) = value.as_i64() {
        return normalize_iso_from_i64(number);
    }
    if let Some(number) = value.as_u64() {
        if number <= i64::MAX as u64 {
            return normalize_iso_from_i64(number as i64);
        }
    }
    None
}

fn resolve_iso_timestamp(
    field_value: Option<i64>,
    roots: &[Option<&Value>],
    value_paths: &[&[&str]],
) -> Option<String> {
    if let Some(value) = field_value.and_then(normalize_iso_from_i64) {
        return Some(value);
    }

    for root in roots {
        for path in value_paths {
            if let Some(value) = extract_json_value(*root, path) {
                if let Some(normalized) = normalize_iso_from_value(Some(&value)) {
                    return Some(normalized);
                }
            }
        }
    }

    if let Some(value) = pick_i64_multi(roots, value_paths) {
        return normalize_iso_from_i64(value);
    }

    None
}

