//! Read-only projection diagnostics. Recovery candidates never overwrite a source.
use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::ops::Range;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::{de::IgnoredAny, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const MAX_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryIssue {
    pub code: &'static str,
    pub line: Option<usize>,
    pub blocks_rewrite: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryHealth {
    pub thread_id: Option<String>,
    pub source_sha256: String,
    pub rollout_bytes: u64,
    pub records: usize,
    pub cursor_offset: Option<u64>,
    pub cursor_ordinal: Option<u64>,
    pub inherited_history_end: Option<u64>,
    pub projected_turns: Option<u64>,
    pub projected_items: Option<u64>,
    pub recovery_plan: Option<ProjectionRecoveryPlan>,
    pub issues: Vec<HistoryIssue>,
    #[serde(skip)]
    has_blocking_issue: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionRecoveryPlan {
    pub kind: &'static str,
    pub from_offset: u64,
    pub to_offset: u64,
    pub next_ordinal: u64,
    pub skipped_lines: usize,
    pub skipped_kinds: Vec<String>,
    pub source_sha256: String,
}

impl HistoryHealth {
    pub fn blocks_rewrite(&self) -> bool {
        self.has_blocking_issue
    }

    fn issue(&mut self, code: &'static str, line: Option<usize>, blocks_rewrite: bool) {
        self.has_blocking_issue |= blocks_rewrite;
        // Keep reports bounded even when a file contains thousands of bad records.
        if self.issues.len() < 100 {
            self.issues.push(HistoryIssue {
                code,
                line,
                blocks_rewrite,
            });
        } else if blocks_rewrite && !self.issues.iter().any(|issue| issue.blocks_rewrite) {
            self.issues[99] = HistoryIssue {
                code,
                line,
                blocks_rewrite,
            };
        }
    }
}

pub fn inspect_bytes(bytes: &[u8], cursor: Option<(u64, u64)>) -> HistoryHealth {
    let mut report = HistoryHealth {
        thread_id: None,
        source_sha256: format!("{:x}", Sha256::digest(bytes)),
        rollout_bytes: bytes.len() as u64,
        records: 0,
        cursor_offset: cursor.map(|value| value.0),
        cursor_ordinal: cursor.map(|value| value.1),
        inherited_history_end: None,
        projected_turns: None,
        projected_items: None,
        recovery_plan: None,
        issues: Vec::new(),
        has_blocking_issue: false,
    };
    let mut offset = 0u64;
    let mut previous = None;
    let mut indexed = false;
    let mut cursor_boundary = cursor.is_none();
    let mut seen = HashSet::new();
    for (index, line) in bytes.split_inclusive(|byte| *byte == b'\n').enumerate() {
        let number = Some(index + 1);
        let record: Value = match serde_json::from_slice(line) {
            Ok(value) => value,
            Err(_) => {
                report.issue("invalid_json", number, true);
                break;
            }
        };
        report.records += 1;
        if index == 0 {
            report.inherited_history_end = record
                .pointer("/payload/subagent_history_start_ordinal")
                .and_then(Value::as_u64);
            report.thread_id = record
                .pointer("/payload/id")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let mode = record
                .pointer("/payload/history_mode")
                .and_then(Value::as_str);
            indexed = record.get("ordinal").is_some() || mode == Some("paginated");
            if record["type"] != "session_meta" || report.thread_id.is_none() {
                report.issue("invalid_session_meta", number, true);
            }
            if mode.is_some_and(|mode| mode != "legacy" && mode != "paginated") {
                report.issue("unsupported_history_mode", number, true);
            }
            if !indexed {
                report.issue("legacy_history", number, false);
            }
        }
        let ordinal = record.get("ordinal").and_then(Value::as_u64);
        if indexed {
            if ordinal_span(line).is_err() {
                report.issue("ambiguous_record", number, true);
            }
            match ordinal {
                None => report.issue("missing_ordinal", number, true),
                Some(value) => {
                    if !seen.insert(value) {
                        report.issue("duplicate_ordinal", number, true);
                    }
                    if previous.is_some_and(|prev| value < prev) {
                        report.issue("ordinal_regressed", number, true);
                    } else if previous
                        .and_then(|prev: u64| prev.checked_add(1))
                        .is_some_and(|next| value > next)
                    {
                        // Gaps alone do not prove that projection is broken.
                        report.issue("ordinal_gap", number, false);
                    }
                    previous = Some(value);
                }
            }
        }
        if let Some((saved_offset, next_ordinal)) = cursor {
            if saved_offset == offset {
                cursor_boundary = true;
                if indexed && ordinal.is_some_and(|value| value < next_ordinal) {
                    report.issue("cursor_ordinal_mismatch", number, true);
                }
            }
        }
        offset += line.len() as u64;
    }
    if bytes.is_empty() {
        report.issue("empty_rollout", None, true);
    } else if !bytes.ends_with(b"\n") {
        report.issue("incomplete_tail", Some(report.records), true);
    }
    if report
        .inherited_history_end
        .is_some_and(|end| previous.is_some_and(|last| last < end))
    {
        report.issue("all_records_inherited", None, false);
    }
    if let Some((saved_offset, next_ordinal)) = cursor {
        if saved_offset == bytes.len() as u64 {
            cursor_boundary = true;
            if indexed && previous.and_then(|last: u64| last.checked_add(1)) != Some(next_ordinal) {
                report.issue("cursor_ordinal_mismatch", None, true);
            }
        } else if saved_offset < bytes.len() as u64 {
            report.issue("projection_pending", None, false);
        }
        if !cursor_boundary {
            report.issue("cursor_offset_mismatch", None, true);
        }
    }
    report
}

fn read_rollout(rollout: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(rollout).map_err(|_| "rollout_unreadable")?;
    if !metadata.file_type().is_file() || metadata.len() > MAX_BYTES {
        return Err("rollout_unsupported_file".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(rollout)
        .map_err(|_| "rollout_unreadable")?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "rollout_unreadable")?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("rollout_too_large".into());
    }
    let after = fs::symlink_metadata(rollout).map_err(|_| "rollout_unreadable")?;
    if metadata.len() != after.len() || metadata.modified().ok() != after.modified().ok() {
        return Err("source_changed_during_scan".into());
    }
    Ok(bytes)
}

pub fn inspect(rollout: &Path, projection_db: &Path) -> Result<HistoryHealth, String> {
    inspect_with_hook(rollout, projection_db, || {})
}

fn inspect_with_hook(rollout: &Path, projection_db: &Path, after_read: impl FnOnce()) -> Result<HistoryHealth, String> {
    let bytes = read_rollout(rollout)?;
    after_read();
    let mut report = inspect_bytes(&bytes, None);
    if projection_db.exists() {
        let read_cursor = || -> rusqlite::Result<Option<(u64, u64)>> {
            let db = Connection::open_with_flags(projection_db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            db.busy_timeout(std::time::Duration::from_millis(250))?;
            db.query_row("SELECT next_rollout_byte_offset,next_rollout_ordinal FROM thread_history_projection_state WHERE thread_id=?1",
                [report.thread_id.as_deref().unwrap_or("")], |row| Ok((row.get(0)?, row.get(1)?))).optional()
        };
        match read_cursor() {
            Ok(Some(cursor)) => report = inspect_bytes(&bytes, Some(cursor)),
            Ok(None) => report.issue("projection_missing", None, false),
            Err(_) => report.issue("projection_unreadable", None, true),
        }
    } else {
        report.issue("projection_missing", None, false);
    }
    if let Ok(db) = Connection::open_with_flags(projection_db, OpenFlags::SQLITE_OPEN_READ_ONLY) {
        let tid = report.thread_id.as_deref().unwrap_or("");
        report.projected_turns = db
            .query_row(
                "SELECT count(*) FROM thread_turns WHERE thread_id=?1",
                [tid],
                |row| row.get(0),
            )
            .ok();
        report.projected_items = db
            .query_row(
                "SELECT count(*) FROM thread_items WHERE thread_id=?1",
                [tid],
                |row| row.get(0),
            )
            .ok();
    }
    if let (Some(offset), Some(ordinal)) = (report.cursor_offset, report.cursor_ordinal) {
        report.recovery_plan = plan_duplicate_cursor_recovery(&bytes, (offset, ordinal)).ok();
    }
    // A writer may advance the checkpoint after the first read. Never report the
    // resulting mixed snapshot as corruption. Also catches equal-size replacements.
    if read_rollout(rollout)? != bytes {
        return Err("source_changed_during_scan".into());
    }
    Ok(report)
}

fn resolve_rollout(data_dir: &Path, thread_id: &str) -> Result<PathBuf, String> {
    if thread_id.trim().is_empty() || thread_id.len() > 200 {
        return Err("invalid_thread_id".into());
    }
    let db = Connection::open_with_flags(
        data_dir.join("state_5.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|_| "state_database_unreadable")?;
    let rollout_path: String = db
        .query_row(
            "SELECT rollout_path FROM threads WHERE id=?1",
            [thread_id],
            |row| row.get(0),
        )
        .map_err(|_| "thread_not_found")?;
    let canonical_home = data_dir.canonicalize().map_err(|_| "instance_unreadable")?;
    let canonical_rollout = data_dir
        .join(&rollout_path)
        .canonicalize()
        .map_err(|_| "rollout_unreadable")?;
    let relative = canonical_rollout
        .strip_prefix(&canonical_home)
        .map_err(|_| "rollout_outside_instance")?;
    let root = relative
        .components()
        .next()
        .and_then(|component| component.as_os_str().to_str());
    if !matches!(root, Some("sessions") | Some("archived_sessions")) {
        return Err("rollout_outside_session_directory".into());
    }
    Ok(canonical_rollout)
}

pub fn inspect_thread(data_dir: &Path, thread_id: &str) -> Result<HistoryHealth, String> {
    let report = inspect(
        &resolve_rollout(data_dir, thread_id)?,
        &data_dir.join("thread_history_1.sqlite"),
    )?;
    if report.thread_id.as_deref() != Some(thread_id) {
        return Err("thread_identity_mismatch".into());
    }
    Ok(report)
}

/// Build, but do not apply, a cursor-only recovery for a verified duplicate metadata boundary.
/// A real message or an unknown record makes the plan ineligible.
pub fn plan_duplicate_cursor_recovery(
    bytes: &[u8],
    cursor: (u64, u64),
) -> Result<ProjectionRecoveryPlan, String> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err("rollout_too_large".into());
    }
    let first: Value = serde_json::from_slice(
        bytes
            .split_inclusive(|byte| *byte == b'\n')
            .next()
            .ok_or("empty_rollout")?,
    )
    .map_err(|_| "invalid_json")?;
    if first
        .pointer("/payload/history_mode")
        .and_then(Value::as_str)
        != Some("paginated")
        || first
            .pointer("/payload/history_base")
            .is_some_and(|value| !value.is_null())
        || first
            .pointer("/payload/subagent_history_start_ordinal")
            .is_some_and(|value| !value.is_null())
    {
        return Err("unsupported_recovery_input".into());
    }
    let (from_offset, next_ordinal) = cursor;
    if from_offset >= bytes.len() as u64
        || from_offset == 0
        || bytes[from_offset as usize - 1] != b'\n'
    {
        return Err("cursor_offset_out_of_bounds".into());
    }
    if inspect_bytes(&bytes[..from_offset as usize], Some(cursor)).blocks_rewrite() {
        return Err("projection_prefix_mismatch".into());
    }
    let health = inspect_bytes(bytes, Some(cursor));
    if health.issues.iter().any(|issue| {
        !matches!(
            issue.code,
            "duplicate_ordinal"
                | "ordinal_regressed"
                | "cursor_ordinal_mismatch"
                | "projection_pending"
        )
    }) {
        return Err("unsupported_recovery_input".into());
    }
    let mut offset = from_offset as usize;
    let mut skipped = 0usize;
    let mut kinds = Vec::new();
    while offset < bytes.len() {
        let end = bytes[offset..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|position| offset + position + 1)
            .ok_or("incomplete_tail")?;
        let line = &bytes[offset..end];
        let record: Value = serde_json::from_slice(line).map_err(|_| "invalid_json")?;
        let ordinal = record
            .get("ordinal")
            .and_then(Value::as_u64)
            .ok_or("missing_ordinal")?;
        if ordinal > next_ordinal {
            return Err("recovery_boundary_has_gap".into());
        }
        if ordinal == next_ordinal {
            if skipped == 0 {
                return Err("cursor_does_not_start_at_duplicate".into());
            }
            validate_recovery_tail(&bytes[offset..], next_ordinal)?;
            return Ok(ProjectionRecoveryPlan {
                kind: "duplicate_metadata_cursor",
                from_offset,
                to_offset: offset as u64,
                next_ordinal,
                skipped_lines: skipped,
                skipped_kinds: kinds,
                source_sha256: format!("{:x}", Sha256::digest(bytes)),
            });
        }
        let top_level_type = record.get("type").and_then(Value::as_str).unwrap_or("");
        let payload_type = record
            .pointer("/payload/type")
            .and_then(Value::as_str)
            .unwrap_or("");
        let safe = top_level_type == "event_msg"
            && matches!(payload_type, "token_count" | "thread_settings_applied");
        if !safe {
            return Err("unsafe_duplicate_cursor_boundary".into());
        }
        kinds.push(payload_type.to_string());
        skipped += 1;
        offset = end;
    }
    Err("recovery_boundary_not_found".into())
}

/// The bounded UI issue list is never a validation authority. A cursor-only
/// candidate must have a complete, continuous tail, including after its first turn.
fn validate_recovery_tail(bytes: &[u8], mut expected: u64) -> Result<(), String> {
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        if !line.ends_with(b"\n") { return Err("incomplete_tail".into()); }
        let record: Value = serde_json::from_slice(line).map_err(|_| "invalid_json")?;
        ordinal_span(line)?;
        if record.get("ordinal").and_then(Value::as_u64) != Some(expected) {
            return Err("recovery_tail_not_continuous".into());
        }
        expected = expected.checked_add(1).ok_or("ordinal_overflow")?;
    }
    Ok(())
}

/// Private: the caller owns this freshly created connection, never a user-selected database.
fn apply_cursor_plan(
    connection: &mut Connection,
    thread_id: &str,
    plan: &ProjectionRecoveryPlan,
    source: &[u8],
) -> Result<(), String> {
    let actual_hash = format!("{:x}", Sha256::digest(source));
    if actual_hash != plan.source_sha256 {
        return Err("rollout_changed_since_plan".into());
    }
    if inspect_bytes(source, None).thread_id.as_deref() != Some(thread_id) {
        return Err("thread_identity_mismatch".into());
    }
    if plan_duplicate_cursor_recovery(source, (plan.from_offset, plan.next_ordinal))? != *plan {
        return Err("recovery_plan_mismatch".into());
    }
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|_| "recovery_database_locked")?;
    let current: Option<(u64, u64)> = transaction
        .query_row(
            "SELECT next_rollout_byte_offset,next_rollout_ordinal FROM thread_history_projection_state WHERE thread_id=?1",
            [thread_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|_| "recovery_cursor_unreadable")?;
    if current != Some((plan.from_offset, plan.next_ordinal)) {
        return Err("recovery_cursor_changed_since_plan".into());
    }
    let changed = transaction
        .execute(
            "UPDATE thread_history_projection_state SET next_rollout_byte_offset=?1,next_rollout_ordinal=?2 WHERE thread_id=?3 AND next_rollout_byte_offset=?4 AND next_rollout_ordinal=?5",
            rusqlite::params![plan.to_offset, plan.next_ordinal, thread_id, plan.from_offset, plan.next_ordinal],
        )
        .map_err(|_| "recovery_cursor_update_failed")?;
    if changed != 1 {
        return Err("recovery_cursor_update_conflict".into());
    }
    transaction
        .commit()
        .map_err(|_| "recovery_commit_failed".into())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryCopy {
    pub directory: String,
    pub thread_id: String,
    pub plan: ProjectionRecoveryPlan,
    pub status: &'static str,
}

/// Creates private local artifacts only. There is deliberately no command to install these
/// over a live Codex database. The copy can contain other threads and must not be uploaded.
pub fn create_recovery_copy(
    data_dir: &Path,
    thread_id: &str,
    expected_hash: &str,
    output_root: &Path,
) -> Result<RecoveryCopy, String> {
    let rollout = resolve_rollout(data_dir, thread_id)?;
    let bytes = read_rollout(&rollout)?;
    let health = inspect_bytes(&bytes, None);
    if health.thread_id.as_deref() != Some(thread_id) {
        return Err("thread_identity_mismatch".into());
    }
    if health.source_sha256 != expected_hash {
        return Err("rollout_changed_since_plan".into());
    }
    let database = Connection::open_with_flags(
        data_dir.join("thread_history_1.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|_| "projection_unreadable")?;
    database
        .busy_timeout(std::time::Duration::from_secs(2))
        .map_err(|_| "projection_unreadable")?;
    let cursor = database.query_row(
        "SELECT next_rollout_byte_offset,next_rollout_ordinal FROM thread_history_projection_state WHERE thread_id=?1",
        [thread_id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).map_err(|_| "projection_missing")?;
    let plan = plan_duplicate_cursor_recovery(&bytes, cursor)?;
    fs::create_dir_all(output_root).map_err(|_| "recovery_output_unavailable")?;
    let directory = output_root.join(uuid::Uuid::new_v4().to_string());
    fs::create_dir(&directory).map_err(|_| "recovery_output_unavailable")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .map_err(|_| "recovery_output_unavailable")?;
    }
    let projection = directory.join("projection.sqlite");
    // VACUUM INTO takes a consistent SQLite snapshot, including committed WAL records.
    database
        .execute("VACUUM INTO ?1", [projection.to_string_lossy().as_ref()])
        .map_err(|_| "projection_snapshot_failed")?;
    let mut copy = Connection::open_with_flags(&projection, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|_| "projection_snapshot_failed")?;
    let check: String = copy
        .query_row("PRAGMA quick_check", [], |row| row.get(0))
        .map_err(|_| "projection_snapshot_invalid")?;
    if check != "ok" {
        return Err("projection_snapshot_invalid".into());
    }
    // Schema extensions that can execute on UPDATE are not part of the tested contract.
    let update_triggers: u64 = copy.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='trigger' AND tbl_name='thread_history_projection_state' AND upper(sql) LIKE '%UPDATE%'",
        [], |row| row.get(0),
    ).map_err(|_| "unsupported_projection_schema")?;
    if update_triggers != 0 {
        return Err("unsupported_projection_schema".into());
    }
    apply_cursor_plan(&mut copy, thread_id, &plan, &bytes)?;
    drop(copy);
    if read_rollout(&rollout)? != bytes {
        return Err("source_changed_during_scan".into());
    }
    fs::write(directory.join("rollout.jsonl"), &bytes)
        .map_err(|_| "recovery_output_unavailable")?;
    let result = RecoveryCopy {
        directory: directory.to_string_lossy().into_owned(),
        thread_id: thread_id.to_string(),
        plan,
        status: "candidate_not_applied",
    };
    let manifest = serde_json::to_vec_pretty(&result).map_err(|_| "recovery_manifest_failed")?;
    fs::write(directory.join("manifest.json"), manifest).map_err(|_| "recovery_manifest_failed")?;
    Ok(result)
}

/// Locate a top-level ordinal using the JSON parser's consumed-byte offsets.
/// All other bytes, including payload number spellings and encrypted data, stay exact.
fn ordinal_span(line: &[u8]) -> Result<Range<usize>, String> {
    let mut pos = 0;
    let skip = |pos: &mut usize| {
        while line.get(*pos).is_some_and(u8::is_ascii_whitespace) {
            *pos += 1;
        }
    };
    skip(&mut pos);
    if line.get(pos) != Some(&b'{') {
        return Err("invalid_record".into());
    }
    pos += 1;
    let mut keys = HashSet::new();
    let mut span = None;
    loop {
        skip(&mut pos);
        if line.get(pos) == Some(&b'}') {
            break;
        }
        let mut parser = serde_json::Deserializer::from_slice(&line[pos..]).into_iter::<String>();
        let key = parser
            .next()
            .ok_or("invalid_key")?
            .map_err(|_| "invalid_key")?;
        pos += parser.byte_offset();
        if !keys.insert(key.clone()) {
            return Err("duplicate_top_level_key".into());
        }
        skip(&mut pos);
        if line.get(pos) != Some(&b':') {
            return Err("invalid_record".into());
        }
        pos += 1;
        skip(&mut pos);
        let start = pos;
        let mut parser =
            serde_json::Deserializer::from_slice(&line[pos..]).into_iter::<IgnoredAny>();
        parser
            .next()
            .ok_or("invalid_value")?
            .map_err(|_| "invalid_value")?;
        pos += parser.byte_offset();
        if key == "ordinal" {
            span = Some(start..pos);
        }
        skip(&mut pos);
        match line.get(pos) {
            Some(b',') => pos += 1,
            Some(b'}') => break,
            _ => return Err("invalid_record".into()),
        }
    }
    span.ok_or_else(|| "missing_ordinal".into())
}

#[cfg(test)]
fn normalized_candidate(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() as u64 > MAX_BYTES {
        return Err("rollout_too_large".into());
    }
    let health = inspect_bytes(bytes, None);
    if health.issues.iter().any(|issue| {
        !matches!(
            issue.code,
            "duplicate_ordinal" | "ordinal_regressed" | "ordinal_gap"
        )
    }) {
        return Err("unsupported_recovery_input".into());
    }
    // A candidate is only meaningful for the format exercised by the native tests.
    let first: Value = serde_json::from_slice(
        bytes
            .split_inclusive(|b| *b == b'\n')
            .next()
            .ok_or("empty_rollout")?,
    )
    .map_err(|_| "invalid_json")?;
    if first
        .pointer("/payload/history_mode")
        .and_then(Value::as_str)
        != Some("paginated")
    {
        return Err("unsupported_history_mode".into());
    }
    let mut result = Vec::with_capacity(bytes.len());
    for (index, line) in bytes.split_inclusive(|byte| *byte == b'\n').enumerate() {
        let span = ordinal_span(line)?;
        let original: u64 =
            serde_json::from_slice(&line[span.clone()]).map_err(|_| "invalid_ordinal")?;
        if original == index as u64 {
            result.extend_from_slice(line);
        } else {
            result.extend_from_slice(&line[..span.start]);
            result.extend_from_slice(index.to_string().as_bytes());
            result.extend_from_slice(&line[span.end..]);
        }
    }
    if inspect_bytes(&result, None).blocks_rewrite() {
        return Err("candidate_validation_failed".into());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    const META: &str = "{\"ordinal\":0,\"type\":\"session_meta\",\"payload\":{\"id\":\"synthetic\",\"history_mode\":\"paginated\"}}\n";
    fn source(tail: &str) -> Vec<u8> {
        format!("{META}{tail}").into_bytes()
    }
    fn has(health: &HistoryHealth, code: &str) -> bool {
        health.issues.iter().any(|issue| issue.code == code)
    }

    #[test]
    fn duplicate_stalls_at_saved_cursor() {
        let data = source("{\"ordinal\":0,\"type\":\"event_msg\",\"payload\":{}}\n");
        let report = inspect_bytes(&data, Some((META.len() as u64, 1)));
        assert!(has(&report, "duplicate_ordinal"));
        assert!(has(&report, "cursor_ordinal_mismatch"));
        assert!(report.blocks_rewrite());
    }
    #[test]
    fn ordinary_unprojected_append_is_not_corruption() {
        let data = source("{\"ordinal\":1}\n");
        let report = inspect_bytes(&data, Some((META.len() as u64, 1)));
        assert!(has(&report, "projection_pending"));
        assert!(!report.blocks_rewrite());
    }
    #[test]
    fn offsets_and_eof_ordinal_are_checked() {
        assert!(has(
            &inspect_bytes(META.as_bytes(), Some((1, 0))),
            "cursor_offset_mismatch"
        ));
        assert!(has(
            &inspect_bytes(META.as_bytes(), Some((META.len() as u64 + 1, 1))),
            "cursor_offset_mismatch"
        ));
        assert!(has(
            &inspect_bytes(META.as_bytes(), Some((META.len() as u64, 9))),
            "cursor_ordinal_mismatch"
        ));
        assert!(!inspect_bytes(META.as_bytes(), Some((META.len() as u64, 1))).blocks_rewrite());
    }
    #[test]
    fn gap_is_not_proof_of_corruption() {
        assert!(!inspect_bytes(&source("{\"ordinal\":7}\n"), None).blocks_rewrite());
    }
    #[test]
    fn only_ordinal_bytes_change_and_second_pass_is_identical() {
        let original = source("{ \"payload\":{\"n\":123456789012345678901234567890,\"ordinal\":999,\"text\":\"\\\"ordinal\\\":8\"}, \"ordinal\" : 0 }\r\n");
        let result = normalized_candidate(&original).unwrap();
        let expected = source("{ \"payload\":{\"n\":123456789012345678901234567890,\"ordinal\":999,\"text\":\"\\\"ordinal\\\":8\"}, \"ordinal\" : 1 }\r\n");
        assert_eq!(result, expected);
        assert_eq!(normalized_candidate(&result).unwrap(), result);
    }
    #[test]
    fn ambiguous_or_partial_inputs_are_refused() {
        for tail in [
            "{\"ordinal\":1}",
            "broken\n",
            "{}\n",
            "{\"ordinal\":0,\"ordinal\":1}\n",
            "{\"ordinal\":-1}\n",
        ] {
            assert!(normalized_candidate(&source(tail)).is_err(), "{tail}");
        }
        assert!(normalized_candidate(b"").is_err());
    }
    #[test]
    fn legacy_is_diagnostic_only() {
        let bytes =
            b"{\"type\":\"session_meta\",\"payload\":{\"id\":\"x\",\"history_mode\":\"legacy\"}}\n";
        assert!(has(&inspect_bytes(bytes, None), "legacy_history"));
        assert!(normalized_candidate(bytes).is_err());
    }

    #[test]
    fn duplicate_metadata_cursor_produces_plan_without_writing() {
        let bytes = source(
            "{\"ordinal\":0,\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\"}}\n{\"ordinal\":0,\"type\":\"event_msg\",\"payload\":{\"type\":\"thread_settings_applied\"}}\n{\"ordinal\":1,\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\"}}\n",
        );
        let boundary = bytes
            .split_inclusive(|byte| *byte == b'\n')
            .next()
            .unwrap()
            .len();
        let plan = plan_duplicate_cursor_recovery(&bytes, (boundary as u64, 1)).unwrap();
        assert_eq!(plan.skipped_lines, 2);
        assert_eq!(
            plan.to_offset,
            boundary as u64
                + bytes[boundary..]
                    .split_inclusive(|byte| *byte == b'\n')
                    .take(2)
                    .map(|line| line.len())
                    .sum::<usize>() as u64
        );
        assert_eq!(plan.next_ordinal, 1);
    }

    #[test]
    fn duplicate_message_boundary_is_refused() {
        let bytes = source(
            "{\"ordinal\":0,\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\"}}\n{\"ordinal\":1,\"type\":\"event_msg\",\"payload\":{\"type\":\"agent_message\"}}\n",
        );
        let first = bytes
            .split_inclusive(|byte| *byte == b'\n')
            .next()
            .unwrap()
            .len();
        assert_eq!(
            plan_duplicate_cursor_recovery(&bytes, (first as u64, 1)).unwrap_err(),
            "unsafe_duplicate_cursor_boundary"
        );
    }

    #[test]
    fn apply_requires_hash_cursor_and_recovery_copy() {
        let unique = format!("cockpit-history-recovery-test-{}", uuid::Uuid::new_v4());
        let root = std::env::temp_dir().join(unique);
        fs::create_dir_all(&root).unwrap();
        fs::create_dir(root.join("sessions")).unwrap();
        let rollout = root.join("sessions/rollout.jsonl");
        let bytes = source(
            "{\"ordinal\":0,\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\"}}\n{\"ordinal\":0,\"type\":\"event_msg\",\"payload\":{\"type\":\"thread_settings_applied\"}}\n{\"ordinal\":1,\"type\":\"event_msg\",\"payload\":{\"type\":\"user_message\"}}\n",
        );
        fs::write(&rollout, &bytes).unwrap();
        let first = bytes
            .split_inclusive(|byte| *byte == b'\n')
            .next()
            .unwrap()
            .len();
        let plan = plan_duplicate_cursor_recovery(&bytes, (first as u64, 1)).unwrap();
        let state = Connection::open(root.join("state_5.sqlite")).unwrap();
        state
            .execute_batch("CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT);")
            .unwrap();
        state
            .execute(
                "INSERT INTO threads VALUES ('synthetic','sessions/rollout.jsonl')",
                [],
            )
            .unwrap();
        drop(state);
        let db = root.join("thread_history_1.sqlite");
        let connection = Connection::open(&db).unwrap();
        connection.execute_batch("CREATE TABLE thread_history_projection_state (thread_id TEXT PRIMARY KEY, next_rollout_byte_offset INTEGER NOT NULL, next_rollout_ordinal INTEGER NOT NULL);") .unwrap();
        connection
            .execute(
                "INSERT INTO thread_history_projection_state VALUES ('synthetic',?1,1)",
                [first as u64],
            )
            .unwrap();
        drop(connection);
        let before = fs::read(&db).unwrap();
        assert!(
            create_recovery_copy(&root, "synthetic", "wrong-hash", &root.join("copies")).is_err()
        );
        let result = create_recovery_copy(
            &root,
            "synthetic",
            &plan.source_sha256,
            &root.join("copies"),
        )
        .unwrap();
        let connection =
            Connection::open(Path::new(&result.directory).join("projection.sqlite")).unwrap();
        let cursor: (u64, u64) = connection.query_row("SELECT next_rollout_byte_offset,next_rollout_ordinal FROM thread_history_projection_state WHERE thread_id='synthetic'", [], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
        assert_eq!(cursor, (plan.to_offset, plan.next_ordinal));
        assert_eq!(fs::read(&db).unwrap(), before);
        assert_eq!(fs::read(&rollout).unwrap(), bytes);
        assert_eq!(result.status, "candidate_not_applied");
        drop(connection);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn gap_and_mid_record_cursor_cannot_be_recovered() {
        let bytes = source("{\"ordinal\":0,\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\"}}\n{\"ordinal\":2}\n");
        assert!(plan_duplicate_cursor_recovery(&bytes, (META.len() as u64, 1)).is_err());
        assert!(plan_duplicate_cursor_recovery(&bytes, (META.len() as u64 + 1, 1)).is_err());
    }

    #[test]
    fn inherited_prefix_is_not_a_cursor_recovery() {
        let bytes = META.replace(
            "\"paginated\"",
            "\"paginated\",\"subagent_history_start_ordinal\":1",
        );
        let health = inspect_bytes(bytes.as_bytes(), None);
        assert!(has(&health, "all_records_inherited"));
        assert!(plan_duplicate_cursor_recovery(bytes.as_bytes(), (bytes.len() as u64, 1)).is_err());
    }

    #[test]
    fn tampered_plan_and_other_thread_are_rejected() {
        let bytes = source("{\"ordinal\":0,\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\"}}\n{\"ordinal\":1}\n");
        let mut plan = plan_duplicate_cursor_recovery(&bytes, (META.len() as u64, 1)).unwrap();
        let mut db = Connection::open_in_memory().unwrap();
        assert_eq!(
            apply_cursor_plan(&mut db, "other", &plan, &bytes).unwrap_err(),
            "thread_identity_mismatch"
        );
        plan.to_offset += 1;
        assert_eq!(
            apply_cursor_plan(&mut db, "synthetic", &plan, &bytes).unwrap_err(),
            "recovery_plan_mismatch"
        );
    }
    #[test]
    fn reports_are_bounded_but_never_hide_blockers() {
        let mut data = META.to_owned();
        for i in 1..150 {
            data.push_str(&format!("{{\"ordinal\":{}}}\n", i * 10));
        }
        data.push_str("{\"ordinal\":0}\n");
        let report = inspect_bytes(data.as_bytes(), None);
        assert_eq!(report.issues.len(), 100);
        assert!(report.blocks_rewrite());
    }

    #[test]
    fn candidate_checks_entire_tail_after_issue_limit() {
        let prefix = source(&"{\"ordinal\":0,\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\"}}\n".repeat(120));
        for suffix in ["{\"ordinal\":1}\nbroken\n", "{\"ordinal\":1}\n{}\n", "{\"ordinal\":1}\n{\"ordinal\":1}\n", "{\"ordinal\":1}\n{\"ordinal\":3}\n", "{\"ordinal\":1}\n{\"ordinal\":2}"] {
            let bytes = [prefix.as_slice(), suffix.as_bytes()].concat();
            assert!(plan_duplicate_cursor_recovery(&bytes, (META.len() as u64, 1)).is_err(), "{suffix}");
        }
        let valid = [prefix.as_slice(), b"{\"ordinal\":1}\n{\"ordinal\":2}\n"].concat();
        assert_eq!(plan_duplicate_cursor_recovery(&valid, (META.len() as u64, 1)).unwrap().skipped_lines, 120);
    }

    #[test]
    fn source_update_during_projection_read_requests_retry() {
        let dir = std::env::temp_dir().join(format!("history-read-race-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let rollout = dir.join("rollout.jsonl");
        fs::write(&rollout, META).unwrap();
        let result = inspect_with_hook(&rollout, &dir.join("absent.sqlite"), || {
            fs::write(&rollout, source("{\"ordinal\":1}\n")).unwrap();
        });
        assert_eq!(result.unwrap_err(), "source_changed_during_scan");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn crlf_recovery_uses_bytes_not_line_count() {
        let bytes = source("{\"ordinal\":0,\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\"}}\n{\"ordinal\":1}\n");
        let windows = String::from_utf8(bytes)
            .unwrap()
            .replace('\n', "\r\n")
            .into_bytes();
        let first = windows.iter().position(|b| *b == b'\n').unwrap() + 1;
        let plan = plan_duplicate_cursor_recovery(&windows, (first as u64, 1)).unwrap();
        assert_eq!(
            plan.to_offset as usize,
            windows.len() - b"{\"ordinal\":1}\r\n".len()
        );
        assert_eq!(plan.skipped_lines, 1);
    }

    #[test]
    fn paths_with_spaces_and_wal_snapshot_preserve_source() {
        let dir = std::env::temp_dir().join(format!("history space '{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(dir.join("sessions")).unwrap();
        let bytes = source("{\"ordinal\":0,\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\"}}\n{\"ordinal\":1}\n");
        let rollout = dir.join("sessions/rollout space.jsonl");
        fs::write(&rollout, &bytes).unwrap();
        let state = Connection::open(dir.join("state_5.sqlite")).unwrap();
        state
            .execute_batch("CREATE TABLE threads(id TEXT, rollout_path TEXT)")
            .unwrap();
        state
            .execute(
                "INSERT INTO threads VALUES ('synthetic',?1)",
                [rollout.to_str().unwrap()],
            )
            .unwrap();
        let source = Connection::open(dir.join("thread_history_1.sqlite")).unwrap();
        source.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE thread_history_projection_state(thread_id TEXT PRIMARY KEY,next_rollout_byte_offset INTEGER,next_rollout_ordinal INTEGER)").unwrap();
        source
            .execute(
                "INSERT INTO thread_history_projection_state VALUES('synthetic',?1,1)",
                [META.len() as u64],
            )
            .unwrap();
        let candidate = create_recovery_copy(
            &dir,
            "synthetic",
            &format!("{:x}", Sha256::digest(&bytes)),
            &dir.join("copies"),
        )
        .unwrap();
        let cursor: u64 = source
            .query_row(
                "SELECT next_rollout_byte_offset FROM thread_history_projection_state",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cursor, META.len() as u64);
        assert_eq!(fs::read(&rollout).unwrap(), bytes);
        assert!(Path::new(&candidate.directory)
            .join("manifest.json")
            .is_file());
        drop(source);
        drop(state);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn locked_projection_is_reported_without_modification() {
        let dir = std::env::temp_dir().join(format!("history-locked-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let rollout = dir.join("rollout.jsonl");
        fs::write(&rollout, META).unwrap();
        let database = dir.join("projection.sqlite");
        let lock = Connection::open(&database).unwrap();
        lock.execute_batch("CREATE TABLE thread_history_projection_state(thread_id TEXT PRIMARY KEY,next_rollout_byte_offset INTEGER,next_rollout_ordinal INTEGER); BEGIN EXCLUSIVE").unwrap();
        let report = inspect(&rollout, &database).unwrap();
        assert!(has(&report, "projection_unreadable"));
        assert_eq!(fs::read(&rollout).unwrap(), META.as_bytes());
        lock.execute_batch("ROLLBACK").unwrap();
        drop(lock);
        fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_deny_share_file_returns_a_read_error() {
        use std::os::windows::fs::OpenOptionsExt;
        let path =
            std::env::temp_dir().join(format!("history-share-{}.jsonl", uuid::Uuid::new_v4()));
        fs::write(&path, META).unwrap();
        let lock = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .unwrap();
        assert!(read_rollout(&path).is_err());
        drop(lock);
        assert_eq!(fs::read(&path).unwrap(), META.as_bytes());
        fs::remove_file(path).unwrap();
    }
}
