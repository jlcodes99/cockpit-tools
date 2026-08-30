// Claude 账号模块：Desktop profile discovery, cookies and local profile metadata。
// 通过 include! 保持原 modules::claude_account 作用域和私有调用关系。
fn desktop_account_display_name(account_name: Option<&str>) -> String {
    if let Some(name) = normalize_non_empty(account_name) {
        return name;
    }
    format!("Claude {}", chrono::Utc::now().format("%Y-%m-%d %H:%M"))
}

fn build_desktop_account_id(label: &str) -> String {
    let random = generate_random_url_token(18);
    format!(
        "claude_desktop_{:x}",
        md5::compute(format!("{}:{}:{}", label, now_ts_ms(), random).as_bytes())
    )
}

fn desktop_cookies_path(profile_dir: &Path) -> PathBuf {
    resolve_desktop_cookies_path(profile_dir)
        .unwrap_or_else(|| default_desktop_cookies_path(profile_dir))
}

fn default_desktop_cookies_path(profile_dir: &Path) -> PathBuf {
    profile_dir.join("Network").join("Cookies")
}

fn desktop_cookie_path_candidates(profile_dir: &Path) -> Vec<PathBuf> {
    vec![
        profile_dir.join("Network").join("Cookies"),
        profile_dir.join("Cookies"),
        profile_dir.join("Default").join("Network").join("Cookies"),
        profile_dir.join("Default").join("Cookies"),
    ]
}

fn resolve_desktop_cookies_path(profile_dir: &Path) -> Option<PathBuf> {
    let candidates = desktop_cookie_path_candidates(profile_dir);
    let mut first_existing = None;
    for candidate in &candidates {
        if !candidate.exists() {
            continue;
        }
        if first_existing.is_none() {
            first_existing = Some(candidate.clone());
        }
        if matches!(cookies_db_has_required_desktop_session(candidate), Ok(true)) {
            return Some(candidate.clone());
        }
    }
    first_existing
}

fn cookies_db_has_required_desktop_session(cookies_path: &Path) -> Result<bool, String> {
    if !cookies_path.exists() {
        return Ok(false);
    }
    let conn = Connection::open_with_flags(cookies_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| {
            format!(
                "读取 Claude Cookies 失败: path={}, error={}",
                cookies_path.display(),
                e
            )
        })?;
    let count: i64 = conn
        .query_row(
            "select count(distinct name) from cookies \
             where name in ('sessionKey', 'lastActiveOrg') \
             and (host_key like '%claude.ai' or host_key like '%claude.com') \
             and (length(value) > 0 or length(encrypted_value) > 0)",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("查询 Claude Cookies 失败: {}", e))?;
    Ok(count >= 2)
}

fn ensure_desktop_profile_logged_in(profile_dir: &Path) -> Result<(), String> {
    if !profile_dir.exists() {
        return Err(format!("Claude profile 不存在: {}", profile_dir.display()));
    }
    let mut last_error = None;
    for cookies_path in desktop_cookie_path_candidates(profile_dir) {
        if !cookies_path.exists() {
            continue;
        }
        match cookies_db_has_required_desktop_session(&cookies_path) {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(error) => last_error = Some(error),
        }
    }
    if let Some(error) = last_error {
        return Err(error);
    }
    Err("未检测到 Claude 登录态，请在授权窗口或官方 Claude 完成登录后再导入。".to_string())
}

fn chromium_cookie_expires_utc_to_unix_ms(expires_utc: i64) -> Option<i64> {
    if expires_utc <= 0 {
        return None;
    }
    let unix_ms = expires_utc / 1000 - CHROMIUM_EPOCH_OFFSET_MS;
    (unix_ms > 0).then_some(unix_ms)
}

fn desktop_session_expiration_to_ms(expiration_date: Option<f64>) -> Option<i64> {
    let seconds = expiration_date?;
    if !seconds.is_finite() || seconds <= 0.0 {
        return None;
    }
    Some((seconds * 1000.0).round() as i64)
}

fn desktop_cookie_names(cookies: &[ClaudeDesktopAuthCookie]) -> Vec<String> {
    let mut names = BTreeSet::new();
    for cookie in cookies {
        if is_claude_cookie_domain(&cookie.domain) {
            names.insert(cookie.name.clone());
        }
    }
    names.into_iter().collect()
}

fn desktop_profile_metadata_from_export(
    export: &ClaudeDesktopAuthCookieExport,
    source: &str,
) -> ClaudeDesktopProfileMetadata {
    let session_key = export.cookies.iter().find(|cookie| {
        cookie.name == "sessionKey"
            && !cookie.value.is_empty()
            && is_claude_cookie_domain(&cookie.domain)
    });
    let last_active_org = export.cookies.iter().find(|cookie| {
        cookie.name == "lastActiveOrg"
            && !cookie.value.is_empty()
            && is_claude_cookie_domain(&cookie.domain)
    });
    ClaudeDesktopProfileMetadata {
        source: source.to_string(),
        has_session_key: session_key.is_some(),
        has_last_active_org: last_active_org.is_some(),
        last_active_org: last_active_org
            .and_then(|cookie| normalize_non_empty(Some(&cookie.value))),
        session_expires_at: session_key
            .and_then(|cookie| desktop_session_expiration_to_ms(cookie.expiration_date)),
        cookie_names: desktop_cookie_names(&export.cookies),
        web_profile: export.web_profile.clone(),
    }
}

fn desktop_profile_metadata_from_cookies_db(
    profile_dir: &Path,
    source: &str,
) -> Result<ClaudeDesktopProfileMetadata, String> {
    let cookies_path = desktop_cookies_path(profile_dir);
    let conn = Connection::open_with_flags(&cookies_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| {
            format!(
                "读取 Claude Cookies 失败: path={}, error={}",
                cookies_path.display(),
                e
            )
        })?;
    let mut stmt = conn
        .prepare(
            "select name, value, coalesce(length(encrypted_value), 0), expires_utc from cookies \
             where (host_key like '%claude.ai' or host_key like '%claude.com')",
        )
        .map_err(|e| format!("查询 Claude Cookies 失败: {}", e))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(|e| format!("读取 Claude Cookies 失败: {}", e))?;

    let mut cookie_names = BTreeSet::new();
    let mut has_session_key = false;
    let mut has_last_active_org = false;
    let mut last_active_org = None;
    let mut session_expires_at = None;
    for row in rows {
        let (name, value, encrypted_len, expires_utc) =
            row.map_err(|e| format!("读取 Claude Cookie 行失败: {}", e))?;
        let has_value = !value.is_empty() || encrypted_len > 0;
        if !has_value {
            continue;
        }
        cookie_names.insert(name.clone());
        if name == "sessionKey" {
            has_session_key = true;
            session_expires_at = chromium_cookie_expires_utc_to_unix_ms(expires_utc);
        } else if name == "lastActiveOrg" {
            has_last_active_org = true;
            last_active_org = normalize_non_empty(Some(&value));
        }
    }

    Ok(ClaudeDesktopProfileMetadata {
        source: source.to_string(),
        has_session_key,
        has_last_active_org,
        last_active_org,
        session_expires_at,
        cookie_names: cookie_names.into_iter().collect(),
        web_profile: None,
    })
}

fn desktop_profile_metadata(
    profile_dir: &Path,
    source: &str,
) -> Result<ClaudeDesktopProfileMetadata, String> {
    match read_desktop_auth_cookie_export(profile_dir)
        .and_then(|export| ensure_desktop_auth_export_logged_in(&export).map(|_| export))
    {
        Ok(export) => Ok(desktop_profile_metadata_from_export(&export, source)),
        Err(_) => desktop_profile_metadata_from_cookies_db(profile_dir, source),
    }
}

fn printable_ascii(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| {
            if (32..=126).contains(byte) {
                *byte as char
            } else {
                ' '
            }
        })
        .collect()
}

fn normalize_profile_text_value(raw: &str) -> Option<String> {
    let mut result = String::new();
    let mut last_was_space = false;
    for ch in raw.chars() {
        if ch == '"' || ch == '\\' || ch == '{' || ch == '}' || ch == '[' || ch == ']' {
            break;
        }
        if ch.is_ascii_control() {
            break;
        }
        let keep = ch.is_ascii_alphanumeric()
            || matches!(
                ch,
                ' ' | '_' | '-' | '.' | '\'' | '@' | '&' | '(' | ')' | '+'
            );
        if !keep {
            if result.is_empty() {
                continue;
            }
            break;
        }
        if ch.is_ascii_whitespace() {
            if !result.is_empty() && !last_was_space {
                result.push(' ');
            }
            last_was_space = true;
        } else {
            result.push(ch);
            last_was_space = false;
        }
        if result.len() >= 120 {
            break;
        }
    }
    normalize_non_empty(Some(result.trim()))
}

fn extract_text_after_key(text: &str, key: &str) -> Option<String> {
    let pos = text.find(key)?;
    let after = &text[pos + key.len()..text.len().min(pos + key.len() + 220)];
    let start = after
        .char_indices()
        .find(|(_, ch)| ch.is_ascii_alphanumeric())?
        .0;
    normalize_profile_text_value(&after[start..])
}

fn is_ignored_profile_email(email: &str) -> bool {
    let email = email.trim().to_ascii_lowercase();
    let Some((local, domain)) = email.split_once('@') else {
        return true;
    };
    if local.len() < 2 || !domain.contains('.') {
        return true;
    }
    if email.contains("example")
        || email.contains("placeholder")
        || email.contains("noreply")
        || email.contains("no-reply")
        || domain.contains("sentry")
        || domain == "w3.org"
        || domain == "schema.org"
        || domain == "chromium.org"
    {
        return true;
    }
    false
}

fn extract_desktop_local_profile_from_bytes(
    source: &Path,
    bytes: &[u8],
) -> Option<ClaudeDesktopLocalProfile> {
    let text = printable_ascii(bytes);
    let mut best: Option<ClaudeDesktopLocalProfile> = None;
    for email_match in EMAIL_RE.find_iter(&text) {
        let email = email_match.as_str().to_ascii_lowercase();
        if is_ignored_profile_email(&email) {
            continue;
        }
        let start = email_match.start().saturating_sub(900);
        let end = (email_match.end() + 2200).min(text.len());
        let window = &text[start..end];
        let email_local_index = email_match.start().saturating_sub(start);
        let before_email = &window[..email_local_index.min(window.len())];
        let after_email = &window[email_local_index.min(window.len())..];
        let profile_context = window.contains("email_address")
            || window.contains("display_name")
            || window.contains("full_name")
            || window.contains("memberships")
            || window.contains("organization");
        if !profile_context {
            continue;
        }

        let account_uuid = UUID_RE
            .find_iter(before_email)
            .last()
            .map(|item| item.as_str().to_string());
        let organization_window = after_email
            .find("organization")
            .map(|pos| &after_email[pos..after_email.len().min(pos + 1200)]);
        let organization_uuid = organization_window
            .and_then(|value| UUID_RE.find(value))
            .map(|item| item.as_str().to_string());
        let organization_name =
            organization_window.and_then(|value| extract_text_after_key(value, "name"));
        let full_name = extract_text_after_key(after_email, "full_name");
        let display_name = extract_text_after_key(after_email, "display_name");
        let candidate = ClaudeDesktopLocalProfile {
            email: Some(email),
            account_uuid,
            full_name,
            display_name,
            organization_uuid,
            organization_name,
            source: Some(source.to_string_lossy().to_string()),
        };
        if best
            .as_ref()
            .map(|current| candidate.score() > current.score())
            .unwrap_or(true)
        {
            best = Some(candidate);
        }
    }
    best
}

fn collect_desktop_local_profile_files(root: &Path, files: &mut Vec<PathBuf>) {
    if files.len() >= CLAUDE_DESKTOP_LOCAL_PROFILE_MAX_FILES {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if files.len() >= CLAUDE_DESKTOP_LOCAL_PROFILE_MAX_FILES {
            return;
        }
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.is_dir() {
            collect_desktop_local_profile_files(&path, files);
        } else if metadata.is_file()
            && metadata.len() > 0
            && metadata.len() <= CLAUDE_DESKTOP_LOCAL_PROFILE_MAX_FILE_BYTES
        {
            files.push(path);
        }
    }
}

fn read_desktop_local_profile(profile_dir: &Path) -> Option<ClaudeDesktopLocalProfile> {
    let mut files = Vec::new();
    for dir in CLAUDE_DESKTOP_LOCAL_PROFILE_SCAN_DIRS {
        let root = profile_dir.join(dir);
        if root.exists() {
            collect_desktop_local_profile_files(&root, &mut files);
        }
    }

    let mut best: Option<ClaudeDesktopLocalProfile> = None;
    for file in files {
        let Ok(bytes) = fs::read(&file) else {
            continue;
        };
        let Some(mut candidate) = extract_desktop_local_profile_from_bytes(&file, &bytes) else {
            continue;
        };
        candidate.source = file
            .strip_prefix(profile_dir)
            .ok()
            .map(|value| value.to_string_lossy().to_string())
            .or(candidate.source);
        if !candidate.has_identity() {
            continue;
        }
        if best
            .as_ref()
            .map(|current| candidate.score() > current.score())
            .unwrap_or(true)
        {
            best = Some(candidate);
        }
    }
    best
}

fn upsert_desktop_profile_json(account: &mut ClaudeAccount, key: &str, value: Value) {
    if account.claude_config_raw.is_none() {
        account.claude_config_raw = Some(json!({}));
    }
    let Some(config) = account.claude_config_raw.as_mut() else {
        return;
    };
    if !config.is_object() {
        *config = json!({});
    }
    if let Some(object) = config.as_object_mut() {
        let desktop_profile = object
            .entry("desktopProfile".to_string())
            .or_insert_with(|| json!({}));
        if !desktop_profile.is_object() {
            *desktop_profile = json!({});
        }
        if let Some(desktop_object) = desktop_profile.as_object_mut() {
            desktop_object.insert(key.to_string(), value);
        }
    }
}

fn apply_desktop_local_profile(account: &mut ClaudeAccount, profile_dir: &Path) -> bool {
    let Some(local_profile) = read_desktop_local_profile(profile_dir) else {
        return false;
    };
    let mut changed = false;
    if let Some(email) = local_profile.email.as_ref() {
        if account.email.trim() != email {
            account.email = email.clone();
            changed = true;
        }
    }
    if account.account_uuid.is_none() && local_profile.account_uuid.is_some() {
        account.account_uuid = local_profile.account_uuid.clone();
        changed = true;
    }
    if account.organization_uuid.is_none() && local_profile.organization_uuid.is_some() {
        account.organization_uuid = local_profile.organization_uuid.clone();
        changed = true;
    }
    if let Some(organization_name) = local_profile.organization_name.as_ref() {
        let should_update = account
            .organization_name
            .as_deref()
            .map(|value| value.trim().is_empty() || value.eq_ignore_ascii_case("Claude"))
            .unwrap_or(true);
        if should_update {
            account.organization_name = Some(organization_name.clone());
            changed = true;
        }
    }
    if account
        .plan_type
        .as_deref()
        .map(is_desktop_plan_placeholder)
        .unwrap_or(false)
    {
        account.plan_type = None;
        changed = true;
    }
    if changed {
        account.profile_updated_at = Some(now_ts_ms());
    }
    let summary = json!({
        "email": local_profile.email,
        "accountUuid": local_profile.account_uuid,
        "fullName": local_profile.full_name,
        "displayName": local_profile.display_name,
        "organizationUuid": local_profile.organization_uuid,
        "organizationName": local_profile.organization_name,
        "source": local_profile.source,
        "fetchedAt": chrono::Utc::now().to_rfc3339(),
    });
    upsert_desktop_profile_json(account, "localProfileSummary", summary);
    changed
}

fn desktop_profile_metadata_json(
    metadata: &ClaudeDesktopProfileMetadata,
    snapshot_dir: &Path,
    imported_at: i64,
) -> Value {
    json!({
        "snapshotDir": snapshot_dir.to_string_lossy().to_string(),
        "importedAt": imported_at,
        "source": metadata.source.clone(),
        "profileSnapshot": true,
        "hasSessionKey": metadata.has_session_key,
        "hasLastActiveOrg": metadata.has_last_active_org,
        "lastActiveOrg": metadata.last_active_org.clone(),
        "sessionExpiresAt": metadata.session_expires_at,
        "cookieNames": metadata.cookie_names.clone(),
        "webProfileFetchedAt": metadata.web_profile.as_ref().and_then(|profile| read_string_path(profile, &["fetchedAt"])),
        "webProfileErrors": metadata.web_profile.as_ref().and_then(|profile| profile.get("errors")).cloned(),
    })
}

fn desktop_auth_export_path(profile_dir: &Path) -> PathBuf {
    profile_dir.join(CLAUDE_DESKTOP_AUTH_EXPORT_FILE)
}

fn read_desktop_auth_cookie_export(
    profile_dir: &Path,
) -> Result<ClaudeDesktopAuthCookieExport, String> {
    let path = desktop_auth_export_path(profile_dir);
    let content = fs::read_to_string(&path).map_err(|e| {
        format!(
            "读取 Claude 授权 cookie 导出失败: path={}, error={}",
            path.display(),
            e
        )
    })?;
    serde_json::from_str(&content).map_err(|e| {
        format!(
            "解析 Claude 授权 cookie 导出失败: path={}, error={}",
            path.display(),
            e
        )
    })
}

#[cfg(target_os = "macos")]
fn read_claude_safe_storage_password() -> Result<String, String> {
    for account in ["Claude", "Claude Key"] {
        let output = std::process::Command::new("security")
            .args([
                "find-generic-password",
                "-a",
                account,
                "-s",
                "Claude Safe Storage",
                "-w",
            ])
            .output()
            .map_err(|e| format!("读取 Claude Safe Storage Keychain 失败: {}", e))?;
        if output.status.success() {
            let password = String::from_utf8_lossy(&output.stdout)
                .trim_end_matches(['\r', '\n'])
                .to_string();
            if !password.is_empty() {
                return Ok(password);
            }
        }
    }
    Err("未找到 Claude Safe Storage Keychain 密钥，无法解密 Claude Cookies。".to_string())
}

#[cfg(target_os = "macos")]
fn decrypt_chromium_v10_cookie(
    host_key: &str,
    encrypted_value: &[u8],
    password: &str,
) -> Result<String, String> {
    const V10_PREFIX: &[u8] = b"v10";
    if !encrypted_value.starts_with(V10_PREFIX) {
        return Err("Claude Cookie 使用了暂不支持的加密格式。".to_string());
    }
    let mut key = [0u8; 16];
    pbkdf2_hmac::<Sha1>(password.as_bytes(), b"saltysalt", 1003, &mut key);
    let iv = [0x20u8; 16];
    let mut buffer = encrypted_value[V10_PREFIX.len()..].to_vec();
    let cipher = Aes128CbcDec::new_from_slices(&key, &iv)
        .map_err(|e| format!("初始化 Claude Cookie 解密器失败: {}", e))?;
    let mut plaintext = cipher
        .decrypt_padded_mut::<Pkcs7>(&mut buffer)
        .map_err(|e| format!("解密 Claude Cookie 失败: {}", e))?
        .to_vec();

    // Chromium DB schema >= 24 prefixes encrypted cookie plaintext with SHA256(host_key).
    let host_digest = Sha256::digest(host_key.as_bytes());
    if plaintext.len() > 32 && plaintext[..32] == host_digest[..] {
        plaintext = plaintext[32..].to_vec();
    }

    if plaintext.iter().any(|byte| !(0x20..=0x7e).contains(byte)) {
        return Err("解密后的 Claude Cookie 含有非法字符。".to_string());
    }
    String::from_utf8(plaintext).map_err(|e| format!("解析 Claude Cookie 失败: {}", e))
}

#[cfg(target_os = "macos")]
fn read_decrypted_desktop_cookie_export(
    profile_dir: &Path,
) -> Result<ClaudeDesktopAuthCookieExport, String> {
    let cookies_path = desktop_cookies_path(profile_dir);
    if !cookies_path.exists() {
        return Err(format!("Claude Cookies 不存在: {}", cookies_path.display()));
    }
    let password = read_claude_safe_storage_password()?;
    let conn = Connection::open_with_flags(&cookies_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| {
            format!(
                "读取 Claude Cookies 失败: path={}, error={}",
                cookies_path.display(),
                e
            )
        })?;
    let mut stmt = conn
        .prepare(
            "select host_key, path, name, value, encrypted_value, expires_utc, is_secure, is_httponly \
             from cookies \
             where (host_key like '%claude.ai' or host_key like '%claude.com') \
             and (length(value) > 0 or length(encrypted_value) > 0)",
        )
        .map_err(|e| format!("查询 Claude Cookies 失败: {}", e))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Vec<u8>>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
            ))
        })
        .map_err(|e| format!("读取 Claude Cookies 失败: {}", e))?;

    let mut cookies = Vec::new();
    for row in rows {
        let (domain, path, name, value, encrypted_value, expires_utc, is_secure, is_httponly) =
            row.map_err(|e| format!("读取 Claude Cookie 行失败: {}", e))?;
        if !is_claude_cookie_domain(&domain) {
            continue;
        }
        let cookie_value = if !value.is_empty() {
            value
        } else if !encrypted_value.is_empty() {
            decrypt_chromium_v10_cookie(&domain, &encrypted_value, &password)?
        } else {
            String::new()
        };
        if cookie_value.is_empty() {
            continue;
        }
        cookies.push(ClaudeDesktopAuthCookie {
            name,
            value: cookie_value,
            domain,
            path,
            secure: is_secure != 0,
            http_only: is_httponly != 0,
            expiration_date: chromium_cookie_expires_utc_to_unix_ms(expires_utc)
                .map(|ms| ms as f64 / 1000.0),
            same_site: None,
        });
    }
    let export = ClaudeDesktopAuthCookieExport {
        cookies,
        web_profile: None,
    };
    ensure_desktop_auth_export_logged_in(&export)?;
    Ok(export)
}

fn is_claude_cookie_domain(domain: &str) -> bool {
    let domain = domain.trim().trim_start_matches('.').to_ascii_lowercase();
    domain == "claude.ai"
        || domain.ends_with(".claude.ai")
        || domain == "claude.com"
        || domain.ends_with(".claude.com")
}

fn exported_cookie_host_key(cookie: &ClaudeDesktopAuthCookie) -> String {
    let domain = cookie.domain.trim();
    if domain.is_empty() {
        return "claude.ai".to_string();
    }
    domain.to_string()
}

fn exported_cookie_path(cookie: &ClaudeDesktopAuthCookie) -> &str {
    let path = cookie.path.trim();
    if path.is_empty() {
        "/"
    } else {
        path
    }
}

fn chromium_cookie_time_now() -> i64 {
    (now_ts_ms() + CHROMIUM_EPOCH_OFFSET_MS) * 1000
}

fn exported_cookie_expires_utc(cookie: &ClaudeDesktopAuthCookie) -> i64 {
    let Some(seconds) = cookie.expiration_date else {
        return 0;
    };
    if !seconds.is_finite() || seconds <= 0.0 {
        return 0;
    }
    ((seconds * 1000.0).round() as i64 + CHROMIUM_EPOCH_OFFSET_MS) * 1000
}

fn exported_cookie_samesite(cookie: &ClaudeDesktopAuthCookie) -> i64 {
    match cookie.same_site.as_deref().map(str::to_ascii_lowercase) {
        Some(value) if value == "strict" => 2,
        Some(value) if value == "lax" => 1,
        Some(value) if value == "no_restriction" || value == "none" => 0,
        _ => -1,
    }
}

fn exported_cookie_source_type(cookie: &ClaudeDesktopAuthCookie) -> i64 {
    if cookie.http_only {
        1
    } else {
        2
    }
}

fn ensure_desktop_auth_export_logged_in(
    export: &ClaudeDesktopAuthCookieExport,
) -> Result<(), String> {
    let has_session_key = export.cookies.iter().any(|cookie| {
        cookie.name == "sessionKey"
            && !cookie.value.is_empty()
            && is_claude_cookie_domain(&cookie.domain)
    });
    let has_last_active_org = export.cookies.iter().any(|cookie| {
        cookie.name == "lastActiveOrg"
            && !cookie.value.is_empty()
            && is_claude_cookie_domain(&cookie.domain)
    });
    if !has_session_key || !has_last_active_org {
        return Err("未检测到 Claude 登录态，请在授权窗口完成登录后再导入。".to_string());
    }
    Ok(())
}

fn wait_for_desktop_auth_export_logged_in(
    profile_dir: &Path,
) -> Result<ClaudeDesktopAuthCookieExport, String> {
    let started_at = Instant::now();
    let timeout = Duration::from_secs(CLAUDE_DESKTOP_AUTH_EXPORT_WAIT_SECONDS);
    let mut last_error = "未检测到 Claude 登录态，请在授权窗口完成登录后再导入。".to_string();

    while started_at.elapsed() <= timeout {
        match read_desktop_auth_cookie_export(profile_dir)
            .and_then(|export| ensure_desktop_auth_export_logged_in(&export).map(|_| export))
        {
            Ok(export) => return Ok(export),
            Err(error) => {
                last_error = error;
                std::thread::sleep(Duration::from_millis(400));
            }
        }
    }

    Err(last_error)
}

fn wait_for_desktop_web_profile_export(
    profile_dir: &Path,
    timeout: Duration,
) -> Result<ClaudeDesktopAuthCookieExport, String> {
    let started_at = Instant::now();
    let mut last_error = "未读取到 Claude 账号资料".to_string();

    while started_at.elapsed() <= timeout {
        match read_desktop_auth_cookie_export(profile_dir)
            .and_then(|export| ensure_desktop_auth_export_logged_in(&export).map(|_| export))
        {
            Ok(export) if export.web_profile.is_some() => return Ok(export),
            Ok(_) => {
                last_error = "Claude 登录态已读取，但资料接口未返回数据".to_string();
            }
            Err(error) => {
                last_error = error;
            }
        }
        std::thread::sleep(Duration::from_millis(500));
    }

    Err(last_error)
}

fn desktop_web_profile_has_usage_error(profile: &Value) -> bool {
    profile
        .get("errors")
        .and_then(|value| value.as_object())
        .and_then(|errors| errors.get("organizationUsage"))
        .is_some()
}

fn desktop_error_is_cloudflare_challenge(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("cloudflare")
        || normalized.contains("just a moment")
        || normalized.contains("cf-ray")
        || normalized.contains("challenge-platform")
        || normalized.contains("verify you are human")
        || normalized.contains("checking your browser")
}

fn desktop_web_profile_error_strings(profile: &Value) -> Vec<String> {
    profile
        .get("errors")
        .and_then(|value| value.as_object())
        .map(|errors| {
            errors
                .values()
                .filter_map(|value| normalize_non_empty(value.as_str()))
                .collect()
        })
        .unwrap_or_default()
}

fn desktop_web_profile_has_cloudflare_challenge(profile: &Value) -> bool {
    desktop_web_profile_error_strings(profile)
        .iter()
        .any(|error| desktop_error_is_cloudflare_challenge(error))
}

fn desktop_web_profile_needs_hidden_probe(profile: &Value) -> bool {
    !desktop_web_profile_error_strings(profile).is_empty()
}

fn should_attempt_desktop_hidden_probe_at(
    attempts: &mut HashMap<String, Instant>,
    account_id: &str,
    now: Instant,
    cooldown: Duration,
) -> bool {
    if let Some(previous) = attempts.get(account_id) {
        if now.duration_since(*previous) < cooldown {
            return false;
        }
    }
    attempts.insert(account_id.to_string(), now);
    true
}

fn should_attempt_desktop_hidden_probe(account_id: &str) -> bool {
    let Ok(mut attempts) = CLAUDE_DESKTOP_HIDDEN_PROBE_ATTEMPTS.lock() else {
        return true;
    };
    should_attempt_desktop_hidden_probe_at(
        &mut attempts,
        account_id,
        Instant::now(),
        Duration::from_secs(CLAUDE_DESKTOP_HIDDEN_PROBE_COOLDOWN_SECONDS),
    )
}

async fn probe_desktop_web_profile_hidden_with_cooldown(
    account_id: &str,
    profile_dir: &Path,
) -> Result<Value, String> {
    if !should_attempt_desktop_hidden_probe(account_id) {
        return Err(format!(
            "隐藏 Electron Cookie 刷新过于频繁，{} 秒内不会重复尝试",
            CLAUDE_DESKTOP_HIDDEN_PROBE_COOLDOWN_SECONDS
        ));
    }
    let profile_dir = profile_dir.to_path_buf();
    tauri::async_runtime::spawn_blocking(move || probe_desktop_web_profile(&profile_dir))
        .await
        .map_err(|error| format!("隐藏 Electron Cookie 刷新任务失败: {}", error))?
}

async fn resolve_desktop_web_profile_with_hidden_probe<F, Fut>(
    account_id: &str,
    silent_result: Result<Value, String>,
    hidden_probe: F,
) -> Result<Value, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Value, String>>,
{
    match silent_result {
        Ok(web_profile) if desktop_web_profile_needs_hidden_probe(&web_profile) => {
            let reason = if desktop_web_profile_has_cloudflare_challenge(&web_profile) {
                " Cloudflare challenge"
            } else {
                "非 CF 错误"
            };
            match hidden_probe().await {
                Ok(probed_profile) => {
                    logger::log_info(&format!(
                        "[Claude] 静默刷新存在{}，已通过隐藏 Electron probe 更新资料: account_id={}",
                        reason, account_id
                    ));
                    Ok(probed_profile)
                }
                Err(error) => {
                    logger::log_warn(&format!(
                        "[Claude] 隐藏 Electron probe 失败，保留静默刷新结果: account_id={}, error={}",
                        account_id, error
                    ));
                    Ok(web_profile)
                }
            }
        }
        Ok(web_profile) => Ok(web_profile),
        Err(error) => match hidden_probe().await {
            Ok(probed_profile) => {
                logger::log_info(&format!(
                    "[Claude] 静默刷新失败，已通过隐藏 Electron probe 更新资料: account_id={}",
                    account_id
                ));
                Ok(probed_profile)
            }
            Err(fallback_error) => Err(format!(
                "{}；隐藏 Electron Cookie 刷新也失败: {}",
                error, fallback_error
            )),
        },
    }
}

fn write_desktop_cookie_probe_file(
    path: &Path,
    export: &ClaudeDesktopAuthCookieExport,
) -> Result<(), String> {
    let content = serde_json::to_string_pretty(export)
        .map_err(|e| format!("序列化 Claude Cookie 探测文件失败: {}", e))?;
    atomic_write::write_string_atomic(path, &content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn probe_desktop_web_profile_with_decrypted_cookies(profile_dir: &Path) -> Result<Value, String> {
    let cookie_export = read_decrypted_desktop_cookie_export(profile_dir)?;
    let probe_root = get_desktop_login_root_dir()?
        .join(format!("cookie_probe_{}", generate_random_url_token(18)));
    let user_data_dir = probe_root.join("profile");
    let status_file = user_data_dir.join(CLAUDE_DESKTOP_AUTH_STATUS_FILE);
    let export_file = user_data_dir.join(CLAUDE_DESKTOP_AUTH_EXPORT_FILE);
    let cookie_file = probe_root.join(CLAUDE_DESKTOP_COOKIE_EXPORT_FILE);
    fs::create_dir_all(&user_data_dir)
        .map_err(|e| format!("创建 Claude Cookie 探测目录失败: {}", e))?;
    let result = (|| {
        write_desktop_cookie_probe_file(&cookie_file, &cookie_export)?;
        let helper_pid = launch_platform_desktop_auth_helper_with_args(
            &user_data_dir,
            &status_file,
            &export_file,
            "cookie_probe",
            &[("--cookie-file", cookie_file.as_path())],
        )?;
        let result = wait_for_desktop_web_profile_export(&user_data_dir, Duration::from_secs(24))
            .and_then(|export| {
                export
                    .web_profile
                    .ok_or_else(|| "Claude 资料接口未返回数据".to_string())
            });
        terminate_desktop_auth_helper(Some(helper_pid));
        result
    })();
    let _ = remove_path_if_exists(&probe_root);
    result
}

#[cfg(not(target_os = "macos"))]
fn probe_desktop_web_profile_with_decrypted_cookies(_profile_dir: &Path) -> Result<Value, String> {
    Err("当前平台不支持解密 Claude Cookies。".to_string())
}

fn probe_desktop_web_profile(profile_dir: &Path) -> Result<Value, String> {
    ensure_desktop_profile_logged_in(profile_dir)?;
    let status_file = profile_dir.join("claude_desktop_profile_probe_status.json");
    let export_file = desktop_auth_export_path(profile_dir);
    let _ = remove_path_if_exists(&status_file);
    let _ = remove_path_if_exists(&export_file);
    let helper_pid =
        launch_platform_desktop_auth_helper(profile_dir, &status_file, &export_file, "probe")?;
    let result = wait_for_desktop_web_profile_export(profile_dir, Duration::from_secs(18))
        .and_then(|export| {
            export
                .web_profile
                .ok_or_else(|| "Claude 资料接口未返回数据".to_string())
        });
    terminate_desktop_auth_helper(Some(helper_pid));
    match result {
        Ok(profile)
            if desktop_web_usage_to_quota(&profile).is_some()
                || !desktop_web_profile_has_usage_error(&profile) =>
        {
            Ok(profile)
        }
        Ok(profile) => match probe_desktop_web_profile_with_decrypted_cookies(profile_dir) {
            Ok(fallback) => Ok(fallback),
            Err(error) => {
                logger::log_warn(&format!(
                    "[Claude] Cookie 页面上下文刷新失败，保留原始资料结果: {}",
                    error
                ));
                Ok(profile)
            }
        },
        Err(error) => match probe_desktop_web_profile_with_decrypted_cookies(profile_dir) {
            Ok(fallback) => Ok(fallback),
            Err(fallback_error) => Err(format!(
                "{}；Cookie 页面上下文刷新也失败: {}",
                error, fallback_error
            )),
        },
    }
}

fn read_desktop_cookie_export_for_silent_refresh(
    profile_dir: &Path,
) -> Result<ClaudeDesktopAuthCookieExport, String> {
    let mut errors = Vec::new();

    #[cfg(target_os = "macos")]
    match read_decrypted_desktop_cookie_export(profile_dir) {
        Ok(export) => return Ok(export),
        Err(error) => errors.push(format!("解密本地 Cookies 失败: {}", error)),
    }

    match read_desktop_auth_cookie_export(profile_dir)
        .and_then(|export| ensure_desktop_auth_export_logged_in(&export).map(|_| export))
    {
        Ok(export) => Ok(export),
        Err(error) => {
            errors.push(format!("读取已导出 Cookies 失败: {}", error));
            Err(format!(
                "无法静默读取 Claude Cookies: {}",
                errors.join("；")
            ))
        }
    }
}

fn desktop_cookie_value(cookies: &[ClaudeDesktopAuthCookie], name: &str) -> Option<String> {
    cookies
        .iter()
        .find(|cookie| {
            cookie.name == name
                && !cookie.value.is_empty()
                && is_claude_cookie_domain(&cookie.domain)
        })
        .map(|cookie| cookie.value.clone())
}

fn desktop_cookie_header(cookies: &[ClaudeDesktopAuthCookie]) -> Result<String, String> {
    let value = cookies
        .iter()
        .filter(|cookie| {
            !cookie.name.is_empty()
                && !cookie.value.is_empty()
                && is_claude_cookie_domain(&cookie.domain)
        })
        .map(|cookie| format!("{}={}", cookie.name, cookie.value))
        .collect::<Vec<_>>()
        .join("; ");
    if value.is_empty() {
        Err("Claude Cookies 为空".to_string())
    } else {
        Ok(value)
    }
}

async fn fetch_claude_web_json_with_cookies(
    client: &reqwest::Client,
    url: &str,
    cookies: &[ClaudeDesktopAuthCookie],
    extra_headers: HeaderMap,
) -> Result<Value, String> {
    let cookie_header = desktop_cookie_header(cookies)?;
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(
        USER_AGENT,
        HeaderValue::from_static(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126 Safari/537.36",
        ),
    );
    headers.insert("origin", HeaderValue::from_static("https://claude.ai"));
    headers.insert("referer", HeaderValue::from_static("https://claude.ai/"));
    headers.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
    headers.insert("sec-fetch-mode", HeaderValue::from_static("cors"));
    headers.insert("sec-fetch-dest", HeaderValue::from_static("empty"));
    headers.insert(
        "cookie",
        HeaderValue::from_str(&cookie_header)
            .map_err(|e| format!("构造 Claude Cookie 请求头失败: {}", e))?,
    );
    for (name, value) in extra_headers.iter() {
        headers.insert(name, value.clone());
    }

    let response = client
        .get(url)
        .headers(headers)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("读取响应失败: {}", e))?;
    if !status.is_success() {
        let preview: String = body.chars().take(500).collect();
        return Err(format!("HTTP {} {}", status, preview));
    }
    if body.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&body).map_err(|e| format!("解析 JSON 失败: {}", e))
}

async fn fetch_desktop_web_profile_endpoint(
    client: &reqwest::Client,
    cookies: &[ClaudeDesktopAuthCookie],
    endpoints: &mut serde_json::Map<String, Value>,
    errors: &mut serde_json::Map<String, Value>,
    key: &str,
    url: &str,
    extra_headers: HeaderMap,
) {
    match fetch_claude_web_json_with_cookies(client, url, cookies, extra_headers).await {
        Ok(value) => {
            endpoints.insert(key.to_string(), value);
        }
        Err(error) => {
            errors.insert(key.to_string(), Value::String(error));
        }
    }
}

async fn fetch_desktop_web_profile_with_cookies(
    cookies: &[ClaudeDesktopAuthCookie],
) -> Result<Value, String> {
    ensure_desktop_auth_export_logged_in(&ClaudeDesktopAuthCookieExport {
        cookies: cookies.to_vec(),
        web_profile: None,
    })?;
    let last_active_org = desktop_cookie_value(cookies, "lastActiveOrg").unwrap_or_default();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| format!("创建 Claude Web HTTP 客户端失败: {}", e))?;

    let mut endpoints = serde_json::Map::new();
    let mut errors = serde_json::Map::new();
    let mut org_headers = HeaderMap::new();
    if !last_active_org.is_empty() {
        org_headers.insert(
            "x-organization-uuid",
            HeaderValue::from_str(&last_active_org)
                .map_err(|e| format!("构造组织请求头失败: {}", e))?,
        );
    }

    fetch_desktop_web_profile_endpoint(
        &client,
        cookies,
        &mut endpoints,
        &mut errors,
        "accountProfile",
        "https://claude.ai/api/account_profile",
        org_headers.clone(),
    )
    .await;
    fetch_desktop_web_profile_endpoint(
        &client,
        cookies,
        &mut endpoints,
        &mut errors,
        "account",
        "https://claude.ai/api/account",
        org_headers.clone(),
    )
    .await;

    if last_active_org.is_empty() {
        errors.insert(
            "bootstrapAppStart".to_string(),
            Value::String("missing lastActiveOrg".to_string()),
        );
        errors.insert(
            "organizationUsage".to_string(),
            Value::String("missing lastActiveOrg".to_string()),
        );
        errors.insert(
            "subscriptionDetails".to_string(),
            Value::String("missing lastActiveOrg".to_string()),
        );
        errors.insert(
            "overageSpendLimit".to_string(),
            Value::String("missing lastActiveOrg".to_string()),
        );
    } else {
        let encoded_org: String =
            form_urlencoded::byte_serialize(last_active_org.as_bytes()).collect();
        let bootstrap_url = format!(
            "https://claude.ai/api/bootstrap/{}/app_start?statsig_hashing_algorithm=djb2&growthbook_format=sdk&include_system_prompts=false",
            encoded_org
        );
        fetch_desktop_web_profile_endpoint(
            &client,
            cookies,
            &mut endpoints,
            &mut errors,
            "bootstrapAppStart",
            &bootstrap_url,
            org_headers.clone(),
        )
        .await;

        let org_base = format!("https://claude.ai/api/organizations/{}", encoded_org);
        let mut usage_headers = org_headers.clone();
        usage_headers.insert(
            "referer",
            HeaderValue::from_static("https://claude.ai/settings/usage"),
        );
        fetch_desktop_web_profile_endpoint(
            &client,
            cookies,
            &mut endpoints,
            &mut errors,
            "organizationUsage",
            &format!("{}/usage", org_base),
            usage_headers.clone(),
        )
        .await;
        fetch_desktop_web_profile_endpoint(
            &client,
            cookies,
            &mut endpoints,
            &mut errors,
            "subscriptionDetails",
            &format!("{}/subscription_details", org_base),
            usage_headers.clone(),
        )
        .await;
        fetch_desktop_web_profile_endpoint(
            &client,
            cookies,
            &mut endpoints,
            &mut errors,
            "overageSpendLimit",
            &format!("{}/overage_spend_limit", org_base),
            usage_headers,
        )
        .await;
    }

    let mut result = serde_json::Map::new();
    result.insert("version".to_string(), Value::Number(1.into()));
    result.insert(
        "fetchContext".to_string(),
        Value::String("cookie_direct".to_string()),
    );
    result.insert(
        "fetchedAt".to_string(),
        Value::String(chrono::Utc::now().to_rfc3339()),
    );
    result.insert("endpoints".to_string(), Value::Object(endpoints));
    if !errors.is_empty() {
        result.insert("errors".to_string(), Value::Object(errors));
    }
    Ok(Value::Object(result))
}

async fn fetch_desktop_web_profile_silent(profile_dir: &Path) -> Result<Value, String> {
    ensure_desktop_profile_logged_in(profile_dir)?;
    let export = read_desktop_cookie_export_for_silent_refresh(profile_dir)?;
    fetch_desktop_web_profile_with_cookies(&export.cookies).await
}

fn rewrite_desktop_cookies_with_exported_plaintext(
    profile_dir: &Path,
    export: &ClaudeDesktopAuthCookieExport,
) -> Result<(), String> {
    ensure_desktop_auth_export_logged_in(&export)?;
    let cookies_path = desktop_cookies_path(profile_dir);
    if !cookies_path.exists() {
        return Err(format!("Claude Cookies 不存在: {}", cookies_path.display()));
    }

    let conn = Connection::open_with_flags(
        &cookies_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| {
        format!(
            "打开 Claude Cookies 失败: path={}, error={}",
            cookies_path.display(),
            e
        )
    })?;
    let empty_encrypted_value: Vec<u8> = Vec::new();
    let mut updated_required_names = HashSet::new();
    let now_chromium = chromium_cookie_time_now();
    for cookie in export
        .cookies
        .iter()
        .filter(|cookie| !cookie.value.is_empty() && is_claude_cookie_domain(&cookie.domain))
    {
        let host_key = exported_cookie_host_key(cookie);
        let cookie_path = exported_cookie_path(cookie);
        let expires_utc = exported_cookie_expires_utc(cookie);
        let is_persistent = i64::from(expires_utc > 0);
        let is_secure = i64::from(cookie.secure);
        let is_httponly = i64::from(cookie.http_only);
        let samesite = exported_cookie_samesite(cookie);
        let source_type = exported_cookie_source_type(cookie);
        let updated_count = conn
            .execute(
                "update cookies set value = ?1, encrypted_value = ?2, expires_utc = ?3, \
                 is_secure = ?4, is_httponly = ?5, last_access_utc = ?6, \
                 has_expires = ?7, is_persistent = ?8, samesite = ?9, \
                 last_update_utc = ?10, source_type = ?11 \
                 where host_key = ?12 and name = ?13 and path = ?14",
                params![
                    cookie.value.as_str(),
                    empty_encrypted_value.as_slice(),
                    expires_utc,
                    is_secure,
                    is_httponly,
                    now_chromium,
                    is_persistent,
                    is_persistent,
                    samesite,
                    now_chromium,
                    source_type,
                    host_key.as_str(),
                    cookie.name.as_str(),
                    cookie_path
                ],
            )
            .map_err(|e| format!("写入 Claude plaintext cookie 失败: {}", e))?;
        if updated_count == 0 {
            conn.execute(
                "insert into cookies (
                    creation_utc, host_key, top_frame_site_key, name, value, encrypted_value,
                    path, expires_utc, is_secure, is_httponly, last_access_utc, has_expires,
                    is_persistent, priority, samesite, source_scheme, source_port,
                    last_update_utc, source_type, has_cross_site_ancestor
                ) values (
                    ?1, ?2, '', ?3, ?4, ?5,
                    ?6, ?7, ?8, ?9, ?10, ?11,
                    ?12, 1, ?13, 2, 443,
                    ?14, ?15, 1
                )
                on conflict(host_key, top_frame_site_key, has_cross_site_ancestor, name, path, source_scheme, source_port)
                do update set
                    value = excluded.value,
                    encrypted_value = excluded.encrypted_value,
                    expires_utc = excluded.expires_utc,
                    is_secure = excluded.is_secure,
                    is_httponly = excluded.is_httponly,
                    last_access_utc = excluded.last_access_utc,
                    has_expires = excluded.has_expires,
                    is_persistent = excluded.is_persistent,
                    samesite = excluded.samesite,
                    last_update_utc = excluded.last_update_utc,
                    source_type = excluded.source_type",
                params![
                    now_chromium,
                    host_key.as_str(),
                    cookie.name.as_str(),
                    cookie.value.as_str(),
                    empty_encrypted_value.as_slice(),
                    cookie_path,
                    expires_utc,
                    is_secure,
                    is_httponly,
                    now_chromium,
                    is_persistent,
                    is_persistent,
                    samesite,
                    now_chromium,
                    source_type
                ],
            )
            .map_err(|e| format!("写入 Claude plaintext cookie 失败: {}", e))?;
        }
        if CLAUDE_DESKTOP_REQUIRED_COOKIE_NAMES
            .iter()
            .any(|name| *name == cookie.name)
        {
            updated_required_names.insert(cookie.name.as_str());
        }
    }

    let missing_required_names = CLAUDE_DESKTOP_REQUIRED_COOKIE_NAMES
        .iter()
        .filter(|name| !updated_required_names.contains(**name))
        .copied()
        .collect::<Vec<_>>();
    if !missing_required_names.is_empty() {
        return Err(format!(
            "Claude Cookies 写入不完整，缺少: {}",
            missing_required_names.join(", ")
        ));
    }
    Ok(())
}

fn remove_path_if_exists(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|e| format!("读取路径信息失败: path={}, error={}", path.display(), e))?;
    if metadata.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
    .map_err(|e| format!("删除旧路径失败: path={}, error={}", path.display(), e))
}

fn copy_path_overwrite(src: &Path, dst: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(src)
        .map_err(|e| format!("读取源路径失败: path={}, error={}", src.display(), e))?;
    if metadata.is_dir() {
        remove_path_if_exists(dst)?;
        fs::create_dir_all(dst)
            .map_err(|e| format!("创建目标目录失败: path={}, error={}", dst.display(), e))?;
        for entry in fs::read_dir(src)
            .map_err(|e| format!("读取源目录失败: path={}, error={}", src.display(), e))?
        {
            let entry = entry.map_err(|e| format!("读取目录项失败: {}", e))?;
            let file_name = entry.file_name();
            if file_name == "LOCK" {
                continue;
            }
            copy_path_overwrite(&entry.path(), &dst.join(file_name))?;
        }
        return Ok(());
    }

    if metadata.is_file() {
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                format!("创建目标父目录失败: path={}, error={}", parent.display(), e)
            })?;
        }
        remove_path_if_exists(dst)?;
        fs::copy(src, dst).map_err(|e| {
            format!(
                "复制文件失败: from={}, to={}, error={}",
                src.display(),
                dst.display(),
                e
            )
        })?;
    }
    Ok(())
}

fn copy_desktop_profile_snapshot(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("创建 Claude 快照目录失败: {}", e))?;
    for item in CLAUDE_DESKTOP_PROFILE_ITEMS {
        let source = src.join(item);
        if !source.exists() {
            continue;
        }
        copy_path_overwrite(&source, &dst.join(item))?;
    }
    Ok(())
}

fn merge_desktop_config_token(
    snapshot_config_path: &Path,
    target_config_path: &Path,
) -> Result<(), String> {
    if !snapshot_config_path.exists() {
        return Ok(());
    }
    let snapshot = read_config_file(snapshot_config_path)?.unwrap_or_else(|| json!({}));
    let Some(token_cache) = snapshot.get("oauth:tokenCache").cloned() else {
        return Ok(());
    };
    let mut target = read_config_file(target_config_path)?.unwrap_or_else(|| json!({}));
    if !target.is_object() {
        target = json!({});
    }
    let object = target
        .as_object_mut()
        .ok_or_else(|| "Claude config.json 结构非法".to_string())?;
    object.insert("oauth:tokenCache".to_string(), token_cache);
    write_config_file(target_config_path, &target)
}

fn restore_desktop_profile_snapshot(snapshot_dir: &Path, target_dir: &Path) -> Result<(), String> {
    if !snapshot_dir.exists() {
        return Err(format!("Claude 快照目录不存在: {}", snapshot_dir.display()));
    }
    fs::create_dir_all(target_dir).map_err(|e| format!("创建 Claude profile 目录失败: {}", e))?;
    for item in CLAUDE_DESKTOP_PROFILE_ITEMS {
        let source = snapshot_dir.join(item);
        if !source.exists() {
            continue;
        }
        if *item == "config.json" {
            merge_desktop_config_token(&source, &target_dir.join(item))?;
        } else {
            copy_path_overwrite(&source, &target_dir.join(item))?;
        }
    }
    Ok(())
}

fn build_desktop_gateway_provider_config(account: &ClaudeAccount) -> Result<Value, String> {
    if account.auth_mode != ClaudeAuthMode::DesktopGateway {
        return Err("账号不是 Claude Gateway 类型".to_string());
    }
    let connection_mode = crate::modules::claude_desktop_gateway::normalize_connection_mode(
        account.desktop_gateway_connection_mode.as_deref(),
    );
    let (base_url, api_key, auth_scheme) = if connection_mode == "local_mapping" {
        let endpoint = crate::modules::claude_desktop_gateway::ensure_gateway_for_account(account)?;
        (endpoint.base_url, endpoint.api_key, "bearer".to_string())
    } else {
        let api_key = account
            .api_key
            .as_deref()
            .and_then(|value| normalize_non_empty(Some(value)))
            .ok_or_else(|| "Claude Gateway 账号缺少 API Key".to_string())?;
        let base_url = account
            .api_base_url
            .as_deref()
            .and_then(|value| normalize_non_empty(Some(value)))
            .ok_or_else(|| "Claude Gateway 账号缺少 Base URL".to_string())?;
        let auth_scheme =
            normalize_desktop_gateway_auth_scheme(account.desktop_gateway_auth_scheme.as_deref());
        (
            base_url,
            api_key.to_string(),
            if auth_scheme == "auto" {
                "bearer".to_string()
            } else {
                auth_scheme
            },
        )
    };
    let credential_kind = account
        .desktop_gateway_credential_kind
        .as_deref()
        .and_then(|value| normalize_non_empty(Some(value)))
        .unwrap_or_else(|| "static".to_string());
    if credential_kind != "static" {
        return Err("当前仅支持 static Gateway API Key".to_string());
    }
    let mut config = json!({
        "coworkEgressAllowedHosts": ["*"],
        "disableDeploymentModeChooser": true,
        "inferenceProvider": "gateway",
        "inferenceGatewayBaseUrl": base_url,
        "inferenceGatewayApiKey": api_key,
        "inferenceGatewayAuthScheme": auth_scheme,
    });
    if let Some(models) = account
        .desktop_gateway_models
        .as_ref()
        .filter(|items| !items.is_empty())
    {
        let mapping_meta = crate::modules::claude_desktop_gateway::normalize_model_mappings(
            account.desktop_gateway_model_mappings.clone(),
        )
        .unwrap_or_default()
        .into_iter()
        .map(|mapping| (mapping.desktop_model.to_ascii_lowercase(), mapping))
        .collect::<BTreeMap<_, _>>();
        config["inferenceModels"] = Value::Array(
            models
                .iter()
                .filter_map(|model| {
                    let name = normalize_non_empty(Some(model))?;
                    let mut item = json!({ "name": name.clone() });
                    if let Some(mapping) = mapping_meta.get(&name.to_ascii_lowercase()) {
                        if let Some(label_override) = mapping
                            .label_override
                            .as_deref()
                            .and_then(|value| normalize_non_empty(Some(value)))
                        {
                            item["labelOverride"] = Value::String(label_override);
                        }
                        if mapping.supports_1m.unwrap_or(false) {
                            item["supports1m"] = Value::Bool(true);
                        }
                    }
                    Some(item)
                })
                .collect(),
        );
    }
    Ok(config)
}

fn is_claude_desktop_gateway_config(value: &Value) -> bool {
    value
        .get("inferenceProvider")
        .and_then(Value::as_str)
        .is_some_and(|provider| provider.eq_ignore_ascii_case("gateway"))
        || value
            .get("deploymentMode")
            .and_then(Value::as_str)
            .is_some_and(|mode| mode.eq_ignore_ascii_case("3p"))
}

fn write_desktop_deployment_mode(config_path: &Path, mode: &str) -> Result<(), String> {
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "创建 Claude Desktop 配置目录失败: path={}, error={}",
                parent.display(),
                e
            )
        })?;
    }
    let mut config = read_config_file(config_path)?.unwrap_or_else(|| json!({}));
    if !config.is_object() {
        config = json!({});
    }
    let object = config
        .as_object_mut()
        .ok_or_else(|| "Claude Desktop 配置结构非法".to_string())?;
    object.insert(
        "deploymentMode".to_string(),
        Value::String(mode.to_string()),
    );
    write_config_file(config_path, &config)
}

fn config_library_gateway_ids(config_library_dir: &Path) -> Result<BTreeSet<String>, String> {
    let mut ids = BTreeSet::new();
    if !config_library_dir.exists() {
        return Ok(ids);
    }
    for entry in fs::read_dir(config_library_dir).map_err(|e| {
        format!(
            "读取 Claude configLibrary 失败: path={}, error={}",
            config_library_dir.display(),
            e
        )
    })? {
        let entry = entry.map_err(|e| format!("读取 Claude configLibrary 项失败: {}", e))?;
        let path = entry.path();
        if path.file_name().and_then(|value| value.to_str()) == Some("_meta.json")
            || path.extension().and_then(|value| value.to_str()) != Some("json")
        {
            continue;
        }
        if let Some(config) = read_config_file(&path)? {
            if is_claude_desktop_gateway_config(&config) {
                if let Some(stem) = path.file_stem().and_then(|value| value.to_str()) {
                    ids.insert(stem.to_string());
                }
            }
        }
    }
    Ok(ids)
}

fn remove_gateway_configs_from_config_library(config_library_dir: &Path) -> Result<(), String> {
    if !config_library_dir.exists() {
        return Ok(());
    }
    let mut gateway_ids = config_library_gateway_ids(config_library_dir)?;
    let meta_path = config_library_dir.join("_meta.json");
    let mut meta = read_config_file(&meta_path)?.unwrap_or_else(|| json!({}));
    if let Some(entries) = meta.get("entries").and_then(Value::as_array) {
        for entry in entries {
            let is_gateway_entry = entry
                .get("provider")
                .and_then(Value::as_str)
                .is_some_and(|provider| provider.eq_ignore_ascii_case("gateway"));
            if is_gateway_entry {
                if let Some(id) = entry.get("id").and_then(Value::as_str) {
                    gateway_ids.insert(id.to_string());
                }
            }
        }
    }

    for id in &gateway_ids {
        remove_path_if_exists(&config_library_dir.join(format!("{}.json", id)))?;
    }

    if meta.is_object() {
        let object = meta
            .as_object_mut()
            .ok_or_else(|| "Claude configLibrary 元数据结构非法".to_string())?;
        let mut entries = object
            .get("entries")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        entries.retain(|entry| {
            let id_removed = entry
                .get("id")
                .and_then(Value::as_str)
                .map(|id| gateway_ids.contains(id))
                .unwrap_or(false);
            let provider_gateway = entry
                .get("provider")
                .and_then(Value::as_str)
                .is_some_and(|provider| provider.eq_ignore_ascii_case("gateway"));
            !id_removed && !provider_gateway
        });

        let should_clear_applied = object
            .get("appliedId")
            .and_then(Value::as_str)
            .map(|id| gateway_ids.contains(id))
            .unwrap_or(false);
        if should_clear_applied {
            if let Some(next_id) = entries
                .iter()
                .find_map(|entry| entry.get("id").and_then(Value::as_str))
            {
                object.insert("appliedId".to_string(), Value::String(next_id.to_string()));
            } else {
                object.remove("appliedId");
            }
        }
        object.insert("entries".to_string(), Value::Array(entries));
        write_config_file(&meta_path, &meta)?;
    }

    Ok(())
}

fn remove_desktop_gateway_profile_config(target_dir: &Path) -> Result<(), String> {
    let desktop_config_path = target_dir.join(CLAUDE_DESKTOP_CONFIG_FILE_NAME);
    if let Some(config) = read_config_file(&desktop_config_path)? {
        if is_claude_desktop_gateway_config(&config) {
            remove_path_if_exists(&desktop_config_path)?;
        }
    }

    let config_library_dir = target_dir.join(CLAUDE_DESKTOP_CONFIG_LIBRARY_DIR);
    remove_gateway_configs_from_config_library(&config_library_dir)?;
    Ok(())
}

fn write_desktop_gateway_config_library(
    account: &ClaudeAccount,
    config_library_dir: &Path,
) -> Result<String, String> {
    let config_id = account
        .desktop_gateway_config_id
        .as_deref()
        .filter(|value| UUID_RE.is_match(value))
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let account_name = account.email.trim();
    let entry_name = if account_name.is_empty() {
        "Default"
    } else {
        account_name
    };
    fs::create_dir_all(&config_library_dir)
        .map_err(|e| format!("创建 Claude Gateway configLibrary 失败: {}", e))?;
    remove_gateway_configs_from_config_library(config_library_dir)?;
    let meta_path = config_library_dir.join("_meta.json");
    let mut meta = read_config_file(&meta_path)?.unwrap_or_else(|| json!({}));
    if !meta.is_object() {
        meta = json!({});
    }
    let object = meta
        .as_object_mut()
        .ok_or_else(|| "Claude configLibrary 元数据结构非法".to_string())?;
    let mut entries = object
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    entries.retain(|entry| entry.get("id").and_then(Value::as_str) != Some(config_id.as_str()));
    entries.push(json!({
        "id": config_id.clone(),
        "name": entry_name,
    }));
    object.insert("appliedId".to_string(), Value::String(config_id.clone()));
    object.insert("entries".to_string(), Value::Array(entries));
    write_config_file(&meta_path, &meta)?;
    write_config_file(
        &config_library_dir.join(format!("{}.json", config_id)),
        &build_desktop_gateway_provider_config(account)?,
    )?;
    Ok(config_id)
}

fn write_desktop_gateway_profile(account: &ClaudeAccount, target_dir: &Path) -> Result<(), String> {
    fs::create_dir_all(target_dir)
        .map_err(|e| format!("创建 Claude Gateway profile 失败: {}", e))?;
    write_desktop_deployment_mode(&target_dir.join(CLAUDE_DESKTOP_CONFIG_FILE_NAME), "3p")?;
    let config_id = write_desktop_gateway_config_library(
        account,
        &target_dir.join(CLAUDE_DESKTOP_CONFIG_LIBRARY_DIR),
    )?;
    validate_desktop_deployment_mode(&target_dir.join(CLAUDE_DESKTOP_CONFIG_FILE_NAME), "3p")?;
    validate_desktop_gateway_meta(
        &target_dir
            .join(CLAUDE_DESKTOP_CONFIG_LIBRARY_DIR)
            .join("_meta.json"),
        &config_id,
    )?;
    Ok(())
}

fn write_default_desktop_gateway_profile(account: &ClaudeAccount) -> Result<(), String> {
    let paths = get_default_claude_desktop_gateway_config_paths()?;
    write_desktop_deployment_mode(&paths.normal_config_path, "3p")?;
    write_desktop_deployment_mode(&paths.threep_config_path, "3p")?;
    let config_id = write_desktop_gateway_config_library(account, &paths.config_library_dir)?;
    validate_desktop_deployment_mode(&paths.normal_config_path, "3p")?;
    validate_desktop_deployment_mode(&paths.threep_config_path, "3p")?;
    validate_desktop_gateway_meta(&paths.config_library_meta_path(), &config_id)?;
    logger::log_info(&format!(
        "[Claude Gateway] default profile applied: account_id={}, config_id={}, normal_config={}, threep_config={}, config_library={}",
        account.id,
        config_id,
        paths.normal_config_path.display(),
        paths.threep_config_path.display(),
        paths.config_library_dir.display()
    ));
    Ok(())
}

fn restore_default_desktop_gateway_official_config() -> Result<(), String> {
    let paths = get_default_claude_desktop_gateway_config_paths()?;
    write_desktop_deployment_mode(&paths.normal_config_path, "1p")?;
    write_desktop_deployment_mode(&paths.threep_config_path, "1p")?;
    remove_gateway_configs_from_config_library(&paths.config_library_dir)?;
    validate_desktop_deployment_mode(&paths.normal_config_path, "1p")?;
    validate_desktop_deployment_mode(&paths.threep_config_path, "1p")?;
    Ok(())
}

pub fn restore_desktop_account_to_profile(
    account_id: &str,
    target_dir: &Path,
    backup_existing: bool,
) -> Result<(), String> {
    let account = load_account(account_id).ok_or_else(|| "Claude 账号不存在".to_string())?;
    if account.auth_mode != ClaudeAuthMode::DesktopOAuth {
        return Err("绑定账号不是 Claude 登录态，无法写入 Claude profile。".to_string());
    }
    let snapshot_dir = account
        .desktop_profile_dir
        .as_deref()
        .and_then(|value| normalize_non_empty(Some(value)))
        .map(PathBuf::from)
        .ok_or_else(|| "Claude 账号缺少 profile 快照".to_string())?;

    if backup_existing {
        let _backup_dir = backup_current_desktop_profile(target_dir)?;
    }
    remove_desktop_gateway_profile_config(target_dir)?;
    restore_desktop_profile_snapshot(&snapshot_dir, target_dir)?;
    remove_desktop_gateway_profile_config(target_dir)?;

    let mut updated = account.clone();
    updated.last_used = now_ts_ms();
    save_account_and_index(updated)?;
    Ok(())
}

pub fn restore_desktop_gateway_account_to_profile(
    account_id: &str,
    target_dir: &Path,
    backup_existing: bool,
) -> Result<(), String> {
    let account = load_account(account_id).ok_or_else(|| "Claude 账号不存在".to_string())?;
    if account.auth_mode != ClaudeAuthMode::DesktopGateway {
        return Err("绑定账号不是 Claude Gateway 类型。".to_string());
    }
    if backup_existing {
        let _backup_dir = backup_current_desktop_profile(target_dir)?;
    }
    write_desktop_gateway_profile(&account, target_dir)?;

    let mut updated = account.clone();
    updated.last_used = now_ts_ms();
    save_account_and_index(updated)?;
    Ok(())
}

fn backup_current_desktop_profile(target_dir: &Path) -> Result<Option<PathBuf>, String> {
    if !target_dir.exists() {
        return Ok(None);
    }
    let backup_dir = crate::modules::backup_storage::behavior_backup_dir(
        "claude",
        &crate::modules::backup_storage::scope_for_path(target_dir),
        &format!("{}", now_ts_ms()),
    )?;
    copy_desktop_profile_snapshot(target_dir, &backup_dir)?;
    let _ = crate::modules::backup_storage::prune_behavior_backups(
        "claude",
        &crate::modules::backup_storage::scope_for_path(target_dir),
    );
    Ok(Some(backup_dir))
}

fn get_desktop_auth_resource_dir() -> Option<PathBuf> {
    crate::get_app_handle()
        .and_then(|app| app.path().resource_dir().ok())
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|dir| dir.join("src-tauri").join("resources"))
        })
        .filter(|path| path.exists())
}

fn find_desktop_auth_helper_script() -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    if let Some(resource_dir) = get_desktop_auth_resource_dir() {
        candidates.push(resource_dir.join(CLAUDE_DESKTOP_AUTH_HELPER_SCRIPT));
    }
    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join(CLAUDE_DESKTOP_AUTH_HELPER_SCRIPT));
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut current = exe.parent();
        while let Some(dir) = current {
            candidates.push(dir.join(CLAUDE_DESKTOP_AUTH_HELPER_SCRIPT));
            current = dir.parent();
        }
    }
    candidates
        .into_iter()
        .find(|path| path.exists())
        .ok_or_else(|| {
            format!(
                "未找到 Claude 授权 helper 脚本，请确认 {} 存在。",
                CLAUDE_DESKTOP_AUTH_HELPER_SCRIPT
            )
        })
}

#[derive(Debug, Clone)]
struct ElectronRuntimeAsset {
    platform_key: &'static str,
    file_name: &'static str,
    sha256: &'static str,
    executable_relative: &'static str,
}

#[derive(Clone)]
struct ClaudeDesktopLoginProgressContext {
    app: AppHandle,
    progress_id: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ClaudeDesktopLoginProgressPayload {
    progress_id: String,
    phase: String,
    percent: Option<f64>,
    downloaded_bytes: Option<u64>,
    total_bytes: Option<u64>,
}

fn emit_desktop_login_progress(
    context: Option<&ClaudeDesktopLoginProgressContext>,
    phase: &str,
    percent: Option<f64>,
    downloaded_bytes: Option<u64>,
    total_bytes: Option<u64>,
) {
    let Some(context) = context else {
        return;
    };
    let payload = ClaudeDesktopLoginProgressPayload {
        progress_id: context.progress_id.clone(),
        phase: phase.to_string(),
        percent: percent.map(|value| value.clamp(0.0, 100.0)),
        downloaded_bytes,
        total_bytes,
    };
    let _ = context
        .app
        .emit(CLAUDE_DESKTOP_LOGIN_PROGRESS_EVENT, payload);
}

fn electron_runtime_asset_for_current_platform() -> Result<ElectronRuntimeAsset, String> {
    let arch = std::env::consts::ARCH;
    #[cfg(target_os = "macos")]
    {
        return match arch {
            "aarch64" => Ok(ElectronRuntimeAsset {
                platform_key: "darwin-arm64",
                file_name: "electron-v42.4.0-darwin-arm64.zip",
                sha256: "3ce55988c9998bcd1e9c69478dd26887b90e8f8010441172e520e94ba575e520",
                executable_relative: "Electron.app/Contents/MacOS/Electron",
            }),
            "x86_64" => Ok(ElectronRuntimeAsset {
                platform_key: "darwin-x64",
                file_name: "electron-v42.4.0-darwin-x64.zip",
                sha256: "0f141809eebe3f3f8c8f8377c10c93f21a39433f71526598de5e989f452cae29",
                executable_relative: "Electron.app/Contents/MacOS/Electron",
            }),
            _ => Err(format!(
                "当前 macOS 架构暂不支持自动下载 Electron: {}",
                arch
            )),
        };
    }

    #[cfg(target_os = "windows")]
    {
        return match arch {
            "x86_64" => Ok(ElectronRuntimeAsset {
                platform_key: "win32-x64",
                file_name: "electron-v42.4.0-win32-x64.zip",
                sha256: "ffc056685b4a769d7977ef3d58bdc332446d081f025ee074d77b498d2962e2cd",
                executable_relative: "electron.exe",
            }),
            "aarch64" => Ok(ElectronRuntimeAsset {
                platform_key: "win32-arm64",
                file_name: "electron-v42.4.0-win32-arm64.zip",
                sha256: "5d576f908c9e88209dfe8a17f7e84c4949288c2ef611637c301d562bc8d08d61",
                executable_relative: "electron.exe",
            }),
            _ => Err(format!(
                "当前 Windows 架构暂不支持自动下载 Electron: {}",
                arch
            )),
        };
    }

    #[cfg(target_os = "linux")]
    {
        return match arch {
            "x86_64" => Ok(ElectronRuntimeAsset {
                platform_key: "linux-x64",
                file_name: "electron-v42.4.0-linux-x64.zip",
                sha256: "9a8194635548490a56099cc4c2b116738ae56834dee4472506d5a8b262bcbda4",
                executable_relative: "electron",
            }),
            "aarch64" => Ok(ElectronRuntimeAsset {
                platform_key: "linux-arm64",
                file_name: "electron-v42.4.0-linux-arm64.zip",
                sha256: "d3bf612de0b651302fb46e50ed3282b609ea9d4d99bb296f7c9bb8ffd92fd69b",
                executable_relative: "electron",
            }),
            _ => Err(format!(
                "当前 Linux 架构暂不支持自动下载 Electron: {}",
                arch
            )),
        };
    }

    #[allow(unreachable_code)]
    Err(format!(
        "当前平台暂不支持自动下载 Electron: {}-{}",
        std::env::consts::OS,
        arch
    ))
}

fn electron_runtime_root_dir() -> Result<PathBuf, String> {
    Ok(get_data_dir()?
        .join(CLAUDE_DESKTOP_ELECTRON_RUNTIME_DIR)
        .join(CLAUDE_DESKTOP_ELECTRON_VERSION))
}

fn electron_runtime_download_url(asset: &ElectronRuntimeAsset) -> String {
    format!(
        "https://github.com/electron/electron/releases/download/v{}/{}",
        CLAUDE_DESKTOP_ELECTRON_VERSION, asset.file_name
    )
}

fn sha256_file_hex(path: &Path) -> Result<String, String> {
    let mut file =
        File::open(path).map_err(|e| format!("读取 Electron runtime 文件失败: {}", e))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 1024 * 256];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("读取 Electron runtime 文件失败: {}", e))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    Ok(hex_encode(&digest))
}

fn electron_runtime_zip_path(asset: &ElectronRuntimeAsset) -> Result<PathBuf, String> {
    Ok(electron_runtime_root_dir()?.join(asset.file_name))
}

fn verify_cached_electron_zip(asset: &ElectronRuntimeAsset, zip_path: &Path) -> bool {
    if !zip_path.exists() {
        return false;
    }
    match sha256_file_hex(zip_path) {
        Ok(actual) if actual.eq_ignore_ascii_case(asset.sha256) => true,
        Ok(actual) => {
            logger::log_warn(&format!(
                "[Claude Auth] Electron runtime 缓存校验失败，准备重新下载: path={}, expected={}, actual={}",
                zip_path.display(),
                asset.sha256,
                actual
            ));
            let _ = fs::remove_file(zip_path);
            false
        }
        Err(error) => {
            logger::log_warn(&format!(
                "[Claude Auth] Electron runtime 缓存读取失败，准备重新下载: path={}, error={}",
                zip_path.display(),
                error
            ));
            let _ = fs::remove_file(zip_path);
            false
        }
    }
}

fn download_electron_runtime_zip(
    asset: &ElectronRuntimeAsset,
    zip_path: &Path,
    progress: Option<&ClaudeDesktopLoginProgressContext>,
) -> Result<(), String> {
    emit_desktop_login_progress(progress, "check-cache", Some(10.0), None, None);
    if verify_cached_electron_zip(asset, zip_path) {
        emit_desktop_login_progress(progress, "cached", Some(82.0), None, None);
        return Ok(());
    }

    if let Some(parent) = zip_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("创建 Electron runtime 缓存目录失败: {}", e))?;
    }

    let url = electron_runtime_download_url(asset);
    emit_desktop_login_progress(progress, "download-start", Some(12.0), Some(0), None);
    logger::log_info(&format!(
        "[Claude Auth] 开始下载 Electron runtime: url={}, target={}",
        url,
        zip_path.display()
    ));

    let client = reqwest::blocking::Client::builder()
        .user_agent("Cockpit-Tools")
        .timeout(Duration::from_secs(15 * 60))
        .build()
        .map_err(|e| format!("创建 Electron runtime 下载客户端失败: {}", e))?;
    let mut response = client
        .get(&url)
        .send()
        .map_err(|e| format!("下载 Electron runtime 失败: {}", e))?;
    if !response.status().is_success() {
        return Err(format!(
            "下载 Electron runtime 失败: HTTP {} ({})",
            response.status(),
            url
        ));
    }
    let total_bytes = response.content_length();

    let temp_path = zip_path.with_extension("zip.part");
    let mut temp_file = File::create(&temp_path)
        .map_err(|e| format!("创建 Electron runtime 临时文件失败: {}", e))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 1024 * 256];
    let mut downloaded: u64 = 0;
    let mut last_progress_emit = Instant::now();
    let mut last_progress_bytes = 0u64;
    const MAX_ELECTRON_RUNTIME_DOWNLOAD_BYTES: u64 = 350 * 1024 * 1024;
    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|e| format!("读取 Electron runtime 下载数据失败: {}", e))?;
        if read == 0 {
            break;
        }
        downloaded += read as u64;
        if downloaded > MAX_ELECTRON_RUNTIME_DOWNLOAD_BYTES {
            let _ = fs::remove_file(&temp_path);
            return Err("Electron runtime 下载内容超过预期大小，已停止。".to_string());
        }
        hasher.update(&buffer[..read]);
        temp_file
            .write_all(&buffer[..read])
            .map_err(|e| format!("写入 Electron runtime 临时文件失败: {}", e))?;
        let should_emit = downloaded.saturating_sub(last_progress_bytes) >= 1024 * 1024
            || last_progress_emit.elapsed() >= Duration::from_millis(500);
        if should_emit {
            let percent = total_bytes
                .filter(|total| *total > 0)
                .map(|total| 15.0 + ((downloaded as f64 / total as f64).min(1.0) * 50.0));
            emit_desktop_login_progress(
                progress,
                "downloading",
                percent,
                Some(downloaded),
                total_bytes,
            );
            last_progress_emit = Instant::now();
            last_progress_bytes = downloaded;
        }
    }
    emit_desktop_login_progress(
        progress,
        "downloaded",
        Some(65.0),
        Some(downloaded),
        total_bytes,
    );
    temp_file
        .sync_all()
        .map_err(|e| format!("同步 Electron runtime 临时文件失败: {}", e))?;
    drop(temp_file);

    emit_desktop_login_progress(
        progress,
        "verify",
        Some(68.0),
        Some(downloaded),
        total_bytes,
    );
    let actual = hex_encode(&hasher.finalize());
    if !actual.eq_ignore_ascii_case(asset.sha256) {
        let _ = fs::remove_file(&temp_path);
        return Err(format!(
            "Electron runtime 校验失败: expected={}, actual={}",
            asset.sha256, actual
        ));
    }

    if zip_path.exists() {
        let _ = fs::remove_file(zip_path);
    }
    fs::rename(&temp_path, zip_path)
        .map_err(|e| format!("保存 Electron runtime 缓存失败: {}", e))?;
    logger::log_info(&format!(
        "[Claude Auth] Electron runtime 下载完成: path={}, bytes={}",
        zip_path.display(),
        downloaded
    ));
    Ok(())
}

fn extract_electron_runtime_zip(
    asset: &ElectronRuntimeAsset,
    zip_path: &Path,
    runtime_dir: &Path,
    progress: Option<&ClaudeDesktopLoginProgressContext>,
) -> Result<(), String> {
    emit_desktop_login_progress(progress, "extract", Some(74.0), None, None);
    let parent = runtime_dir
        .parent()
        .ok_or_else(|| format!("无法定位 Electron runtime 目录: {}", runtime_dir.display()))?;
    fs::create_dir_all(parent).map_err(|e| format!("创建 Electron runtime 目录失败: {}", e))?;
    let staging_dir = parent.join(format!(
        ".{}.extracting.{}",
        asset.platform_key,
        std::process::id()
    ));
    let _ = remove_path_if_exists(&staging_dir);
    fs::create_dir_all(&staging_dir)
        .map_err(|e| format!("创建 Electron runtime 解压目录失败: {}", e))?;

    let archive_file =
        File::open(zip_path).map_err(|e| format!("打开 Electron runtime 压缩包失败: {}", e))?;
    let mut archive = zip::ZipArchive::new(archive_file)
        .map_err(|e| format!("解析 Electron runtime 压缩包失败: {}", e))?;
    archive
        .extract(&staging_dir)
        .map_err(|e| format!("解压 Electron runtime 失败: {}", e))?;

    let executable = staging_dir.join(asset.executable_relative);
    if !executable.exists() {
        let _ = remove_path_if_exists(&staging_dir);
        return Err(format!(
            "Electron runtime 解压后缺少可执行文件: {}",
            executable.display()
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&executable, fs::Permissions::from_mode(0o755));
    }

    if runtime_dir.exists() {
        remove_path_if_exists(runtime_dir)?;
    }
    fs::rename(&staging_dir, runtime_dir)
        .map_err(|e| format!("保存 Electron runtime 解压目录失败: {}", e))?;
    emit_desktop_login_progress(progress, "runtime-ready", Some(86.0), None, None);
    Ok(())
}

fn ensure_downloaded_electron_runtime(
    progress: Option<&ClaudeDesktopLoginProgressContext>,
) -> Result<PathBuf, String> {
    let _guard = CLAUDE_DESKTOP_ELECTRON_RUNTIME_LOCK
        .lock()
        .map_err(|_| "Electron runtime 下载锁已损坏".to_string())?;
    emit_desktop_login_progress(progress, "resolve-runtime", Some(6.0), None, None);
    let asset = electron_runtime_asset_for_current_platform()?;
    let runtime_dir = electron_runtime_root_dir()?.join(asset.platform_key);
    let executable = runtime_dir.join(asset.executable_relative);
    if executable.exists() {
        logger::log_info(&format!(
            "[Claude Auth] 使用已缓存 Electron runtime: {}",
            executable.display()
        ));
        emit_desktop_login_progress(progress, "cached", Some(86.0), None, None);
        return Ok(executable);
    }

    let zip_path = electron_runtime_zip_path(&asset)?;
    download_electron_runtime_zip(&asset, &zip_path, progress)?;
    extract_electron_runtime_zip(&asset, &zip_path, &runtime_dir, progress)?;
    let executable = runtime_dir.join(asset.executable_relative);
    if executable.exists() {
        logger::log_info(&format!(
            "[Claude Auth] Electron runtime 已准备: {}",
            executable.display()
        ));
        return Ok(executable);
    }
    Err(format!(
        "Electron runtime 已下载但不可用: {}",
        executable.display()
    ))
}

fn find_electron_executable_for_desktop_auth(
    progress: Option<&ClaudeDesktopLoginProgressContext>,
) -> Result<PathBuf, String> {
    emit_desktop_login_progress(progress, "resolve-runtime", Some(3.0), None, None);
    if let Ok(value) = std::env::var("CLAUDE_DESKTOP_AUTH_ELECTRON") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            let path = PathBuf::from(trimmed);
            if path.exists() {
                emit_desktop_login_progress(progress, "cached", Some(86.0), None, None);
                return Ok(path);
            }
        }
    }

    let mut candidates = Vec::new();
    if let Ok(current_dir) = std::env::current_dir() {
        candidates.extend(electron_node_modules_executable_candidates(&current_dir));
    }
    if let Ok(exe) = std::env::current_exe() {
        let mut current = exe.parent();
        while let Some(dir) = current {
            candidates.extend(electron_node_modules_executable_candidates(dir));
            current = dir.parent();
        }
    }

    let checked = candidates
        .iter()
        .map(|path| {
            format!(
                "{} [{}]",
                path.display(),
                if path.exists() { "exists" } else { "missing" }
            )
        })
        .collect::<Vec<_>>()
        .join("; ");

    for path in candidates {
        if path.exists() {
            logger::log_info(&format!("[Claude Auth] 使用 Electron: {}", path.display()));
            emit_desktop_login_progress(progress, "cached", Some(86.0), None, None);
            return Ok(path);
        }
    }

    match ensure_downloaded_electron_runtime(progress) {
        Ok(path) => return Ok(path),
        Err(error) => {
            let checked_detail = if checked.is_empty() {
                "(无本地候选路径)".to_string()
            } else {
                checked
            };
            return Err(format!(
                "未找到本地 Electron 运行时，且自动准备失败: {}。已检查: {}",
                error, checked_detail
            ));
        }
    }
}

fn electron_node_modules_executable_candidates(root: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let electron_pkg = root.join("node_modules").join("electron");
    let electron_bin = root.join("node_modules").join(".bin");

    #[cfg(target_os = "windows")]
    {
        candidates.push(electron_pkg.join("dist").join("electron.exe"));
        candidates.push(electron_bin.join("electron.exe"));
        candidates.push(electron_bin.join("electron.cmd"));
    }
    #[cfg(target_os = "macos")]
    {
        candidates.push(
            electron_pkg
                .join("dist")
                .join("Electron.app")
                .join("Contents")
                .join("MacOS")
                .join("Electron"),
        );
        candidates.push(electron_bin.join("electron"));
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        candidates.push(electron_pkg.join("dist").join("electron"));
        candidates.push(electron_bin.join("electron"));
    }

    candidates
}

#[cfg(test)]
fn test_electron_runtime_asset_for(os: &str, arch: &str) -> Option<ElectronRuntimeAsset> {
    match (os, arch) {
        ("macos", "aarch64") => Some(ElectronRuntimeAsset {
            platform_key: "darwin-arm64",
            file_name: "electron-v42.4.0-darwin-arm64.zip",
            sha256: "3ce55988c9998bcd1e9c69478dd26887b90e8f8010441172e520e94ba575e520",
            executable_relative: "Electron.app/Contents/MacOS/Electron",
        }),
        ("macos", "x86_64") => Some(ElectronRuntimeAsset {
            platform_key: "darwin-x64",
            file_name: "electron-v42.4.0-darwin-x64.zip",
            sha256: "0f141809eebe3f3f8c8f8377c10c93f21a39433f71526598de5e989f452cae29",
            executable_relative: "Electron.app/Contents/MacOS/Electron",
        }),
        ("windows", "x86_64") => Some(ElectronRuntimeAsset {
            platform_key: "win32-x64",
            file_name: "electron-v42.4.0-win32-x64.zip",
            sha256: "ffc056685b4a769d7977ef3d58bdc332446d081f025ee074d77b498d2962e2cd",
            executable_relative: "electron.exe",
        }),
        ("windows", "aarch64") => Some(ElectronRuntimeAsset {
            platform_key: "win32-arm64",
            file_name: "electron-v42.4.0-win32-arm64.zip",
            sha256: "5d576f908c9e88209dfe8a17f7e84c4949288c2ef611637c301d562bc8d08d61",
            executable_relative: "electron.exe",
        }),
        ("linux", "x86_64") => Some(ElectronRuntimeAsset {
            platform_key: "linux-x64",
            file_name: "electron-v42.4.0-linux-x64.zip",
            sha256: "9a8194635548490a56099cc4c2b116738ae56834dee4472506d5a8b262bcbda4",
            executable_relative: "electron",
        }),
        ("linux", "aarch64") => Some(ElectronRuntimeAsset {
            platform_key: "linux-arm64",
            file_name: "electron-v42.4.0-linux-arm64.zip",
            sha256: "d3bf612de0b651302fb46e50ed3282b609ea9d4d99bb296f7c9bb8ffd92fd69b",
            executable_relative: "electron",
        }),
        _ => None,
    }
}
