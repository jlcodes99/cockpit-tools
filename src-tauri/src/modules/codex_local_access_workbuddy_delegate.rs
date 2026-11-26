// Explicit WorkBuddy Native DelegateTool configuration and isolated Codex profiles.
// Included in codex_local_access's parent scope; never selects a fallback source.
use crate::models::codex_local_access::CodexLocalAccessWorkbuddyDelegate;

const WORKBUDDY_DELEGATE_PROFILE_OWNER: &str = "cockpit-workbuddy-native-delegate-v1";
const WORKBUDDY_DELEGATE_PROVIDER_ID: &str = "cockpit_workbuddy_native_delegate";
const WORKBUDDY_DELEGATE_PROFILE_MARKER: &str = "cockpit-workbuddy-profile.json";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexWorkbuddyDelegateModel {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexWorkbuddyDelegateCatalog {
    pub models: Vec<CodexWorkbuddyDelegateModel>,
    pub current_model_id: String,
    pub reasoning_efforts: Vec<String>,
    pub current_reasoning_effort: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexWorkbuddyDelegateProfile {
    pub id: String,
    pub api_key_id: String,
    pub workbuddy_account_id: String,
    pub profile_dir: String,
    pub public_model: String,
    pub model_id: String,
    pub reasoning_effort: String,
    pub launch_command: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkbuddyDelegateProfileBackup {
    owner: String,
    profile: CodexWorkbuddyDelegateProfile,
    configuration: CodexLocalAccessWorkbuddyDelegate,
    local_key_sha256: String,
    // The profile is created in a new directory: restore means removing only
    // these unchanged owned files, never overwriting another user's profile.
    config_sha256: String,
    auth_sha256: String,
    config_was_absent: bool,
    auth_was_absent: bool,
}

fn workbuddy_delegate_validate_config(
    config: &CodexLocalAccessWorkbuddyDelegate,
) -> Result<(), String> {
    let valid_id = |value: &str| {
        !value.is_empty()
            && value.len() <= 160
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._/:".contains(&b))
    };
    let automatic_alias = |value: &str| {
        ["auto", "default", "enabled"]
            .iter()
            .any(|alias| value.eq_ignore_ascii_case(alias))
    };
    if config.provider_account_id.trim().is_empty()
        || automatic_alias(&config.model_id)
        || automatic_alias(&config.reasoning_effort)
        || !valid_id(&config.model_id)
        || config.public_model != format!("workbuddy/{}", config.model_id)
        || !valid_id(&config.reasoning_effort)
    {
        return Err("WORKBUDDY_DELEGATE_INVALID_CONFIGURATION".into());
    }
    Ok(())
}

fn workbuddy_delegate_validate_key_scope(
    collection: &CodexLocalAccessCollection,
    key: &CodexLocalAccessApiKey,
    config: &CodexLocalAccessWorkbuddyDelegate,
    model: &str,
) -> Result<(), native_contract::ServiceError> {
    workbuddy_delegate_validate_config(config).map_err(|_| {
        native_error(
            409,
            "workbuddy_delegate_configuration_invalid",
            "WorkBuddy bridge configuration is invalid",
        )
    })?;
    if collection.gateway_mode != CodexLocalAccessGatewayMode::Native {
        return Err(native_error(
            409,
            "workbuddy_delegate_native_required",
            "WorkBuddy tool bridging requires the Native API service",
        ));
    }
    if key.inherit_account_pool != Some(false)
        || key.account_pool_id.is_some()
        || key.account_ids != [config.provider_account_id.clone()]
        || key.provider_gateway.is_some()
        || key.model_routing.is_some()
        || key.model_prefix.is_some()
        || key.allowed_models != [config.public_model.clone()]
        || key.preferred_account_id.is_some()
        || !key.priority_account_ids.is_empty()
    {
        return Err(native_error(
            409,
            "workbuddy_delegate_scope_invalid",
            "WorkBuddy bridge must use its dedicated fixed-source API key",
        ));
    }
    if model != config.public_model || native_model_for_key(collection, Some(key), model)? != model
    {
        return Err(native_error(
            403,
            "model_not_allowed",
            "Only the exact configured WorkBuddy model is allowed",
        ));
    }
    if service_account_paused(collection, &config.provider_account_id) {
        return Err(native_error(
            403,
            "workbuddy_delegate_source_paused",
            "The configured WorkBuddy source is paused",
        ));
    }
    Ok(())
}

fn workbuddy_delegate_validate_source(
    source: &crate::models::workbuddy::WorkbuddyAccount,
) -> Result<(), String> {
    if source
        .enterprise_id
        .as_deref()
        .is_some_and(|id| !id.trim().is_empty())
    {
        // The installed CLI bootstrap currently accepts only a personal token;
        // without a proven tenant pin, silently selecting its default is unsafe.
        return Err("WORKBUDDY_DELEGATE_ENTERPRISE_UNSUPPORTED".into());
    }
    if source.access_token.trim().is_empty()
        || source.expires_at.is_some_and(|expiry| expiry <= now_ms())
        || source
            .status
            .as_deref()
            .is_some_and(|status| !matches!(status, "normal" | "active" | "ok"))
    {
        return Err("WORKBUDDY_DELEGATE_SOURCE_UNAVAILABLE".into());
    }
    Ok(())
}

fn workbuddy_delegate_source_generation(
    reference: &str,
    source: &crate::models::workbuddy::WorkbuddyAccount,
) -> String {
    // Credential changes invalidate the actor, without storing or exposing the
    // credential itself in a Route, public identifier, profile, or log.
    let credential_hash = format!("{:x}", Sha256::digest(source.access_token.as_bytes()));
    let identity = json!([
        reference,
        source.id,
        source.uid,
        source.enterprise_id,
        source.domain,
        credential_hash
    ]);
    format!("{:x}", Sha256::digest(identity.to_string().as_bytes()))
}

fn workbuddy_delegate_domestic(
    _source: &crate::models::workbuddy::WorkbuddyAccount,
) -> Result<bool, String> {
    // WorkBuddy is the domestic platform; WorkBuddy AI has a separate account
    // module. source.domain is an X-Domain tenant routing value, NOT a region.
    Ok(true)
}

async fn native_workbuddy_delegate_selection(
    host: &NativeHostBridge,
    caller: &native_contract::Caller,
    public_model: &str,
) -> Result<Option<native_contract::WorkbuddyDelegateSelection>, native_contract::ServiceError> {
    let collection = host.collection().await?;
    let Some(key) = native_caller_key(&collection, caller)? else {
        return Ok(None);
    };
    let Some(config) = key.workbuddy_delegate.as_ref() else {
        return Ok(None);
    };
    workbuddy_delegate_validate_key_scope(&collection, key, config, public_model)?;
    let config = config.clone();
    let paused = collection.paused_account_ids.clone();
    #[cfg(test)]
    let fixture_accounts = host.account_overrides.clone();
    #[cfg(test)]
    let fixture_sources = host.workbuddy_delegate_test_sources.clone();
    #[cfg(test)]
    let fixture_only = !fixture_accounts.is_empty() || !fixture_sources.is_empty();
    let (account, source) = tokio::task::spawn_blocking(move || {
        #[cfg(test)]
        let loaded_account = if fixture_only { fixture_accounts.get(&config.provider_account_id).cloned() }
            else { codex_account::load_account(&config.provider_account_id) };
        #[cfg(not(test))]
        let loaded_account = codex_account::load_account(&config.provider_account_id);
        let account = loaded_account
            .ok_or_else(|| "WORKBUDDY_DELEGATE_SOURCE_UNAVAILABLE".to_string())?;
        if account.id != config.provider_account_id {
            return Err("WORKBUDDY_DELEGATE_SOURCE_UNAVAILABLE".into());
        }
        native_workbuddy::validate_reference(&account)?;
        let reference = account.api_credential_ref.as_deref().ok_or("WORKBUDDY_DELEGATE_SOURCE_UNAVAILABLE")?;
        if paused.iter().any(|id| id == reference) || account.requires_reauth {
            return Err("WORKBUDDY_DELEGATE_SOURCE_UNAVAILABLE".into());
        }
        #[cfg(test)]
        let loaded_source = if fixture_only { fixture_sources.get(reference).cloned() }
            else { codex_account::workbuddy_upstream_account(&account) };
        #[cfg(not(test))]
        let loaded_source = codex_account::workbuddy_upstream_account(&account);
        let source = loaded_source
            .ok_or_else(|| "WORKBUDDY_DELEGATE_SOURCE_UNAVAILABLE".to_string())?;
        if source.id != reference {
            return Err("WORKBUDDY_DELEGATE_SOURCE_UNAVAILABLE".into());
        }
        workbuddy_delegate_validate_source(&source)?;
        Ok::<_, String>((account, source))
    }).await.map_err(|_| native_error(503, "workbuddy_delegate_unavailable", "Unable to read the configured WorkBuddy source"))?
      .map_err(|_| native_error(403, "workbuddy_delegate_source_unavailable", "The configured WorkBuddy source is unavailable; restore it or start a new conversation"))?;
    let config = key.workbuddy_delegate.as_ref().unwrap();
    let mut route = native_workbuddy::route(
        &account,
        codex_account::WORKBUDDY_ACP_BASE_URL,
        config.model_id.clone(),
        None,
    )?;
    route.base_url = "workbuddy-delegate://native".into();
    let domestic = workbuddy_delegate_domestic(&source).map_err(|_| {
        native_error(
            409,
            "workbuddy_delegate_region_unsupported",
            "This WorkBuddy source region is unsupported",
        )
    })?;
    let reference = account.api_credential_ref.as_deref().unwrap_or_default();
    Ok(Some(native_contract::WorkbuddyDelegateSelection {
        route,
        source_generation: workbuddy_delegate_source_generation(reference, &source),
        access_token: source.access_token,
        domestic,
        effort: config.reasoning_effort.clone(),
    }))
}

fn workbuddy_delegate_catalog(config: &CodexLocalAccessWorkbuddyDelegate) -> Value {
    json!({"object":"list", "data":[{
        "id":config.public_model, "object":"model", "created":0, "owned_by":"workbuddy",
        "display_name":format!("WorkBuddy / {}", config.model_id),
        "capabilities":["completion","tools","thinking"],
        "supported_endpoints":["responses"], "supports_websockets":false,
        "supports_images":false, "supports_hosted_tools":false,
        "model_info":{"upstream_id":config.model_id, "input_modalities":["text"],
            "supported_reasoning_efforts":[config.reasoning_effort], "default_reasoning_effort":config.reasoning_effort}
    }]})
}

fn workbuddy_delegate_parse_catalog(raw: &Value) -> Result<CodexWorkbuddyDelegateCatalog, String> {
    let current = raw
        .pointer("/models/currentModelId")
        .and_then(Value::as_str)
        .ok_or("WORKBUDDY_DELEGATE_CATALOG_INVALID")?;
    let available = raw
        .pointer("/models/availableModels")
        .and_then(Value::as_array)
        .ok_or("WORKBUDDY_DELEGATE_CATALOG_INVALID")?;
    let mut models = Vec::new();
    for model in available {
        let Some(id) = model.get("modelId").and_then(Value::as_str) else {
            continue;
        };
        if !models
            .iter()
            .any(|item: &CodexWorkbuddyDelegateModel| item.id == id)
        {
            models.push(CodexWorkbuddyDelegateModel {
                id: id.into(),
                name: model
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(id)
                    .into(),
            });
        }
    }
    if models.is_empty() || !models.iter().any(|model| model.id == current) {
        return Err("WORKBUDDY_DELEGATE_CATALOG_INVALID".into());
    }
    let options = raw
        .get("configOptions")
        .and_then(Value::as_array)
        .ok_or("WORKBUDDY_DELEGATE_EFFORT_UNAVAILABLE")?;
    let effort_options: Vec<_> = options
        .iter()
        .filter(|option| {
            let id = option["id"]
                .as_str()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let category = option["category"]
                .as_str()
                .unwrap_or_default()
                .to_ascii_lowercase();
            matches!(
                category.as_str(),
                "thought_level" | "thinking" | "reasoning" | "effort"
            ) || matches!(
                id.as_str(),
                "thinking" | "thought_level" | "reasoning_effort" | "effort" | "thinkinglevel"
            )
        })
        .collect();
    if effort_options.len() != 1 {
        return Err("WORKBUDDY_DELEGATE_EFFORT_UNAVAILABLE".into());
    }
    let option = effort_options[0];
    let mut efforts = Vec::new();
    fn collect_options(value: &Value, into: &mut Vec<String>) {
        if let Some(items) = value.as_array() {
            for item in items {
                if let Some(value) = item.get("value").and_then(Value::as_str) {
                    if !into.iter().any(|existing| existing == value) {
                        into.push(value.into());
                    }
                } else if let Some(nested) = item.get("options") {
                    collect_options(nested, into);
                }
            }
        }
    }
    collect_options(&option["options"], &mut efforts);
    let selected_model = available
        .iter()
        .find(|model| model["modelId"].as_str() == Some(current))
        .ok_or("WORKBUDDY_DELEGATE_CATALOG_INVALID")?;
    let supported: Vec<&str> = selected_model
        .pointer("/_meta/effortOptions")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().or_else(|| item["value"].as_str()))
                .collect()
        })
        .unwrap_or_default();
    // Global CLI config options include aliases such as "enabled" which do not
    // pin an exact effort. Only the selected model's explicit values are valid.
    efforts.retain(|value| {
        supported.contains(&value.as_str())
            && !matches!(value.as_str(), "enabled" | "default" | "auto")
    });
    let current_effort = option
        .get("currentValue")
        .and_then(Value::as_str)
        .filter(|value| efforts.iter().any(|effort| effort == value))
        .map(str::to_owned);
    Ok(CodexWorkbuddyDelegateCatalog {
        models,
        current_model_id: current.into(),
        reasoning_efforts: efforts,
        current_reasoning_effort: current_effort,
    })
}

pub async fn get_workbuddy_delegate_catalog(
    workbuddy_account_id: String,
    model_id: Option<String>,
) -> Result<CodexWorkbuddyDelegateCatalog, String> {
    let source = tokio::task::spawn_blocking(move || {
        let source = crate::modules::workbuddy_account::load_account(&workbuddy_account_id)
            .ok_or("WORKBUDDY_DELEGATE_SOURCE_UNAVAILABLE")?;
        workbuddy_delegate_validate_source(&source)?;
        workbuddy_delegate_domestic(&source)?;
        Ok::<_, String>(source)
    })
    .await
    .map_err(|_| "WORKBUDDY_DELEGATE_SOURCE_UNAVAILABLE")??;
    let raw = match model_id.filter(|model| !model.trim().is_empty()) {
        Some(model) => native_api::workbuddy_delegate_catalog_for_model(source, model).await,
        None => native_api::workbuddy_delegate_catalog_for_source(source).await,
    }
    .map_err(|_| "WORKBUDDY_DELEGATE_CATALOG_UNAVAILABLE".to_string())?;
    workbuddy_delegate_parse_catalog(&raw)
}

fn workbuddy_delegate_profile_root() -> Result<PathBuf, String> {
    Ok(local_access_sidecar_dir()?.join("workbuddy-delegate-profiles"))
}

fn workbuddy_delegate_profile_id_valid(id: &str) -> bool {
    id.strip_prefix("wb-native-")
        .is_some_and(|suffix| suffix.len() == 32 && suffix.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn workbuddy_delegate_hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn workbuddy_delegate_launch_command(path: &Path) -> String {
    let raw = path.to_string_lossy();
    #[cfg(windows)]
    {
        format!("$env:CODEX_HOME = '{}'; codex", raw.replace('\'', "''"))
    }
    #[cfg(not(windows))]
    {
        format!("CODEX_HOME='{}' codex", raw.replace('\'', "'\"'\"'"))
    }
}

fn workbuddy_delegate_profile_contents(
    config: &CodexLocalAccessWorkbuddyDelegate,
    local_key: &str,
    port: u16,
) -> Result<(String, String), String> {
    workbuddy_delegate_validate_config(config)?;
    if local_key.trim().is_empty() || port == 0 {
        return Err("WORKBUDDY_DELEGATE_PROFILE_INVALID".into());
    }
    let mut doc = toml_edit::Document::new();
    doc["model_provider"] = toml_edit::value(WORKBUDDY_DELEGATE_PROVIDER_ID);
    doc["model"] = toml_edit::value(config.public_model.as_str());
    doc["model_reasoning_effort"] = toml_edit::value(config.reasoning_effort.as_str());
    doc["cli_auth_credentials_store"] = toml_edit::value("file");
    // Codex defaults to a hosted web_search declaration. This provider cannot
    // execute hosted tools; declare that production capability boundary in its
    // isolated profile rather than stripping tools from a client request.
    doc["web_search"] = toml_edit::value("disabled");
    doc["model_providers"] = toml_edit::table();
    let provider = &mut doc["model_providers"][WORKBUDDY_DELEGATE_PROVIDER_ID];
    *provider = toml_edit::table();
    provider["name"] = toml_edit::value("WorkBuddy Native Tool Bridge");
    provider["base_url"] = toml_edit::value(format!("http://127.0.0.1:{port}/v1"));
    provider["wire_api"] = toml_edit::value("responses");
    provider["requires_openai_auth"] = toml_edit::value(false);
    provider["experimental_bearer_token"] = toml_edit::value(local_key);
    provider["supports_websockets"] = toml_edit::value(false);
    provider["request_max_retries"] = toml_edit::value(0i64);
    provider["stream_max_retries"] = toml_edit::value(0i64);
    let auth = serde_json::to_string_pretty(&json!({"OPENAI_API_KEY":local_key}))
        .map_err(|_| "WORKBUDDY_DELEGATE_PROFILE_INVALID")?;
    Ok((doc.to_string(), auth))
}

fn workbuddy_delegate_write_profile(
    root: &Path,
    key: &CodexLocalAccessApiKey,
    workbuddy_account_id: &str,
    port: u16,
) -> Result<CodexWorkbuddyDelegateProfile, String> {
    let config = key
        .workbuddy_delegate
        .as_ref()
        .ok_or("WORKBUDDY_DELEGATE_PROFILE_INVALID")?;
    fs::create_dir_all(root).map_err(|_| "WORKBUDDY_DELEGATE_PROFILE_WRITE_FAILED")?;
    if fs::symlink_metadata(root)
        .map_err(|_| "WORKBUDDY_DELEGATE_PROFILE_WRITE_FAILED")?
        .file_type()
        .is_symlink()
    {
        return Err("WORKBUDDY_DELEGATE_PROFILE_UNSAFE".into());
    }
    let id = format!("wb-native-{}", uuid::Uuid::new_v4().simple());
    let dir = root.join(&id);
    fs::create_dir(&dir).map_err(|_| "WORKBUDDY_DELEGATE_PROFILE_WRITE_FAILED")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
            .map_err(|_| "WORKBUDDY_DELEGATE_PROFILE_WRITE_FAILED")?;
    }
    let (config_content, auth_content) =
        workbuddy_delegate_profile_contents(config, &key.key, port)?;
    let profile = CodexWorkbuddyDelegateProfile {
        id,
        api_key_id: key.id.clone(),
        workbuddy_account_id: workbuddy_account_id.into(),
        profile_dir: dir.to_string_lossy().into(),
        public_model: config.public_model.clone(),
        model_id: config.model_id.clone(),
        reasoning_effort: config.reasoning_effort.clone(),
        launch_command: workbuddy_delegate_launch_command(&dir),
    };
    let backup = WorkbuddyDelegateProfileBackup {
        owner: WORKBUDDY_DELEGATE_PROFILE_OWNER.into(),
        profile: profile.clone(),
        configuration: config.clone(),
        local_key_sha256: workbuddy_delegate_hash(&key.key),
        config_sha256: workbuddy_delegate_hash(&config_content),
        auth_sha256: workbuddy_delegate_hash(&auth_content),
        config_was_absent: true,
        auth_was_absent: true,
    };
    write_secret_string_atomic(&dir.join("config.toml"), &config_content)
        .map_err(|_| "WORKBUDDY_DELEGATE_PROFILE_WRITE_FAILED")?;
    write_secret_string_atomic(&dir.join("auth.json"), &auth_content)
        .map_err(|_| "WORKBUDDY_DELEGATE_PROFILE_WRITE_FAILED")?;
    write_secret_string_atomic(
        &dir.join(WORKBUDDY_DELEGATE_PROFILE_MARKER),
        &serde_json::to_string_pretty(&backup)
            .map_err(|_| "WORKBUDDY_DELEGATE_PROFILE_WRITE_FAILED")?,
    )
    .map_err(|_| "WORKBUDDY_DELEGATE_PROFILE_WRITE_FAILED")?;
    Ok(profile)
}

fn workbuddy_delegate_read_backup(
    root: &Path,
    id: &str,
) -> Result<(PathBuf, WorkbuddyDelegateProfileBackup), String> {
    if !workbuddy_delegate_profile_id_valid(id) {
        return Err("WORKBUDDY_DELEGATE_PROFILE_NOT_OWNED".into());
    }
    let dir = root.join(id);
    if fs::symlink_metadata(root)
        .map_err(|_| "WORKBUDDY_DELEGATE_PROFILE_NOT_OWNED")?
        .file_type()
        .is_symlink()
        || fs::symlink_metadata(&dir)
            .map_err(|_| "WORKBUDDY_DELEGATE_PROFILE_NOT_OWNED")?
            .file_type()
            .is_symlink()
    {
        return Err("WORKBUDDY_DELEGATE_PROFILE_NOT_OWNED".into());
    }
    let marker = dir.join(WORKBUDDY_DELEGATE_PROFILE_MARKER);
    if fs::symlink_metadata(&marker)
        .map_err(|_| "WORKBUDDY_DELEGATE_PROFILE_NOT_OWNED")?
        .file_type()
        .is_symlink()
    {
        return Err("WORKBUDDY_DELEGATE_PROFILE_NOT_OWNED".into());
    }
    let mut backup: WorkbuddyDelegateProfileBackup = serde_json::from_str(
        &fs::read_to_string(marker).map_err(|_| "WORKBUDDY_DELEGATE_PROFILE_NOT_OWNED")?,
    )
    .map_err(|_| "WORKBUDDY_DELEGATE_PROFILE_NOT_OWNED")?;
    if backup.owner != WORKBUDDY_DELEGATE_PROFILE_OWNER
        || backup.profile.id != id
        || Path::new(&backup.profile.profile_dir) != dir
        || !backup.config_was_absent
        || !backup.auth_was_absent
        || backup.profile.public_model != backup.configuration.public_model
        || backup.profile.model_id != backup.configuration.model_id
        || backup.profile.reasoning_effort != backup.configuration.reasoning_effort
    {
        return Err("WORKBUDDY_DELEGATE_PROFILE_NOT_OWNED".into());
    }
    workbuddy_delegate_validate_config(&backup.configuration)?;
    // Never execute or display a command taken verbatim from a mutable marker.
    backup.profile.launch_command = workbuddy_delegate_launch_command(&dir);
    Ok((dir, backup))
}

fn workbuddy_delegate_check_profile_unchanged(
    dir: &Path,
    backup: &WorkbuddyDelegateProfileBackup,
) -> Result<(), String> {
    for (name, expected) in [
        ("config.toml", &backup.config_sha256),
        ("auth.json", &backup.auth_sha256),
    ] {
        let path = dir.join(name);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err("WORKBUDDY_DELEGATE_PROFILE_CHANGED".into()),
        };
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || workbuddy_delegate_hash(
                &fs::read_to_string(path).map_err(|_| "WORKBUDDY_DELEGATE_PROFILE_CHANGED")?,
            ) != *expected
        {
            return Err("WORKBUDDY_DELEGATE_PROFILE_CHANGED".into());
        }
    }
    Ok(())
}

fn workbuddy_delegate_restore_owned_files(
    root: &Path,
    id: &str,
    expected: &WorkbuddyDelegateProfileBackup,
) -> Result<(), String> {
    // Recheck ownership after any awaited collection transaction. A moved,
    // replaced, modified or symlinked profile is not an authorized target.
    let (dir, backup) = workbuddy_delegate_read_backup(root, id)?;
    if backup.profile.api_key_id != expected.profile.api_key_id
        || backup.configuration != expected.configuration
        || backup.local_key_sha256 != expected.local_key_sha256
        || backup.config_sha256 != expected.config_sha256
        || backup.auth_sha256 != expected.auth_sha256
    {
        return Err("WORKBUDDY_DELEGATE_PROFILE_CHANGED".into());
    }
    workbuddy_delegate_check_profile_unchanged(&dir, &backup)?;
    // Never remove the directory: Codex may have added user sessions/state.
    for name in [
        "config.toml",
        "auth.json",
        WORKBUDDY_DELEGATE_PROFILE_MARKER,
    ] {
        match fs::remove_file(dir.join(name)) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err("WORKBUDDY_DELEGATE_PROFILE_RESTORE_FAILED".into()),
        }
    }
    Ok(())
}

pub async fn create_workbuddy_delegate_profile(
    workbuddy_account_id: String,
    model_id: String,
    reasoning_effort: String,
) -> Result<CodexWorkbuddyDelegateProfile, String> {
    ensure_runtime_loaded_without_start().await?;
    // Fail before launching a metadata CLI when the selected service mode cannot work.
    {
        let runtime = gateway_runtime().lock().await;
        let collection = runtime
            .collection
            .as_ref()
            .ok_or("WORKBUDDY_DELEGATE_NATIVE_SERVICE_REQUIRED")?;
        if !collection.enabled || collection.gateway_mode != CodexLocalAccessGatewayMode::Native {
            return Err("WORKBUDDY_DELEGATE_NATIVE_SERVICE_REQUIRED".into());
        }
    }
    let catalog =
        get_workbuddy_delegate_catalog(workbuddy_account_id.clone(), Some(model_id.clone()))
            .await?;
    if catalog.current_model_id != model_id
        || !catalog.reasoning_efforts.contains(&reasoning_effort)
    {
        return Err("WORKBUDDY_DELEGATE_MODEL_OR_EFFORT_UNAVAILABLE".into());
    }
    let source_id = workbuddy_account_id.clone();
    let account = tokio::task::spawn_blocking(move || {
        codex_account::upsert_workbuddy_provider_account(&source_id, None)
    })
    .await
    .map_err(|_| "WORKBUDDY_DELEGATE_SOURCE_UNAVAILABLE")?
    .map_err(|_| "WORKBUDDY_DELEGATE_SOURCE_UNAVAILABLE")?;
    let config = CodexLocalAccessWorkbuddyDelegate {
        provider_account_id: account.id.clone(),
        public_model: format!("workbuddy/{model_id}"),
        model_id,
        reasoning_effort,
    };
    workbuddy_delegate_validate_config(&config)?;
    let mut key =
        build_local_access_api_key(Some(&format!("WorkBuddy Native: {}", config.model_id)));
    key.inherit_account_pool = Some(false);
    key.account_pool_id = None;
    key.account_ids = vec![account.id];
    key.allowed_models = vec![config.public_model.clone()];
    key.workbuddy_delegate = Some(config);
    let root = workbuddy_delegate_profile_root()?;
    let profile = {
        // Read the latest collection only while holding the runtime lock; never
        // publish a pre-catalog snapshot over intervening key/settings changes.
        let mut runtime = gateway_runtime().lock().await;
        let mut collection = runtime
            .collection
            .clone()
            .ok_or("WORKBUDDY_DELEGATE_NATIVE_SERVICE_REQUIRED")?;
        if !collection.enabled || collection.gateway_mode != CodexLocalAccessGatewayMode::Native {
            return Err("WORKBUDDY_DELEGATE_NATIVE_SERVICE_REQUIRED".into());
        }
        workbuddy_delegate_validate_key_scope(
            &collection,
            &key,
            key.workbuddy_delegate.as_ref().unwrap(),
            &key.workbuddy_delegate.as_ref().unwrap().public_model,
        )
        .map_err(|_| "WORKBUDDY_DELEGATE_SCOPE_UNAVAILABLE")?;
        let profile =
            workbuddy_delegate_write_profile(&root, &key, &workbuddy_account_id, collection.port)?;
        normalize_collection_api_keys(&mut collection);
        collection.api_keys.push(key);
        collection.updated_at = now_ms();
        if save_collection_to_disk(&collection).is_err() {
            if let Ok((_, backup)) = workbuddy_delegate_read_backup(&root, &profile.id) {
                let _ = workbuddy_delegate_restore_owned_files(&root, &profile.id, &backup);
            }
            return Err("WORKBUDDY_DELEGATE_CONFIGURATION_WRITE_FAILED".into());
        }
        sync_runtime_collection(&mut runtime, collection);
        profile
    };
    ensure_gateway_matches_runtime()
        .await
        .map_err(|_| "WORKBUDDY_DELEGATE_SERVICE_START_FAILED")?;
    emit_local_access_state_updated();
    Ok(profile)
}

pub fn list_workbuddy_delegate_profiles() -> Result<Vec<CodexWorkbuddyDelegateProfile>, String> {
    let root = workbuddy_delegate_profile_root()?;
    if !root.exists() {
        return Ok(Vec::new());
    }
    if fs::symlink_metadata(&root)
        .map_err(|_| "WORKBUDDY_DELEGATE_PROFILE_READ_FAILED")?
        .file_type()
        .is_symlink()
    {
        return Err("WORKBUDDY_DELEGATE_PROFILE_UNSAFE".into());
    }
    let mut profiles = Vec::new();
    for entry in fs::read_dir(&root)
        .map_err(|_| "WORKBUDDY_DELEGATE_PROFILE_READ_FAILED")?
        .flatten()
    {
        let id = entry.file_name().to_string_lossy().into_owned();
        if let Ok((_, backup)) = workbuddy_delegate_read_backup(&root, &id) {
            profiles.push(backup.profile);
        }
    }
    Ok(profiles)
}

fn workbuddy_delegate_revoke_owned_key(
    collection: &mut CodexLocalAccessCollection,
    backup: &WorkbuddyDelegateProfileBackup,
) -> Result<bool, String> {
    let Some(key) = collection
        .api_keys
        .iter_mut()
        .find(|key| key.id == backup.profile.api_key_id)
    else {
        // Already deleted is already revoked; it does not authorize modifying
        // any replacement key with a different identity.
        return Ok(false);
    };
    let config = key
        .workbuddy_delegate
        .as_ref()
        .ok_or("WORKBUDDY_DELEGATE_PROFILE_NOT_OWNED")?;
    if *config != backup.configuration
        || workbuddy_delegate_hash(&key.key) != backup.local_key_sha256
    {
        return Err("WORKBUDDY_DELEGATE_PROFILE_NOT_OWNED".into());
    }
    if !key.enabled {
        return Ok(false);
    }
    key.enabled = false;
    key.updated_at = now_ms();
    collection.updated_at = now_ms();
    Ok(true)
}

pub async fn restore_workbuddy_delegate_profile(profile_id: String) -> Result<(), String> {
    ensure_runtime_loaded_without_start().await?;
    let root = workbuddy_delegate_profile_root()?;
    let (_, backup) = workbuddy_delegate_read_backup(&root, &profile_id)?;
    {
        let mut runtime = gateway_runtime().lock().await;
        let mut collection = runtime
            .collection
            .clone()
            .ok_or("WORKBUDDY_DELEGATE_CONFIGURATION_UNAVAILABLE")?;
        if workbuddy_delegate_revoke_owned_key(&mut collection, &backup)? {
            // Revoke first. A user's changes to profile files must never prevent
            // withdrawal of a matching owned key's authorization.
            save_collection_to_disk(&collection)
                .map_err(|_| "WORKBUDDY_DELEGATE_CONFIGURATION_WRITE_FAILED")?;
            sync_runtime_collection(&mut runtime, collection);
        }
    }
    emit_local_access_state_updated();
    // Changed files, symlinks or replaced ownership are preserved. The marker
    // remains visible for a later retry, but the matching key is already off.
    workbuddy_delegate_restore_owned_files(&root, &profile_id, &backup)
        .map_err(|_| "WORKBUDDY_DELEGATE_KEY_REVOKED_FILES_PRESERVED".to_string())?;
    Ok(())
}

#[cfg(test)]
mod workbuddy_delegate_config_tests {
    use super::*;

    fn config() -> CodexLocalAccessWorkbuddyDelegate {
        CodexLocalAccessWorkbuddyDelegate {
            provider_account_id: "linked-workbuddy".into(),
            model_id: "glm-5.3-flash".into(),
            public_model: "workbuddy/glm-5.3-flash".into(),
            reasoning_effort: "low".into(),
        }
    }
    fn key() -> CodexLocalAccessApiKey {
        let mut key = build_local_access_api_key(Some("Synthetic delegate fixture"));
        key.key = "synthetic-scoped-local-key".into();
        key.inherit_account_pool = Some(false);
        key.account_ids = vec![config().provider_account_id.clone()];
        key.allowed_models = vec![config().public_model.clone()];
        key.workbuddy_delegate = Some(config());
        key
    }
    fn collection() -> CodexLocalAccessCollection {
        let mut collection = new_empty_local_access_collection().unwrap();
        collection.enabled = true;
        collection.gateway_mode = CodexLocalAccessGatewayMode::Native;
        collection
    }
    fn source() -> crate::models::workbuddy::WorkbuddyAccount {
        serde_json::from_value(
            json!({"id":"synthetic-source","email":"fixture@example.invalid",
            "uid":"synthetic-uid", "access_token":"synthetic-workbuddy-secret", "status":"normal",
            "created_at":1,"last_used":1}),
        )
        .unwrap()
    }

    #[test]
    fn workbuddy_delegate_configuration_rejects_automatic_model_and_effort_aliases() {
        assert!(workbuddy_delegate_validate_config(&config()).is_ok());
        for alias in ["auto", "AUTO", "default", "Default", "enabled", "Enabled"] {
            let mut selected = config();
            selected.model_id = alias.into();
            selected.public_model = format!("workbuddy/{alias}");
            assert!(
                workbuddy_delegate_validate_config(&selected).is_err(),
                "model {alias} must not select an implicit provider model"
            );
            let mut selected = config();
            selected.reasoning_effort = alias.into();
            assert!(
                workbuddy_delegate_validate_config(&selected).is_err(),
                "effort {alias} must not select an implicit level"
            );
        }
    }

    #[test]
    fn workbuddy_delegate_key_serialization_preserves_old_keys_and_explicit_mode() {
        let old = build_local_access_api_key(None);
        let old_json = serde_json::to_value(&old).unwrap();
        assert!(old_json.get("workbuddyDelegate").is_none());
        let restored: CodexLocalAccessApiKey = serde_json::from_value(old_json).unwrap();
        assert!(restored.workbuddy_delegate.is_none());
        let selected = key();
        let json = serde_json::to_value(&selected).unwrap();
        assert_eq!(
            json["workbuddyDelegate"]["publicModel"],
            "workbuddy/glm-5.3-flash"
        );
        let roundtrip: CodexLocalAccessApiKey = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip.workbuddy_delegate, Some(config()));
    }

    #[test]
    fn workbuddy_delegate_scope_is_fixed_and_respects_model_policy_and_pause() {
        let mut collection = collection();
        let mut selected = key();
        let model = config().public_model;
        assert!(
            workbuddy_delegate_validate_key_scope(&collection, &selected, &config(), &model)
                .is_ok()
        );
        assert!(workbuddy_delegate_validate_key_scope(
            &collection,
            &selected,
            &config(),
            "gpt-5.4"
        )
        .is_err());
        selected.account_ids.push("oauth-other".into());
        assert!(
            workbuddy_delegate_validate_key_scope(&collection, &selected, &config(), &model)
                .is_err()
        );
        selected = key();
        selected.inherit_account_pool = Some(true);
        assert!(
            workbuddy_delegate_validate_key_scope(&collection, &selected, &config(), &model)
                .is_err()
        );
        selected = key();
        collection
            .paused_account_ids
            .push(config().provider_account_id);
        assert!(
            workbuddy_delegate_validate_key_scope(&collection, &selected, &config(), &model)
                .is_err()
        );
        collection.paused_account_ids.clear();
        collection.excluded_models.push(model.clone());
        assert!(
            workbuddy_delegate_validate_key_scope(&collection, &selected, &config(), &model)
                .is_err()
        );
        collection.excluded_models.clear();
        collection.gateway_mode = CodexLocalAccessGatewayMode::Sidecar;
        assert!(
            workbuddy_delegate_validate_key_scope(&collection, &selected, &config(), &model)
                .is_err()
        );
    }

    #[tokio::test]
    async fn workbuddy_delegate_disabled_key_fails_before_source_load_and_legacy_returns_none() {
        let mut collection = collection();
        let mut legacy = build_local_access_api_key(None);
        legacy.id = "legacy-fixture".into();
        collection.api_keys.push(legacy.clone());
        let caller = native_contract::Caller {
            id: legacy.id.clone(),
            label: "fixture".into(),
        };
        let host = NativeHostBridge::for_collection(collection.clone());
        assert!(
            native_workbuddy_delegate_selection(&host, &caller, "ordinary-model")
                .await
                .unwrap()
                .is_none()
        );
        collection.api_keys[0].enabled = false;
        let host = NativeHostBridge::for_collection(collection);
        let result = native_workbuddy_delegate_selection(&host, &caller, "ordinary-model").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn workbuddy_delegate_selection_uses_only_memory_fixture_and_pins_source_generation() {
        fn fixture_host(
            collection: CodexLocalAccessCollection,
            source: crate::models::workbuddy::WorkbuddyAccount,
        ) -> NativeHostBridge {
            let mut account = CodexAccount::new_api_key(
                config().provider_account_id,
                "fixture@example.invalid".into(),
                String::new(),
                CodexApiProviderMode::Custom,
                Some(codex_account::WORKBUDDY_ACP_BASE_URL.into()),
                Some("workbuddy_acp".into()),
                Some("WorkBuddy".into()),
                vec!["workbuddy-default".into()],
            );
            account.api_credential_kind = Some(codex_account::WORKBUDDY_ACP_PROVIDER_KIND.into());
            account.api_credential_ref = Some(source.id.clone());
            let mut host = NativeHostBridge::for_collection(collection);
            host.account_overrides.insert(account.id.clone(), account);
            host.workbuddy_delegate_test_sources
                .insert(source.id.clone(), source);
            host
        }
        let mut collection = collection();
        let selected_key = key();
        let caller = native_contract::Caller {
            id: selected_key.id.clone(),
            label: "fixture".into(),
        };
        collection.api_keys.push(selected_key);
        let host = fixture_host(collection.clone(), source());
        let selected = native_workbuddy_delegate_selection(&host, &caller, &config().public_model)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(selected.route.base_url, "workbuddy-delegate://native");
        assert_eq!(selected.route.model, "glm-5.3-flash");
        assert!(selected.route.headers.is_empty());
        assert!(selected.domestic);
        let generation = selected.source_generation;
        let mut changed_source = source();
        changed_source.access_token = "synthetic-rotated-secret".into();
        let changed = fixture_host(collection.clone(), changed_source);
        let selected =
            native_workbuddy_delegate_selection(&changed, &caller, &config().public_model)
                .await
                .unwrap()
                .unwrap();
        assert_ne!(generation, selected.source_generation);
        collection
            .paused_account_ids
            .push(config().provider_account_id);
        let paused = fixture_host(collection.clone(), source());
        assert!(
            native_workbuddy_delegate_selection(&paused, &caller, &config().public_model)
                .await
                .is_err()
        );
        collection.paused_account_ids.clear();
        let mut missing = fixture_host(collection, source());
        missing.workbuddy_delegate_test_sources.clear();
        assert!(
            native_workbuddy_delegate_selection(&missing, &caller, &config().public_model)
                .await
                .is_err()
        );
    }

    #[test]
    fn workbuddy_delegate_source_validation_and_generation_fail_closed() {
        let original = source();
        assert!(workbuddy_delegate_validate_source(&original).is_ok());
        assert!(workbuddy_delegate_domestic(&original).unwrap());
        let generation = workbuddy_delegate_source_generation("source-ref", &original);
        assert!(!generation.contains("synthetic-workbuddy-secret"));
        let mut changed = original.clone();
        changed.access_token = "synthetic-new-secret".into();
        assert_ne!(
            generation,
            workbuddy_delegate_source_generation("source-ref", &changed)
        );
        changed = original.clone();
        changed.domain = Some("tenant-routing-value".into());
        assert!(workbuddy_delegate_domestic(&changed).unwrap());
        assert_ne!(
            generation,
            workbuddy_delegate_source_generation("source-ref", &changed)
        );
        changed = original.clone();
        changed.expires_at = Some(1);
        assert!(workbuddy_delegate_validate_source(&changed).is_err());
        changed = original.clone();
        changed.status = Some("disabled".into());
        assert!(workbuddy_delegate_validate_source(&changed).is_err());
        changed = original;
        changed.enterprise_id = Some("enterprise-fixture".into());
        assert_eq!(
            workbuddy_delegate_validate_source(&changed).unwrap_err(),
            "WORKBUDDY_DELEGATE_ENTERPRISE_UNSUPPORTED"
        );
    }

    #[test]
    fn workbuddy_delegate_catalog_uses_exact_selected_model_efforts_not_global_aliases() {
        let raw = json!({"models":{"currentModelId":"hy4std", "availableModels":[
            {"modelId":"glm-5.3-flash","name":"GLM","_meta":{"effortOptions":["low","high","max"]}},
            {"modelId":"hy4std","name":"HY","_meta":{"effortOptions":[{"value":"high"}]}}
        ]}, "configOptions":[{"id":"thought_level","category":"thought_level","currentValue":"low",
            "options":[{"value":"low"},{"value":"high"},{"value":"max"},{"value":"enabled"}]}]});
        let parsed = workbuddy_delegate_parse_catalog(&raw).unwrap();
        assert_eq!(parsed.reasoning_efforts, vec!["high"]);
        assert!(parsed.current_reasoning_effort.is_none());
        let mut glm = raw;
        glm["models"]["currentModelId"] = json!("glm-5.3-flash");
        let parsed = workbuddy_delegate_parse_catalog(&glm).unwrap();
        assert_eq!(parsed.reasoning_efforts, vec!["low", "high", "max"]);
        assert_eq!(parsed.current_reasoning_effort.as_deref(), Some("low"));
        let public = workbuddy_delegate_catalog(&config());
        assert_eq!(public["data"].as_array().unwrap().len(), 1);
        assert_eq!(public["data"][0]["supports_websockets"], false);
        assert_eq!(public["data"][0]["supports_images"], false);
        assert_eq!(
            public["data"][0]["model_info"]["supported_reasoning_efforts"],
            json!(["low"])
        );
    }

    #[test]
    fn workbuddy_delegate_modified_profile_still_revokes_owned_key_and_preserves_user_data() {
        let temp = tempfile::tempdir().unwrap();
        let owned_key = key();
        let profile =
            workbuddy_delegate_write_profile(temp.path(), &owned_key, "synthetic-source", 12345)
                .unwrap();
        let (dir, mut backup) = workbuddy_delegate_read_backup(temp.path(), &profile.id).unwrap();
        fs::write(dir.join("config.toml"), "modified by user").unwrap();
        fs::write(dir.join("user-state.txt"), "keep user state").unwrap();
        let mut collection = collection();
        collection.api_keys.push(owned_key);
        assert!(workbuddy_delegate_revoke_owned_key(&mut collection, &backup).unwrap());
        assert!(!collection.api_keys[0].enabled);
        // Revocation succeeds before, and independently of, changed-file checks.
        assert!(workbuddy_delegate_restore_owned_files(temp.path(), &profile.id, &backup).is_err());
        assert_eq!(
            fs::read_to_string(dir.join("config.toml")).unwrap(),
            "modified by user"
        );
        assert_eq!(
            fs::read_to_string(dir.join("user-state.txt")).unwrap(),
            "keep user state"
        );
        assert!(dir.join(WORKBUDDY_DELEGATE_PROFILE_MARKER).exists());
        assert!(!workbuddy_delegate_revoke_owned_key(&mut collection, &backup).unwrap());
        assert!(!collection.api_keys[0].enabled);
        // A forged key reference cannot revoke an unrelated key.
        let mut unrelated = key();
        unrelated.key = "different-synthetic-scoped-key".into();
        backup.profile.api_key_id = unrelated.id.clone();
        collection.api_keys.push(unrelated);
        assert!(workbuddy_delegate_revoke_owned_key(&mut collection, &backup).is_err());
        assert!(collection.api_keys[1].enabled);
    }

    #[test]
    fn workbuddy_delegate_restore_removes_only_owned_unchanged_files_and_preserves_user_state() {
        let temp = tempfile::tempdir().unwrap();
        let profile =
            workbuddy_delegate_write_profile(temp.path(), &key(), "synthetic-source", 12345)
                .unwrap();
        let (dir, backup) = workbuddy_delegate_read_backup(temp.path(), &profile.id).unwrap();
        fs::write(dir.join("user-session-state.txt"), "keep user state").unwrap();
        // A previous partial restore may already have removed one owned file.
        fs::remove_file(dir.join("auth.json")).unwrap();
        workbuddy_delegate_restore_owned_files(temp.path(), &profile.id, &backup).unwrap();
        assert!(dir.is_dir());
        assert!(!dir.join("config.toml").exists());
        assert!(!dir.join("auth.json").exists());
        assert!(!dir.join(WORKBUDDY_DELEGATE_PROFILE_MARKER).exists());
        assert_eq!(
            fs::read_to_string(dir.join("user-session-state.txt")).unwrap(),
            "keep user state"
        );
    }

    #[test]
    fn workbuddy_delegate_profile_uses_only_scoped_key_and_protects_changed_files() {
        let temp = tempfile::tempdir().unwrap();
        let selected = key();
        let profile =
            workbuddy_delegate_write_profile(temp.path(), &selected, "synthetic-source", 12345)
                .unwrap();
        let (dir, backup) = workbuddy_delegate_read_backup(temp.path(), &profile.id).unwrap();
        let text = fs::read_to_string(dir.join("config.toml")).unwrap();
        let config: toml_edit::Document = text.parse().unwrap();
        assert_eq!(
            config["web_search"].as_str(),
            Some("disabled"),
            "The production profile must not advertise hosted web search"
        );
        let provider = &config["model_providers"][WORKBUDDY_DELEGATE_PROVIDER_ID];
        assert_eq!(provider["supports_websockets"].as_bool(), Some(false));
        assert_eq!(provider["wire_api"].as_str(), Some("responses"));
        assert_eq!(
            provider["base_url"].as_str(),
            Some("http://127.0.0.1:12345/v1")
        );
        assert_eq!(
            provider["experimental_bearer_token"].as_str(),
            Some("synthetic-scoped-local-key")
        );
        let auth = fs::read_to_string(dir.join("auth.json")).unwrap();
        assert!(!text.contains("synthetic-workbuddy-secret"));
        assert!(!auth.contains("synthetic-workbuddy-secret"));
        assert!(!profile
            .launch_command
            .contains("synthetic-scoped-local-key"));
        assert!(workbuddy_delegate_check_profile_unchanged(&dir, &backup).is_ok());
        fs::write(dir.join("config.toml"), "user changed configuration").unwrap();
        assert!(workbuddy_delegate_check_profile_unchanged(&dir, &backup).is_err());
        assert_eq!(
            fs::read_to_string(dir.join("config.toml")).unwrap(),
            "user changed configuration"
        );
        assert!(workbuddy_delegate_read_backup(temp.path(), "../default").is_err());
    }
}
