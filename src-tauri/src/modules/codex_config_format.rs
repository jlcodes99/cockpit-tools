use std::fs;
use std::path::Path;

use toml_edit::Document;

const CODEX_FEATURES_KEY: &str = "features";
const CODEX_MODEL_KEY: &str = "model";
const CODEX_MODEL_PROVIDER_KEY: &str = "model_provider";
const CODEX_MODEL_CATALOG_JSON_KEY: &str = "model_catalog_json";

pub fn normalize_config_toml_spacing(content: &str) -> String {
    let mut normalized = String::with_capacity(content.len());
    let mut blank_line_count = 0usize;

    for line in content.lines() {
        if line.trim().is_empty() {
            blank_line_count += 1;
            if blank_line_count <= 1 {
                normalized.push('\n');
            }
            continue;
        }

        blank_line_count = 0;
        normalized.push_str(line);
        normalized.push('\n');
    }

    normalized
}

pub fn sanitize_codex_config_doc(doc: &mut Document) -> bool {
    if doc
        .get(CODEX_FEATURES_KEY)
        .and_then(|item| item.as_table())
        .is_none()
    {
        return false;
    }

    let _ = doc.remove(CODEX_FEATURES_KEY);
    true
}

pub fn codex_config_doc_to_string(doc: &mut Document) -> String {
    sanitize_codex_config_doc(doc);
    normalize_config_toml_spacing(&doc.to_string())
}

pub fn sanitize_codex_config_toml_file(path: &Path) -> Result<bool, String> {
    log_codex_config_audit(path, "before-sanitize");
    let changed = sanitize_codex_config_toml_file_once(path)?;
    let backup_path = path.with_file_name(format!(
        "{}.bak",
        path.file_name()
            .and_then(|item| item.to_str())
            .unwrap_or("config.toml")
    ));
    let backup_changed = sanitize_codex_config_toml_file_once(&backup_path)?;
    let changed_any = changed || backup_changed;
    log_codex_config_audit(path, "after-sanitize");
    Ok(changed_any)
}

pub fn log_codex_config_audit(path: &Path, context: &str) {
    log_codex_config_file_audit(path, context);
    let backup_path = path.with_file_name(format!(
        "{}.bak",
        path.file_name()
            .and_then(|item| item.to_str())
            .unwrap_or("config.toml")
    ));
    log_codex_config_file_audit(&backup_path, context);
}

fn log_codex_config_file_audit(path: &Path, context: &str) {
    match inspect_codex_config_file(path) {
        Ok(summary) => crate::modules::logger::log_info(&format!(
            "[Codex Config Audit] context={}, path={}, {}",
            context,
            path.display(),
            summary
        )),
        Err(error) => crate::modules::logger::log_warn(&format!(
            "[Codex Config Audit] context={}, path={}, error={}",
            context,
            path.display(),
            error
        )),
    }
}

fn inspect_codex_config_file(path: &Path) -> Result<String, String> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok("exists=false".to_string())
        }
        Err(error) => return Err(format!("read_failed={}", error)),
    };
    if content.trim().is_empty() {
        return Ok(format!("exists=true bytes={} empty=true", content.len()));
    }

    let doc = content
        .parse::<Document>()
        .map_err(|error| format!("parse_failed={}", error))?;
    let features = match doc.get(CODEX_FEATURES_KEY) {
        Some(item) if item.as_table().is_some() => "legacy_table".to_string(),
        Some(item) if item.as_value().and_then(|value| value.as_bool()).is_some() => format!(
            "bool:{}",
            item.as_value()
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
        ),
        Some(item) if item.as_value().and_then(|value| value.as_str()).is_some() => {
            "string".to_string()
        }
        Some(_) => "other".to_string(),
        None => "absent".to_string(),
    };
    let model = doc
        .get(CODEX_MODEL_KEY)
        .and_then(|item| item.as_value())
        .and_then(|value| value.as_str())
        .unwrap_or("<absent>");
    let provider = doc
        .get(CODEX_MODEL_PROVIDER_KEY)
        .and_then(|item| item.as_value())
        .and_then(|value| value.as_str())
        .unwrap_or("<absent>");
    let catalog = doc
        .get(CODEX_MODEL_CATALOG_JSON_KEY)
        .and_then(|item| item.as_value())
        .and_then(|value| value.as_str())
        .unwrap_or("<absent>");
    Ok(format!(
        "exists=true bytes={} features={} model={} model_provider={} model_catalog_json={}",
        content.len(),
        features,
        model,
        provider,
        catalog
    ))
}

fn sanitize_codex_config_toml_file_once(path: &Path) -> Result<bool, String> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(format!(
                "读取 Codex config.toml 失败 ({}): {}",
                path.display(),
                error
            ));
        }
    };
    if content.trim().is_empty() {
        return Ok(false);
    }

    let mut doc = content.parse::<Document>().map_err(|error| {
        format!(
            "解析 Codex config.toml 失败 ({}): {}",
            path.display(),
            error
        )
    })?;
    if !sanitize_codex_config_doc(&mut doc) {
        return Ok(false);
    }

    let normalized = normalize_config_toml_spacing(&doc.to_string());
    crate::modules::atomic_write::write_string_atomic(path, &normalized).map_err(|error| {
        format!(
            "写入 Codex config.toml 失败 ({}): {}",
            path.display(),
            error
        )
    })?;
    crate::modules::logger::log_info(&format!(
        "[Codex Config] removed legacy [features] table before launch: {}",
        path.display()
    ));
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::{
        codex_config_doc_to_string, normalize_config_toml_spacing, sanitize_codex_config_doc,
        sanitize_codex_config_toml_file,
    };
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};
    use toml_edit::Document;

    #[test]
    fn collapses_repeated_blank_lines() {
        let input = "model = \"gpt-5\"\n\n\n\nsandbox_mode = \"danger-full-access\"\n\n[desktop]\n";
        let output = normalize_config_toml_spacing(input);

        assert_eq!(
            output,
            "model = \"gpt-5\"\n\nsandbox_mode = \"danger-full-access\"\n\n[desktop]\n"
        );
    }

    #[test]
    fn removes_legacy_features_table() {
        let mut doc = r#"
model = "deepseek-v4-pro"

[features]
multi_agent = true
js_repl = false

[desktop]
default-service-tier = "priority"
"#
        .parse::<Document>()
        .expect("parse config");

        assert!(sanitize_codex_config_doc(&mut doc));

        let output = doc.to_string();
        assert!(!output.contains("[features]"));
        assert!(output.contains("model = \"deepseek-v4-pro\""));
        assert!(output.contains("[desktop]"));
    }

    #[test]
    fn keeps_boolean_features_value() {
        let mut doc = r#"
model = "gpt-5"
features = true
"#
        .parse::<Document>()
        .expect("parse config");

        assert!(!sanitize_codex_config_doc(&mut doc));

        let output = codex_config_doc_to_string(&mut doc);
        assert!(output.contains("features = true"));
    }

    #[test]
    fn sanitizes_backup_file_next_to_config() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "cockpit-codex-config-format-{}-{}",
            std::process::id(),
            unique
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        let config_path = dir.join("config.toml");
        let backup_path = dir.join("config.toml.bak");

        fs::write(&config_path, "model = \"gpt-5\"\n").expect("write config");
        fs::write(
            &backup_path,
            "model = \"gpt-5\"\n\n[features]\njs_repl = false\n",
        )
        .expect("write backup");

        assert!(sanitize_codex_config_toml_file(&config_path).expect("sanitize config"));

        let backup = fs::read_to_string(&backup_path).expect("read backup");
        assert!(!backup.contains("[features]"));
        assert!(backup.contains("model = \"gpt-5\""));

        let _ = fs::remove_dir_all(&dir);
    }
}
