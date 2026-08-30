// Codex 账号模块：Local import, token candidate parsing and batch import workflow。
// 通过 include! 保持原 modules::codex_account 作用域，完整保留私有调用关系。
/// 从官方 Codex 本机凭据存储导入账号（auth.json / macOS Keychain）
pub fn import_from_local() -> Result<CodexAccount, String> {
    let codex_home = get_codex_home();
    let auth_path = codex_home.join("auth.json");
    let content = fs::read_to_string(&auth_path).ok();
    let raw_value = content
        .as_deref()
        .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok());

    // API Key / Agent Identity / personal access token 仍按 auth.json 的原有格式处理。
    // OAuth 则必须走官方统一凭据存储，不能因为 auth.json 存在就绕过 Keychain。
    if let Some(raw_value) = raw_value.as_ref() {
        if let Some(identity) = parse_agent_identity_from_value(raw_value)? {
            return upsert_agent_identity_account(identity);
        }
    }

    let auth_file = content
        .as_deref()
        .and_then(|value| serde_json::from_str::<CodexAuthFile>(value).ok());
    let Some(auth_file) = auth_file.as_ref() else {
        let snapshot = load_local_oauth_snapshot_from_official_store(&codex_home)
            .ok_or_else(|| format!("未找到可导入的官方 Codex 凭据: {}", codex_home.display()))?;
        let account = upsert_account_with_import_hints(
            snapshot.tokens,
            snapshot.account_id,
            snapshot.organization_id,
            snapshot.subscription_active_until,
        )?;
        logger::log_info(&format!(
            "Codex 本机导入已采用官方凭据存储: account_id={}, home={}",
            account.id,
            codex_home.display()
        ));
        return Ok(account);
    };

    let fallback_api_key = extract_api_key_from_auth_file(&auth_file);
    let config_provider = read_api_provider_from_config_toml(&codex_home);
    let fallback_provider = infer_api_provider_config(
        extract_api_base_url_from_auth_file(&auth_file)
            .or_else(|| config_provider.base_url.clone())
            .as_deref(),
        Some(config_provider.mode.clone()),
        config_provider.provider_id.as_deref(),
        config_provider.provider_name.as_deref(),
    );

    if is_auth_mode_apikey(auth_file.auth_mode.as_deref()) {
        let api_key = fallback_api_key.ok_or("auth.json 缺少 OPENAI_API_KEY")?;
        return upsert_api_key_account(
            api_key,
            fallback_provider.base_url.clone(),
            Some(fallback_provider.mode),
            fallback_provider.provider_id.clone(),
            fallback_provider.provider_name.clone(),
            Vec::new(),
            Some(false),
            None,
            false,
            false,
            std::collections::HashMap::new(),
            None,
            None,
            None,
        );
    }

    if let Some(personal_access_token) =
        normalize_optional_ref(auth_file.personal_access_token.as_deref())
    {
        return upsert_account_from_access_token(personal_access_token, None);
    }

    if !is_auth_mode_apikey(auth_file.auth_mode.as_deref()) {
        if let Some(snapshot) = load_local_oauth_snapshot_from_official_store(&codex_home) {
            let account = upsert_account_with_import_hints(
                snapshot.tokens,
                snapshot.account_id,
                snapshot.organization_id,
                snapshot.subscription_active_until,
            )?;
            logger::log_info(&format!(
                "Codex 本机导入已采用官方凭据存储: account_id={}, home={}",
                account.id,
                codex_home.display()
            ));
            return Ok(account);
        }
    }

    if let Some(tokens) = auth_file.tokens.clone() {
        return upsert_account_from_auth_tokens(tokens);
    }

    if let Some(api_key) = fallback_api_key {
        return upsert_api_key_account(
            api_key,
            fallback_provider.base_url.clone(),
            Some(fallback_provider.mode),
            fallback_provider.provider_id.clone(),
            fallback_provider.provider_name.clone(),
            Vec::new(),
            Some(false),
            None,
            false,
            false,
            std::collections::HashMap::new(),
            None,
            None,
            None,
        );
    }

    Err(format!(
        "未找到可导入的官方 Codex 凭据: {}",
        auth_path.display()
    ))
}

fn import_account_struct(account: CodexAccount) -> Result<CodexAccount, String> {
    if let Some(identity) = account.agent_identity.clone() {
        let mut imported = upsert_agent_identity_account(identity)?;
        let mut changed = false;
        if let Some(tags) = account.tags {
            imported.tags = Some(tags);
            changed = true;
        }
        if let Some(note) = account.account_note {
            imported.account_note = Some(note);
            changed = true;
        }
        if changed {
            save_account(&imported)?;
        }
        return Ok(imported);
    }

    if is_pending_oauth_account(&account) {
        let mut imported = create_pending_oauth_account(
            account.email.clone(),
            codex_account_note_update_from_account(&account),
        )?;
        if let Some(tags) = account.tags {
            imported.tags = Some(tags);
            save_account(&imported)?;
        }
        return Ok(imported);
    }

    if account.is_api_key_auth() || account.openai_api_key.is_some() {
        let api_key = normalize_optional_ref(account.openai_api_key.as_deref())
            .ok_or("API Key 账号缺少 OPENAI_API_KEY")?;
        let mut api_acc = upsert_api_key_account(
            api_key,
            account.api_base_url.clone(),
            Some(account.api_provider_mode),
            account.api_provider_id.clone(),
            account.api_provider_name.clone(),
            account.api_model_catalog.clone(),
            Some(account.api_sync_model_catalog_to_codex),
            account.api_wire_api.clone(),
            account.api_supports_websockets,
            account.api_supports_vision,
            account.api_model_vision_support.clone(),
            account.api_vision_routing_model.clone(),
            account.account_name.clone(),
            Some(account.api_model_context_windows.clone()),
        )?;
        let mut changed = false;
        if let Some(tags) = account.tags {
            api_acc.tags = Some(tags);
            changed = true;
        }
        if let Some(note) = account.account_note {
            api_acc.account_note = Some(note);
            changed = true;
        }
        if let Some(secret) = account.two_factor_secret {
            api_acc.two_factor_secret = Some(secret);
            changed = true;
        }
        if let Some(password) = account.account_password {
            api_acc.account_password = Some(password);
            changed = true;
        }
        if let Some(phone_number) = account.phone_number {
            api_acc.phone_number = Some(phone_number);
            changed = true;
        }
        if let Some(mail_url) = account.mail_url {
            api_acc.mail_url = Some(mail_url);
            changed = true;
        }
        if changed {
            save_account(&api_acc)?;
        }
        return Ok(api_acc);
    }

    let imported_auth_file_plan_type =
        normalize_auth_file_plan_type(account.auth_file_plan_type.as_deref());
    let mut imported = upsert_account(account.tokens)?;
    let mut changed = apply_auth_file_plan_type(&mut imported, imported_auth_file_plan_type);

    if let Some(tags) = account.tags {
        imported.tags = Some(tags);
        changed = true;
    }
    if let Some(note) = account.account_note {
        imported.account_note = Some(note);
        changed = true;
    }
    if let Some(secret) = account.two_factor_secret {
        imported.two_factor_secret = Some(secret);
        changed = true;
    }
    if let Some(password) = account.account_password {
        imported.account_password = Some(password);
        changed = true;
    }
    if let Some(phone_number) = account.phone_number {
        imported.phone_number = Some(phone_number);
        changed = true;
    }
    if let Some(mail_url) = account.mail_url {
        imported.mail_url = Some(mail_url);
        changed = true;
    }

    if changed {
        save_account(&imported)?;
    }

    Ok(imported)
}

fn upsert_account_from_auth_tokens(tokens: CodexAuthTokens) -> Result<CodexAccount, String> {
    let account_id_hint = tokens.account_id.clone();
    let tokens = CodexTokens {
        id_token: tokens.id_token,
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
    };

    if normalize_optional_ref(Some(&tokens.id_token)).is_none()
        && is_importable_access_token(&tokens.access_token)
    {
        return upsert_account_from_access_token_with_hints(
            tokens.access_token,
            CodexAccessTokenImportHints {
                account_id: account_id_hint,
                ..Default::default()
            },
        );
    }

    upsert_account_with_hints(tokens, account_id_hint, None)
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
struct CodexAccessTokenImportHints {
    email: Option<String>,
    user_id: Option<String>,
    plan_type: Option<String>,
    subscription_active_until: Option<String>,
    account_id: Option<String>,
    organization_id: Option<String>,
    account_name: Option<String>,
    account_structure: Option<String>,
    account_note: Option<String>,
    two_factor_secret: Option<String>,
    account_password: Option<String>,
    phone_number: Option<String>,
    mail_url: Option<String>,
}

enum CodexJsonImportCandidate {
    FullToken {
        tokens: CodexTokens,
        account_id_hint: Option<String>,
        subscription_active_until_hint: Option<String>,
        note_update: CodexAccountNoteUpdate,
    },
    AccessToken {
        access_token: String,
        hints: CodexAccessTokenImportHints,
    },
    RefreshToken {
        refresh_token: String,
        note_update: CodexAccountNoteUpdate,
    },
}

fn codex_account_note_update_from_value(value: &serde_json::Value) -> CodexAccountNoteUpdate {
    CodexAccountNoteUpdate {
        note: read_json_string(
            value,
            &["account_note", "accountNote", "note", "notes", "remark"],
        ),
        two_factor_secret: read_json_string(
            value,
            &[
                "two_factor_secret",
                "twoFactorSecret",
                "account_two_factor_secret",
                "accountTwoFactorSecret",
            ],
        ),
        account_password: read_json_string(
            value,
            &["account_password", "accountPassword", "password"],
        ),
        phone_number: read_json_string(
            value,
            &[
                "phone_number",
                "phoneNumber",
                "account_phone_number",
                "accountPhoneNumber",
            ],
        ),
        mail_url: read_account_mail_url(value),
    }
}

fn has_codex_account_note_update(update: &CodexAccountNoteUpdate) -> bool {
    update.note.is_some()
        || update.two_factor_secret.is_some()
        || update.account_password.is_some()
        || update.phone_number.is_some()
        || update.mail_url.is_some()
}

fn merge_codex_account_note_update(
    mut primary: CodexAccountNoteUpdate,
    fallback: CodexAccountNoteUpdate,
) -> CodexAccountNoteUpdate {
    if primary.note.is_none() {
        primary.note = fallback.note;
    }
    if primary.two_factor_secret.is_none() {
        primary.two_factor_secret = fallback.two_factor_secret;
    }
    if primary.account_password.is_none() {
        primary.account_password = fallback.account_password;
    }
    if primary.phone_number.is_none() {
        primary.phone_number = fallback.phone_number;
    }
    if primary.mail_url.is_none() {
        primary.mail_url = fallback.mail_url;
    }
    primary
}

fn codex_account_note_update_from_hints(
    hints: &CodexAccessTokenImportHints,
) -> CodexAccountNoteUpdate {
    CodexAccountNoteUpdate {
        note: hints.account_note.clone(),
        two_factor_secret: hints.two_factor_secret.clone(),
        account_password: hints.account_password.clone(),
        phone_number: hints.phone_number.clone(),
        mail_url: hints.mail_url.clone(),
    }
}

fn apply_account_note_update_if_present(
    account: &mut CodexAccount,
    update: CodexAccountNoteUpdate,
) -> bool {
    if !has_codex_account_note_update(&update) {
        return false;
    }
    apply_account_note_update(account, update);
    true
}

fn save_account_note_update_if_present(
    account: &mut CodexAccount,
    update: CodexAccountNoteUpdate,
) -> Result<(), String> {
    if apply_account_note_update_if_present(account, update) {
        save_account(account)?;
    }
    Ok(())
}

fn is_blank_codex_token_fields(value: &serde_json::Value) -> bool {
    let id_token = first_json_string(
        value,
        &[&["id_token"], &["idToken"], &["tokens", "id_token"]],
    );
    let access_token = first_json_string(
        value,
        &[
            &["access_token"],
            &["accessToken"],
            &["tokens", "access_token"],
        ],
    );
    let refresh_token = first_json_string(
        value,
        &[
            &["refresh_token"],
            &["refreshToken"],
            &["tokens", "refresh_token"],
            &["tokens", "refreshToken"],
        ],
    );

    id_token.is_none() && access_token.is_none() && refresh_token.is_none()
}

fn pending_oauth_account_from_value(value: &serde_json::Value) -> Option<CodexAccount> {
    let obj = value.as_object()?;
    let auth_mode = read_json_string(value, &["auth_mode", "authMode"])
        .unwrap_or_else(|| "oauth".to_string())
        .to_ascii_lowercase();
    if auth_mode == "apikey" {
        return None;
    }

    let account_type = read_json_string(value, &["type"])
        .unwrap_or_default()
        .to_ascii_lowercase();
    let authorization_status =
        read_json_string(value, &["authorization_status", "authorizationStatus"])
            .unwrap_or_default()
            .to_ascii_lowercase();
    let update = codex_account_note_update_from_value(value);
    let has_pending_marker = authorization_status == CODEX_AUTHORIZATION_STATUS_PENDING
        || account_type == "codex"
        || has_codex_account_note_update(&update);

    if !has_pending_marker || !is_blank_codex_token_fields(value) {
        return None;
    }

    let email = read_json_string(value, &["email", "account_email", "accountEmail"])
        .or_else(|| read_json_string(value, &["account_name", "accountName"]))
        .filter(|value| !value.trim().is_empty())?;
    let account_id = build_account_storage_id(&email, Some("pending_oauth"), None);
    let now = now_timestamp();
    let mut account = CodexAccount::new(
        account_id,
        email,
        CodexTokens {
            id_token: String::new(),
            access_token: String::new(),
            refresh_token: None,
        },
    );
    account.auth_mode = CodexAuthMode::OAuth;
    account.authorization_status = Some(CODEX_AUTHORIZATION_STATUS_PENDING.to_string());
    account.token_updated_at = None;
    account.token_generation = 0;
    account.created_at = read_json_i64(value, &["created_at", "createdAt"]).unwrap_or(now);
    account.last_used =
        read_json_i64(value, &["last_used", "lastUsed"]).unwrap_or(account.created_at);
    apply_account_note_update(&mut account, update);
    account.tags = read_json_string_array(value, &["tags"]);

    // Treat a token-less Codex object as a saved draft only when it actually
    // carries pending metadata. This avoids silently importing malformed auth files.
    if authorization_status == CODEX_AUTHORIZATION_STATUS_PENDING
        || has_codex_account_note_details(&account)
        || obj.contains_key("account_note")
        || obj.contains_key("accountNote")
    {
        Some(account)
    } else {
        None
    }
}

fn has_codex_account_note_details(account: &CodexAccount) -> bool {
    account
        .account_note
        .as_deref()
        .and_then(|value| normalize_optional_ref(Some(value)))
        .is_some()
        || account
            .two_factor_secret
            .as_deref()
            .and_then(|value| normalize_optional_ref(Some(value)))
            .is_some()
        || account
            .account_password
            .as_deref()
            .and_then(|value| normalize_optional_ref(Some(value)))
            .is_some()
        || account
            .phone_number
            .as_deref()
            .and_then(|value| normalize_optional_ref(Some(value)))
            .is_some()
        || account
            .mail_url
            .as_deref()
            .and_then(|value| normalize_optional_ref(Some(value)))
            .is_some()
}

fn codex_account_note_update_from_account(account: &CodexAccount) -> CodexAccountNoteUpdate {
    CodexAccountNoteUpdate {
        note: account.account_note.clone(),
        two_factor_secret: account.two_factor_secret.clone(),
        account_password: account.account_password.clone(),
        phone_number: account.phone_number.clone(),
        mail_url: account.mail_url.clone(),
    }
}

fn is_opaque_access_token(token: &str) -> bool {
    normalize_optional_ref(Some(token))
        .map(|token| token.starts_with("at-"))
        .unwrap_or(false)
}

fn is_importable_access_token(token: &str) -> bool {
    decode_jwt_payload_value(token).is_some() || is_opaque_access_token(token)
}

fn extract_bearer_token_from_header(value: &str) -> Option<String> {
    let value = normalize_optional_ref(Some(value))?;
    let mut parts = value.split_whitespace();
    let scheme = parts.next()?;
    let token = parts.next()?;
    if parts.next().is_some() || !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = normalize_optional_ref(Some(token))?;
    is_importable_access_token(&token).then(|| token.to_string())
}

fn extract_opaque_access_token_from_text(value: &str) -> Option<String> {
    let value = normalize_optional_ref(Some(value))?;
    for (start, _) in value.match_indices("at-") {
        let token: String = value[start..]
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
            .collect();
        if is_opaque_access_token(&token) {
            return Some(token);
        }
    }
    None
}

fn first_json_scalar_string(value: &serde_json::Value, paths: &[&[&str]]) -> Option<String> {
    paths.iter().find_map(|path| {
        let mut current = value;
        for key in *path {
            current = current.get(*key)?;
        }
        normalize_optional_json_scalar(Some(current))
    })
}

fn merge_access_token_import_hints(
    mut primary: CodexAccessTokenImportHints,
    fallback: CodexAccessTokenImportHints,
) -> CodexAccessTokenImportHints {
    if primary.email.is_none() {
        primary.email = fallback.email;
    }
    if primary.user_id.is_none() {
        primary.user_id = fallback.user_id;
    }
    if primary.plan_type.is_none() {
        primary.plan_type = fallback.plan_type;
    }
    if primary.subscription_active_until.is_none() {
        primary.subscription_active_until = fallback.subscription_active_until;
    }
    if primary.account_id.is_none() {
        primary.account_id = fallback.account_id;
    }
    if primary.organization_id.is_none() {
        primary.organization_id = fallback.organization_id;
    }
    if primary.account_name.is_none() {
        primary.account_name = fallback.account_name;
    }
    if primary.account_structure.is_none() {
        primary.account_structure = fallback.account_structure;
    }
    if primary.account_note.is_none() {
        primary.account_note = fallback.account_note;
    }
    if primary.two_factor_secret.is_none() {
        primary.two_factor_secret = fallback.two_factor_secret;
    }
    if primary.account_password.is_none() {
        primary.account_password = fallback.account_password;
    }
    if primary.phone_number.is_none() {
        primary.phone_number = fallback.phone_number;
    }
    if primary.mail_url.is_none() {
        primary.mail_url = fallback.mail_url;
    }
    primary
}

fn first_explicit_personal_access_token_string(value: &serde_json::Value) -> Option<String> {
    first_json_scalar_string(
        value,
        &[
            &["personal_access_token"],
            &["personalAccessToken"],
            &["at_token"],
            &["atToken"],
            &["tokens", "personal_access_token"],
            &["tokens", "personalAccessToken"],
            &["tokens", "at_token"],
            &["tokens", "atToken"],
            &["credentials", "personal_access_token"],
            &["credentials", "personalAccessToken"],
            &["credentials", "at_token"],
            &["credentials", "atToken"],
        ],
    )
    .filter(|token| is_importable_access_token(token))
    .or_else(|| {
        first_json_scalar_string(
            value,
            &[
                &["headers", "authorization"],
                &["headers", "Authorization"],
                &["credentials", "headers", "authorization"],
                &["credentials", "headers", "Authorization"],
            ],
        )
        .and_then(|header| extract_bearer_token_from_header(&header))
    })
}

fn first_personal_access_token_string(value: &serde_json::Value) -> Option<String> {
    first_explicit_personal_access_token_string(value).or_else(|| {
        first_json_scalar_string(
            value,
            &[
                &["credentials", "access_token"],
                &["credentials", "accessToken"],
                &["access_token"],
                &["accessToken"],
            ],
        )
        .filter(|token| is_opaque_access_token(token))
    })
}

fn extract_access_token_import_hints_from_value(
    value: &serde_json::Value,
) -> CodexAccessTokenImportHints {
    let note_update = codex_account_note_update_from_value(value);
    CodexAccessTokenImportHints {
        email: first_json_scalar_string(
            value,
            &[
                &["email"],
                &["account_email"],
                &["accountEmail"],
                &["user", "email"],
                &["profile", "email"],
                &["account", "email"],
                &["credentials", "email"],
            ],
        ),
        user_id: first_json_scalar_string(
            value,
            &[
                &["user_id"],
                &["userId"],
                &["user", "id"],
                &["account", "user_id"],
                &["account", "userId"],
            ],
        ),
        plan_type: first_json_scalar_string(
            value,
            &[
                &["plan_type"],
                &["planType"],
                &["account", "plan_type"],
                &["account", "planType"],
                &["account", "plan"],
                &["credentials", "plan_type"],
                &["credentials", "planType"],
                &["credentials", "chatgpt_plan_type"],
            ],
        ),
        subscription_active_until: first_json_scalar_string(
            value,
            &[
                &["subscription_active_until"],
                &["subscriptionActiveUntil"],
                &["subscription_expires_at"],
                &["subscriptionExpiresAt"],
                &["account", "subscription_active_until"],
                &["account", "subscriptionActiveUntil"],
                &["account", "subscription_expires_at"],
                &["account", "subscriptionExpiresAt"],
                &["credentials", "subscription_active_until"],
                &["credentials", "subscriptionActiveUntil"],
                &["credentials", "subscription_expires_at"],
                &["credentials", "subscriptionExpiresAt"],
            ],
        ),
        account_id: first_json_scalar_string(
            value,
            &[
                &["account_id"],
                &["accountId"],
                &["chatgpt_account_id"],
                &["workspace_id"],
                &["chatgptAccountId"],
                &["workspaceId"],
                &["headers", "ChatGPT-Account-Id"],
                &["headers", "Chatgpt-Account-Id"],
                &["custom_headers", "ChatGPT-Account-Id"],
                &["customHeaders", "ChatGPT-Account-Id"],
                &["account", "id"],
                &["account", "account_id"],
                &["account", "accountId"],
                &["credentials", "account_id"],
                &["credentials", "accountId"],
                &["credentials", "chatgpt_account_id"],
                &["credentials", "workspace_id"],
            ],
        ),
        organization_id: first_json_scalar_string(
            value,
            &[
                &["organization_id"],
                &["organizationId"],
                &["org_id"],
                &["orgId"],
                &["poid"],
                &["POID"],
                &["account", "organization_id"],
                &["account", "organizationId"],
                &["account", "org_id"],
                &["account", "orgId"],
            ],
        ),
        account_name: first_json_scalar_string(
            value,
            &[
                &["account_name"],
                &["accountName"],
                &["name"],
                &["user", "name"],
                &["display_name"],
                &["account", "name"],
                &["account", "display_name"],
                &["account", "account_name"],
                &["account", "accountName"],
            ],
        ),
        account_structure: first_json_scalar_string(
            value,
            &[
                &["account_structure"],
                &["accountStructure"],
                &["structure"],
                &["account", "structure"],
                &["account", "account_structure"],
                &["account", "accountStructure"],
                &["account", "type"],
            ],
        ),
        account_note: note_update.note,
        two_factor_secret: note_update.two_factor_secret,
        account_password: note_update.account_password,
        phone_number: note_update.phone_number,
        mail_url: note_update.mail_url,
    }
}

fn is_codex_session_object(value: &serde_json::Value) -> bool {
    let Some(obj) = value.as_object() else {
        return false;
    };
    let has_access_token = first_json_string(value, &[&["accessToken"], &["access_token"]])
        .filter(|token| is_importable_access_token(token))
        .is_some();
    if !has_access_token {
        return false;
    }

    obj.get("user").and_then(|item| item.as_object()).is_some()
        || obj
            .get("account")
            .and_then(|item| item.as_object())
            .is_some()
        || obj.get("expires").is_some()
        || obj.get("sessionToken").is_some()
        || obj
            .get("authProvider")
            .and_then(|item| item.as_str())
            .map(|provider| provider.eq_ignore_ascii_case("openai"))
            .unwrap_or(false)
}

fn normalize_codex_session_value(
    value: &serde_json::Value,
    depth: usize,
) -> Option<serde_json::Value> {
    if depth > 4 {
        return None;
    }
    let obj = value.as_object()?;

    for key in ["session_json", "session"] {
        let Some(nested) = obj.get(key) else {
            continue;
        };
        match nested {
            serde_json::Value::Object(_) => {
                if let Some(session) = normalize_codex_session_value(nested, depth + 1) {
                    return Some(session);
                }
            }
            serde_json::Value::String(raw) => {
                let parsed = serde_json::from_str::<serde_json::Value>(raw).ok()?;
                if let Some(session) = normalize_codex_session_value(&parsed, depth + 1) {
                    return Some(session);
                }
            }
            _ => {}
        }
    }

    if is_codex_session_object(value) {
        return Some(value.clone());
    }

    None
}

fn mark_imported_web_session_account(mut account: CodexAccount) -> Result<CodexAccount, String> {
    if account.is_api_key_auth() || account.is_agent_identity_auth() {
        return Ok(account);
    }
    if account.token_source_mode.trim() != CODEX_TOKEN_SOURCE_WEB_SESSION {
        account.token_source_mode = CODEX_TOKEN_SOURCE_WEB_SESSION.to_string();
        save_account(&account)?;
    }
    Ok(account)
}

fn extract_codex_session_candidate_from_value(
    value: &serde_json::Value,
) -> Option<CodexJsonImportCandidate> {
    let session = normalize_codex_session_value(value, 0)?;
    let access_token = first_json_string(&session, &[&["accessToken"], &["access_token"]])
        .filter(|token| is_importable_access_token(token))?;
    let account_id_hint = first_json_string(&session, &[&["account", "id"], &["account_id"]]);
    let note_update = merge_codex_account_note_update(
        codex_account_note_update_from_value(value),
        codex_account_note_update_from_value(&session),
    );
    let mut session_hints = merge_access_token_import_hints(
        extract_access_token_import_hints_from_value(&session),
        extract_access_token_import_hints_from_value(value),
    );
    if session_hints.account_id.is_none() {
        session_hints.account_id = account_id_hint.clone();
    }
    let session_hints_note_update = codex_account_note_update_from_hints(&session_hints);
    let session_hints_note_update =
        merge_codex_account_note_update(session_hints_note_update, note_update.clone());
    session_hints.account_note = session_hints_note_update.note;
    session_hints.two_factor_secret = session_hints_note_update.two_factor_secret;
    session_hints.account_password = session_hints_note_update.account_password;
    session_hints.phone_number = session_hints_note_update.phone_number;
    session_hints.mail_url = session_hints_note_update.mail_url;

    if let Some(id_token) = first_json_string(&session, &[&["idToken"], &["id_token"]]) {
        let refresh_token = first_json_string(&session, &[&["refreshToken"], &["refresh_token"]]);
        return Some(CodexJsonImportCandidate::FullToken {
            tokens: CodexTokens {
                id_token,
                access_token,
                refresh_token,
            },
            account_id_hint,
            subscription_active_until_hint: session_hints.subscription_active_until.clone(),
            note_update,
        });
    }

    if decode_jwt_payload_value(&access_token).is_some() {
        let refresh_token = first_json_string(&session, &[&["refreshToken"], &["refresh_token"]]);
        return Some(CodexJsonImportCandidate::FullToken {
            tokens: CodexTokens {
                id_token: access_token.clone(),
                access_token,
                refresh_token,
            },
            account_id_hint,
            subscription_active_until_hint: session_hints.subscription_active_until.clone(),
            note_update,
        });
    }

    Some(CodexJsonImportCandidate::AccessToken {
        access_token,
        hints: session_hints,
    })
}

fn extract_refresh_token_only_from_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(raw) => normalize_optional_ref(Some(raw)).filter(|token| {
            decode_jwt_payload_value(token).is_none()
                && !is_opaque_access_token(token)
                && extract_opaque_access_token_from_text(raw).is_none()
        }),
        serde_json::Value::Object(_) => first_json_string(
            value,
            &[
                &["refresh_token"],
                &["refreshToken"],
                &["tokens", "refresh_token"],
                &["tokens", "refreshToken"],
            ],
        ),
        _ => None,
    }
}

fn extract_access_token_only_from_value(
    value: &serde_json::Value,
) -> Option<(String, CodexAccessTokenImportHints)> {
    match value {
        serde_json::Value::String(raw) => normalize_optional_ref(Some(raw))
            .filter(|token| is_importable_access_token(token))
            .or_else(|| extract_opaque_access_token_from_text(raw))
            .map(|token| (token, CodexAccessTokenImportHints::default())),
        serde_json::Value::Object(_) => first_personal_access_token_string(value)
            .or_else(|| {
                first_json_string(
                    value,
                    &[
                        &["tokens", "access_token"],
                        &["tokens", "accessToken"],
                        &["credentials", "access_token"],
                        &["credentials", "accessToken"],
                        &["access_token"],
                        &["accessToken"],
                        &["token"],
                    ],
                )
                .filter(|token| is_importable_access_token(token))
            })
            .map(|token| (token, extract_access_token_import_hints_from_value(value))),
        _ => None,
    }
}

fn extract_codex_import_candidate_from_value(
    value: &serde_json::Value,
) -> Option<CodexJsonImportCandidate> {
    if value.is_object() {
        if let Some(access_token) = first_explicit_personal_access_token_string(value) {
            let hints = extract_access_token_import_hints_from_value(value);
            return Some(CodexJsonImportCandidate::AccessToken {
                access_token,
                hints,
            });
        }
    }

    if let Some(candidate) = extract_codex_session_candidate_from_value(value) {
        return Some(candidate);
    }

    if let Some((tokens, account_id_hint)) = extract_codex_tokens_from_value(value)
        .or_else(|| extract_codex_tokens_from_credentials_value(value))
    {
        return Some(CodexJsonImportCandidate::FullToken {
            tokens,
            account_id_hint,
            subscription_active_until_hint: extract_access_token_import_hints_from_value(value)
                .subscription_active_until,
            note_update: codex_account_note_update_from_value(value),
        });
    }

    if let Some(refresh_token) = extract_refresh_token_only_from_value(value) {
        return Some(CodexJsonImportCandidate::RefreshToken {
            refresh_token,
            note_update: codex_account_note_update_from_value(value),
        });
    }

    extract_access_token_only_from_value(value).map(|(access_token, mut hints)| {
        let hints_note_update = codex_account_note_update_from_hints(&hints);
        let hints_note_update = merge_codex_account_note_update(
            hints_note_update,
            codex_account_note_update_from_value(value),
        );
        hints.account_note = hints_note_update.note;
        hints.two_factor_secret = hints_note_update.two_factor_secret;
        hints.account_password = hints_note_update.account_password;
        hints.phone_number = hints_note_update.phone_number;
        hints.mail_url = hints_note_update.mail_url;
        CodexJsonImportCandidate::AccessToken {
            access_token,
            hints,
        }
    })
}

async fn upsert_account_from_refresh_token(
    refresh_token: String,
    note_update: CodexAccountNoteUpdate,
) -> Result<CodexAccount, String> {
    let tokens = codex_oauth::refresh_access_token(&refresh_token).await?;
    let mut account = upsert_account(tokens)?;
    save_account_note_update_if_present(&mut account, note_update)?;
    Ok(account)
}

fn upsert_account_from_access_token(
    access_token: String,
    account_note: Option<String>,
) -> Result<CodexAccount, String> {
    upsert_account_from_access_token_with_hints(
        access_token,
        CodexAccessTokenImportHints {
            account_note,
            ..Default::default()
        },
    )
}

/// Named access-token import (community #1448): store as OAuth-shaped account with
/// optional display name; projection uses personal_access_token when no refresh/id.
pub fn import_access_token_account(
    account_name: String,
    access_token: String,
) -> Result<CodexAccount, String> {
    let account_name =
        normalize_optional_value(Some(account_name)).ok_or("账户名不能为空".to_string())?;
    let access_token = normalize_optional_value(Some(access_token))
        .ok_or("Codex access token 不能为空".to_string())?;
    if !is_importable_access_token(&access_token) {
        return Err("无效的 Codex access token".to_string());
    }

    upsert_account_from_access_token_with_hints(
        access_token,
        CodexAccessTokenImportHints {
            account_name: Some(account_name),
            ..Default::default()
        },
    )
}

fn upsert_account_from_access_token_with_hints(
    access_token: String,
    hints: CodexAccessTokenImportHints,
) -> Result<CodexAccount, String> {
    let note_update = codex_account_note_update_from_hints(&hints);
    let access_token =
        normalize_optional_value(Some(access_token)).ok_or("accessToken 不能为空")?;
    let (
        token_email,
        token_user_id,
        token_plan_type,
        token_subscription,
        token_account_id,
        token_org_id,
    ) = extract_access_token_identity(&access_token);
    let account_id = normalize_optional_value(token_account_id.or(hints.account_id.clone()));
    let organization_id = normalize_optional_value(token_org_id.or(hints.organization_id.clone()));
    let email = token_email
        .or(hints.email.clone())
        .or_else(|| account_id.as_ref().map(|value| format!("codex-{}", value)))
        .or_else(|| {
            token_user_id
                .as_ref()
                .map(|value| format!("codex-{}", value))
        })
        .or_else(|| {
            hints
                .user_id
                .as_ref()
                .map(|value| format!("codex-{}", value))
        })
        .unwrap_or_else(|| format!("codex-access-{}", access_token_fingerprint(&access_token)));
    let user_id = normalize_optional_value(token_user_id.or(hints.user_id.clone()));
    let plan_type = normalize_optional_value(token_plan_type.or(hints.plan_type.clone()));
    let subscription_active_until = normalize_optional_value(
        hints
            .subscription_active_until
            .clone()
            .or(token_subscription),
    );
    let mut tokens = CodexTokens {
        id_token: String::new(),
        access_token,
        refresh_token: None,
    };

    let mut index = load_account_index();
    let generated_id =
        build_account_storage_id(&email, account_id.as_deref(), organization_id.as_deref());
    let existing_id = find_existing_account_id(
        &index,
        &email,
        account_id.as_deref(),
        organization_id.as_deref(),
    )
    .unwrap_or_else(|| generated_id.clone());

    let mut account = if let Some(mut acc) = load_account(&existing_id) {
        tokens = retain_existing_refresh_token_if_missing(tokens, Some(&acc));
        acc.tokens = tokens;
        mark_token_chain_updated(&mut acc);
        acc.auth_mode = CodexAuthMode::OAuth;
        acc.authorization_status = None;
        acc.openai_api_key = None;
        acc.api_base_url = None;
        acc.api_provider_mode = CodexApiProviderMode::OpenaiBuiltin;
        acc.api_provider_id = None;
        acc.api_provider_name = None;
        acc.bound_oauth_account_id = None;
        acc.bound_oauth_use_local_gateway = false;
        acc.user_id = user_id;
        acc.plan_type = plan_type.clone();
        acc.subscription_active_until = subscription_active_until.clone();
        acc.account_id = account_id.clone();
        acc.organization_id = organization_id.clone();
        if hints.account_name.is_some() {
            acc.account_name = hints.account_name.clone();
        }
        if hints.account_structure.is_some() {
            acc.account_structure = hints.account_structure.clone();
        }
        acc.update_last_used();
        acc
    } else {
        tokens = retain_existing_refresh_token_if_missing(tokens, None);
        let mut acc = CodexAccount::new(existing_id.clone(), email.clone(), tokens);
        mark_token_chain_updated(&mut acc);
        acc.auth_mode = CodexAuthMode::OAuth;
        acc.authorization_status = None;
        acc.openai_api_key = None;
        acc.api_base_url = None;
        acc.api_provider_mode = CodexApiProviderMode::OpenaiBuiltin;
        acc.api_provider_id = None;
        acc.api_provider_name = None;
        acc.bound_oauth_account_id = None;
        acc.bound_oauth_use_local_gateway = false;
        acc.user_id = user_id;
        acc.plan_type = plan_type.clone();
        acc.subscription_active_until = subscription_active_until.clone();
        acc.account_id = account_id.clone();
        acc.organization_id = organization_id.clone();
        acc.account_name = hints.account_name.clone();
        acc.account_structure = hints.account_structure.clone();

        index.accounts.retain(|item| item.id != existing_id);
        index.accounts.push(CodexAccountSummary {
            id: existing_id.clone(),
            email: email.clone(),
            plan_type: plan_type.clone(),
            subscription_active_until: subscription_active_until.clone(),
            created_at: acc.created_at,
            last_used: acc.last_used,
        });
        acc
    };
    apply_account_note_update_if_present(&mut account, note_update);

    save_account_from_user_action(&mut account)?;

    if let Some(summary) = index.accounts.iter_mut().find(|item| item.id == account.id) {
        summary.email = account.email.clone();
        summary.plan_type = account.plan_type.clone();
        summary.subscription_active_until = account.subscription_active_until.clone();
        summary.last_used = account.last_used;
    } else {
        index.accounts.push(CodexAccountSummary {
            id: account.id.clone(),
            email: account.email.clone(),
            plan_type: account.plan_type.clone(),
            subscription_active_until: account.subscription_active_until.clone(),
            created_at: account.created_at,
            last_used: account.last_used,
        });
    }

    save_account_index(&index)?;

    logger::log_info(&format!(
        "Codex accessToken 账号已保存: email={}, account_id={:?}, organization_id={:?}",
        email, account_id, organization_id
    ));

    Ok(account)
}

async fn import_codex_candidate(
    candidate: CodexJsonImportCandidate,
) -> Result<CodexAccount, String> {
    match candidate {
        CodexJsonImportCandidate::FullToken {
            tokens,
            account_id_hint,
            subscription_active_until_hint,
            note_update,
        } => {
            let mut account = upsert_account_with_import_hints(
                tokens,
                account_id_hint,
                None,
                subscription_active_until_hint,
            )?;
            save_account_note_update_if_present(&mut account, note_update)?;
            Ok(account)
        }
        CodexJsonImportCandidate::AccessToken {
            access_token,
            hints,
        } => upsert_account_from_access_token_with_hints(access_token, hints),
        CodexJsonImportCandidate::RefreshToken {
            refresh_token,
            note_update,
        } => upsert_account_from_refresh_token(refresh_token, note_update).await,
    }
}

/// 快速待授权行格式：
/// `邮箱----账号密码----2FA秘钥----邮件地址`
/// 也兼容 3 段（无邮件地址）：`邮箱----账号密码----2FA秘钥`
fn try_parse_pending_oauth_delimited_line(line: &str) -> Option<(String, CodexAccountNoteUpdate)> {
    let line = normalize_optional_ref(Some(line))?;
    if !line.contains("----") {
        return None;
    }
    // 避免把 JSON / URL 误判成该格式
    let trimmed_start = line.trim_start();
    if trimmed_start.starts_with('{') || trimmed_start.starts_with('[') {
        return None;
    }

    let parts: Vec<&str> = line.splitn(4, "----").map(str::trim).collect();
    if parts.len() < 3 || parts.len() > 4 {
        return None;
    }

    let email = parts[0];
    if email.is_empty() || !email.contains('@') {
        return None;
    }
    // 基础邮箱形态：本地部分与域名均非空
    let (local, domain) = email.split_once('@')?;
    if local.is_empty() || domain.is_empty() || !domain.contains('.') {
        return None;
    }

    let password = parts.get(1).copied().unwrap_or("").trim();
    let two_factor = parts.get(2).copied().unwrap_or("").trim();
    let mail_url = parts.get(3).copied().unwrap_or("").trim();

    // 至少需要密码或 2FA 之一，避免把普通带 ---- 的 token 误导入为待授权
    if password.is_empty() && two_factor.is_empty() && mail_url.is_empty() {
        return None;
    }

    Some((
        email.to_string(),
        CodexAccountNoteUpdate {
            note: None,
            two_factor_secret: normalize_optional_ref(Some(two_factor)),
            account_password: normalize_optional_ref(Some(password)),
            phone_number: None,
            mail_url: normalize_optional_ref(Some(mail_url)),
        },
    ))
}

async fn import_accounts_from_token_lines(content: &str) -> Result<Vec<CodexAccount>, String> {
    let lines: Vec<String> = content
        .lines()
        .filter_map(|line| normalize_optional_ref(Some(line)))
        .collect();

    if lines.is_empty() {
        return Err("Token 不能为空".to_string());
    }

    let mut accounts = Vec::new();
    for (index, line) in lines.into_iter().enumerate() {
        if let Some((email, update)) = try_parse_pending_oauth_delimited_line(&line) {
            accounts.push(
                create_pending_oauth_account(email, update)
                    .map_err(|err| format!("第 {} 行待授权账号导入失败: {}", index + 1, err))?,
            );
            continue;
        }

        let values = match serde_json::from_str::<serde_json::Value>(&line) {
            Ok(serde_json::Value::Array(items)) => items,
            Ok(value) => vec![value],
            Err(_) => vec![serde_json::Value::String(line)],
        };

        for value in values {
            if let Some(identity) = parse_agent_identity_from_value(&value)? {
                accounts.push(upsert_agent_identity_account(identity)?);
                continue;
            }
            let candidate = extract_codex_import_candidate_from_value(&value).ok_or_else(|| {
                "未找到有效的 Codex 凭据（需要 Agent Identity、session JSON、accessToken/access_token、id_token + access_token，或 refresh_token）"
                    .to_string()
            })?;
            accounts.push(import_codex_candidate(candidate).await?);
        }
    }

    Ok(accounts)
}

fn is_sub2api_codex_oauth_account(value: &serde_json::Value) -> bool {
    let platform = first_json_string(value, &[&["platform"]])
        .unwrap_or_default()
        .to_ascii_lowercase();
    let account_type = first_json_string(value, &[&["type"]])
        .unwrap_or_default()
        .to_ascii_lowercase();

    platform == "openai" && account_type == "oauth"
}

fn looks_like_sub2api_export(value: &serde_json::Value) -> bool {
    let Some(accounts) = value.get("accounts").and_then(|item| item.as_array()) else {
        return false;
    };

    value.get("exported_at").is_some()
        || value.get("proxies").is_some()
        || accounts
            .iter()
            .any(|item| item.get("credentials").is_some() && item.get("platform").is_some())
}

async fn import_sub2api_export_from_value(
    value: &serde_json::Value,
) -> Result<Option<Vec<CodexAccount>>, String> {
    if !looks_like_sub2api_export(value) {
        return Ok(None);
    }

    let accounts = value
        .get("accounts")
        .and_then(|item| item.as_array())
        .ok_or("Sub2API JSON 缺少 accounts 数组")?;
    let mut imported = Vec::new();

    for (index, item) in accounts.iter().enumerate() {
        if !is_sub2api_codex_oauth_account(item) {
            continue;
        }
        if let Some(identity) = parse_agent_identity_from_value(item)? {
            imported.push(upsert_agent_identity_account(identity)?);
            continue;
        }
        let candidate = extract_codex_import_candidate_from_value(item).ok_or_else(|| {
            format!(
                "Sub2API 第 {} 个 OpenAI OAuth 账号缺少有效 access_token 或 Agent Identity",
                index + 1
            )
        })?;
        let mut account = import_codex_candidate(candidate).await?;
        account.codex_fingerprint_mode = read_codex_fingerprint_mode(item);
        account.codex_cli_only =
            read_codex_client_policy_bool(item, "codex_cli_only").unwrap_or(false);
        account.codex_cli_only_allow_app_server =
            read_codex_client_policy_bool(item, "codex_cli_only_allow_app_server").unwrap_or(false);
        save_account(&account)?;
        imported.push(account);
    }

    if imported.is_empty() {
        return Err(
            "Sub2API JSON 中未找到可导入的 OpenAI OAuth access_token 或 Agent Identity".to_string(),
        );
    }

    Ok(Some(imported))
}

async fn import_account_from_json_value(
    value: serde_json::Value,
) -> Result<Option<CodexAccount>, String> {
    let is_web_session = normalize_codex_session_value(&value, 0).is_some();

    if let Some(identity) = parse_agent_identity_from_value(&value)? {
        return Ok(Some(upsert_agent_identity_account(identity)?));
    }

    if let Some(account) = pending_oauth_account_from_value(&value) {
        return Ok(Some(import_account_struct(account)?));
    }

    if is_auth_mode_apikey(
        value
            .get("auth_mode")
            .and_then(|value| value.as_str())
            .or_else(|| value.get("authMode").and_then(|value| value.as_str())),
    ) {
        if let Some(api_key) = value
            .get("OPENAI_API_KEY")
            .and_then(|value| value.as_str())
            .and_then(normalize_api_key)
        {
            let mut account = upsert_api_key_account(
                api_key,
                extract_api_base_url_from_json_value(&value),
                read_codex_api_provider_mode(&value),
                value
                    .get("api_provider_id")
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string()),
                value
                    .get("api_provider_name")
                    .and_then(|value| value.as_str())
                    .map(|value| value.to_string()),
                Vec::new(),
                Some(false),
                None,
                false,
                false,
                std::collections::HashMap::new(),
                None,
                None,
                None,
            )?;
            apply_api_key_import_metadata(&mut account, &value);
            save_account(&account)?;
            update_account_plan_type_in_index(
                &account.id,
                &account.plan_type,
                &account.subscription_active_until,
            )?;
            return Ok(Some(account));
        }
    }

    if let Some(candidate) = extract_codex_import_candidate_from_value(&value) {
        let account = import_codex_candidate(candidate).await?;
        return Ok(Some(if is_web_session {
            mark_imported_web_session_account(account)?
        } else {
            account
        }));
    }

    if let Ok(account) = serde_json::from_value::<CodexAccount>(value) {
        let account = import_account_struct(account)?;
        return Ok(Some(if is_web_session {
            mark_imported_web_session_account(account)?
        } else {
            account
        }));
    }

    Ok(None)
}

fn parse_line_delimited_json_values(
    json_content: &str,
) -> Result<Option<Vec<serde_json::Value>>, String> {
    let lines: Vec<(usize, &str)> = json_content
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some((index + 1, trimmed))
            }
        })
        .collect();

    if lines.len() <= 1 {
        return Ok(None);
    }

    let mut values = Vec::with_capacity(lines.len());
    for (line_number, line) in lines {
        let parsed = serde_json::from_str::<serde_json::Value>(line)
            .map_err(|e| format!("第 {} 行不是有效 JSON: {}", line_number, e))?;
        if !parsed.is_object() {
            return Err(format!("第 {} 行不是 JSON 对象", line_number));
        }
        values.push(parsed);
    }

    Ok(Some(values))
}

/// 从 JSON 字符串导入账号。
/// Web Session 格式会按普通 Token 账号落盘并标记为仅查额（不可启动/切号/加入 API）。
pub async fn import_from_json(json_content: &str) -> Result<Vec<CodexAccount>, String> {
    ensure_storage_writable_for_import()?;
    if !json_content.trim().is_empty()
        && !json_content.trim_start().starts_with('{')
        && !json_content.trim_start().starts_with('[')
    {
        return import_accounts_from_token_lines(json_content).await;
    }

    // 尝试解析为 auth.json 格式
    if let Ok(auth_file) = serde_json::from_str::<CodexAuthFile>(json_content) {
        let raw_value = serde_json::from_str::<serde_json::Value>(json_content).ok();
        let fallback_api_key = extract_api_key_from_auth_file(&auth_file);
        let fallback_provider = if let Some(value) = raw_value.as_ref() {
            infer_api_provider_config(
                extract_api_base_url_from_auth_file(&auth_file).as_deref(),
                read_codex_api_provider_mode(value),
                value.get("api_provider_id").and_then(|item| item.as_str()),
                value
                    .get("api_provider_name")
                    .and_then(|item| item.as_str()),
            )
        } else {
            infer_api_provider_config(
                extract_api_base_url_from_auth_file(&auth_file).as_deref(),
                None,
                None,
                None,
            )
        };
        if is_auth_mode_apikey(auth_file.auth_mode.as_deref()) {
            let api_key = fallback_api_key.ok_or("auth.json 缺少 OPENAI_API_KEY")?;
            let mut account = upsert_api_key_account(
                api_key,
                fallback_provider.base_url.clone(),
                Some(fallback_provider.mode),
                fallback_provider.provider_id.clone(),
                fallback_provider.provider_name.clone(),
                Vec::new(),
                Some(false),
                None,
                false,
                false,
                std::collections::HashMap::new(),
                None,
                None,
                None,
            )?;
            if let Some(value) = raw_value.as_ref() {
                apply_api_key_import_metadata(&mut account, value);
                save_account(&account)?;
                update_account_plan_type_in_index(
                    &account.id,
                    &account.plan_type,
                    &account.subscription_active_until,
                )?;
            }
            return Ok(vec![account]);
        }

        if let Some(tokens) = auth_file.tokens {
            let mut account = upsert_account_from_auth_tokens(tokens)?;
            if let Some(value) = raw_value.as_ref() {
                save_account_note_update_if_present(
                    &mut account,
                    codex_account_note_update_from_value(value),
                )?;
            }
            return Ok(vec![account]);
        }

        if let Some(api_key) = fallback_api_key {
            let mut account = upsert_api_key_account(
                api_key,
                fallback_provider.base_url.clone(),
                Some(fallback_provider.mode),
                fallback_provider.provider_id.clone(),
                fallback_provider.provider_name.clone(),
                Vec::new(),
                Some(false),
                None,
                false,
                false,
                std::collections::HashMap::new(),
                None,
                None,
                None,
            )?;
            if let Some(value) = raw_value.as_ref() {
                apply_api_key_import_metadata(&mut account, value);
                save_account(&account)?;
                update_account_plan_type_in_index(
                    &account.id,
                    &account.plan_type,
                    &account.subscription_active_until,
                )?;
            }
            return Ok(vec![account]);
        }
    }

    // 尝试解析为单账号（顶层 token）或通用数组（支持混合对象）
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_content) {
        if let Some(accounts) = import_sub2api_export_from_value(&parsed).await? {
            return Ok(accounts);
        }

        match parsed {
            serde_json::Value::Object(_) => {
                if let Some(account) = import_account_from_json_value(parsed).await? {
                    return Ok(vec![account]);
                }
            }
            serde_json::Value::Array(items) => {
                let mut result = Vec::new();

                for item in items {
                    if let Some(account) = import_account_from_json_value(item).await? {
                        result.push(account);
                    }
                }

                if !result.is_empty() {
                    return Ok(result);
                }
            }
            _ => {}
        }
    }

    if let Some(items) = parse_line_delimited_json_values(json_content)? {
        let mut result = Vec::new();

        for (index, item) in items.into_iter().enumerate() {
            match import_account_from_json_value(item).await? {
                Some(account) => result.push(account),
                None => {
                    return Err(format!(
                        "第 {} 行未找到有效的 Codex Token（需要 session JSON、accessToken/access_token、id_token + access_token，或 refresh_token）",
                        index + 1
                    ));
                }
            }
        }

        if !result.is_empty() {
            return Ok(result);
        }
    }

    Err("无法解析 JSON 内容".to_string())
}

/// 导出账号为 JSON
pub fn export_accounts(account_ids: &[String]) -> Result<String, String> {
    let accounts: Vec<CodexAccount> = account_ids
        .iter()
        .filter_map(|id| load_account(id))
        .collect();

    serde_json::to_string_pretty(&accounts).map_err(|e| format!("序列化失败: {}", e))
}

#[derive(serde::Serialize, Clone)]
pub struct CodexFileImportResult {
    pub imported: Vec<CodexAccount>,
    pub failed: Vec<CodexFileImportFailure>,
}

#[derive(serde::Serialize, Clone)]
pub struct CodexFileImportFailure {
    pub email: String,
    pub error: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CodexBatchImportStartResult {
    pub session_id: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CodexBatchImportItem {
    pub item_id: String,
    pub source: String,
    pub label: String,
    pub account_id: Option<String>,
    pub email: Option<String>,
    pub account_type: String,
    pub provider: Option<String>,
    pub quota_status: String,
    pub quota_error: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub default_selected: bool,
    pub selectable: bool,
    pub existing: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CodexBatchImportProgress {
    pub session_id: String,
    pub phase: String,
    pub check_quota: bool,
    pub current: usize,
    pub total: usize,
    pub success: usize,
    pub failed: usize,
    pub quota_failed: usize,
    pub existing: usize,
    pub current_label: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CodexBatchImportPreview {
    pub session_id: String,
    pub status: String,
    pub check_quota: bool,
    pub total: usize,
    pub items: Vec<CodexBatchImportItem>,
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CodexBatchImportConfirmResult {
    pub imported: Vec<CodexAccount>,
    pub failed: Vec<CodexFileImportFailure>,
    pub cancelled: bool,
    pub processed: usize,
    pub total: usize,
}

#[derive(Clone)]
struct CodexBatchImportSession {
    status: String,
    check_quota: bool,
    cancel: Arc<AtomicBool>,
    source_items: Vec<CodexBatchImportSourceItem>,
    next_index: usize,
    total: usize,
    items: Vec<CodexBatchImportCachedItem>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct CodexBatchImportSourceItem {
    source: String,
    value: serde_json::Value,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct CodexBatchImportCachedItem {
    preview: CodexBatchImportItem,
    draft: Option<CodexBatchImportDraft>,
    quota: Option<crate::models::codex::CodexQuota>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
enum CodexBatchImportDraft {
    Account(CodexAccount),
    FullToken {
        tokens: CodexTokens,
        account_id_hint: Option<String>,
        #[serde(default)]
        subscription_active_until_hint: Option<String>,
        #[serde(default)]
        note_update: CodexAccountNoteUpdate,
    },
    AccessToken {
        access_token: String,
        hints: CodexAccessTokenImportHints,
    },
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexBatchImportSessionSnapshot {
    version: u32,
    status: String,
    check_quota: bool,
    source_items: Vec<CodexBatchImportSourceItem>,
    next_index: usize,
    total: usize,
    items: Vec<CodexBatchImportCachedItem>,
    updated_at: i64,
}

fn next_codex_batch_import_session_id() -> String {
    let id = CODEX_BATCH_IMPORT_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!(
        "codex-import-{}-{}",
        chrono::Utc::now().timestamp_millis(),
        id
    )
}

fn get_codex_batch_import_sessions_dir() -> PathBuf {
    let data_dir = account::get_data_dir()
        .or_else(|_| account::resolve_data_dir())
        .unwrap_or_else(|_| PathBuf::from(".antigravity_cockpit"));
    data_dir.join(CODEX_BATCH_IMPORT_SESSIONS_DIR)
}

fn sanitize_codex_batch_import_session_id(session_id: &str) -> Result<String, String> {
    let trimmed = session_id.trim();
    if trimmed.is_empty() {
        return Err("导入会话 ID 为空".to_string());
    }
    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err("导入会话 ID 不合法".to_string());
    }
    Ok(trimmed.to_string())
}

fn codex_batch_import_session_snapshot_path(session_id: &str) -> Result<PathBuf, String> {
    let safe_id = sanitize_codex_batch_import_session_id(session_id)?;
    Ok(get_codex_batch_import_sessions_dir().join(format!("{}.json", safe_id)))
}

fn ensure_codex_batch_import_sessions_dir(path: &Path) -> Result<(), String> {
    if path.is_dir() {
        return Ok(());
    }
    if path.exists() {
        return Err(format!(
            "创建导入会话目录失败: path={} 不是目录",
            path.display()
        ));
    }
    fs::create_dir(path).map_err(|error| {
        format!(
            "创建导入会话目录失败: path={}, error={}",
            path.display(),
            error
        )
    })
}

fn codex_batch_import_snapshot_from_session(
    session: &CodexBatchImportSession,
) -> CodexBatchImportSessionSnapshot {
    CodexBatchImportSessionSnapshot {
        version: 1,
        status: session.status.clone(),
        check_quota: session.check_quota,
        source_items: session.source_items.clone(),
        next_index: session.next_index,
        total: session.total,
        items: session.items.clone(),
        updated_at: chrono::Utc::now().timestamp(),
    }
}

fn codex_batch_import_session_from_snapshot(
    snapshot: CodexBatchImportSessionSnapshot,
) -> CodexBatchImportSession {
    let status = if snapshot.status == "scanning" {
        "cancelled".to_string()
    } else {
        snapshot.status
    };
    CodexBatchImportSession {
        status,
        check_quota: snapshot.check_quota,
        cancel: Arc::new(AtomicBool::new(false)),
        source_items: snapshot.source_items,
        next_index: snapshot.next_index,
        total: snapshot.total,
        items: snapshot.items,
    }
}

fn save_codex_batch_import_session_snapshot(
    session_id: &str,
    session: &CodexBatchImportSession,
) -> Result<(), String> {
    let path = codex_batch_import_session_snapshot_path(session_id)?;
    if let Some(parent) = path.parent() {
        ensure_codex_batch_import_sessions_dir(parent)?;
    }
    let snapshot = codex_batch_import_snapshot_from_session(session);
    let content = serde_json::to_string_pretty(&snapshot)
        .map_err(|error| format!("序列化导入会话快照失败: {}", error))?;
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, content).map_err(|error| {
        format!(
            "写入导入会话快照失败: path={}, error={}",
            tmp_path.display(),
            error
        )
    })?;
    fs::rename(&tmp_path, &path).map_err(|error| {
        let _ = fs::remove_file(&tmp_path);
        format!(
            "更新导入会话快照失败: path={}, error={}",
            path.display(),
            error
        )
    })
}

fn save_codex_batch_import_session_snapshot_best_effort(
    session_id: &str,
    session: &CodexBatchImportSession,
) {
    if let Err(error) = save_codex_batch_import_session_snapshot(session_id, session) {
        logger::log_warn(&format!(
            "[Codex Batch Import] 保存导入会话快照失败: session_id={}, error={}",
            session_id, error
        ));
    }
}

fn load_codex_batch_import_session_snapshot(
    session_id: &str,
) -> Result<Option<CodexBatchImportSession>, String> {
    let path = codex_batch_import_session_snapshot_path(session_id)?;
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path).map_err(|error| {
        format!(
            "读取导入会话快照失败: path={}, error={}",
            path.display(),
            error
        )
    })?;
    let snapshot: CodexBatchImportSessionSnapshot =
        serde_json::from_str(&content).map_err(|error| {
            format!(
                "解析导入会话快照失败: path={}, error={}",
                path.display(),
                error
            )
        })?;
    Ok(Some(codex_batch_import_session_from_snapshot(snapshot)))
}

fn remove_codex_batch_import_session_snapshot(session_id: &str) {
    if let Ok(path) = codex_batch_import_session_snapshot_path(session_id) {
        let _ = fs::remove_file(path);
    }
}

fn ensure_codex_batch_import_session_loaded(session_id: &str) -> Result<(), String> {
    {
        let sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
        if sessions.contains_key(session_id) {
            return Ok(());
        }
    }
    let Some(session) = load_codex_batch_import_session_snapshot(session_id)? else {
        return Err("导入会话不存在".to_string());
    };
    let mut sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
    sessions.entry(session_id.to_string()).or_insert(session);
    Ok(())
}

fn emit_codex_batch_import_progress(app: &tauri::AppHandle, payload: CodexBatchImportProgress) {
    use tauri::Emitter;
    let _ = app.emit("codex:batch-import-progress", payload);
}

fn emit_codex_batch_import_completed(app: &tauri::AppHandle, payload: CodexBatchImportPreview) {
    use tauri::Emitter;
    let _ = app.emit("codex:batch-import-completed", payload);
}

fn emit_codex_batch_import_preview(app: &tauri::AppHandle, payload: CodexBatchImportPreview) {
    use tauri::Emitter;
    let _ = app.emit("codex:batch-import-preview", payload);
}

fn codex_batch_import_preview_from_session(
    session_id: &str,
    session: &CodexBatchImportSession,
) -> CodexBatchImportPreview {
    CodexBatchImportPreview {
        session_id: session_id.to_string(),
        status: session.status.clone(),
        check_quota: session.check_quota,
        total: session.total,
        items: session
            .items
            .iter()
            .map(|item| item.preview.clone())
            .collect(),
    }
}

fn codex_batch_import_progress_from_items(
    session_id: &str,
    phase: &str,
    check_quota: bool,
    current: usize,
    total: usize,
    items: &[CodexBatchImportCachedItem],
    current_label: Option<String>,
) -> CodexBatchImportProgress {
    CodexBatchImportProgress {
        session_id: session_id.to_string(),
        phase: phase.to_string(),
        check_quota,
        current,
        total,
        success: items
            .iter()
            .filter(|item| item.preview.status == "ready")
            .count(),
        failed: items
            .iter()
            .filter(|item| item.preview.status == "invalid")
            .count(),
        quota_failed: items
            .iter()
            .filter(|item| item.preview.status == "quota_failed")
            .count(),
        existing: items.iter().filter(|item| item.preview.existing).count(),
        current_label,
    }
}

fn preview_account_from_full_tokens(
    mut tokens: CodexTokens,
    account_id_hint: Option<String>,
    subscription_active_until_hint: Option<String>,
    note_update: CodexAccountNoteUpdate,
) -> Result<CodexAccount, String> {
    let (
        email,
        user_id,
        plan_type,
        token_subscription_active_until,
        id_token_account_id,
        id_token_org_id,
    ) = extract_user_info(&tokens.id_token)?;
    let subscription_active_until = normalize_optional_value(
        subscription_active_until_hint.or(token_subscription_active_until),
    );
    let account_id = normalize_optional_value(
        extract_chatgpt_account_id_from_access_token(&tokens.access_token)
            .or(id_token_account_id)
            .or(account_id_hint),
    );
    let organization_id = normalize_optional_value(
        extract_chatgpt_organization_id_from_access_token(&tokens.access_token).or(id_token_org_id),
    );
    tokens = retain_existing_refresh_token_if_missing(tokens, None);
    let storage_id =
        build_account_storage_id(&email, account_id.as_deref(), organization_id.as_deref());
    let mut account = CodexAccount::new(storage_id, email, tokens);
    mark_token_chain_updated(&mut account);
    account.auth_mode = CodexAuthMode::OAuth;
    account.user_id = user_id;
    account.plan_type = plan_type;
    account.subscription_active_until = subscription_active_until;
    account.account_id = account_id;
    account.organization_id = organization_id;
    apply_account_note_update_if_present(&mut account, note_update);
    Ok(account)
}

fn preview_account_from_access_token(
    access_token: String,
    hints: CodexAccessTokenImportHints,
) -> Result<CodexAccount, String> {
    let access_token =
        normalize_optional_value(Some(access_token)).ok_or("accessToken 不能为空")?;
    let (
        token_email,
        token_user_id,
        token_plan_type,
        token_subscription,
        token_account_id,
        token_org_id,
    ) = extract_access_token_identity(&access_token);
    let account_id = normalize_optional_value(token_account_id.or(hints.account_id.clone()));
    let organization_id = normalize_optional_value(token_org_id.or(hints.organization_id.clone()));
    let email = token_email
        .or(hints.email.clone())
        .or_else(|| account_id.as_ref().map(|value| format!("codex-{}", value)))
        .or_else(|| {
            token_user_id
                .as_ref()
                .map(|value| format!("codex-{}", value))
        })
        .or_else(|| {
            hints
                .user_id
                .as_ref()
                .map(|value| format!("codex-{}", value))
        })
        .unwrap_or_else(|| format!("codex-access-{}", access_token_fingerprint(&access_token)));
    let tokens = CodexTokens {
        id_token: String::new(),
        access_token,
        refresh_token: None,
    };
    let storage_id =
        build_account_storage_id(&email, account_id.as_deref(), organization_id.as_deref());
    let mut account = CodexAccount::new(storage_id, email, tokens);
    mark_token_chain_updated(&mut account);
    account.auth_mode = CodexAuthMode::OAuth;
    account.authorization_status = None;
    account.user_id = normalize_optional_value(token_user_id.or(hints.user_id));
    account.plan_type = normalize_optional_value(token_plan_type.or(hints.plan_type));
    account.subscription_active_until =
        normalize_optional_value(hints.subscription_active_until.or(token_subscription));
    account.account_id = account_id;
    account.organization_id = organization_id;
    account.account_name = hints.account_name;
    account.account_structure = hints.account_structure;
    account.account_note = hints.account_note;
    account.two_factor_secret = hints.two_factor_secret;
    account.account_password = hints.account_password;
    account.phone_number = hints.phone_number;
    account.mail_url = hints.mail_url;
    Ok(account)
}

fn preview_account_for_draft(draft: &CodexBatchImportDraft) -> Result<CodexAccount, String> {
    match draft {
        CodexBatchImportDraft::Account(account) => Ok(account.clone()),
        CodexBatchImportDraft::FullToken {
            tokens,
            account_id_hint,
            subscription_active_until_hint,
            note_update,
        } => preview_account_from_full_tokens(
            tokens.clone(),
            account_id_hint.clone(),
            subscription_active_until_hint.clone(),
            note_update.clone(),
        ),
        CodexBatchImportDraft::AccessToken {
            access_token,
            hints,
        } => preview_account_from_access_token(access_token.clone(), hints.clone()),
    }
}

fn codex_batch_import_draft_from_candidate(
    candidate: CodexJsonImportCandidate,
) -> CodexBatchImportDraft {
    match candidate {
        CodexJsonImportCandidate::FullToken {
            tokens,
            account_id_hint,
            subscription_active_until_hint,
            note_update,
        } => CodexBatchImportDraft::FullToken {
            tokens,
            account_id_hint,
            subscription_active_until_hint,
            note_update,
        },
        CodexJsonImportCandidate::AccessToken {
            access_token,
            hints,
        } => CodexBatchImportDraft::AccessToken {
            access_token,
            hints,
        },
        CodexJsonImportCandidate::RefreshToken { .. } => {
            unreachable!("refresh_token candidates are resolved before creating a draft")
        }
    }
}

fn api_key_draft_from_value(
    value: &serde_json::Value,
    fallback_id: Option<String>,
) -> Result<Option<CodexBatchImportDraft>, String> {
    if !is_auth_mode_apikey(
        value
            .get("auth_mode")
            .and_then(|value| value.as_str())
            .or_else(|| value.get("authMode").and_then(|value| value.as_str())),
    ) {
        return Ok(None);
    }
    let Some(api_key) = value
        .get("OPENAI_API_KEY")
        .and_then(|value| value.as_str())
        .and_then(normalize_api_key)
    else {
        return Ok(None);
    };
    let (api_key, api_base_url) = validate_api_key_credentials(
        &api_key,
        extract_api_base_url_from_json_value(value).as_deref(),
    )?;
    let provider_config = resolve_api_provider_config(
        api_base_url.as_deref(),
        read_codex_api_provider_mode(value),
        value
            .get("api_provider_id")
            .and_then(|value| value.as_str()),
        value
            .get("api_provider_name")
            .and_then(|value| value.as_str()),
    )?;
    let mut account = CodexAccount::new_api_key(
        fallback_id.unwrap_or_else(|| build_api_key_account_id(&api_key)),
        read_json_string(value, &["email", "account_email"])
            .unwrap_or_else(|| build_api_key_email(&api_key)),
        api_key,
        provider_config.mode,
        provider_config.base_url,
        provider_config.provider_id,
        provider_config.provider_name,
        Vec::new(),
    );
    apply_api_key_import_metadata(&mut account, value);
    Ok(Some(CodexBatchImportDraft::Account(account)))
}

async fn codex_batch_import_draft_from_value(
    value: serde_json::Value,
) -> Result<Option<CodexBatchImportDraft>, String> {
    if let Some(identity) = parse_agent_identity_from_value(&value)? {
        return Ok(Some(CodexBatchImportDraft::Account(
            build_agent_identity_account_draft(identity)?,
        )));
    }

    if let Some(account) = pending_oauth_account_from_value(&value) {
        return Ok(Some(CodexBatchImportDraft::Account(account)));
    }

    if let Ok(auth_file) = serde_json::from_value::<CodexAuthFile>(value.clone()) {
        let fallback_api_key = extract_api_key_from_auth_file(&auth_file);
        let fallback_provider = infer_api_provider_config(
            extract_api_base_url_from_auth_file(&auth_file).as_deref(),
            read_codex_api_provider_mode(&value),
            value.get("api_provider_id").and_then(|item| item.as_str()),
            value
                .get("api_provider_name")
                .and_then(|item| item.as_str()),
        );
        if is_auth_mode_apikey(auth_file.auth_mode.as_deref()) {
            let api_key = fallback_api_key.ok_or("auth.json 缺少 OPENAI_API_KEY")?;
            let mut account = CodexAccount::new_api_key(
                build_api_key_account_id(&api_key),
                build_api_key_email(&api_key),
                api_key,
                fallback_provider.mode,
                fallback_provider.base_url,
                fallback_provider.provider_id,
                fallback_provider.provider_name,
                Vec::new(),
            );
            apply_api_key_import_metadata(&mut account, &value);
            return Ok(Some(CodexBatchImportDraft::Account(account)));
        }
        if let Some(tokens) = auth_file.tokens {
            let account_id_hint = tokens.account_id.clone();
            let tokens = CodexTokens {
                id_token: tokens.id_token,
                access_token: tokens.access_token,
                refresh_token: tokens.refresh_token,
            };
            if normalize_optional_ref(Some(&tokens.id_token)).is_none()
                && is_importable_access_token(&tokens.access_token)
            {
                let note_update = codex_account_note_update_from_value(&value);
                return Ok(Some(CodexBatchImportDraft::AccessToken {
                    access_token: tokens.access_token,
                    hints: CodexAccessTokenImportHints {
                        account_id: account_id_hint,
                        account_note: note_update.note,
                        two_factor_secret: note_update.two_factor_secret,
                        account_password: note_update.account_password,
                        phone_number: note_update.phone_number,
                        mail_url: note_update.mail_url,
                        ..Default::default()
                    },
                }));
            }
            return Ok(Some(CodexBatchImportDraft::FullToken {
                tokens,
                account_id_hint,
                subscription_active_until_hint: extract_access_token_import_hints_from_value(
                    &value,
                )
                .subscription_active_until,
                note_update: codex_account_note_update_from_value(&value),
            }));
        }
        if let Some(api_key) = fallback_api_key {
            let mut account = CodexAccount::new_api_key(
                build_api_key_account_id(&api_key),
                build_api_key_email(&api_key),
                api_key,
                fallback_provider.mode,
                fallback_provider.base_url,
                fallback_provider.provider_id,
                fallback_provider.provider_name,
                Vec::new(),
            );
            apply_api_key_import_metadata(&mut account, &value);
            return Ok(Some(CodexBatchImportDraft::Account(account)));
        }
    }

    if let Some(draft) = api_key_draft_from_value(&value, None)? {
        return Ok(Some(draft));
    }

    if let Some(candidate) = extract_codex_import_candidate_from_value(&value) {
        return match candidate {
            CodexJsonImportCandidate::RefreshToken {
                refresh_token,
                note_update,
            } => {
                let tokens = codex_oauth::refresh_access_token(&refresh_token).await?;
                Ok(Some(CodexBatchImportDraft::FullToken {
                    tokens,
                    account_id_hint: None,
                    subscription_active_until_hint: None,
                    note_update,
                }))
            }
            other => Ok(Some(codex_batch_import_draft_from_candidate(other))),
        };
    }

    if let Ok(account) = serde_json::from_value::<CodexAccount>(value) {
        return Ok(Some(CodexBatchImportDraft::Account(account)));
    }

    Ok(None)
}

fn codex_batch_import_values_from_content(content: &str) -> Result<Vec<serde_json::Value>, String> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        let mut values = Vec::new();
        for line in trimmed
            .lines()
            .filter_map(|line| normalize_optional_ref(Some(line)))
        {
            match serde_json::from_str::<serde_json::Value>(&line) {
                Ok(serde_json::Value::Array(items)) => values.extend(items),
                Ok(value) => values.push(value),
                Err(_) => values.push(serde_json::Value::String(line)),
            }
        }
        return Ok(values);
    }

    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(value) => {
            if looks_like_sub2api_export(&value) {
                let accounts = value
                    .get("accounts")
                    .and_then(|item| item.as_array())
                    .ok_or("Sub2API JSON 缺少 accounts 数组")?;
                return Ok(accounts
                    .iter()
                    .filter(|item| is_sub2api_codex_oauth_account(item))
                    .cloned()
                    .collect());
            }
            match value {
                serde_json::Value::Array(items) => Ok(items),
                other => Ok(vec![other]),
            }
        }
        Err(_) => parse_line_delimited_json_values(trimmed).map(|items| items.unwrap_or_default()),
    }
}

fn codex_batch_import_account_type(account: &CodexAccount) -> String {
    if account.is_api_key_auth() {
        "API Key".to_string()
    } else if account.is_agent_identity_auth() {
        "Agent Identity".to_string()
    } else if normalize_optional_ref(account.tokens.refresh_token.as_deref()).is_some() {
        "OAuth".to_string()
    } else {
        "Access Token".to_string()
    }
}

async fn build_codex_batch_import_item(
    session_id: &str,
    index: usize,
    source: String,
    value: serde_json::Value,
    check_quota: bool,
) -> CodexBatchImportCachedItem {
    let item_id = format!("{}-item-{}", session_id, index + 1);
    let draft = match codex_batch_import_draft_from_value(value).await {
        Ok(Some(draft)) => draft,
        Ok(None) => {
            return CodexBatchImportCachedItem {
                preview: CodexBatchImportItem {
                    item_id,
                    source,
                    label: "未识别账号".to_string(),
                    account_id: None,
                    email: None,
                    account_type: "-".to_string(),
                    provider: None,
                    quota_status: "skipped".to_string(),
                    quota_error: None,
                    status: "invalid".to_string(),
                    error: Some("未找到有效的 Codex 账号凭据".to_string()),
                    default_selected: false,
                    selectable: false,
                    existing: false,
                },
                draft: None,
                quota: None,
            };
        }
        Err(error) => {
            return CodexBatchImportCachedItem {
                preview: CodexBatchImportItem {
                    item_id,
                    source,
                    label: "解析失败".to_string(),
                    account_id: None,
                    email: None,
                    account_type: "-".to_string(),
                    provider: None,
                    quota_status: "skipped".to_string(),
                    quota_error: None,
                    status: "invalid".to_string(),
                    error: Some(error),
                    default_selected: false,
                    selectable: false,
                    existing: false,
                },
                draft: None,
                quota: None,
            };
        }
    };

    let account = match preview_account_for_draft(&draft) {
        Ok(account) => account,
        Err(error) => {
            return CodexBatchImportCachedItem {
                preview: CodexBatchImportItem {
                    item_id,
                    source,
                    label: "解析失败".to_string(),
                    account_id: None,
                    email: None,
                    account_type: "-".to_string(),
                    provider: None,
                    quota_status: "skipped".to_string(),
                    quota_error: None,
                    status: "invalid".to_string(),
                    error: Some(error),
                    default_selected: false,
                    selectable: false,
                    existing: false,
                },
                draft: None,
                quota: None,
            };
        }
    };

    let existing = load_account(&account.id).is_some();
    let (quota_status, quota_error, quota, status) = if check_quota
        && !account.is_agent_identity_auth()
    {
        let quota_result = crate::modules::codex_quota::probe_import_account_quota(&account).await;
        let (quota_status, quota_error, quota) = match quota_result {
            Ok(quota) => ("success".to_string(), None, Some(quota)),
            Err(error) => ("failed".to_string(), Some(error), None),
        };
        let status = if quota_status == "failed" {
            "quota_failed".to_string()
        } else if existing {
            "existing".to_string()
        } else {
            "ready".to_string()
        };
        (quota_status, quota_error, quota, status)
    } else if existing {
        ("skipped".to_string(), None, None, "existing".to_string())
    } else {
        ("skipped".to_string(), None, None, "ready".to_string())
    };
    let default_selected = status == "ready" || status == "existing";
    CodexBatchImportCachedItem {
        preview: CodexBatchImportItem {
            item_id,
            source,
            label: account
                .account_name
                .clone()
                .unwrap_or_else(|| account.email.clone()),
            account_id: Some(account.id.clone()),
            email: Some(account.email.clone()),
            account_type: codex_batch_import_account_type(&account),
            provider: account
                .api_provider_name
                .clone()
                .or(account.api_provider_id.clone())
                .or(account.api_base_url.clone()),
            quota_status,
            quota_error,
            status,
            error: None,
            default_selected,
            selectable: true,
            existing,
        },
        draft: Some(draft),
        quota,
    }
}

async fn run_codex_batch_import_scan(
    app: tauri::AppHandle,
    session_id: String,
    file_paths: Vec<String>,
    check_quota: bool,
) {
    let cancel = {
        let sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
        sessions
            .get(&session_id)
            .map(|session| session.cancel.clone())
            .unwrap_or_else(|| Arc::new(AtomicBool::new(true)))
    };
    let mut values: Vec<CodexBatchImportSourceItem> = Vec::new();
    let mut read_failures: Vec<CodexBatchImportCachedItem> = Vec::new();

    for file_path in file_paths {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        let path = Path::new(&file_path);
        let source = path
            .file_name()
            .and_then(|item| item.to_str())
            .unwrap_or(&file_path)
            .to_string();
        match fs::read_to_string(path) {
            Ok(content) => match codex_batch_import_values_from_content(&content) {
                Ok(items) => {
                    values.extend(items.into_iter().map(|item| CodexBatchImportSourceItem {
                        source: source.clone(),
                        value: item,
                    }));
                }
                Err(error) => read_failures.push(CodexBatchImportCachedItem {
                    preview: CodexBatchImportItem {
                        item_id: format!("{}-file-error-{}", session_id, read_failures.len() + 1),
                        source,
                        label: "文件解析失败".to_string(),
                        account_id: None,
                        email: None,
                        account_type: "-".to_string(),
                        provider: None,
                        quota_status: "skipped".to_string(),
                        quota_error: None,
                        status: "invalid".to_string(),
                        error: Some(error),
                        default_selected: false,
                        selectable: false,
                        existing: false,
                    },
                    draft: None,
                    quota: None,
                }),
            },
            Err(error) => read_failures.push(CodexBatchImportCachedItem {
                preview: CodexBatchImportItem {
                    item_id: format!("{}-file-error-{}", session_id, read_failures.len() + 1),
                    source,
                    label: "文件读取失败".to_string(),
                    account_id: None,
                    email: None,
                    account_type: "-".to_string(),
                    provider: None,
                    quota_status: "skipped".to_string(),
                    quota_error: None,
                    status: "invalid".to_string(),
                    error: Some(error.to_string()),
                    default_selected: false,
                    selectable: false,
                    existing: false,
                },
                draft: None,
                quota: None,
            }),
        }
    }

    let total = values.len() + read_failures.len();
    let session_snapshot = {
        let mut sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
        if let Some(session) = sessions.get_mut(&session_id) {
            session.source_items = values;
            session.next_index = 0;
            session.total = total;
            session.items = read_failures;
            session.check_quota = check_quota;
            Some(session.clone())
        } else {
            None
        }
    };
    if let Some(session) = session_snapshot {
        save_codex_batch_import_session_snapshot_best_effort(&session_id, &session);
    }
    run_codex_batch_import_resume(app, session_id).await;
}

async fn run_codex_batch_import_resume(app: tauri::AppHandle, session_id: String) {
    let (cancel, check_quota, source_items, start_index, mut items, total, session_snapshot) = {
        let mut sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
        let Some(session) = sessions.get_mut(&session_id) else {
            return;
        };
        session.cancel.store(false, Ordering::SeqCst);
        session.status = "scanning".to_string();
        (
            session.cancel.clone(),
            session.check_quota,
            session.source_items.clone(),
            session.next_index,
            session.items.clone(),
            session.total,
            session.clone(),
        )
    };
    save_codex_batch_import_session_snapshot_best_effort(&session_id, &session_snapshot);

    emit_codex_batch_import_progress(
        &app,
        codex_batch_import_progress_from_items(
            &session_id,
            "scanning",
            check_quota,
            items.len(),
            total,
            &items,
            None,
        ),
    );

    for (index, source_item) in source_items.into_iter().enumerate().skip(start_index) {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        let cached = build_codex_batch_import_item(
            &session_id,
            index,
            source_item.source,
            source_item.value,
            check_quota,
        )
        .await;
        let current_label = Some(cached.preview.label.clone());
        items.push(cached);
        let session_snapshot = {
            let mut sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
            if let Some(session) = sessions.get_mut(&session_id) {
                session.next_index = index + 1;
                session.items = items.clone();
                Some(session.clone())
            } else {
                None
            }
        };
        if let Some(session) = session_snapshot {
            save_codex_batch_import_session_snapshot_best_effort(&session_id, &session);
        }
        emit_codex_batch_import_progress(
            &app,
            codex_batch_import_progress_from_items(
                &session_id,
                "scanning",
                check_quota,
                items.len(),
                total,
                &items,
                current_label,
            ),
        );
        let preview = {
            let sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
            sessions
                .get(&session_id)
                .map(|session| codex_batch_import_preview_from_session(&session_id, session))
        };
        if let Some(preview) = preview {
            emit_codex_batch_import_preview(&app, preview);
        }
    }

    let status = if cancel.load(Ordering::SeqCst) {
        "cancelled"
    } else if {
        let sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
        sessions
            .get(&session_id)
            .map(|session| session.next_index < session.source_items.len())
            .unwrap_or(false)
    } {
        "cancelled"
    } else {
        "ready"
    };
    let (preview, session_snapshot) = {
        let mut sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
        let session =
            sessions
                .entry(session_id.clone())
                .or_insert_with(|| CodexBatchImportSession {
                    status: status.to_string(),
                    check_quota,
                    cancel: cancel.clone(),
                    source_items: Vec::new(),
                    next_index: 0,
                    total: items.len(),
                    items: Vec::new(),
                });
        session.status = status.to_string();
        session.items = items;
        (
            codex_batch_import_preview_from_session(&session_id, session),
            session.clone(),
        )
    };
    save_codex_batch_import_session_snapshot_best_effort(&session_id, &session_snapshot);
    emit_codex_batch_import_completed(&app, preview);
}

pub fn start_codex_batch_import_from_files(
    app: tauri::AppHandle,
    file_paths: Vec<String>,
    check_quota: bool,
) -> Result<CodexBatchImportStartResult, String> {
    if file_paths.is_empty() {
        return Err("未选择任何文件".to_string());
    }
    ensure_storage_writable_for_import()?;
    let session_id = next_codex_batch_import_session_id();
    let cancel = Arc::new(AtomicBool::new(false));
    let session = CodexBatchImportSession {
        status: "scanning".to_string(),
        check_quota,
        cancel,
        source_items: Vec::new(),
        next_index: 0,
        total: 0,
        items: Vec::new(),
    };
    // 会话快照用于崩溃恢复，失败时保留当前进程内任务，不能阻断批量导入。
    save_codex_batch_import_session_snapshot_best_effort(&session_id, &session);
    {
        let mut sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
        sessions.insert(session_id.clone(), session);
    }
    let task_session_id = session_id.clone();
    tauri::async_runtime::spawn(async move {
        run_codex_batch_import_scan(app, task_session_id, file_paths, check_quota).await;
    });
    Ok(CodexBatchImportStartResult { session_id })
}

pub fn cancel_codex_batch_import(session_id: &str) -> Result<(), String> {
    ensure_codex_batch_import_session_loaded(session_id)?;
    let session_snapshot = {
        let mut sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| "导入会话不存在".to_string())?;
        session.cancel.store(true, Ordering::SeqCst);
        session.status = "cancelled".to_string();
        session.clone()
    };
    save_codex_batch_import_session_snapshot_best_effort(session_id, &session_snapshot);
    Ok(())
}

pub fn resume_codex_batch_import(app: tauri::AppHandle, session_id: &str) -> Result<(), String> {
    {
        ensure_codex_batch_import_session_loaded(session_id)?;
        let mut sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| "导入会话不存在".to_string())?;
        if session.status != "cancelled" {
            return Err("只有已取消的导入会话可以继续".to_string());
        }
        if session.next_index >= session.source_items.len() {
            session.status = "ready".to_string();
            save_codex_batch_import_session_snapshot_best_effort(session_id, session);
            return Ok(());
        }
        session.cancel.store(false, Ordering::SeqCst);
        session.status = "scanning".to_string();
        save_codex_batch_import_session_snapshot_best_effort(session_id, session);
    }

    let task_session_id = session_id.to_string();
    tauri::async_runtime::spawn(async move {
        run_codex_batch_import_resume(app, task_session_id).await;
    });
    Ok(())
}

pub fn get_codex_batch_import_preview(session_id: &str) -> Result<CodexBatchImportPreview, String> {
    ensure_codex_batch_import_session_loaded(session_id)?;
    let sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
    let session = sessions
        .get(session_id)
        .ok_or_else(|| "导入会话不存在".to_string())?;
    Ok(codex_batch_import_preview_from_session(session_id, session))
}

pub fn confirm_codex_batch_import(
    app: &tauri::AppHandle,
    session_id: &str,
    item_ids: &[String],
) -> Result<CodexBatchImportConfirmResult, String> {
    ensure_storage_writable_for_import()?;
    ensure_codex_batch_import_session_loaded(session_id)?;
    let selected: HashSet<String> = item_ids.iter().cloned().collect();
    let (cached_items, cancel, session_snapshot) = {
        let mut sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
        let session = sessions
            .get_mut(session_id)
            .ok_or_else(|| "导入会话不存在".to_string())?;
        session.cancel.store(false, Ordering::SeqCst);
        session.status = "importing".to_string();
        (
            session
                .items
                .iter()
                .filter(|cached| selected.contains(&cached.preview.item_id))
                .cloned()
                .collect::<Vec<_>>(),
            session.cancel.clone(),
            session.clone(),
        )
    };
    save_codex_batch_import_session_snapshot_best_effort(session_id, &session_snapshot);

    let mut imported = Vec::new();
    let mut failed = Vec::new();
    let total = cached_items.len();
    let mut processed = 0usize;
    emit_codex_batch_import_progress(
        app,
        CodexBatchImportProgress {
            session_id: session_id.to_string(),
            phase: "importing".to_string(),
            check_quota: session_snapshot.check_quota,
            current: 0,
            total,
            success: 0,
            failed: 0,
            quota_failed: 0,
            existing: 0,
            current_label: None,
        },
    );

    for cached in cached_items {
        if cancel.load(Ordering::SeqCst) {
            break;
        }
        let current_label = Some(cached.preview.label.clone());
        let Some(draft) = cached.draft else {
            failed.push(CodexFileImportFailure {
                email: cached.preview.label,
                error: cached
                    .preview
                    .error
                    .unwrap_or_else(|| "无可导入账号".to_string()),
            });
            processed += 1;
            emit_codex_batch_import_progress(
                app,
                CodexBatchImportProgress {
                    session_id: session_id.to_string(),
                    phase: "importing".to_string(),
                    check_quota: session_snapshot.check_quota,
                    current: processed,
                    total,
                    success: imported.len(),
                    failed: failed.len(),
                    quota_failed: 0,
                    existing: 0,
                    current_label,
                },
            );
            continue;
        };
        let result = (|| -> Result<CodexAccount, String> {
            let mut account = match draft {
                CodexBatchImportDraft::Account(account) => import_account_struct(account)?,
                CodexBatchImportDraft::FullToken {
                    tokens,
                    account_id_hint,
                    subscription_active_until_hint,
                    note_update,
                } => {
                    let mut account = upsert_account_with_import_hints(
                        tokens,
                        account_id_hint,
                        None,
                        subscription_active_until_hint,
                    )?;
                    save_account_note_update_if_present(&mut account, note_update)?;
                    account
                }
                CodexBatchImportDraft::AccessToken {
                    access_token,
                    hints,
                } => upsert_account_from_access_token_with_hints(access_token, hints)?,
            };
            if let Some(quota) = cached.quota.clone() {
                account.quota = Some(quota);
                account.quota_error = None;
                account.usage_updated_at = Some(chrono::Utc::now().timestamp());
                save_account(&account)?;
            }
            Ok(account)
        })();
        match result {
            Ok(account) => imported.push(account),
            Err(error) => failed.push(CodexFileImportFailure {
                email: cached.preview.label,
                error,
            }),
        }
        processed += 1;
        emit_codex_batch_import_progress(
            app,
            CodexBatchImportProgress {
                session_id: session_id.to_string(),
                phase: "importing".to_string(),
                check_quota: session_snapshot.check_quota,
                current: processed,
                total,
                success: imported.len(),
                failed: failed.len(),
                quota_failed: 0,
                existing: 0,
                current_label,
            },
        );
    }
    let cancelled = cancel.load(Ordering::SeqCst);

    {
        let mut sessions = CODEX_BATCH_IMPORT_SESSIONS.lock().unwrap();
        sessions.remove(session_id);
    }
    remove_codex_batch_import_session_snapshot(session_id);

    Ok(CodexBatchImportConfirmResult {
        imported,
        failed,
        cancelled,
        processed,
        total,
    })
}

fn normalize_auth_file_plan_type(value: Option<&str>) -> Option<String> {
    let normalized = normalize_optional_ref(value)?
        .to_ascii_lowercase()
        .replace('_', "-")
        .replace(' ', "-");

    match normalized.as_str() {
        "prolite" | "pro-lite" => Some("prolite".to_string()),
        "promax" | "pro-max" => Some("promax".to_string()),
        _ => None,
    }
}

fn detect_auth_file_plan_type_from_path(path: &std::path::Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let normalized = stem
        .trim()
        .to_ascii_lowercase()
        .replace('_', "-")
        .replace(' ', "-");

    if normalized.ends_with("-prolite") || normalized.ends_with("-pro-lite") {
        return Some("prolite".to_string());
    }
    if normalized.ends_with("-promax") || normalized.ends_with("-pro-max") {
        return Some("promax".to_string());
    }

    None
}

fn apply_auth_file_plan_type(
    account: &mut CodexAccount,
    auth_file_plan_type: Option<String>,
) -> bool {
    let Some(normalized) = normalize_auth_file_plan_type(auth_file_plan_type.as_deref()) else {
        return false;
    };

    if account.auth_file_plan_type.as_deref() == Some(normalized.as_str()) {
        return false;
    }

    account.auth_file_plan_type = Some(normalized);
    true
}

/// 从单个 JSON 值中提取 CodexTokens
fn extract_codex_tokens_from_value(
    value: &serde_json::Value,
) -> Option<(CodexTokens, Option<String>)> {
    let obj = value.as_object()?;

    // 格式1: 顶层 access_token + id_token（用户导出格式）
    if let (Some(id_token), Some(access_token)) = (
        first_json_string(value, &[&["id_token"], &["idToken"]]),
        first_json_string(value, &[&["access_token"], &["accessToken"]]),
    ) {
        let refresh_token = first_json_string(value, &[&["refresh_token"], &["refreshToken"]]);
        let account_id_hint = first_json_string(value, &[&["account_id"], &["accountId"]]);
        return Some((
            CodexTokens {
                id_token,
                access_token,
                refresh_token,
            },
            account_id_hint,
        ));
    }

    // 格式2: 嵌套 tokens 对象（CodexAuthFile 或 CodexAccount 格式）
    if obj.get("tokens").and_then(|v| v.as_object()).is_some() {
        if let (Some(id_token), Some(access_token)) = (
            first_json_string(value, &[&["tokens", "id_token"], &["tokens", "idToken"]]),
            first_json_string(
                value,
                &[&["tokens", "access_token"], &["tokens", "accessToken"]],
            ),
        ) {
            let refresh_token = first_json_string(
                value,
                &[&["tokens", "refresh_token"], &["tokens", "refreshToken"]],
            );
            let account_id_hint = first_json_string(
                value,
                &[
                    &["tokens", "account_id"],
                    &["tokens", "accountId"],
                    &["account_id"],
                    &["accountId"],
                ],
            );
            return Some((
                CodexTokens {
                    id_token,
                    access_token,
                    refresh_token,
                },
                account_id_hint,
            ));
        }
    }

    None
}

fn extract_codex_tokens_from_credentials_value(
    value: &serde_json::Value,
) -> Option<(CodexTokens, Option<String>)> {
    let obj = value.as_object()?;
    if obj
        .get("credentials")
        .and_then(|value| value.as_object())
        .is_some()
    {
        if let (Some(id_token), Some(access_token)) = (
            first_json_string(
                value,
                &[&["credentials", "id_token"], &["credentials", "idToken"]],
            ),
            first_json_string(
                value,
                &[
                    &["credentials", "access_token"],
                    &["credentials", "accessToken"],
                ],
            ),
        ) {
            let refresh_token = first_json_string(
                value,
                &[
                    &["credentials", "refresh_token"],
                    &["credentials", "refreshToken"],
                ],
            );
            let account_id_hint = first_json_string(
                value,
                &[
                    &["credentials", "account_id"],
                    &["credentials", "accountId"],
                    &["credentials", "chatgpt_account_id"],
                    &["credentials", "chatgptAccountId"],
                    &["credentials", "workspace_id"],
                    &["credentials", "workspaceId"],
                    &["account_id"],
                    &["accountId"],
                ],
            );
            return Some((
                CodexTokens {
                    id_token,
                    access_token,
                    refresh_token,
                },
                account_id_hint,
            ));
        }
    }

    None
}

