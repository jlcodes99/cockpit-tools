// Codex 账号模块：Official auth-store and runtime authority synchronization。
// 通过 include! 保持原 modules::codex_account 作用域，完整保留私有调用关系。
#[derive(Debug, Clone)]
struct LocalCodexOAuthSnapshot {
    tokens: CodexTokens,
    email: String,
    user_id: Option<String>,
    subscription_active_until: Option<String>,
    account_id: Option<String>,
    organization_id: Option<String>,
    last_refresh_at: Option<i64>,
}

/// 官方 profile 持久化的 OAuth 身份，不包含任何 Token 内容。
///
/// 账号总览、默认实例和多开实例都通过 profile 的 auth.json/Keychain 读取此身份，
/// 用于确认“当前实例实际落盘账号”后再写入 CDP 观测状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodexOfficialOAuthIdentity {
    pub(crate) email: String,
    pub(crate) user_id: Option<String>,
    pub(crate) account_id: Option<String>,
    pub(crate) organization_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodexOfficialOAuthIdentityMatch {
    Matched,
    Mismatched,
    Unknown,
}

fn parse_auth_file_last_refresh(value: Option<&serde_json::Value>) -> Option<i64> {
    let value = value?;
    if let Some(raw) = value.as_i64() {
        return Some(if raw > 1_000_000_000_000 {
            raw / 1000
        } else {
            raw
        });
    }
    if let Some(raw) = value.as_u64() {
        let normalized = if raw > 1_000_000_000_000 {
            raw / 1000
        } else {
            raw
        };
        return i64::try_from(normalized).ok();
    }

    let raw = value.as_str()?.trim();
    if raw.is_empty() {
        return None;
    }
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(raw) {
        return Some(parsed.timestamp());
    }
    if let Ok(parsed) = raw.parse::<i64>() {
        return Some(if parsed > 1_000_000_000_000 {
            parsed / 1000
        } else {
            parsed
        });
    }

    None
}

fn build_local_oauth_snapshot(tokens: CodexAuthTokens) -> Option<LocalCodexOAuthSnapshot> {
    let id_token_info = extract_user_info(&tokens.id_token).ok();
    let (
        access_email,
        access_user_id,
        _,
        access_subscription_active_until,
        access_account_id,
        access_organization_id,
    ) = extract_access_token_identity(&tokens.access_token);
    let email = id_token_info
        .as_ref()
        .map(|info| info.0.clone())
        .or(access_email)?;
    let user_id = id_token_info
        .as_ref()
        .and_then(|info| info.1.clone())
        .or(access_user_id);
    let subscription_active_until = id_token_info
        .as_ref()
        .and_then(|info| info.3.clone())
        .or(access_subscription_active_until);
    let id_token_account_id = id_token_info.as_ref().and_then(|info| info.4.clone());
    let id_token_org_id = id_token_info.as_ref().and_then(|info| info.5.clone());
    let account_id = normalize_optional_value(
        tokens
            .account_id
            .clone()
            .or(access_account_id)
            .or(id_token_account_id),
    );
    let organization_id = normalize_optional_value(access_organization_id.or(id_token_org_id));

    Some(LocalCodexOAuthSnapshot {
        tokens: CodexTokens {
            id_token: tokens.id_token,
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
        },
        email,
        user_id,
        subscription_active_until,
        account_id,
        organization_id,
        last_refresh_at: None,
    })
}

/// 从官方认证存储读取当前 profile 的身份摘要。
///
/// 认证存储可能是 auth.json，也可能是配置指定的 macOS Keychain；两者均属于
/// 官方持久化数据。读取失败返回 None，由调用方按 Unknown 处理，不能据此判定账号失效。
pub(crate) fn read_official_oauth_identity(base_dir: &Path) -> Option<CodexOfficialOAuthIdentity> {
    load_local_oauth_snapshot_from_official_store(base_dir).map(|snapshot| {
        CodexOfficialOAuthIdentity {
            email: snapshot.email,
            user_id: snapshot.user_id,
            account_id: snapshot.account_id,
            organization_id: snapshot.organization_id,
        }
    })
}

/// 将官方 profile 身份与本地账号身份进行强字段优先匹配。
///
/// account ID、组织 ID 和 user ID 只要双方都有就必须一致；如果没有任何强字段，
/// 才回退到不区分大小写的 email，避免 profile 串号时把错误状态写入绑定账号。
pub(crate) fn compare_official_oauth_identity(
    identity: &CodexOfficialOAuthIdentity,
    account: &CodexAccount,
) -> CodexOfficialOAuthIdentityMatch {
    let account_id_pair = (
        account.account_id.as_deref(),
        identity.account_id.as_deref(),
    );
    let user_id_pair = (account.user_id.as_deref(), identity.user_id.as_deref());
    let mut primary_identity_matched = false;

    for (expected, observed) in [account_id_pair, user_id_pair] {
        if let (Some(expected), Some(observed)) = (expected, observed) {
            if !expected.trim().eq_ignore_ascii_case(observed.trim()) {
                return CodexOfficialOAuthIdentityMatch::Mismatched;
            }
            primary_identity_matched = true;
        }
    }

    if let (Some(expected), Some(observed)) = (
        account.organization_id.as_deref(),
        identity.organization_id.as_deref(),
    ) {
        if !expected.trim().eq_ignore_ascii_case(observed.trim()) {
            return CodexOfficialOAuthIdentityMatch::Mismatched;
        }
    }

    if primary_identity_matched {
        return CodexOfficialOAuthIdentityMatch::Matched;
    }

    let has_one_sided_primary_identity =
        matches!(account_id_pair, (Some(_), None) | (None, Some(_)))
            || matches!(user_id_pair, (Some(_), None) | (None, Some(_)));
    if has_one_sided_primary_identity {
        return CodexOfficialOAuthIdentityMatch::Unknown;
    }

    if account.email.eq_ignore_ascii_case(&identity.email) {
        CodexOfficialOAuthIdentityMatch::Matched
    } else {
        CodexOfficialOAuthIdentityMatch::Mismatched
    }
}

fn read_codex_auth_file_from_dir(base_dir: &Path) -> Option<CodexAuthFile> {
    let auth_path = base_dir.join("auth.json");
    if !auth_path.exists() {
        return None;
    }

    let content = fs::read_to_string(&auth_path).ok()?;
    serde_json::from_str(&content).ok()
}

fn load_local_oauth_snapshot_from_auth_file(
    auth_file: CodexAuthFile,
) -> Option<LocalCodexOAuthSnapshot> {
    if is_auth_mode_apikey(auth_file.auth_mode.as_deref()) {
        return None;
    }

    let last_refresh_at = parse_auth_file_last_refresh(auth_file.last_refresh.as_ref());
    let mut snapshot = build_local_oauth_snapshot(auth_file.tokens?)?;
    snapshot.last_refresh_at = last_refresh_at;
    Some(snapshot)
}

#[cfg(all(target_os = "macos", not(test)))]
fn is_codex_keychain_item_not_found(status: std::process::ExitStatus, stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    status.code() == Some(44)
        || lower.contains("could not be found")
        || lower.contains("errsecitemnotfound")
        || lower.contains("specified item could not be found")
}

#[cfg(all(target_os = "macos", not(test)))]
fn read_codex_keychain_auth_file_from_dir(
    base_dir: &Path,
) -> Result<Option<CodexAuthFile>, String> {
    let keychain_account = build_codex_keychain_account(base_dir);
    let output = std::process::Command::new("security")
        .arg("find-generic-password")
        .arg("-s")
        .arg(CODEX_KEYCHAIN_SERVICE)
        .arg("-a")
        .arg(&keychain_account)
        .arg("-w")
        .output()
        .map_err(|e| format!("执行 security 命令失败: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if is_codex_keychain_item_not_found(output.status, &stderr) {
            return Ok(None);
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!(
            "读取 Codex keychain 失败: status={}, stderr={}, stdout={}",
            output.status,
            if stderr.trim().is_empty() {
                "<empty>"
            } else {
                stderr.trim()
            },
            if stdout.trim().is_empty() {
                "<empty>"
            } else {
                stdout.trim()
            }
        ));
    }

    let secret = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if secret.is_empty() {
        return Ok(None);
    }

    let auth_file: CodexAuthFile = serde_json::from_str(&secret)
        .map_err(|e| format!("解析 Codex keychain JSON 失败: {}", e))?;
    Ok(Some(auth_file))
}

#[cfg(all(target_os = "macos", test))]
fn read_codex_keychain_auth_file_from_dir(
    _base_dir: &Path,
) -> Result<Option<CodexAuthFile>, String> {
    Ok(None)
}

#[cfg(not(target_os = "macos"))]
fn read_codex_keychain_auth_file_from_dir(
    _base_dir: &Path,
) -> Result<Option<CodexAuthFile>, String> {
    Ok(None)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexAuthCredentialsStoreMode {
    File,
    Keyring,
    Auto,
}

fn codex_auth_credentials_store_mode(base_dir: &Path) -> CodexAuthCredentialsStoreMode {
    let config_path = get_config_toml_path(base_dir);
    let Ok(content) = fs::read_to_string(config_path) else {
        return CodexAuthCredentialsStoreMode::File;
    };
    let Ok(doc) = crate::modules::codex_config_format::read_codex_config_doc_from_str(&content)
    else {
        return CodexAuthCredentialsStoreMode::File;
    };

    match doc
        .get(CODEX_CONFIG_CLI_AUTH_CREDENTIALS_STORE_KEY)
        .and_then(|item| item.as_str())
        .map(|value| value.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("keyring") => CodexAuthCredentialsStoreMode::Keyring,
        Some("auto") => CodexAuthCredentialsStoreMode::Auto,
        _ => CodexAuthCredentialsStoreMode::File,
    }
}

fn cli_auth_credentials_store_prefers_keychain(base_dir: &Path) -> bool {
    matches!(
        codex_auth_credentials_store_mode(base_dir),
        CodexAuthCredentialsStoreMode::Keyring | CodexAuthCredentialsStoreMode::Auto
    )
}

fn load_local_oauth_snapshot_from_official_store_with_keychain_reader<F>(
    base_dir: &Path,
    read_keychain: F,
) -> Option<LocalCodexOAuthSnapshot>
where
    F: FnOnce(&Path) -> Result<Option<CodexAuthFile>, String>,
{
    let auth_json = read_codex_auth_file_from_dir(base_dir);
    if auth_json
        .as_ref()
        .map(|auth_file| is_auth_mode_apikey(auth_file.auth_mode.as_deref()))
        .unwrap_or(false)
    {
        return None;
    }

    let auth_json_snapshot = auth_json.and_then(load_local_oauth_snapshot_from_auth_file);
    let prefers_keychain = cli_auth_credentials_store_prefers_keychain(base_dir);
    if !prefers_keychain && auth_json_snapshot.is_some() {
        return auth_json_snapshot;
    }

    match read_keychain(base_dir) {
        Ok(Some(auth_file)) => {
            if let Some(snapshot) = load_local_oauth_snapshot_from_auth_file(auth_file) {
                return Some(snapshot);
            }
        }
        Ok(None) => {}
        Err(err) => {
            logger::log_warn(&format!(
                "读取 Codex 官方 keychain 凭证失败，回退读取 auth.json: target_dir={}, error={}",
                base_dir.display(),
                err
            ));
        }
    }

    auth_json_snapshot
}

fn load_local_oauth_snapshot_from_official_store(
    base_dir: &Path,
) -> Option<LocalCodexOAuthSnapshot> {
    load_local_oauth_snapshot_from_official_store_with_keychain_reader(
        base_dir,
        read_codex_keychain_auth_file_from_dir,
    )
}

/// 读取官方 app-server 登录完成后写入的 OAuth 凭据。
/// 官方存储（auth.json 或 Keychain）是认证权威源，账号库只保存管理索引。
pub(crate) fn read_official_oauth_tokens(base_dir: &Path) -> Option<CodexTokens> {
    load_local_oauth_snapshot_from_official_store(base_dir).map(|snapshot| snapshot.tokens)
}

fn local_oauth_snapshot_matches_account(
    snapshot: &LocalCodexOAuthSnapshot,
    account: &CodexAccount,
) -> bool {
    if !account.email.eq_ignore_ascii_case(&snapshot.email) {
        return false;
    }

    let expected_id = build_account_storage_id(
        &snapshot.email,
        snapshot.account_id.as_deref(),
        snapshot.organization_id.as_deref(),
    );
    if account.id == expected_id {
        return true;
    }

    if let Some(account_id) = snapshot.account_id.as_deref() {
        if normalize_optional_ref(account.account_id.as_deref()).as_deref() != Some(account_id) {
            return false;
        }
    }

    if let Some(organization_id) = snapshot.organization_id.as_deref() {
        if normalize_optional_ref(account.organization_id.as_deref()).as_deref()
            != Some(organization_id)
        {
            return false;
        }
    }

    true
}

fn apply_local_oauth_snapshot(
    account: &mut CodexAccount,
    snapshot: &LocalCodexOAuthSnapshot,
) -> bool {
    let mut changed = false;
    let mut token_changed = false;

    if account.tokens.id_token != snapshot.tokens.id_token {
        account.tokens.id_token = snapshot.tokens.id_token.clone();
        changed = true;
        token_changed = true;
    }

    if account.tokens.access_token != snapshot.tokens.access_token {
        account.tokens.access_token = snapshot.tokens.access_token.clone();
        changed = true;
        token_changed = true;
    }

    if let Some(refresh_token) = normalize_optional_ref(snapshot.tokens.refresh_token.as_deref()) {
        if account.tokens.refresh_token.as_deref() != Some(refresh_token.as_str()) {
            account.tokens.refresh_token = Some(refresh_token);
            changed = true;
            token_changed = true;
        }
    }

    if normalize_optional_ref(account.account_id.as_deref()) != snapshot.account_id {
        account.account_id = snapshot.account_id.clone();
        changed = true;
    }

    if normalize_optional_ref(account.organization_id.as_deref()) != snapshot.organization_id {
        account.organization_id = snapshot.organization_id.clone();
        changed = true;
    }

    if normalize_optional_ref(account.subscription_active_until.as_deref())
        != snapshot.subscription_active_until
    {
        account.subscription_active_until = snapshot.subscription_active_until.clone();
        changed = true;
    }

    if token_changed {
        mark_token_chain_updated(account);
    }

    changed
}

fn local_oauth_snapshot_has_token_delta(
    account: &CodexAccount,
    snapshot: &LocalCodexOAuthSnapshot,
) -> bool {
    account.tokens.id_token != snapshot.tokens.id_token
        || account.tokens.access_token != snapshot.tokens.access_token
        || normalize_optional_ref(account.tokens.refresh_token.as_deref())
            != normalize_optional_ref(snapshot.tokens.refresh_token.as_deref())
}

fn authority_snapshot_has_older_access_token(
    account: &CodexAccount,
    snapshot: &LocalCodexOAuthSnapshot,
) -> bool {
    let Some(account_exp) =
        codex_oauth::jwt_token_expiration_timestamp(&account.tokens.access_token)
    else {
        return false;
    };
    let Some(snapshot_exp) =
        codex_oauth::jwt_token_expiration_timestamp(&snapshot.tokens.access_token)
    else {
        return false;
    };
    snapshot_exp < account_exp
}

fn should_accept_authority_snapshot(
    account: &CodexAccount,
    snapshot: &LocalCodexOAuthSnapshot,
) -> bool {
    if !local_oauth_snapshot_has_token_delta(account, snapshot) {
        return false;
    }

    // `last_refresh` 由官方 auth.json 提供，不能单独证明 Token 链更新了。
    // 某些旧文件会在凭据未轮换时刷新这个时间戳；如果 snapshot 的 JWT
    // access_token 明确比账号库里的 Token 更早过期，禁止回写覆盖新凭据。
    if authority_snapshot_has_older_access_token(account, snapshot) {
        return false;
    }

    let account_updated_at = account.token_updated_at.unwrap_or(0);
    if snapshot
        .last_refresh_at
        .map(|value| value >= account_updated_at)
        .unwrap_or(false)
    {
        return true;
    }

    managed_account_tokens_need_refresh(account)
        && !codex_oauth::is_token_expired(&snapshot.tokens.access_token)
}

fn should_accept_managed_authority_snapshot(
    account: &CodexAccount,
    snapshot: &LocalCodexOAuthSnapshot,
    base_dir: &Path,
) -> bool {
    if authority_snapshot_has_older_access_token(account, snapshot) {
        return false;
    }
    if should_accept_authority_snapshot(account, snapshot) {
        return true;
    }
    if !local_oauth_snapshot_has_token_delta(account, snapshot) {
        return false;
    }

    let Some(projection) = read_managed_projection_from_dir(base_dir) else {
        return false;
    };
    let projection_is_not_older = projection.written_at >= account.token_updated_at.unwrap_or(0);
    if let Some(credential_account_id) = projection.credential_account_id.as_deref() {
        return credential_account_id == account.id
            && (projection.credential_token_generation == Some(account.token_generation)
                || projection_is_not_older);
    }
    if projection.account_id == account.id {
        return projection.token_generation == account.token_generation || projection_is_not_older;
    }

    // v1 的 API Key + OAuth 组合投影只记录 API Key 账号。只有确认它确实是
    // API Key 配置，且该目录的写入时间不早于账号库 Token 时，才把身份匹配的
    // auth.json/keychain 视为同一轮 RT 链产生的新凭据。
    load_account(&projection.account_id)
        .is_some_and(|runtime_account| runtime_account.is_api_key_auth())
        && projection_is_not_older
}

fn sync_account_from_authority_dir_if_current(
    account: &mut CodexAccount,
    base_dir: &Path,
) -> Result<bool, String> {
    let Some(snapshot) = load_local_oauth_snapshot_from_official_store(base_dir) else {
        crate::modules::codex_auth_diagnostic::log_event(
            "authority_snapshot_missing",
            serde_json::json!({
                "account_id": account.id,
                "source_dir": base_dir.display().to_string(),
            }),
        );
        return Ok(false);
    };

    if !local_oauth_snapshot_matches_account(&snapshot, account) {
        crate::modules::codex_auth_diagnostic::log_event(
            "authority_snapshot_account_mismatch",
            serde_json::json!({
                "account_id": account.id,
                "source_dir": base_dir.display().to_string(),
                "snapshot_account_id": snapshot.account_id,
                "snapshot_email": snapshot.email,
                "snapshot_last_refresh_at": snapshot.last_refresh_at,
                "tokens": crate::modules::codex_auth_diagnostic::tokens_summary(&snapshot.tokens),
            }),
        );
        return Ok(false);
    }

    if !should_accept_managed_authority_snapshot(account, &snapshot, base_dir) {
        crate::modules::codex_auth_diagnostic::log_event(
            "authority_snapshot_rejected_as_older",
            serde_json::json!({
                "account_id": account.id,
                "source_dir": base_dir.display().to_string(),
                "account_token_generation": account.token_generation,
                "account_token_updated_at": account.token_updated_at,
                "snapshot_last_refresh_at": snapshot.last_refresh_at,
                "tokens": crate::modules::codex_auth_diagnostic::tokens_summary(&snapshot.tokens),
            }),
        );
        persist_managed_projection_credential_owner_best_effort(
            base_dir,
            account,
            "authority-snapshot-current",
        );
        return Ok(false);
    }

    if apply_local_oauth_snapshot(account, &snapshot) {
        save_account(account)?;
        persist_managed_projection_credential_owner_best_effort(
            base_dir,
            account,
            "authority-snapshot-updated",
        );
        logger::log_info(&format!(
            "Codex 账号刷新前已采用更近的官方凭证: account_id={}, source_dir={}, last_refresh_at={}",
            account.id,
            base_dir.display(),
            snapshot
                .last_refresh_at
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string())
        ));
        crate::modules::codex_auth_diagnostic::log_event(
            "authority_snapshot_applied",
            serde_json::json!({
                "account_id": account.id,
                "source_dir": base_dir.display().to_string(),
                "token_generation": account.token_generation,
                "last_refresh_at": snapshot.last_refresh_at,
                "tokens": crate::modules::codex_auth_diagnostic::tokens_summary(&account.tokens),
            }),
        );
        return Ok(true);
    }

    Ok(false)
}

fn local_oauth_snapshot_freshness_key(snapshot: &LocalCodexOAuthSnapshot) -> (i64, i64, i64) {
    (
        codex_oauth::jwt_token_expiration_timestamp(&snapshot.tokens.access_token).unwrap_or(0),
        snapshot.last_refresh_at.unwrap_or(0),
        codex_oauth::jwt_token_expiration_timestamp(&snapshot.tokens.id_token).unwrap_or(0),
    )
}

pub(crate) fn sync_account_from_runtime_authority_dirs(
    account_id: &str,
    runtime_dirs: &[PathBuf],
) -> Result<bool, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    let mut candidates = runtime_dirs
        .iter()
        .filter_map(|dir| {
            let snapshot = load_local_oauth_snapshot_from_official_store(dir)?;
            local_oauth_snapshot_matches_account(&snapshot, &account).then_some((dir, snapshot))
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|(_, snapshot)| local_oauth_snapshot_freshness_key(snapshot));
    let Some((source_dir, snapshot)) = candidates.pop() else {
        return Ok(false);
    };

    let stored_access_exp =
        codex_oauth::jwt_token_expiration_timestamp(&account.tokens.access_token).unwrap_or(0);
    let snapshot_access_exp =
        codex_oauth::jwt_token_expiration_timestamp(&snapshot.tokens.access_token).unwrap_or(0);
    if !should_accept_managed_authority_snapshot(&account, &snapshot, source_dir)
        && snapshot_access_exp <= stored_access_exp
    {
        return Ok(false);
    }

    if !apply_local_oauth_snapshot(&mut account, &snapshot) {
        persist_managed_projection_credential_owner_best_effort(
            source_dir,
            &account,
            "runtime-transfer-current",
        );
        return Ok(false);
    }
    save_account(&account)?;
    persist_managed_projection_credential_owner_best_effort(
        source_dir,
        &account,
        "runtime-transfer-updated",
    );
    crate::modules::codex_local_access::sync_sidecar_auth_file_for_account(&account)?;
    logger::log_info(&format!(
        "Codex 已从多个运行态 profile 中采用最新凭证并写回账号库: account_id={}, source_dir={}",
        account.id,
        source_dir.display()
    ));
    Ok(true)
}

fn sync_account_from_authority_sources(account: &mut CodexAccount) -> Result<bool, String> {
    let process_entries = crate::modules::process::collect_codex_process_entries();
    sync_account_from_authority_sources_with_entries(account, &process_entries)
}

fn sync_account_from_authority_sources_with_entries(
    account: &mut CodexAccount,
    process_entries: &[(u32, Option<String>)],
) -> Result<bool, String> {
    let mut dirs = vec![get_codex_home()];
    dirs.extend(authority_projection_dirs_for_account_with_entries(
        account,
        process_entries,
    ));

    let mut seen = HashSet::new();
    dirs.retain(|dir| seen.insert(dir.to_string_lossy().to_string()));

    let mut changed = false;
    for dir in dirs {
        if sync_account_from_authority_dir_if_current(account, &dir)? {
            changed = true;
        }
    }
    Ok(changed)
}

fn sync_account_from_live_authority_sources(account: &mut CodexAccount) -> Result<bool, String> {
    let process_entries = crate::modules::process::collect_codex_process_entries();
    sync_account_from_live_authority_sources_with_entries(account, &process_entries)
}

fn sync_account_from_live_authority_sources_with_entries(
    account: &mut CodexAccount,
    process_entries: &[(u32, Option<String>)],
) -> Result<bool, String> {
    let default_home = get_codex_home();
    let mut dirs = process_entries
        .iter()
        .map(|(_, runtime_home)| {
            runtime_home
                .as_deref()
                .map(PathBuf::from)
                .unwrap_or_else(|| default_home.clone())
        })
        .collect::<Vec<_>>();
    let mut seen = HashSet::new();
    dirs.retain(|dir| seen.insert(dir.to_string_lossy().to_string()));
    let changed = sync_account_from_runtime_authority_dirs(&account.id, &dirs)?;
    if changed {
        let account_id = account.id.clone();
        *account = load_account(&account_id)
            .ok_or_else(|| format!("同步运行态凭据后账号不存在: {}", account_id))?;
        logger::log_info(&format!(
            "Codex 已采用全部官方运行态中的最新 bearer token: account_id={}",
            account.id
        ));
    }
    Ok(changed)
}

async fn sync_active_official_account_before_switch() -> Result<bool, String> {
    let Some(current_account_id) = load_account_index().current_account_id else {
        return Ok(false);
    };
    let Some(current_account) = load_account(&current_account_id) else {
        return Ok(false);
    };

    let oauth_account_id = if current_account.is_api_key_auth() {
        let Some(bound_oauth_account_id) =
            normalize_optional_ref(current_account.bound_oauth_account_id.as_deref())
        else {
            return Ok(false);
        };
        bound_oauth_account_id
    } else {
        current_account_id
    };
    let Some(mut oauth_account) = load_account(&oauth_account_id) else {
        return Ok(false);
    };
    if oauth_account.is_api_key_auth()
        || oauth_account.is_agent_identity_auth()
        || oauth_account.is_web_session_auth()
    {
        return Ok(false);
    }

    let lock = codex_token_lock_for(&oauth_account_id);
    let _guard = lock.lock().await;
    let _file_guard =
        acquire_codex_token_refresh_file_lock(&oauth_account_id, "switch-current").await?;
    let changed = sync_account_from_live_authority_sources(&mut oauth_account)?;
    if changed {
        logger::log_info(&format!(
            "[Codex切号] 覆盖前已从全部运行态 profile 保存最新官方凭证: account_id={}",
            oauth_account.id
        ));
    }
    Ok(changed)
}

fn sync_account_from_auth_dir_if_current(
    account: &mut CodexAccount,
    base_dir: &Path,
) -> Result<bool, String> {
    let Some(snapshot) = load_local_oauth_snapshot_from_official_store(base_dir) else {
        return Ok(false);
    };

    if !local_oauth_snapshot_matches_account(&snapshot, account) {
        return Ok(false);
    }

    if apply_local_oauth_snapshot(account, &snapshot) {
        save_account(account)?;
        logger::log_info(&format!(
            "Codex 账号已从官方凭证源同步最新 Token: account_id={}, source_dir={}",
            account.id,
            base_dir.display()
        ));
    }
    persist_managed_projection_credential_owner_best_effort(
        base_dir,
        account,
        "explicit-auth-sync",
    );

    Ok(true)
}

/// 显式导入/同步入口：只在用户主动选择从官方目录回读时使用，业务主路径禁止自动调用。
pub fn sync_current_official_account_from_dir(
    base_dir: &Path,
) -> Result<Option<CodexAccount>, String> {
    let Some(snapshot) = load_local_oauth_snapshot_from_official_store(base_dir) else {
        return Ok(None);
    };

    for mut account in list_accounts() {
        if account.is_api_key_auth() {
            continue;
        }
        if !local_oauth_snapshot_matches_account(&snapshot, &account) {
            continue;
        }

        if apply_local_oauth_snapshot(&mut account, &snapshot) {
            save_account(&account)?;
            logger::log_info(&format!(
                "Codex 当前官方凭证已同步回账号库: account_id={}, source_dir={}",
                account.id,
                base_dir.display()
            ));
        }
        persist_managed_projection_credential_owner_best_effort(
            base_dir,
            &account,
            "official-account-import",
        );
        return Ok(Some(account));
    }

    Ok(None)
}

/// 显式导入/同步入口：只在用户主动选择从指定目录回读时使用，业务主路径禁止自动调用。
pub fn sync_account_from_auth_dir(
    account_id: &str,
    base_dir: &Path,
) -> Result<CodexAccount, String> {
    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if account.is_api_key_auth() || account.is_agent_identity_auth() {
        return Ok(account);
    }

    let _ = sync_account_from_auth_dir_if_current(&mut account, base_dir)?;
    Ok(account)
}

pub fn sync_managed_projection_from_auth_dir(
    account_id: &str,
    base_dir: &Path,
) -> Result<CodexAccount, String> {
    let mut projection = read_managed_projection_from_dir(base_dir)
        .ok_or_else(|| "目标目录不是 Cockpit 受管 Codex 投影，已拒绝反向同步".to_string())?;

    let mut account =
        load_account(account_id).ok_or_else(|| format!("账号不存在: {}", account_id))?;
    if account.is_api_key_auth() || account.is_agent_identity_auth() {
        return Ok(account);
    }

    let snapshot = load_local_oauth_snapshot_from_official_store(base_dir)
        .ok_or_else(|| "受管投影缺少可同步的 OAuth Token".to_string())?;
    if !local_oauth_snapshot_matches_account(&snapshot, &account) {
        return Err("受管投影 Token 与账号不匹配，已拒绝反向同步".to_string());
    }

    if let Some(credential_account_id) = projection.credential_account_id.as_deref() {
        if credential_account_id != account_id {
            return Err(format!(
                "受管投影凭据账号不匹配: expected={}, actual={}",
                account_id, credential_account_id
            ));
        }
    } else if projection.account_id == account_id {
        // v1 普通 OAuth 投影只有 account_id/token_generation。
        if account.token_generation != projection.token_generation {
            return Err(format!(
                "受管投影版本已过期，跳过反向同步: account_id={}, store_generation={}, projection_generation={}",
                account_id, account.token_generation, projection.token_generation
            ));
        }
    }

    if let Some(projection_generation) = projection.credential_token_generation {
        if account.token_generation != projection_generation {
            return Err(format!(
                "受管投影凭据版本已过期，跳过反向同步: account_id={}, store_generation={}, projection_generation={}",
                account_id, account.token_generation, projection_generation
            ));
        }
    }

    let token_changed = apply_local_oauth_snapshot(&mut account, &snapshot);
    if token_changed {
        save_account(&account)?;
    }

    let projection_owner_changed = projection.version < CODEX_AUTH_PROJECTION_VERSION
        || projection.credential_account_id.as_deref() != Some(account.id.as_str())
        || projection.credential_email.as_deref() != Some(account.email.as_str())
        || projection.credential_token_generation != Some(account.token_generation);
    if projection_owner_changed {
        projection.version = CODEX_AUTH_PROJECTION_VERSION;
        projection.credential_account_id = Some(account.id.clone());
        projection.credential_email = Some(account.email.clone());
        projection.credential_token_generation = Some(account.token_generation);
        projection.written_at = now_timestamp();
        write_managed_projection_value_to_dir(base_dir, &projection)?;
    }

    if token_changed {
        // 最新凭据只写回 Cockpit 账号库及 API Service sidecar；其它官方 profile
        // 保留当前运行态，并在下次显式启动/切换时投影最新凭据。
        sync_managed_account_sidecar(&account);
        logger::log_info(&format!(
            "Codex 受管投影已同步回账号库: account_id={}, generation={}, source_dir={}",
            account.id,
            account.token_generation,
            base_dir.display()
        ));
    }

    Ok(account)
}

/// Local API Service / loopback client URLs must not overwrite a stored real upstream.
fn is_loopback_or_local_gateway_base_url(raw: Option<&str>) -> bool {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };
    let Ok(parsed) = reqwest::Url::parse(raw) else {
        return false;
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    let host = parsed
        .host_str()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    matches!(
        host.as_str(),
        "localhost" | "127.0.0.1" | "0.0.0.0" | "::1" | "[::1]"
    )
}

fn is_loopback_http_base_url(raw: Option<&str>) -> bool {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };
    let Ok(parsed) = reqwest::Url::parse(raw) else {
        return false;
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    match parsed.host() {
        Some(url::Host::Ipv4(addr)) => addr.is_loopback(),
        Some(url::Host::Ipv6(addr)) => addr.is_loopback(),
        Some(url::Host::Domain(host)) => {
            host.eq_ignore_ascii_case("localhost") || host.eq_ignore_ascii_case("localhost.")
        }
        None => false,
    }
}

fn sync_api_key_account_from_local_state(account: &mut CodexAccount, base_dir: &Path) {
    let auth_path = base_dir.join("auth.json");
    if !auth_path.exists() || !account.is_api_key_auth() {
        return;
    }

    let Ok(content) = fs::read_to_string(&auth_path) else {
        return;
    };
    let Ok(auth_file) = serde_json::from_str::<CodexAuthFile>(&content) else {
        return;
    };
    let is_apikey_mode = is_auth_mode_apikey(auth_file.auth_mode.as_deref());
    let local_api_key = extract_api_key_from_auth_file(&auth_file);
    if !(is_apikey_mode || (auth_file.tokens.is_none() && local_api_key.is_some())) {
        return;
    }

    let Some(local_api_key) = normalize_optional_ref(local_api_key.as_deref()) else {
        return;
    };
    let Some(account_api_key) = normalize_optional_ref(account.openai_api_key.as_deref()) else {
        return;
    };
    if local_api_key != account_api_key {
        return;
    }

    let config_provider = read_api_provider_from_config_toml(base_dir);
    // Local access / provider gateway profiles rewrite client base_url to loopback.
    // Never treat that runtime endpoint as the account's real upstream provider URL,
    // or sidecar codex-api-key base-url will form a self-proxy loop after switch.
    let using_runtime_local_provider = config_provider.provider_id.as_deref()
        == Some(CODEX_RUNTIME_MODEL_PROVIDER_ID)
        || is_loopback_http_base_url(config_provider.base_url.as_deref());
    if using_runtime_local_provider {
        return;
    }

    let resolved_base_url = extract_api_base_url_from_auth_file(&auth_file)
        .or_else(|| config_provider.base_url.clone());
    if is_loopback_http_base_url(resolved_base_url.as_deref()) {
        return;
    }
    let account_provider = infer_api_provider_config(
        account.api_base_url.as_deref(),
        Some(account.api_provider_mode.clone()),
        account.api_provider_id.as_deref(),
        account.api_provider_name.as_deref(),
    );
    let preserve_account_provider_identity = should_preserve_account_provider_identity(
        &account_provider,
        &config_provider,
        resolved_base_url.as_deref(),
    );
    let provider_mode = if preserve_account_provider_identity {
        account.api_provider_mode.clone()
    } else {
        config_provider.mode.clone()
    };
    let provider_id = if preserve_account_provider_identity {
        account.api_provider_id.as_deref()
    } else {
        config_provider.provider_id.as_deref()
    };
    let provider_name = if preserve_account_provider_identity {
        account.api_provider_name.as_deref()
    } else {
        config_provider.provider_name.as_deref()
    };
    let current_provider = infer_api_provider_config(
        resolved_base_url.as_deref(),
        Some(provider_mode),
        provider_id,
        provider_name,
    );

    if account_provider == current_provider {
        return;
    }

    // Profile after local API attach uses localhost as the *client* Base URL.
    // Never write that back as the account's real upstream (breaks sidecar).
    if is_loopback_or_local_gateway_base_url(current_provider.base_url.as_deref()) {
        return;
    }

    account.api_base_url = current_provider.base_url.clone();
    account.api_provider_mode = current_provider.mode.clone();
    account.api_provider_id = current_provider.provider_id.clone();
    account.api_provider_name = current_provider.provider_name.clone();
    let _ = save_account(account);
}
