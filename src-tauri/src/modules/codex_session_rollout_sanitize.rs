//! 会话日志（rollout `*.jsonl`）里的第三方推理签名清洗。
//!
//! 背景：第三方 Responses 上游（DeepSeek 等）会在 reasoning 项里带上假的
//! `encrypted_content`（形如 `<response-id>-0`）。Codex 客户端把它写进会话日志后，
//! ChatGPT 账号的自动压缩（remote compact）会把这段历史原样发给官方后端并失败：
//! `The encrypted content ... could not be verified. Reason: Encrypted content could not be decrypted or parsed.`
//!
//! 网关响应出口已经不再产生这类字段（见 sidecar `responses_reasoning_sanitize.go`），
//! 这里负责一次性清理已经落盘的旧日志：只删除非官方格式的 `encrypted_content`，
//! 其它字段与行结构保持不变。正在写入的会话日志会跳过并记录，下次启动再补做。

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::modules;

const SESSION_DIRS: [&str; 2] = ["sessions", "archived_sessions"];
const ENCRYPTED_CONTENT_FIELD: &str = "encrypted_content";
const SCAN_CHUNK_BYTES: usize = 64 * 1024;
/// 最近修改过的会话日志可能仍被客户端写入，跳过以免与追加写冲突。
const ACTIVE_FILE_GRACE_SECONDS: u64 = 300;

/// 一次清洗结果，用于日志与上层判断。
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct CodexRolloutSanitizeSummary {
    pub files_scanned: usize,
    pub files_changed: usize,
    pub removed_signatures: usize,
    pub pending_files: Vec<String>,
}

impl CodexRolloutSanitizeSummary {
    pub fn changed_anything(&self) -> bool {
        self.files_changed > 0
    }
}

/// 被删除的假签名记录，写入回滚备份。
#[derive(Debug, Serialize)]
pub struct CodexRolloutSignatureBackup {
    pub file: String,
    pub item_id: String,
    pub encrypted_content: String,
}

/// 清洗指定 profile 目录下所有会话日志里的第三方推理签名（幂等）。
///
/// `retry_files` 传入上次跳过的活跃文件时只处理这些文件，避免每次都全量扫描。
pub fn sanitize_official_incompatible_rollout_signatures(
    data_dir: &Path,
    retry_files: &[PathBuf],
) -> Result<
    (
        CodexRolloutSanitizeSummary,
        Vec<CodexRolloutSignatureBackup>,
    ),
    String,
> {
    sanitize_rollout_signatures_with_grace(data_dir, retry_files, ACTIVE_FILE_GRACE_SECONDS)
}

fn sanitize_rollout_signatures_with_grace(
    data_dir: &Path,
    retry_files: &[PathBuf],
    active_file_grace_seconds: u64,
) -> Result<
    (
        CodexRolloutSanitizeSummary,
        Vec<CodexRolloutSignatureBackup>,
    ),
    String,
> {
    let candidates = if retry_files.is_empty() {
        collect_rollout_candidates(data_dir)
    } else {
        retry_files.to_vec()
    };

    let mut summary = CodexRolloutSanitizeSummary::default();
    let mut backups = Vec::new();
    for path in candidates {
        let Some((content, metadata)) = read_rollout_if_relevant(&path)? else {
            continue;
        };
        summary.files_scanned += 1;
        if is_recently_modified(&path, active_file_grace_seconds) {
            summary
                .pending_files
                .push(path.to_string_lossy().to_string());
            continue;
        }
        let (updated, removed) = sanitize_rollout_content(&content);
        if removed == 0 {
            continue;
        }
        // 扫描期间被客户端追加写的文件不覆盖，留给下次启动处理。
        if changed_since_read(&path, &metadata) {
            summary
                .pending_files
                .push(path.to_string_lossy().to_string());
            continue;
        }
        write_rollout_file(&path, &updated)?;
        backups.extend(removed_signatures(&content, &path));
        summary.files_changed += 1;
        summary.removed_signatures += removed;
        modules::logger::log_info(&format!(
            "[Codex Rollout Sanitize] 已清理会话日志推理签名: file={}, removed={}",
            path.display(),
            removed
        ));
    }
    Ok((summary, backups))
}

fn changed_since_read(path: &Path, metadata: &fs::Metadata) -> bool {
    let Ok(current) = fs::metadata(path) else {
        return true;
    };
    current.len() != metadata.len()
        || !super::codex_session_file_time::same_modified_time_millis(
            current.modified().ok(),
            metadata.modified().ok(),
        )
}

fn collect_rollout_candidates(data_dir: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for dir_name in SESSION_DIRS {
        collect_rollout_files(&data_dir.join(dir_name), &mut paths);
    }
    paths.sort();
    paths
}

fn collect_rollout_files(dir: &Path, paths: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rollout_files(&path, paths);
            continue;
        }
        if path.extension().and_then(|extension| extension.to_str()) == Some("jsonl") {
            paths.push(path);
        }
    }
}

/// 先做一次子串扫描：不含 `encrypted_content` 的文件直接跳过，不解析 JSON。
fn read_rollout_if_relevant(path: &Path) -> Result<Option<(String, fs::Metadata)>, String> {
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return Ok(None),
    };
    let metadata = file
        .metadata()
        .map_err(|error| format!("读取会话日志状态失败 ({}): {}", path.display(), error))?;
    let mut buffer = vec![0u8; SCAN_CHUNK_BYTES];
    let mut matched = false;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("读取会话日志失败 ({}): {}", path.display(), error))?;
        if read == 0 {
            break;
        }
        if buffer[..read]
            .windows(ENCRYPTED_CONTENT_FIELD.len())
            .any(|window| window == ENCRYPTED_CONTENT_FIELD.as_bytes())
        {
            matched = true;
            break;
        }
    }
    if !matched {
        return Ok(None);
    }

    match fs::read_to_string(path) {
        Ok(content) => Ok(Some((content, metadata))),
        // 非 UTF-8（例如压缩会话）保持原样，避免破坏文件。
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
            modules::logger::log_warn(&format!(
                "[Codex Rollout Sanitize] 跳过非 UTF-8 会话日志 ({}): {}",
                path.display(),
                error
            ));
            Ok(None)
        }
        Err(error) => Err(format!("读取会话日志失败 ({}): {}", path.display(), error)),
    }
}

fn is_recently_modified(path: &Path, active_file_grace_seconds: u64) -> bool {
    if active_file_grace_seconds == 0 {
        return false;
    }
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    modified
        .elapsed()
        .map(|elapsed| elapsed.as_secs() < active_file_grace_seconds)
        .unwrap_or(false)
}

/// 原子写回，并保留原修改时间，避免会话列表顺序被这次清洗打乱。
fn write_rollout_file(path: &Path, content: &str) -> Result<(), String> {
    let original_modified_at = super::codex_session_file_time::read_modified_time(path);
    super::atomic_write::write_bytes_atomic(path, content.as_bytes())?;
    super::codex_session_file_time::restore_modified_time(path, original_modified_at)
}

/// 逐行清洗：只删除 reasoning 项里非官方格式的 `encrypted_content`。
fn sanitize_rollout_content(content: &str) -> (String, usize) {
    let mut removed = 0;
    let mut result = String::with_capacity(content.len());
    for line in content.split_inclusive('\n') {
        let (body, eol) = split_line_ending(line);
        if !body.contains(ENCRYPTED_CONTENT_FIELD) {
            result.push_str(line);
            continue;
        }
        match sanitize_rollout_line(body) {
            Some(sanitized) => {
                removed += 1;
                result.push_str(&sanitized);
                result.push_str(eol);
            }
            None => result.push_str(line),
        }
    }
    if removed == 0 {
        return (content.to_string(), 0);
    }
    (result, removed)
}

fn split_line_ending(line: &str) -> (&str, &str) {
    if let Some(body) = line.strip_suffix("\r\n") {
        return (body, "\r\n");
    }
    if let Some(body) = line.strip_suffix('\n') {
        return (body, "\n");
    }
    (line, "")
}

/// 返回需要替换的行内容；无需改动时返回 None。
fn sanitize_rollout_line(line: &str) -> Option<String> {
    let mut record: Value = serde_json::from_str(line).ok()?;
    let mut removed = 0;
    strip_invalid_reasoning_signatures(&mut record, &mut removed);
    if removed == 0 {
        return None;
    }
    serde_json::to_string(&record).ok()
}

/// 递归清洗：只处理 `type == "reasoning"` 的对象，正文里的同名字符串不受影响。
/// 覆盖 reasoning 项可能出现的所有容器（顶层 payload、replacement_history、guardian_history 等）。
fn strip_invalid_reasoning_signatures(node: &mut Value, removed: &mut usize) {
    match node {
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("reasoning") {
                match map.get(ENCRYPTED_CONTENT_FIELD) {
                    Some(Value::String(value)) if is_official_gpt_signature(value) => {}
                    Some(_) => {
                        map.remove(ENCRYPTED_CONTENT_FIELD);
                        *removed += 1;
                    }
                    None => {}
                }
            }
            for value in map.values_mut() {
                strip_invalid_reasoning_signatures(value, removed);
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                strip_invalid_reasoning_signatures(item, removed);
            }
        }
        _ => {}
    }
}

fn removed_signatures(content: &str, path: &Path) -> Vec<CodexRolloutSignatureBackup> {
    let mut backups = Vec::new();
    let reader = BufReader::new(content.as_bytes());
    for line in reader.lines().map_while(Result::ok) {
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        collect_invalid_signatures(&record, path, &mut backups);
    }
    backups
}

fn collect_invalid_signatures(
    node: &Value,
    path: &Path,
    backups: &mut Vec<CodexRolloutSignatureBackup>,
) {
    match node {
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("reasoning") {
                if let Some(value) = map.get(ENCRYPTED_CONTENT_FIELD).and_then(Value::as_str) {
                    if !is_official_gpt_signature(value) {
                        backups.push(CodexRolloutSignatureBackup {
                            file: path.to_string_lossy().to_string(),
                            item_id: map
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                            encrypted_content: value.to_string(),
                        });
                    }
                }
            }
            for value in map.values() {
                collect_invalid_signatures(value, path, backups);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_invalid_signatures(item, path, backups);
            }
        }
        _ => {}
    }
}

/// 官方签名判定与请求侧清洗保持一致，避免两处规则漂移。
fn is_official_gpt_signature(raw_signature: &str) -> bool {
    crate::modules::codex_local_access::is_valid_gpt_reasoning_signature(raw_signature)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("{}-{}-{}", prefix, std::process::id(), unique));
        if dir.exists() {
            fs::remove_dir_all(&dir).expect("cleanup");
        }
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn test_official_signature() -> String {
        let mut payload = vec![0x80];
        payload.extend([0u8; 8]);
        payload.extend([1u8; 16]);
        payload.extend([2u8; 16]);
        payload.extend([3u8; 32]);
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload)
    }

    #[test]
    fn sanitize_rollout_content_removes_third_party_signature_only() {
        let official = test_official_signature();
        let content = format!(
            "{{\"ordinal\":11,\"payload\":{{\"type\":\"reasoning\",\"id\":\"r1\",\"encrypted_content\":\"bf0ff0e2-ffac-4bf0-a39a-80b340896e1c-0\"}}}}\n\
             {{\"ordinal\":12,\"payload\":{{\"type\":\"reasoning\",\"id\":\"r2\",\"encrypted_content\":\"{}\"}}}}\n\
             {{\"ordinal\":13,\"payload\":{{\"type\":\"agentMessage\",\"text\":\"encrypted_content is just text\"}}}}\n",
            official
        );

        let (updated, removed) = sanitize_rollout_content(&content);

        assert_eq!(removed, 1);
        assert!(!updated.contains("bf0ff0e2-ffac-4bf0-a39a-80b340896e1c-0"));
        assert!(updated.contains(&official), "官方签名必须保留");
        assert!(updated.contains("encrypted_content is just text"));
        assert!(updated.ends_with('\n'));
    }

    #[test]
    fn sanitize_rollout_content_keeps_untouched_files_byte_identical() {
        let content = "{\"ordinal\":1,\"payload\":{\"type\":\"reasoning\",\"id\":\"r1\"}}\n";
        let (updated, removed) = sanitize_rollout_content(content);
        assert_eq!(removed, 0);
        assert_eq!(updated, content);
    }

    #[test]
    fn sanitize_rollout_content_covers_nested_reasoning_history() {
        let official = test_official_signature();
        let content = format!(
            "{{\"ordinal\":18,\"type\":\"compacted\",\"payload\":{{\"replacement_history\":[{{\"type\":\"reasoning\",\"id\":\"r1\",\"encrypted_content\":\"1a82bcbc-013b-486b-8e51-0d6621158973-0\"}},{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"encrypted_content 是文本\"}}]}}],\"guardian_history\":[{{\"type\":\"reasoning\",\"id\":\"r2\",\"encrypted_content\":\"{}\"}}]}}}}\n",
            official
        );

        let (updated, removed) = sanitize_rollout_content(&content);

        assert_eq!(removed, 1);
        assert!(!updated.contains("1a82bcbc-013b-486b-8e51-0d6621158973-0"));
        assert!(updated.contains(&official), "官方签名必须保留");
        assert!(
            updated.contains("encrypted_content 是文本"),
            "正文文本不得改动"
        );
    }

    #[test]
    fn sanitize_official_incompatible_rollout_signatures_rewrites_and_reports() {
        let dir = make_temp_dir("codex-rollout-sanitize");
        let session_dir = dir.join("sessions").join("2026").join("09").join("14");
        fs::create_dir_all(&session_dir).expect("create session dir");
        let rollout = session_dir.join("rollout-1.jsonl");
        fs::write(
            &rollout,
            "{\"ordinal\":1,\"payload\":{\"type\":\"reasoning\",\"id\":\"r1\",\"encrypted_content\":\"73cbd22c-9f79-439f-b5a3-001d40bb4a80-0\"}}\n",
        )
        .expect("write rollout");

        let (summary, backups) =
            sanitize_rollout_signatures_with_grace(&dir, &[], 0).expect("sanitize");

        assert_eq!(summary.files_changed, 1);
        assert_eq!(summary.removed_signatures, 1);
        assert!(summary.pending_files.is_empty());
        assert_eq!(backups.len(), 1);
        assert_eq!(backups[0].item_id, "r1");
        let updated = fs::read_to_string(&rollout).expect("read rollout");
        assert!(!updated.contains("encrypted_content"));

        // 幂等：再次执行不再有改动。
        let (second, _) =
            sanitize_rollout_signatures_with_grace(&dir, &[], 0).expect("sanitize again");
        assert_eq!(second.files_changed, 0);
    }

    #[test]
    fn recently_written_rollout_files_are_deferred() {
        let dir = make_temp_dir("codex-rollout-sanitize-active");
        let session_dir = dir.join("sessions");
        fs::create_dir_all(&session_dir).expect("create session dir");
        let rollout = session_dir.join("rollout-1.jsonl");
        fs::write(
            &rollout,
            "{\"ordinal\":1,\"payload\":{\"type\":\"reasoning\",\"id\":\"r1\",\"encrypted_content\":\"73cbd22c-9f79-439f-b5a3-001d40bb4a80-0\"}}\n",
        )
        .expect("write rollout");

        let (summary, backups) =
            sanitize_official_incompatible_rollout_signatures(&dir, &[]).expect("sanitize");

        assert_eq!(summary.files_changed, 0);
        assert_eq!(summary.pending_files.len(), 1);
        assert!(backups.is_empty());
        assert!(fs::read_to_string(&rollout)
            .expect("read rollout")
            .contains("encrypted_content"));

        // 下次启动按待处理清单重试（这里用 0 秒宽限期模拟文件已不再活跃）。
        let retry_files = summary
            .pending_files
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        let (retried, _) =
            sanitize_rollout_signatures_with_grace(&dir, &retry_files, 0).expect("retry");
        assert_eq!(retried.files_changed, 1);
        assert!(retried.pending_files.is_empty());
    }
}
