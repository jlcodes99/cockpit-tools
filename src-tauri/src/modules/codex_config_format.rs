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

fn sibling_config_backup_path(path: &Path) -> Option<std::path::PathBuf> {
    let name = path.file_name()?.to_str()?;
    if name.ends_with(".bak") {
        return None;
    }
    Some(path.with_file_name(format!("{}.bak", name)))
}

fn decode_utf16_units(bytes: &[u8], little_endian: bool) -> Option<String> {
    if bytes.len() % 2 != 0 {
        return None;
    }
    let units = bytes
        .chunks_exact(2)
        .map(|chunk| {
            if little_endian {
                u16::from_le_bytes([chunk[0], chunk[1]])
            } else {
                u16::from_be_bytes([chunk[0], chunk[1]])
            }
        })
        .collect::<Vec<_>>();
    String::from_utf16(&units).ok()
}

fn decode_codex_config_bytes(bytes: &[u8]) -> Result<(String, bool), String> {
    if bytes.is_empty() {
        return Ok((String::new(), false));
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return decode_utf16_units(&bytes[2..], true)
            .map(|text| (text, true))
            .ok_or_else(|| "UTF-16 LE 配置无法解码".to_string());
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return decode_utf16_units(&bytes[2..], false)
            .map(|text| (text, true))
            .ok_or_else(|| "UTF-16 BE 配置无法解码".to_string());
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok((text.to_string(), false)),
        Err(_) => decode_utf16_units(bytes, true)
            .map(|text| (text, true))
            .ok_or_else(|| "配置既不是 UTF-8 也不是 UTF-16".to_string()),
    }
}

fn read_codex_config_file_text(path: &Path) -> Result<Option<(String, bool)>, String> {
    match fs::read(path) {
        Ok(bytes) => decode_codex_config_bytes(&bytes).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "读取 Codex config.toml 失败 ({}): {}",
            path.display(),
            error
        )),
    }
}

fn write_repaired_codex_config(path: &Path, doc: &Document) -> Result<(), String> {
    let normalized = normalize_config_toml_spacing(&doc.to_string());
    write_codex_config_toml_atomic(path, &normalized)
}

fn recover_invalid_codex_config(
    path: &Path,
    parse_error: &str,
) -> Result<(Document, bool), String> {
    if let Some(backup_path) = sibling_config_backup_path(path) {
        if let Ok(Some((backup_text, _))) = read_codex_config_file_text(&backup_path) {
            if !backup_text.trim().is_empty() {
                if let Ok((doc, _)) = parse_codex_config_doc(&backup_text) {
                    let quarantined = crate::modules::atomic_write::quarantine_file(
                        path,
                        "invalid-toml",
                    )?;
                    write_repaired_codex_config(path, &doc)?;
                    crate::modules::logger::log_warn(&format!(
                        "[Codex Config] 已从备份恢复损坏的 config.toml: path={}, backup={}, quarantined={:?}, error={}",
                        path.display(),
                        backup_path.display(),
                        quarantined,
                        parse_error
                    ));
                    return Ok((doc, true));
                }
            }
        }
    }

    let quarantined =
        crate::modules::atomic_write::quarantine_file(path, "invalid-toml")?;
    crate::modules::logger::log_warn(&format!(
        "[Codex Config] 已隔离无法解析的 config.toml 并继续使用空配置: path={}, quarantined={:?}, error={}",
        path.display(),
        quarantined,
        parse_error
    ));
    Ok((Document::new(), true))
}

pub fn load_codex_config_doc(path: &Path) -> Result<Document, String> {
    Ok(repair_codex_config_toml_file(path)?.0)
}

pub fn repair_codex_config_toml_file(path: &Path) -> Result<(Document, bool), String> {
    let Some((text, reencoded)) = read_codex_config_file_text(path)? else {
        return Ok((Document::new(), false));
    };
    if text.trim().is_empty() {
        return Ok((Document::new(), false));
    }

    match parse_codex_config_doc(&text) {
        Ok((doc, changed)) => {
            if reencoded || changed {
                write_repaired_codex_config(path, &doc)?;
                crate::modules::logger::log_info(&format!(
                    "[Codex Config] 已规范化 config.toml: path={}, reencoded={}, sanitized={}",
                    path.display(),
                    reencoded,
                    changed
                ));
                return Ok((doc, true));
            }
            Ok((doc, false))
        }
        Err(error) => recover_invalid_codex_config(path, &error),
    }
}

fn strip_utf8_bom(content: &str) -> (&str, bool) {
    match content.strip_prefix(UTF8_BOM) {
        Some(stripped) => (stripped, true),
        None => (content, false),
    }
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

/// 只有「残缺头部」和「无法作为 TOML 表解析」才算不安全。
/// `[projects.'E:\中文项目\店铺管理']` 这类含非 ASCII 的合法路径（Codex 自己的写法，单引号字面量）
/// 以及带 `\uXXXX` 转义的合法写法都必须保留，不能仅因为不是 ASCII 就整段删除。
fn is_unsafe_projects_header(trimmed_line: &str) -> bool {
    is_projects_table_header(trimmed_line)
        && (!trimmed_line.contains(']') || !header_parses_as_toml_table(trimmed_line))
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
    Ok(repair_codex_config_toml_file(path)?.1)
}

#[cfg(test)]
mod tests {
    use super::{
        codex_config_doc_to_string, load_codex_config_doc, normalize_config_toml_spacing,
        parse_codex_config_doc, sanitize_codex_config_toml_file,
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
    fn parse_keeps_non_ascii_project_sections() {
        let input = "model = \"gpt-5\"\n\n[projects.'E:\\中文项目\\店铺管理']\ntrust_level = \"trusted\"\n\n[mcp_servers.demo]\ncommand = \"node\"\n";
        let (doc, changed) = parse_codex_config_doc(input).expect("parse config");
        let output = doc.to_string();

        assert!(!changed);
        assert!(output.contains("model = \"gpt-5\""));
        assert!(output.contains("[mcp_servers.demo]"));
        assert!(output.contains("[projects."));
        assert!(output.contains("trust_level = \"trusted\""));
    }

    #[test]
    fn parse_keeps_unicode_escape_project_sections() {
        let input = "model = \"gpt-5\"\n\n[projects.\"C:\\\\Users\\\\demo\\\\GitHub\\u8d5a\\u94b1\"]\ntrust_level = \"trusted\"\n";
        let (doc, changed) = parse_codex_config_doc(input).expect("parse config");
        let output = doc.to_string();

        assert!(!changed);
        assert!(output.contains("model = \"gpt-5\""));
        assert!(output.contains("[projects."));
        assert!(output.contains("trust_level = \"trusted\""));
    }

    #[test]
    fn parse_removes_project_sections_with_unescaped_backslash_header() {
        let input = "model = \"gpt-5\"\n\n[projects.\"C:\\Users\\demo\\repo\"]\ntrust_level = \"trusted\"\n";
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

    #[test]
    fn sanitize_keeps_non_ascii_project_trust_entries_on_disk() {
        let dir = unique_temp_dir();
        fs::create_dir_all(&dir).expect("create temp dir");
        let config_path = dir.join("config.toml");
        let backup_path = dir.join("config.toml.bak");
        let input =
            "model = \"gpt-5\"\n\n[projects.'E:\\中文项目\\店铺管理']\ntrust_level = \"trusted\"\n";
        fs::write(&config_path, input).expect("write config");
        fs::write(&backup_path, input).expect("write backup");

        assert!(!sanitize_codex_config_toml_file(&config_path).expect("sanitize config"));

        assert_eq!(fs::read_to_string(&config_path).expect("read config"), input);
        assert_eq!(fs::read_to_string(&backup_path).expect("read backup"), input);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_restores_invalid_config_from_backup() {
        let dir = unique_temp_dir();
        fs::create_dir_all(&dir).expect("create temp dir");
        let config_path = dir.join("config.toml");
        fs::write(&config_path, "{\"model\":\"gpt-5\"}\n").expect("write invalid config");
        fs::write(dir.join("config.toml.bak"), "model = \"gpt-5.5\"\n").expect("write backup");

        let doc = load_codex_config_doc(&config_path).expect("load recovered config");
        assert_eq!(
            doc.get("model").and_then(|item| item.as_str()),
            Some("gpt-5.5")
        );
        let restored = fs::read_to_string(&config_path).expect("read restored config");
        assert!(restored.contains("model = \"gpt-5.5\""));
        let quarantined = fs::read_dir(&dir)
            .expect("read dir")
            .flatten()
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains("config.toml.invalid-toml.")
            });
        assert!(quarantined, "invalid config should be quarantined");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_quarantines_invalid_config_without_backup() {
        let dir = unique_temp_dir();
        fs::create_dir_all(&dir).expect("create temp dir");
        let config_path = dir.join("config.toml");
        fs::write(&config_path, "{not toml").expect("write invalid config");

        let doc = load_codex_config_doc(&config_path).expect("load empty recovered config");
        assert!(doc.as_table().is_empty() || doc.get("model").is_none());
        assert!(!config_path.exists());
        let quarantined = fs::read_dir(&dir)
            .expect("read dir")
            .flatten()
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains("config.toml.invalid-toml.")
            });
        assert!(quarantined);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_rewrites_utf16_config_as_utf8() {
        let dir = unique_temp_dir();
        fs::create_dir_all(&dir).expect("create temp dir");
        let config_path = dir.join("config.toml");
        let text = "model = \"gpt-5\"\n";
        let mut encoded = vec![0xFF, 0xFE];
        encoded.extend(text.encode_utf16().flat_map(|unit| unit.to_le_bytes()));
        fs::write(&config_path, encoded).expect("write utf16 config");

        let doc = load_codex_config_doc(&config_path).expect("load utf16 config");
        assert_eq!(
            doc.get("model").and_then(|item| item.as_str()),
            Some("gpt-5")
        );
        let restored = fs::read_to_string(&config_path).expect("read rewritten utf8 config");
        assert!(restored.contains("model = \"gpt-5\""));
        let _ = fs::remove_dir_all(&dir);
    }
}
