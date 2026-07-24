use std::fs;
use std::path::Path;

use toml_edit::Document;

const CODEX_FEATURES_KEY: &str = "features";
const CODEX_MODEL_KEY: &str = "model";
const CODEX_MODEL_PROVIDER_KEY: &str = "model_provider";
const CODEX_MODEL_CATALOG_JSON_KEY: &str = "model_catalog_json";
const CODEX_PROJECTS_TABLE_PREFIX: &str = "[projects.";
const UTF8_BOM: char = '\u{feff}';

#[cfg(target_os = "windows")]
fn clear_windows_config_file_attributes(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }

    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        GetFileAttributesW, SetFileAttributesW, FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_READONLY,
        FILE_ATTRIBUTE_SYSTEM, FILE_FLAGS_AND_ATTRIBUTES, INVALID_FILE_ATTRIBUTES,
    };

    let wide_path = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let attributes = unsafe { GetFileAttributesW(PCWSTR(wide_path.as_ptr())) };
    if attributes == INVALID_FILE_ATTRIBUTES {
        return Err(format!(
            "读取 Codex config.toml 文件属性失败: {}",
            path.display()
        ));
    }

    let protected_attributes =
        FILE_ATTRIBUTE_READONLY.0 | FILE_ATTRIBUTE_HIDDEN.0 | FILE_ATTRIBUTE_SYSTEM.0;
    let next_attributes = attributes & !protected_attributes;
    if next_attributes == attributes {
        return Ok(());
    }

    unsafe {
        SetFileAttributesW(
            PCWSTR(wide_path.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(next_attributes),
        )
    }
    .map_err(|error| {
        format!(
            "清理 Codex config.toml 文件属性失败: path={}, error={}",
            path.display(),
            error
        )
    })?;
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn clear_windows_config_file_attributes(_path: &Path) -> Result<(), String> {
    Ok(())
}

pub fn prepare_codex_config_file_for_write(path: &Path) -> Result<(), String> {
    clear_windows_config_file_attributes(path)
}

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

pub fn codex_config_doc_to_string(doc: &mut Document) -> String {
    normalize_config_toml_spacing(&doc.to_string())
}

pub fn write_codex_config_toml_atomic(path: &Path, content: &str) -> Result<(), String> {
    prepare_codex_config_file_for_write(path)?;
    crate::modules::atomic_write::write_string_atomic(path, content)
}

fn strip_utf8_bom(content: &str) -> (&str, bool) {
    match content.strip_prefix(UTF8_BOM) {
        Some(stripped) => (stripped, true),
        None => (content, false),
    }
}

fn contains_toml_unicode_escape(value: &str) -> bool {
    let chars = value.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    while index + 1 < chars.len() {
        if chars[index] == '\\' && matches!(chars[index + 1], 'u' | 'U') {
            let expected_len = if chars[index + 1] == 'u' { 4 } else { 8 };
            if chars
                .iter()
                .skip(index + 2)
                .take(expected_len)
                .filter(|ch| ch.is_ascii_hexdigit())
                .count()
                == expected_len
            {
                return true;
            }
        }
        index += 1;
    }
    false
}

fn is_table_header_line(trimmed_line: &str) -> bool {
    trimmed_line.starts_with('[')
}

fn is_projects_table_header(trimmed_line: &str) -> bool {
    trimmed_line.starts_with(CODEX_PROJECTS_TABLE_PREFIX)
}

fn header_parses_as_toml_table(trimmed_line: &str) -> bool {
    format!("{}\n__cockpit_probe = true\n", trimmed_line)
        .parse::<Document>()
        .is_ok()
}

fn is_unsafe_projects_header(trimmed_line: &str) -> bool {
    is_projects_table_header(trimmed_line)
        && (!trimmed_line.contains(']')
            || !trimmed_line.is_ascii()
            || contains_toml_unicode_escape(trimmed_line)
            || !header_parses_as_toml_table(trimmed_line))
}

fn remove_project_sections(content: &str, aggressive: bool) -> (String, bool) {
    let mut output = String::with_capacity(content.len());
    let mut skipping_project = false;
    let mut changed = false;

    for line in content.lines() {
        let trimmed = line.trim_start();
        let should_start_skip = if aggressive {
            is_projects_table_header(trimmed)
        } else {
            is_unsafe_projects_header(trimmed)
        };

        if should_start_skip {
            skipping_project = true;
            changed = true;
            continue;
        }

        if skipping_project && is_table_header_line(trimmed) {
            skipping_project = false;
        }

        if !skipping_project {
            output.push_str(line);
            output.push('\n');
        }
    }

    if changed {
        (normalize_config_toml_spacing(&output), true)
    } else {
        (content.to_string(), false)
    }
}

pub fn normalize_codex_config_input(content: &str) -> (String, bool) {
    let (without_bom, removed_bom) = strip_utf8_bom(content);
    let (without_unsafe_projects, removed_projects) = remove_project_sections(without_bom, false);
    (without_unsafe_projects, removed_bom || removed_projects)
}

pub fn parse_codex_config_doc(content: &str) -> Result<(Document, bool), String> {
    let (normalized, changed) = normalize_codex_config_input(content);
    if normalized.trim().is_empty() {
        return Ok((Document::new(), changed));
    }

    match normalized.parse::<Document>() {
        Ok(doc) => Ok((doc, changed)),
        Err(original_error) => {
            let (without_projects, removed_projects) = remove_project_sections(&normalized, true);
            if removed_projects {
                if without_projects.trim().is_empty() {
                    return Ok((Document::new(), true));
                }
                if let Ok(doc) = without_projects.parse::<Document>() {
                    return Ok((doc, true));
                }
            }
            Err(original_error.to_string())
        }
    }
}

pub fn read_codex_config_doc_from_str(content: &str) -> Result<Document, String> {
    parse_codex_config_doc(content).map(|(doc, _)| doc)
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

    let (doc, sanitized) =
        parse_codex_config_doc(&content).map_err(|error| format!("parse_failed={}", error))?;
    let features = match doc.get(CODEX_FEATURES_KEY) {
        Some(item) if item.as_table().is_some() => "table".to_string(),
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
        "exists=true bytes={} sanitized={} features={} model={} model_provider={} model_catalog_json={}",
        content.len(),
        sanitized,
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

    prepare_codex_config_file_for_write(path)?;

    let (doc, input_changed) = parse_codex_config_doc(&content).map_err(|error| {
        format!(
            "解析 Codex config.toml 失败 ({}): {}",
            path.display(),
            error
        )
    })?;
    if !input_changed {
        return Ok(false);
    }

    let normalized = normalize_config_toml_spacing(&doc.to_string());
    write_codex_config_toml_atomic(path, &normalized).map_err(|error| {
        format!(
            "写入 Codex config.toml 失败 ({}): {}",
            path.display(),
            error
        )
    })?;
    crate::modules::logger::log_info(&format!(
        "[Codex Config] sanitized config.toml before launch: {}",
        path.display()
    ));
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::{
        codex_config_doc_to_string, normalize_config_toml_spacing, parse_codex_config_doc,
        sanitize_codex_config_toml_file,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};
    use toml_edit::Document;

    fn unique_temp_dir() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "cockpit-codex-config-format-{}-{}",
            std::process::id(),
            unique
        ))
    }

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
    fn keeps_official_features_table() {
        let dir = unique_temp_dir();
        fs::create_dir_all(&dir).expect("create temp dir");
        let config_path = dir.join("config.toml");
        let input = r#"
model = "deepseek-v4-pro"

[features]
memories = true
multi_agent = true
js_repl = false

[desktop]
default-service-tier = "priority"
"#;
        fs::write(&config_path, input).expect("write config");

        assert!(!sanitize_codex_config_toml_file(&config_path).expect("sanitize config"));

        let output = fs::read_to_string(&config_path).expect("read config");
        assert!(output.contains("[features]"));
        assert!(output.contains("memories = true"));
        assert!(output.contains("multi_agent = true"));
        assert!(output.contains("js_repl = false"));
        assert!(output.contains("model = \"deepseek-v4-pro\""));
        assert!(output.contains("[desktop]"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeps_boolean_features_value() {
        let mut doc = r#"
model = "gpt-5"
features = true
"#
        .parse::<Document>()
        .expect("parse config");

        let output = codex_config_doc_to_string(&mut doc);
        assert!(output.contains("features = true"));
    }

    #[test]
    fn parse_removes_utf8_bom() {
        let (doc, changed) =
            parse_codex_config_doc("\u{feff}model = \"gpt-5\"\n").expect("parse config");

        assert!(changed);
        assert_eq!(
            doc.get("model").and_then(|item| item.as_str()),
            Some("gpt-5")
        );
    }

    #[test]
    fn parse_removes_non_ascii_project_sections() {
        let input = "model = \"gpt-5\"\n\n[projects.'C:\\Users\\demo\\赚钱']\ntrust_level = \"trusted\"\n\n[mcp_servers.demo]\ncommand = \"node\"\n";
        let (doc, changed) = parse_codex_config_doc(input).expect("parse config");
        let output = doc.to_string();

        assert!(changed);
        assert!(output.contains("model = \"gpt-5\""));
        assert!(output.contains("[mcp_servers.demo]"));
        assert!(!output.contains("[projects."));
        assert!(!output.contains("trust_level"));
    }

    #[test]
    fn parse_removes_unicode_escape_project_sections() {
        let input = "model = \"gpt-5\"\n\n[projects.\"C:\\\\Users\\\\demo\\\\GitHub\\u8d5a\\u94b1\"]\ntrust_level = \"trusted\"\n";
        let (doc, changed) = parse_codex_config_doc(input).expect("parse config");
        let output = doc.to_string();

        assert!(changed);
        assert!(output.contains("model = \"gpt-5\""));
        assert!(!output.contains("[projects."));
    }

    #[test]
    fn parse_keeps_ascii_project_sections() {
        let input = "model = \"gpt-5\"\n\n[projects.\"C:\\\\Users\\\\demo\\\\repo\"]\ntrust_level = \"trusted\"\n";
        let (doc, changed) = parse_codex_config_doc(input).expect("parse config");
        let output = doc.to_string();

        assert!(!changed);
        assert!(output.contains("[projects.\"C:\\\\Users\\\\demo\\\\repo\"]"));
        assert!(output.contains("trust_level = \"trusted\""));
    }

    #[test]
    fn parse_falls_back_by_removing_all_projects_when_project_body_is_invalid() {
        let input = "model = \"gpt-5\"\n\n[projects.\"C:\\\\Users\\\\demo\\\\repo\"]\ntrust_level = \"trusted\n\n[mcp_servers.demo]\ncommand = \"node\"\n";
        let (doc, changed) = parse_codex_config_doc(input).expect("parse config");
        let output = doc.to_string();

        assert!(changed);
        assert!(output.contains("model = \"gpt-5\""));
        assert!(output.contains("[mcp_servers.demo]"));
        assert!(!output.contains("[projects."));
    }

    #[test]
    fn sanitizes_backup_file_next_to_config() {
        let dir = unique_temp_dir();
        fs::create_dir_all(&dir).expect("create temp dir");
        let config_path = dir.join("config.toml");
        let backup_path = dir.join("config.toml.bak");

        fs::write(&config_path, "model = \"gpt-5\"\n").expect("write config");
        fs::write(
            &backup_path,
            "\u{feff}model = \"gpt-5\"\n\n[features]\nmemories = true\njs_repl = false\n",
        )
        .expect("write backup");

        assert!(sanitize_codex_config_toml_file(&config_path).expect("sanitize config"));

        let backup = fs::read_to_string(&backup_path).expect("read backup");
        assert!(!backup.starts_with('\u{feff}'));
        assert!(backup.contains("[features]"));
        assert!(backup.contains("memories = true"));
        assert!(backup.contains("model = \"gpt-5\""));

        let _ = fs::remove_dir_all(&dir);
    }
}
