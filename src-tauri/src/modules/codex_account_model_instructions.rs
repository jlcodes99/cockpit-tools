const CODEX_MODEL_INSTRUCTIONS_FILE_KEY: &str = "model_instructions_file";

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexModelInstructionsSetting {
    path: Option<String>,
    deferred: bool,
}

fn read_model_instructions_backup(
    base_dir: &Path,
) -> Result<Option<serde_json::Map<String, serde_json::Value>>, String> {
    match fs::read_to_string(deepseek_compaction_backup_path(base_dir)) {
        Ok(content) => serde_json::from_str(&content)
            .map(Some)
            .map_err(|error| format!("无法解析 DeepSeek 配置备份: {error}")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("无法读取 DeepSeek 配置备份: {error}")),
    }
}

pub fn read_model_instructions_setting(base_dir: &Path) -> Result<CodexModelInstructionsSetting, String> {
    let backup = read_model_instructions_backup(base_dir)?;
    let config = get_config_toml_path(base_dir);
    let mut doc = crate::modules::codex_config_format::load_codex_config_doc(&config)?;
    if let Some(record) = &backup {
        restore_deepseek_top_level_backup(&mut doc, record);
    }
    Ok(CodexModelInstructionsSetting {
        path: doc.get(CODEX_MODEL_INSTRUCTIONS_FILE_KEY)
            .and_then(toml_edit::Item::as_str)
            .map(str::to_owned),
        deferred: backup.is_some(),
    })
}

pub fn save_model_instructions_setting(
    base_dir: &Path,
    path: Option<String>,
) -> Result<CodexModelInstructionsSetting, String> {
    let selected = if let Some(path) = path {
        let absolute = std::path::absolute(base_dir.join(path))
            .map_err(|error| format!("无法打开指令文件: {error}"))?;
        fs::read_to_string(&absolute)
            .map_err(|error| format!("无法读取 UTF-8 指令文件: {error}"))?;
        Some(absolute.into_os_string().into_string()
            .map_err(|_| "指令文件路径无法写入 Codex 配置".to_string())?)
    } else {
        None
    };

    let _guard = CODEX_ACCOUNT_MUTATION_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let _lease = try_acquire_profile_mutation_lease(base_dir, "model-instructions-file")?;
    if let Some(mut backup) = read_model_instructions_backup(base_dir)? {
        let entry = match &selected {
            Some(path) => serde_json::json!({
                "present": true,
                "raw": toml_edit::Value::from(path.as_str()).to_string()
            }),
            None => serde_json::json!({"present": false}),
        };
        backup.entry(DEEPSEEK_TOP_LEVEL_BACKUP_SECTION)
            .or_insert_with(|| serde_json::json!({}))
            .as_object_mut()
            .ok_or_else(|| "DeepSeek 配置备份格式无效".to_string())?
            .insert(CODEX_MODEL_INSTRUCTIONS_FILE_KEY.into(), entry);
        let content = serde_json::to_string_pretty(&backup).map_err(|error| error.to_string())?;
        write_string_atomic(&deepseek_compaction_backup_path(base_dir), &content)?;
        return Ok(CodexModelInstructionsSetting { path: selected, deferred: true });
    }

    let config = get_config_toml_path(base_dir);
    let mut doc = crate::modules::codex_config_format::load_codex_config_doc(&config)?;
    let changed = if let Some(path) = &selected {
        doc[CODEX_MODEL_INSTRUCTIONS_FILE_KEY] = toml_edit::value(path.as_str());
        true
    } else {
        doc.remove(CODEX_MODEL_INSTRUCTIONS_FILE_KEY).is_some()
    };
    if changed {
        fs::create_dir_all(base_dir).map_err(|error| format!("创建 Codex 配置目录失败: {error}"))?;
        let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
        crate::modules::codex_config_format::write_codex_config_toml_atomic(&config, &content)?;
    }
    Ok(CodexModelInstructionsSetting { path: selected, deferred: false })
}

#[cfg(test)]
mod model_instructions_setting_tests {
    use super::*;

    #[test]
    fn model_instructions_file_switch_is_local_to_the_config_key() {
        let dir = tempfile::tempdir().unwrap();
        let config = get_config_toml_path(dir.path());
        let original = "# keep comment\nmodel = 'gpt-6.1-sol'\n";
        fs::write(&config, original).unwrap();
        assert!(read_model_instructions_setting(dir.path()).unwrap().path.is_none());
        for filename in ["first prompt.md", "第二份指令.md"] {
            let file = dir.path().join(filename);
            fs::write(&file, "Instructions fixture\n").unwrap();
            let saved = save_model_instructions_setting(dir.path(), Some(filename.into())).unwrap();
            assert_eq!(Path::new(saved.path.as_deref().unwrap()), file);
            assert!(!saved.deferred);
            assert_eq!(read_model_instructions_setting(dir.path()).unwrap().path, saved.path);
            assert_eq!(fs::read_to_string(file).unwrap(), "Instructions fixture\n");
        }
        let before = fs::read_to_string(&config).unwrap();
        assert!(save_model_instructions_setting(dir.path(), Some("missing.md".into())).is_err());
        assert_eq!(fs::read_to_string(&config).unwrap(), before);
        assert!(save_model_instructions_setting(dir.path(), None).unwrap().path.is_none());
        assert_eq!(fs::read_to_string(&config).unwrap(), original);
    }

    #[test]
    fn model_instructions_choice_survives_deepseek_takeover_and_restore() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("next.md"), "Fixture").unwrap();
        let config = get_config_toml_path(dir.path());
        let mut doc = "model_instructions_file = 'old.md'\nmodel_verbosity = 'low'\n"
            .parse::<Document>().unwrap();
        apply_deepseek_config_overrides(&mut doc, dir.path());
        fs::write(&config, doc.to_string()).unwrap();
        let active = fs::read_to_string(&config).unwrap();
        let stored = read_model_instructions_setting(dir.path()).unwrap();
        assert!(stored.deferred);
        assert_eq!(stored.path.as_deref(), Some("old.md"));
        let saved = save_model_instructions_setting(dir.path(), Some("next.md".into())).unwrap();
        assert!(saved.deferred);
        assert_eq!(fs::read_to_string(&config).unwrap(), active);
        assert_eq!(read_model_instructions_setting(dir.path()).unwrap().path, saved.path);
        assert!(restore_deepseek_config_overrides(&mut doc, dir.path()));
        assert_eq!(doc[CODEX_MODEL_INSTRUCTIONS_FILE_KEY].as_str(), saved.path.as_deref());
        assert_eq!(doc["model_verbosity"].as_str(), Some("low"));
        apply_deepseek_config_overrides(&mut doc, dir.path());
        fs::write(&config, doc.to_string()).unwrap();
        assert!(save_model_instructions_setting(dir.path(), None).unwrap().deferred);
        assert!(restore_deepseek_config_overrides(&mut doc, dir.path()));
        assert!(doc.get(CODEX_MODEL_INSTRUCTIONS_FILE_KEY).is_none());
    }
}
