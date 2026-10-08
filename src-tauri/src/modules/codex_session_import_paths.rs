use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use serde_json::Value;

pub(crate) fn validate_target_paths(mappings: &HashMap<String, String>) -> HashMap<String, String> {
    mappings
        .iter()
        .filter_map(|(source, target)| {
            let target = target.trim();
            if target.is_empty() || target == source.trim() {
                return None;
            }
            let error = if !Path::new(target).is_absolute() {
                "cwdPathAbsolute"
            } else if !Path::new(target).is_dir() {
                "cwdPathMissing"
            } else {
                return None;
            };
            Some((source.clone(), error.to_string()))
        })
        .collect()
}

fn normalized_path(value: &str) -> String {
    let mut normalized = value.trim().replace('\\', "/");
    let lower = normalized.to_ascii_lowercase();
    if lower.starts_with("//?/unc/") {
        normalized = format!("//{}", &normalized[8..]);
    } else if lower.starts_with("//?/") {
        normalized = normalized[4..].to_string();
    }
    if normalized == "/" {
        normalized
    } else {
        normalized.trim_end_matches('/').to_string()
    }
}

fn relative_workspace_path(value: &str, root: &str) -> Option<String> {
    let value = normalized_path(value);
    let root = normalized_path(root);
    if root.is_empty() {
        return None;
    }
    let windows_root = root.as_bytes().get(1) == Some(&b':') || root.starts_with("//");
    let prefix = value.get(..root.len())?;
    if !(prefix == root || windows_root && prefix.eq_ignore_ascii_case(&root)) {
        return None;
    }
    let suffix = &value[root.len()..];
    if suffix.is_empty() {
        Some(String::new())
    } else if root == "/" || suffix.starts_with('/') {
        Some(suffix.trim_start_matches('/').to_string())
    } else {
        None
    }
}

fn remap_path(value: &str, source_cwd: &str, target_cwd: &str) -> Option<String> {
    let relative = relative_workspace_path(value, source_cwd)?;
    // Paths leaving the source workspace are outside this mapping's scope.
    if relative.split('/').any(|part| part == "..") {
        return None;
    }
    if relative.is_empty() {
        Some(target_cwd.to_string())
    } else {
        Some(
            PathBuf::from(target_cwd)
                .join(relative)
                .to_string_lossy()
                .into_owned(),
        )
    }
}

fn remap_path_values(value: &mut Value, source_cwd: &str, target_cwd: &str) -> bool {
    match value {
        Value::String(path) => {
            if let Some(mapped) = remap_path(path, source_cwd, target_cwd) {
                if mapped != *path {
                    *path = mapped;
                    return true;
                }
            }
            false
        }
        Value::Array(values) => values.iter_mut().fold(false, |changed, value| {
            remap_path_values(value, source_cwd, target_cwd) || changed
        }),
        Value::Object(values) => values.values_mut().fold(false, |changed, value| {
            remap_path_values(value, source_cwd, target_cwd) || changed
        }),
        _ => false,
    }
}

fn remap_workspace_metadata(payload: &mut Value, source_cwd: &str, target_cwd: &str) -> bool {
    let mut changed = false;
    // Restrict replacements to execution metadata, leaving message records and instruction values untouched.
    for key in [
        "cwd",
        "runtime_workspace_roots",
        "workspace_roots",
        "sandbox_policy",
        "permission_profile",
        "file_system_sandbox_policy",
    ] {
        if let Some(value) = payload.get_mut(key) {
            changed = remap_path_values(value, source_cwd, target_cwd) || changed;
        }
    }
    changed
}

pub(crate) fn rewrite_rollout_workspace_paths(
    source_path: &Path,
    target_path: &Path,
    source_cwd: &str,
    target_cwd: &str,
) -> Result<(), String> {
    if source_path == target_path {
        return Err("映射副本不能覆盖原始会话文件".to_string());
    }
    let result = (|| -> Result<(), String> {
        let source = File::open(source_path).map_err(|error| error.to_string())?;
        let mut reader = BufReader::new(source);
        let target = File::create(target_path).map_err(|error| error.to_string())?;
        let mut writer = BufWriter::new(target);
        let mut line = Vec::new();
        let mut found_metadata = false;
        loop {
            line.clear();
            if reader
                .read_until(b'\n', &mut line)
                .map_err(|error| error.to_string())?
                == 0
            {
                break;
            }
            let content_end = if line.ends_with(b"\r\n") {
                line.len() - 2
            } else if line.ends_with(b"\n") {
                line.len() - 1
            } else {
                line.len()
            };
            if let Ok(mut record) = serde_json::from_slice::<Value>(&line[..content_end]) {
                let mut changed = false;
                match record.get("type").and_then(Value::as_str) {
                    Some("session_meta") => {
                        let payload = record
                            .get_mut("payload")
                            .filter(|value| value.is_object())
                            .ok_or("会话元数据格式无效，无法映射工作目录")?;
                        changed = remap_workspace_metadata(payload, source_cwd, target_cwd);
                        if !found_metadata {
                            payload["cwd"] = Value::String(target_cwd.to_string());
                            found_metadata = true;
                            changed = true;
                        }
                    }
                    Some("turn_context") => {
                        if let Some(payload) = record.get_mut("payload") {
                            changed = remap_workspace_metadata(payload, source_cwd, target_cwd);
                        }
                    }
                    Some("event_msg") => {
                        if let Some(payload) = record.get_mut("payload") {
                            if payload.get("type").and_then(Value::as_str)
                                == Some("thread_settings_applied")
                            {
                                if let Some(settings) = payload.get_mut("thread_settings") {
                                    changed =
                                        remap_workspace_metadata(settings, source_cwd, target_cwd);
                                }
                            }
                        }
                    }
                    _ => {}
                }
                if changed {
                    let mut serialized =
                        serde_json::to_vec(&record).map_err(|error| error.to_string())?;
                    serialized.extend_from_slice(&line[content_end..]);
                    line = serialized;
                }
            }
            writer.write_all(&line).map_err(|error| error.to_string())?;
        }
        if !found_metadata {
            return Err("会话文件中未找到 session_meta，无法映射工作目录".to_string());
        }
        writer.flush().map_err(|error| error.to_string())
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(target_path);
        return Err(format!(
            "映射会话工作目录失败 ({}): {}",
            source_path.display(),
            error
        ));
    }
    Ok(())
}

pub(crate) fn project_id_for_cwd(projects: &[Value], cwd: &str) -> Result<Option<String>, String> {
    let mut best_length = 0;
    let mut best_ids = HashSet::new();
    for project in projects {
        let Some(id) = project.get("id").and_then(Value::as_str) else {
            continue;
        };
        let Some(roots) = project.get("roots").and_then(Value::as_array) else {
            continue;
        };
        for root in roots {
            let Some(path) = root.get("path").and_then(Value::as_str) else {
                continue;
            };
            if relative_workspace_path(cwd, path).is_none() {
                continue;
            }
            let length = normalized_path(path).len();
            if length > best_length {
                best_length = length;
                best_ids.clear();
            }
            if length == best_length {
                best_ids.insert(id.to_string());
            }
        }
    }
    if best_ids.len() > 1 {
        return Err(format!("工作目录匹配到多个 Codex 项目: {}", cwd));
    }
    Ok(best_ids.into_iter().next())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn validates_mapping_targets_without_creating_missing_directories() {
        let dir = temp_dir("validation");
        let missing = dir.join("missing");
        let mappings = HashMap::from([
            ("source-good".into(), dir.to_string_lossy().into_owned()),
            ("source-relative".into(), "relative/project".into()),
            (
                "source-missing".into(),
                missing.to_string_lossy().into_owned(),
            ),
            ("source-empty".into(), " ".into()),
        ]);
        let errors = validate_target_paths(&mappings);
        assert_eq!(errors.len(), 2);
        assert_eq!(errors["source-relative"], "cwdPathAbsolute");
        assert_eq!(errors["source-missing"], "cwdPathMissing");
        assert!(!missing.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn mapping_does_not_rewrite_parent_traversals_or_destroy_the_source() {
        assert_eq!(
            remap_path("/old/repo/../elsewhere", "/old/repo", "/new/repo"),
            None
        );
        let dir = temp_dir("same-source");
        let source = dir.join("source.jsonl");
        fs::write(&source, b"original").unwrap();
        assert!(rewrite_rollout_workspace_paths(&source, &source, "/old", "/new").is_err());
        assert_eq!(fs::read(&source).unwrap(), b"original");
        fs::remove_dir_all(dir).unwrap();
    }

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "cockpit-import-paths-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn remaps_foreign_windows_paths_and_children_without_matching_siblings() {
        let target = std::env::temp_dir()
            .join("new-project")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            remap_path(r"c:\OLD\repo", r"C:\old\repo", &target),
            Some(target.clone())
        );
        assert_eq!(
            remap_path(r"C:\old\repo\src", r"C:\old\repo", &target),
            Some(
                PathBuf::from(&target)
                    .join("src")
                    .to_string_lossy()
                    .into_owned()
            )
        );
        assert_eq!(
            remap_path(r"C:\old\repo-other", r"C:\old\repo", &target),
            None
        );
        assert_eq!(
            remap_path(r"\\?\C:\old\repo", r"C:\old\repo", &target),
            Some(target.clone())
        );
        assert_eq!(
            remap_path(r"\\?\UNC\host\repo", r"\\host\repo", &target),
            Some(target.clone())
        );
        assert_eq!(remap_path("/old/Repo", "/old/repo", &target), None);
    }

    #[test]
    fn remaps_resume_snapshots_contexts_and_permission_roots_but_preserves_messages() {
        let dir = temp_dir("snapshots");
        let source = dir.join("source.jsonl");
        let target = dir.join("target.jsonl");
        let cwd = dir.join("project-b").to_string_lossy().into_owned();
        let records = [
            json!({"type":"session_meta","payload":{"id":"test","cwd":"/a/project","runtime_workspace_roots":["/a/project","/other"],"base_instructions":{"text":"/a/project"}}}),
            json!({"type":"turn_context","payload":{"cwd":"/a/project","workspace_roots":["/a/project/src"],"sandbox_policy":{"writable_roots":["/a/project"]}}}),
            json!({"type":"event_msg","payload":{"type":"thread_settings_applied","thread_settings":{"cwd":"/a/project","runtime_workspace_roots":["/a/project"],"permission_profile":{"file_system":{"entries":[{"path":{"type":"path","path":"/a/project"}}]}}}}}),
        ];
        let message = b"{ \"type\": \"event_msg\", \"payload\": {\"type\":\"user_message\", \"message\":\"/a/project\"}}\r\n";
        let mut content = records
            .iter()
            .map(|value| format!("{value}\r\n"))
            .collect::<String>()
            .into_bytes();
        content.extend_from_slice(message);
        fs::write(&source, &content).unwrap();
        rewrite_rollout_workspace_paths(&source, &target, "/a/project", &cwd).unwrap();
        let rewritten = fs::read(&target).unwrap();
        assert!(rewritten.ends_with(message));
        let parsed = String::from_utf8(rewritten)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(parsed[0]["payload"]["cwd"], cwd);
        assert_eq!(parsed[0]["payload"]["runtime_workspace_roots"][1], "/other");
        assert_eq!(
            parsed[0]["payload"]["base_instructions"]["text"],
            "/a/project"
        );
        assert_eq!(parsed[1]["payload"]["cwd"], cwd);
        assert_eq!(
            parsed[1]["payload"]["sandbox_policy"]["writable_roots"][0],
            cwd
        );
        assert_eq!(parsed[2]["payload"]["thread_settings"]["cwd"], cwd);
        assert_eq!(
            parsed[2]["payload"]["thread_settings"]["permission_profile"]["file_system"]["entries"]
                [0]["path"]["path"],
            cwd
        );
        assert_eq!(fs::read(&source).unwrap(), content);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn preserves_missing_final_newline_and_cleans_failed_output() {
        let dir = temp_dir("invalid");
        let source = dir.join("source.jsonl");
        let target = dir.join("target.jsonl");
        fs::write(
            &source,
            b"{\"type\":\"session_meta\",\"payload\":{\"cwd\":\"/a\"}}",
        )
        .unwrap();
        rewrite_rollout_workspace_paths(&source, &target, "/a", "/b").unwrap();
        assert!(!fs::read(&target).unwrap().ends_with(b"\n"));
        fs::write(&source, b"{\"type\":\"event_msg\",\"payload\":{}}\n").unwrap();
        assert!(rewrite_rollout_workspace_paths(&source, &target, "/a", "/b").is_err());
        assert!(!target.exists());
        fs::write(&source, b"{\"type\":\"session_meta\",\"payload\":null}\n").unwrap();
        assert!(rewrite_rollout_workspace_paths(&source, &target, "/a", "/b").is_err());
        assert!(!target.exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn selects_existing_project_by_longest_root_and_rejects_ambiguity() {
        let projects = vec![
            json!({"id":"parent","roots":[{"path":"/b"}]}),
            json!({"id":"child","roots":[{"path":"/b/project"}]}),
        ];
        assert_eq!(
            project_id_for_cwd(&projects, "/b/project/src").unwrap(),
            Some("child".into())
        );
        assert_eq!(project_id_for_cwd(&projects, "/b-other").unwrap(), None);
        let duplicates = vec![
            json!({"id":"one","roots":[{"path":"C:\\B"}]}),
            json!({"id":"two","roots":[{"path":"c:/b/"}]}),
        ];
        assert!(project_id_for_cwd(&duplicates, "C:\\B").is_err());
        let mut nested = duplicates;
        nested.push(json!({"id":"nested","roots":[{"path":"C:\\B\\project"}]}));
        assert_eq!(
            project_id_for_cwd(&nested, "C:\\B\\project").unwrap(),
            Some("nested".into())
        );
    }
}
