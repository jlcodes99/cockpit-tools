const CODEX_SUBAGENT_MODEL_KEY: &str = "default_subagent_model";
const CODEX_SUBAGENT_REASONING_EFFORT_KEY: &str = "default_subagent_reasoning_effort";

#[derive(Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexSubagentDefaults {
    model: Option<String>,
    reasoning_effort: Option<String>,
}

fn read_subagent_config(path: &Path) -> Result<Document, String> {
    match crate::modules::codex_config_format::read_codex_config_file_text(path)? {
        Some((text, _)) => text
            .parse::<Document>()
            .map_err(|error| format!("无法解析 Codex config.toml: {error}")),
        None => Ok(Document::new()),
    }
}

fn subagent_defaults_from_doc(doc: &Document) -> Result<CodexSubagentDefaults, String> {
    let Some(agents) = doc.get("agents") else {
        return Ok(CodexSubagentDefaults::default());
    };
    let agents = agents
        .as_table_like()
        .ok_or_else(|| "Codex config.toml 中的 agents 不是表".to_string())?;
    let string_field = |key| {
        agents
            .get(key)
            .map(|item| {
                item.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| format!("Codex config.toml 中的 agents.{key} 不是字符串"))
            })
            .transpose()
    };
    Ok(CodexSubagentDefaults {
        model: string_field(CODEX_SUBAGENT_MODEL_KEY)?,
        reasoning_effort: string_field(CODEX_SUBAGENT_REASONING_EFFORT_KEY)?,
    })
}

pub fn read_subagent_defaults(base_dir: &Path) -> Result<CodexSubagentDefaults, String> {
    subagent_defaults_from_doc(&read_subagent_config(&get_config_toml_path(base_dir))?)
}

pub fn save_subagent_defaults(
    base_dir: &Path,
    defaults: CodexSubagentDefaults,
) -> Result<CodexSubagentDefaults, String> {
    let normalize = |value: Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let defaults = CodexSubagentDefaults {
        model: normalize(defaults.model),
        reasoning_effort: normalize(defaults.reasoning_effort),
    };
    let _guard = CODEX_ACCOUNT_MUTATION_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _lease = try_acquire_profile_mutation_lease(base_dir, "subagent-defaults")?;
    let path = get_config_toml_path(base_dir);
    let mut doc = read_subagent_config(&path)?;
    let previous = subagent_defaults_from_doc(&doc)?;
    if previous == defaults {
        return Ok(defaults);
    }
    if doc.get("agents").is_none() {
        doc["agents"] = toml_edit::table();
    }
    let agents = doc["agents"]
        .as_table_like_mut()
        .ok_or_else(|| "Codex config.toml 中的 agents 不是表".to_string())?;
    for (key, current, next) in [
        (CODEX_SUBAGENT_MODEL_KEY, &previous.model, &defaults.model),
        (
            CODEX_SUBAGENT_REASONING_EFFORT_KEY,
            &previous.reasoning_effort,
            &defaults.reasoning_effort,
        ),
    ] {
        if current == next {
            continue;
        }
        if let Some(value) = next {
            let mut replacement = toml_edit::Value::from(value.as_str());
            if let Some(old) = agents.get(key).and_then(toml_edit::Item::as_value) {
                *replacement.decor_mut() = old.decor().clone();
            }
            agents.insert(key, toml_edit::Item::Value(replacement));
        } else {
            agents.remove(key);
        }
    }
    fs::create_dir_all(base_dir).map_err(|error| format!("创建 Codex 配置目录失败: {error}"))?;
    crate::modules::codex_config_format::write_codex_config_toml_atomic(&path, &doc.to_string())?;
    Ok(defaults)
}

#[cfg(test)]
mod subagent_defaults_tests {
    use super::*;

    #[test]
    fn subagent_defaults_update_just_the_native_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = get_config_toml_path(dir.path());
        let before = "model = 'parent'\n\n[agents]\nmax_threads = 4\ndefault_subagent_model = 'old' # keep\n\n[agents.reviewer]\nconfig_file = 'reviewer.toml'\n";
        fs::write(&path, before).unwrap();
        let expected = CodexSubagentDefaults {
            model: Some("custom/provider-model".into()),
            reasoning_effort: Some("persistent".into()),
        };
        assert_eq!(
            save_subagent_defaults(
                dir.path(),
                CodexSubagentDefaults {
                    model: Some(" custom/provider-model ".into()),
                    reasoning_effort: expected.reasoning_effort.clone(),
                }
            )
            .unwrap(),
            expected
        );
        let updated = fs::read_to_string(&path).unwrap();
        assert!(updated.contains("model = 'parent'"));
        assert!(updated.contains("max_threads = 4"));
        assert!(updated.contains(" # keep"));
        assert!(updated.contains("[agents.reviewer]\nconfig_file = 'reviewer.toml'"));
        assert_eq!(read_subagent_defaults(dir.path()).unwrap(), expected);
        save_subagent_defaults(dir.path(), CodexSubagentDefaults::default()).unwrap();
        let reset = fs::read_to_string(&path).unwrap();
        assert!(reset.contains("config_file = 'reviewer.toml'"));
        assert!(!reset.contains("default_subagent_"));
    }

    #[test]
    fn subagent_defaults_keep_inline_and_reject_damaged_toml_without_rewriting() {
        let dir = tempfile::tempdir().unwrap();
        let path = get_config_toml_path(dir.path());
        assert_eq!(
            save_subagent_defaults(dir.path(), CodexSubagentDefaults::default()).unwrap(),
            CodexSubagentDefaults::default()
        );
        assert!(!path.exists());
        fs::write(&path, "agents = { enabled = true }\n").unwrap();
        save_subagent_defaults(
            dir.path(),
            CodexSubagentDefaults {
                model: None,
                reasoning_effort: Some("low".into()),
            },
        )
        .unwrap();
        let config = fs::read_to_string(&path)
            .unwrap()
            .parse::<Document>()
            .unwrap();
        let agents = config["agents"].as_inline_table().unwrap();
        assert_eq!(
            agents.get("enabled").and_then(toml_edit::Value::as_bool),
            Some(true)
        );
        assert_eq!(
            agents
                .get(CODEX_SUBAGENT_REASONING_EFFORT_KEY)
                .and_then(toml_edit::Value::as_str),
            Some("low")
        );
        let utf16 = "\u{feff}[agents]\nmax_threads = 4\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        fs::write(&path, &utf16).unwrap();
        assert_eq!(
            read_subagent_defaults(dir.path()).unwrap(),
            CodexSubagentDefaults::default()
        );
        assert_eq!(
            fs::read(&path).unwrap(),
            utf16,
            "reading does not rewrite config"
        );
        save_subagent_defaults(
            dir.path(),
            CodexSubagentDefaults {
                model: Some("gpt-6-luna".into()),
                reasoning_effort: None,
            },
        )
        .unwrap();
        assert!(fs::read_to_string(&path)
            .unwrap()
            .contains("max_threads = 4"));
        for invalid in ["[agents\n", "agents = true\n"] {
            fs::write(&path, invalid).unwrap();
            assert!(save_subagent_defaults(dir.path(), CodexSubagentDefaults::default()).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), invalid);
        }
    }
}
