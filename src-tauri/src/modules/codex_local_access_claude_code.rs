// Claude Code user settings for the selected local API Service key.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeCodeSettingsPreview {
    path: String,
    revision: String,
    changes: std::collections::BTreeMap<String, Option<String>>,
    changed: bool,
}

struct ClaudeCodeSettingsPlan {
    before: Option<String>,
    after: String,
    preview: ClaudeCodeSettingsPreview,
}

#[derive(Serialize, Deserialize)]
struct ClaudeCodeSettingsBackup {
    before: Option<String>,
}

const CLAUDE_CODE_SETTINGS_BACKUP_KIND: &str = "codex-api-claude-code-settings";
const CLAUDE_CODE_PRESET_MODELS: [(&str, &str); 4] = [
    ("ANTHROPIC_DEFAULT_SONNET_MODEL", "claude-sonnet-5"),
    ("ANTHROPIC_DEFAULT_OPUS_MODEL", "claude-opus-4-8"),
    ("ANTHROPIC_DEFAULT_HAIKU_MODEL", "claude-haiku-4-5-20251001"),
    ("ANTHROPIC_DEFAULT_FABLE_MODEL", "claude-fable-5"),
];
static CLAUDE_CODE_SETTINGS_TRANSACTION: Mutex<()> = Mutex::new(());

fn claude_code_settings_path() -> Result<PathBuf, String> {
    resolve_claude_code_settings_path(
        crate::modules::claude_account::get_default_claude_code_config_dir()?.join("settings.json"),
    )
}

fn resolve_claude_code_settings_path(path: PathBuf) -> Result<PathBuf, String> {
    match fs::symlink_metadata(&path) {
        Ok(_) => fs::canonicalize(&path)
            .map_err(|error| format!("解析 Claude Code 配置路径失败: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let dir = path.parent().ok_or("无法定位 Claude Code 配置目录")?;
            match fs::canonicalize(dir) {
                Ok(dir) => Ok(dir.join("settings.json")),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let home = dir.parent().ok_or("无法定位 Claude Code 用户目录")?;
                    let home = fs::canonicalize(home).map_err(|error| error.to_string())?;
                    Ok(home.join(".claude").join("settings.json"))
                }
                Err(error) => Err(format!("解析 Claude Code 配置目录失败: {error}")),
            }
        }
        Err(error) => Err(format!("读取 Claude Code 配置路径失败: {error}")),
    }
}

fn read_claude_code_settings(path: &Path) -> Result<Option<String>, String> {
    match fs::read_to_string(path) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("读取 Claude Code 配置失败: {error}")),
    }
}

fn claude_code_settings_plan(
    path: &Path,
    before: Option<String>,
    base_url: &str,
    key: &str,
    models: &[String],
) -> Result<ClaudeCodeSettingsPlan, String> {
    let mut updates = std::collections::BTreeMap::new();
    let mut main_model = None;
    for (env, alias) in CLAUDE_CODE_PRESET_MODELS {
        let model = models
            .iter()
            .find(|model| model.as_str() == alias)
            .or_else(|| {
                models.iter().find(|model| {
                    model
                        .rsplit_once('/')
                        .is_some_and(|(_, name)| name == alias)
                })
            });
        if main_model.is_none() {
            main_model = model;
        }
        updates.insert(env, model.cloned());
    }
    let main_model = main_model.ok_or("CODEX_CLAUDE_CODE_NO_PRESET_MODELS")?;
    updates.insert("ANTHROPIC_MODEL", Some(main_model.clone()));
    updates.insert("ANTHROPIC_BASE_URL", Some(base_url.to_owned()));
    updates.insert("ANTHROPIC_AUTH_TOKEN", Some(key.to_owned()));
    // Leaving the previous provider key in env can make Claude Code use it.
    updates.insert("ANTHROPIC_API_KEY", None);

    let mut settings: Value = before
        .as_deref()
        .map_or_else(
            || Ok(json!({})),
            |value| serde_json::from_str(value.trim_start_matches('\u{feff}')),
        )
        .map_err(|error| format!("Claude Code settings.json 无法解析: {error}"))?;
    let env = settings
        .as_object_mut()
        .ok_or("Claude Code settings.json 不是对象")?
        .entry("env")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("Claude Code settings.json 的 env 不是对象")?;
    let mut changes = std::collections::BTreeMap::new();
    for (name, value) in updates {
        let replacement = value.as_ref().map(|value| Value::String(value.clone()));
        if env.get(name) == replacement.as_ref() {
            continue;
        }
        let masked = if name == "ANTHROPIC_AUTH_TOKEN" {
            Some("••••••••".to_string())
        } else {
            value
        };
        changes.insert(name.to_owned(), masked);
        match replacement {
            Some(value) => {
                env.insert(name.to_owned(), value);
            }
            None => {
                env.remove(name);
            }
        }
    }
    let changed = !changes.is_empty();
    let after = if changed {
        format!(
            "{}\n",
            serde_json::to_string_pretty(&settings).map_err(|error| error.to_string())?
        )
    } else {
        before
            .clone()
            .expect("an unchanged configuration already exists")
    };
    use sha2::Digest as _;
    let revision = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(path, &before, &after)).map_err(|error| error.to_string())?
        )
    );
    let preview = ClaudeCodeSettingsPreview {
        path: path.to_string_lossy().into_owned(),
        revision,
        changes,
        changed,
    };
    Ok(ClaudeCodeSettingsPlan {
        before,
        after,
        preview,
    })
}

fn claude_code_backup_path(root: &Path, path: &Path, content: &str) -> PathBuf {
    use sha2::Digest as _;
    let mut hash = Sha256::new();
    hash.update(path.as_os_str().as_encoded_bytes());
    hash.update([0]);
    hash.update(content.as_bytes());
    root.join("claude-code-settings-backups")
        .join(format!("{:x}.json", hash.finalize()))
}

fn apply_claude_code_settings_plan(
    path: &Path,
    root: &Path,
    plan: ClaudeCodeSettingsPlan,
    revision: &str,
) -> Result<(), String> {
    if plan.preview.revision != revision {
        return Err("CODEX_CLAUDE_CODE_SETTINGS_CHANGED".into());
    }
    if !plan.preview.changed {
        return Ok(());
    }
    let _lock = CLAUDE_CODE_SETTINGS_TRANSACTION
        .lock()
        .map_err(|error| error.to_string())?;
    if read_claude_code_settings(path)? != plan.before {
        return Err("CODEX_CLAUDE_CODE_SETTINGS_CHANGED".into());
    }
    let cipher = crate::modules::secure_account_storage::LocalVaultCipher::open(root, true)?;
    let backup = ClaudeCodeSettingsBackup {
        before: plan.before,
    };
    let snapshot = cipher.seal(CLAUDE_CODE_SETTINGS_BACKUP_KIND, &backup)?;
    crate::modules::atomic_write::write_secret_string_atomic(
        &claude_code_backup_path(root, path, &plan.after),
        &snapshot,
    )?;
    if !crate::modules::atomic_write::replace_private_string_if_matches(
        path,
        backup.before.as_deref(),
        Some(&plan.after),
    )? {
        return Err("CODEX_CLAUDE_CODE_SETTINGS_CHANGED".into());
    }
    Ok(())
}

fn restore_claude_code_settings_at(path: &Path, root: &Path) -> Result<(), String> {
    let _lock = CLAUDE_CODE_SETTINGS_TRANSACTION
        .lock()
        .map_err(|error| error.to_string())?;
    let current = read_claude_code_settings(path)?.ok_or("CODEX_CLAUDE_CODE_NO_BACKUP")?;
    let snapshot = read_claude_code_settings(&claude_code_backup_path(root, path, &current))?
        .ok_or("CODEX_CLAUDE_CODE_NO_BACKUP")?;
    let cipher = crate::modules::secure_account_storage::LocalVaultCipher::open(root, false)?;
    let backup: ClaudeCodeSettingsBackup =
        cipher.unseal(CLAUDE_CODE_SETTINGS_BACKUP_KIND, &snapshot)?;
    if !crate::modules::atomic_write::replace_private_string_if_matches(
        path,
        Some(&current),
        backup.before.as_deref(),
    )? {
        return Err("CODEX_CLAUDE_CODE_SETTINGS_CHANGED".into());
    }
    Ok(())
}

async fn claude_code_connection(key_id: &str) -> Result<(String, String, Vec<String>), String> {
    ensure_runtime_loaded_without_start().await?;
    let (base_url, port, key) = {
        let runtime = gateway_runtime().lock().await;
        let collection = runtime
            .collection
            .as_ref()
            .filter(|item| item.enabled && runtime.running)
            .ok_or("CODEX_CLAUDE_CODE_SERVICE_UNAVAILABLE")?;
        let key = if key_id == "legacy"
            && !collection
                .api_keys
                .iter()
                .any(|key| key.key == collection.api_key)
        {
            Some(collection.api_key.as_str())
        } else {
            collection
                .api_keys
                .iter()
                .find(|key| key.id == key_id && key.enabled)
                .map(|key| key.key.as_str())
        }
        .filter(|value| !value.is_empty())
        .ok_or("CODEX_CLAUDE_CODE_SERVICE_UNAVAILABLE")?;
        (
            format!(
                "http://{}:{}",
                client_base_url_host_text(collection.client_base_url_host),
                collection.port
            ),
            collection.port,
            key.to_owned(),
        )
    };
    let models = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|error| error.to_string())?
        .get(format!("http://127.0.0.1:{port}/v1/models"))
        .bearer_auth(&key)
        .send()
        .await
        .map_err(|_| "CODEX_CLAUDE_CODE_SERVICE_UNAVAILABLE")?
        .error_for_status()
        .map_err(|_| "CODEX_CLAUDE_CODE_SERVICE_UNAVAILABLE")?
        .json::<Value>()
        .await
        .map_err(|_| "CODEX_CLAUDE_CODE_SERVICE_UNAVAILABLE")?;
    let models = models
        .get("data")
        .and_then(Value::as_array)
        .ok_or("CODEX_CLAUDE_CODE_SERVICE_UNAVAILABLE")?
        .iter()
        .filter_map(|item| item.get("id").and_then(Value::as_str).map(str::to_owned))
        .collect();
    Ok((base_url, key, models))
}

pub async fn preview_claude_code_settings(
    key_id: String,
) -> Result<ClaudeCodeSettingsPreview, String> {
    let (base_url, key, models) = claude_code_connection(&key_id).await?;
    tauri::async_runtime::spawn_blocking(move || {
        let path = claude_code_settings_path()?;
        let before = read_claude_code_settings(&path)?;
        Ok(claude_code_settings_plan(&path, before, &base_url, &key, &models)?.preview)
    })
    .await
    .map_err(|error| error.to_string())?
}

pub async fn apply_claude_code_settings(key_id: String, revision: String) -> Result<(), String> {
    let (base_url, key, models) = claude_code_connection(&key_id).await?;
    tauri::async_runtime::spawn_blocking(move || {
        let path = claude_code_settings_path()?;
        let root = crate::modules::account::get_data_dir()?;
        let plan = claude_code_settings_plan(
            &path,
            read_claude_code_settings(&path)?,
            &base_url,
            &key,
            &models,
        )?;
        apply_claude_code_settings_plan(&path, &root, plan, &revision)
    })
    .await
    .map_err(|error| error.to_string())?
}

pub async fn restore_claude_code_settings() -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        restore_claude_code_settings_at(
            &claude_code_settings_path()?,
            &crate::modules::account::get_data_dir()?,
        )
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(test)]
mod claude_code_settings_tests {
    use super::*;

    fn plan(path: &Path, before: Option<String>) -> ClaudeCodeSettingsPlan {
        claude_code_settings_plan(
            path,
            before,
            "http://localhost:13000",
            "fixture-new-key",
            &[
                "team/claude-sonnet-5".to_string(),
                "team/claude-fable-5".to_string(),
            ],
        )
        .unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn new_settings_and_existing_settings_share_the_same_symlinked_profile_path() {
        let temp = tempfile::tempdir().unwrap();
        let config_dir = temp.path().join("profile");
        fs::create_dir(&config_dir).unwrap();
        let link = temp.path().join(".claude");
        std::os::unix::fs::symlink(&config_dir, &link).unwrap();
        let entry = link.join("settings.json");
        let initial = resolve_claude_code_settings_path(entry.clone()).unwrap();
        fs::write(&initial, "{}").unwrap();
        assert_eq!(resolve_claude_code_settings_path(entry).unwrap(), initial);
        assert_eq!(
            initial.parent().unwrap(),
            fs::canonicalize(config_dir).unwrap()
        );
    }

    #[test]
    fn preview_merges_only_gateway_variables_and_restore_recovers_exact_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        let before = r#"{"permissions":{"defaultMode":"plan"},"env":{"HTTPS_PROXY":"fixture","ANTHROPIC_API_KEY":"fixture-old-key","ANTHROPIC_DEFAULT_HAIKU_MODEL":"old-haiku"}}"#;
        fs::write(&path, before).unwrap();
        fs::write(
            temp.path().join("settings.json.bak"),
            "another tool's backup",
        )
        .unwrap();
        let plan = plan(&path, Some(before.to_string()));
        let preview = serde_json::to_string(&plan.preview).unwrap();
        assert!(!preview.contains("fixture-old-key"));
        assert!(!preview.contains("fixture-new-key"));
        assert_eq!(plan.preview.changes["ANTHROPIC_API_KEY"], None);
        assert_eq!(plan.preview.changes["ANTHROPIC_DEFAULT_HAIKU_MODEL"], None);
        assert_eq!(fs::read_to_string(&path).unwrap(), before);

        let revision = plan.preview.revision.clone();
        let backup_path = claude_code_backup_path(temp.path(), &path, &plan.after);
        apply_claude_code_settings_plan(&path, temp.path(), plan, &revision).unwrap();
        let settings: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(settings["permissions"]["defaultMode"], "plan");
        assert_eq!(settings["env"]["HTTPS_PROXY"], "fixture");
        assert_eq!(settings["env"]["ANTHROPIC_MODEL"], "team/claude-sonnet-5");
        assert_eq!(
            settings["env"]["ANTHROPIC_DEFAULT_FABLE_MODEL"],
            "team/claude-fable-5"
        );
        assert_eq!(settings["env"]["ANTHROPIC_AUTH_TOKEN"], "fixture-new-key");
        assert!(settings["env"].get("ANTHROPIC_API_KEY").is_none());
        assert!(!fs::read_to_string(backup_path)
            .unwrap()
            .contains("fixture-old-key"));
        restore_claude_code_settings_at(&path, temp.path()).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        assert_eq!(
            fs::read_to_string(temp.path().join("settings.json.bak")).unwrap(),
            "another tool's backup"
        );
    }

    #[test]
    fn apply_and_restore_preserve_settings_edited_elsewhere() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        let original = r#"{"model":"original"}"#;
        fs::write(&path, original).unwrap();
        let stale = plan(&path, Some(original.to_string()));
        let revision = stale.preview.revision.clone();
        let changed = r#"{"model":"edited"}"#;
        fs::write(&path, changed).unwrap();
        assert_eq!(
            apply_claude_code_settings_plan(&path, temp.path(), stale, &revision).unwrap_err(),
            "CODEX_CLAUDE_CODE_SETTINGS_CHANGED"
        );
        let fresh = plan(&path, Some(changed.to_string()));
        let revision = fresh.preview.revision.clone();
        apply_claude_code_settings_plan(&path, temp.path(), fresh, &revision).unwrap();
        fs::write(&path, original).unwrap();
        assert_eq!(
            restore_claude_code_settings_at(&path, temp.path()).unwrap_err(),
            "CODEX_CLAUDE_CODE_NO_BACKUP"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn restore_removes_only_the_new_settings_file_and_an_empty_catalog_cannot_apply() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("settings.json");
        let invalid =
            claude_code_settings_plan(&path, None, "http://localhost:13000", "fixture", &[]);
        assert_eq!(
            invalid.err().as_deref(),
            Some("CODEX_CLAUDE_CODE_NO_PRESET_MODELS")
        );
        let plan = plan(&path, None);
        let revision = plan.preview.revision.clone();
        apply_claude_code_settings_plan(&path, temp.path(), plan, &revision).unwrap();
        assert!(path.is_file());
        restore_claude_code_settings_at(&path, temp.path()).unwrap();
        assert!(!path.exists());
    }
}
