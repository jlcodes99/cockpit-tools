//! Read T3 Code's existing thread projections without migrating or writing its database.

use std::path::Path;
use std::time::Duration;

use chrono::DateTime;
use rusqlite::types::ValueRef;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::modules::antigravity_token_usage::protobuf::{
    parse_gen_metadata_proto, parse_step_metadata_slim, ParsedGenMetadataProto,
};
use crate::modules::{logger, t3code_provider_bindings::antigravity_profile_dir};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Conversation {
    pub environment_id: String,
    pub thread_id: String,
    pub provider_instance_id: String,
    pub driver: String,
    pub context_provider_thread_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ContextUsage {
    used_tokens: u64,
    max_tokens: u64,
}

impl ContextUsage {
    fn from_payload(raw: Option<&str>) -> Option<Self> {
        let usage: serde_json::Value = serde_json::from_str(raw?).ok()?;
        let used_tokens = usage.get("usedTokens")?.as_u64()?;
        let max_tokens = usage.get("maxTokens")?.as_u64().filter(|max| *max > 0)?;
        Some(Self {
            used_tokens,
            max_tokens,
        })
    }
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ThreadUsage {
    pub native_thread_id: Option<String>,
    pub total_input_tokens: Option<i64>,
    pub total_cached_tokens: Option<i64>,
    pub context: Option<ContextUsage>,
    antigravity_speed: Option<AntigravitySpeed>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AntigravitySpeed {
    rate: f64,
    overall_rate: Option<f64>,
    streaming_rate: Option<f64>,
    output_tokens: u64,
    elapsed_seconds: f64,
    first_response_ms: f64,
}

impl AntigravitySpeed {
    fn from_metadata(metadata: &ParsedGenMetadataProto) -> Option<Self> {
        let usage = metadata.usage.as_ref()?;
        let output_tokens = usage.output_tokens;
        let first_response = metadata.time_to_first_token?;
        let streaming = metadata.streaming_duration?;
        if output_tokens < 100 || streaming < Duration::from_millis(100) {
            return None;
        }
        let elapsed_seconds = first_response.checked_add(streaming)?.as_secs_f64();
        let combined_output = usage.raw_combined_output.or_else(|| {
            if usage.thinking_tokens == 0 {
                return None;
            }
            output_tokens.checked_add(usage.thinking_tokens)
        });
        Some(Self {
            rate: output_tokens as f64 / elapsed_seconds,
            overall_rate: combined_output.map(|tokens| tokens as f64 / elapsed_seconds),
            streaming_rate: combined_output.map(|tokens| tokens as f64 / streaming.as_secs_f64()),
            output_tokens,
            elapsed_seconds,
            first_response_ms: first_response.as_secs_f64() * 1000.0,
        })
    }
}

pub(super) fn read_thread_usage(
    state_dir: &Path,
    conversation: &Conversation,
    model: Option<&str>,
) -> rusqlite::Result<ThreadUsage> {
    let path = state_dir.join("statev2.sqlite");
    if !path.is_file() {
        return Ok(ThreadUsage::default());
    }
    let conn = open_read_only(&path)?;
    let mut usage = read_projection(&conn, conversation)?;
    if let ("antigravity", Some(provider_thread), Some(native_session), Some(model)) = (
        conversation.driver.as_str(),
        conversation.context_provider_thread_id.as_deref(),
        usage.native_thread_id.as_deref(),
        model,
    ) {
        match read_antigravity_speed(
            &conn,
            state_dir,
            conversation,
            provider_thread,
            native_session,
            model,
        ) {
            Ok(speed) => usage.antigravity_speed = speed,
            Err(error) => logger::log_warn(&format!(
                "[T3 Code Usage] Antigravity speed unavailable: {error}"
            )),
        }
    }
    Ok(usage)
}

fn open_read_only(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.busy_timeout(Duration::from_millis(50))?;
    Ok(conn)
}

fn read_projection(
    conn: &Connection,
    conversation: &Conversation,
) -> rusqlite::Result<ThreadUsage> {
    let current = conn
        .query_row(
            "SELECT provider_thread_id,
                CASE WHEN json_valid(payload_json)
                     AND json_extract(payload_json, '$.nativeThreadRef.driver') = ?3
                     AND json_extract(payload_json, '$.nativeThreadRef.strength') = 'strong'
                     THEN json_extract(payload_json, '$.nativeThreadRef.nativeId') END,
                CASE WHEN json_valid(payload_json)
                     THEN json_extract(payload_json, '$.contextUsage') END
         FROM orchestration_v2_projection_provider_threads
         WHERE thread_id=?1 AND provider_instance_id=?2 AND driver=?3
           AND owner_node_id IS NULL
         ORDER BY (provider_thread_id = ?4) DESC, updated_at DESC, provider_thread_id DESC LIMIT 1",
            params![
                conversation.thread_id,
                conversation.provider_instance_id,
                conversation.driver,
                conversation.context_provider_thread_id,
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((provider_thread_id, native_id, context)) = current else {
        return Ok(ThreadUsage::default());
    };
    let mut usage = ThreadUsage::default();
    if conversation.context_provider_thread_id.as_deref() == Some(provider_thread_id.as_str()) {
        usage.context = ContextUsage::from_payload(context.as_deref());
        if usage.context.is_none() {
            let context: Option<String> = conn
                .query_row(
                    "SELECT json_extract(payload_json, '$.tokenUsage')
                     FROM (
                       SELECT payload_json
                       FROM orchestration_v2_projection_provider_turns
                       WHERE provider_thread_id=?1 AND thread_id=?2 ORDER BY ordinal DESC LIMIT 64
                     )
                     WHERE CASE WHEN json_valid(payload_json)
                          THEN json_type(payload_json, '$.tokenUsage')='object' END
                     LIMIT 1",
                    params![provider_thread_id, conversation.thread_id],
                    |row| row.get(0),
                )
                .optional()?;
            usage.context = ContextUsage::from_payload(context.as_deref());
        }
    }
    if conversation.driver == "codex" {
        // The T3 chat UUID is distinct from the Codex UUID in gateway requests.
        usage.native_thread_id = native_id
            .as_deref()
            .and_then(|id| uuid::Uuid::parse_str(&id).ok())
            .map(|id| id.hyphenated().to_string());
    }
    if conversation.driver != "antigravity" {
        return Ok(usage);
    }
    if conversation.context_provider_thread_id.as_deref() == Some(provider_thread_id.as_str()) {
        usage.native_thread_id = native_id;
    }

    // The upstream per-turn counters include cache reads in inputTokens.
    // A turn can span several model calls. Use its time window only to scope the native sample.
    let mut statement = conn.prepare(
        "SELECT CASE WHEN json_valid(payload_json)
                    AND json_extract(payload_json, '$.turnTokenUsage.usageStatus')='complete'
                    THEN json_extract(payload_json, '$.turnTokenUsage.inputTokens') END,
                CASE WHEN json_valid(payload_json)
                    AND json_extract(payload_json, '$.turnTokenUsage.usageStatus')='complete'
                    THEN json_extract(payload_json, '$.turnTokenUsage.cachedInputTokens') END
         FROM (
           SELECT payload_json, thread_id, status
           FROM orchestration_v2_projection_provider_turns
           WHERE provider_thread_id=?1 ORDER BY ordinal DESC LIMIT 64
         )
         WHERE thread_id=?2 AND status='completed'",
    )?;
    let mut rows = statement.query(params![provider_thread_id, conversation.thread_id])?;
    let mut input = 0_i64;
    let mut cached = 0_i64;
    while let Some(row) = rows.next()? {
        let (Ok(Some(total)), Ok(Some(hit))) =
            (row.get::<_, Option<i64>>(0), row.get::<_, Option<i64>>(1))
        else {
            continue;
        };
        if total <= 0 || !(0..=total).contains(&hit) {
            continue;
        }
        let Some(next_input) = input
            .checked_add(total)
            .filter(|value| *value <= 9_007_199_254_740_991)
        else {
            break;
        };
        input = next_input;
        cached += hit;
    }
    if input > 0 {
        usage.total_input_tokens = Some(input);
        usage.total_cached_tokens = Some(cached);
    }
    Ok(usage)
}

fn read_antigravity_speed(
    projection: &Connection,
    state_dir: &Path,
    conversation: &Conversation,
    provider_thread: &str,
    native_session: &str,
    model: &str,
) -> rusqlite::Result<Option<AntigravitySpeed>> {
    if native_session.is_empty()
        || !native_session
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Ok(None);
    }
    // Match the model recorded by T3 for this run. Native response model names can differ
    // from the picker slug, even for a single model (for example, gemini-4-argon-trial).
    let window: Option<(String, String)> = projection.query_row(
        "SELECT turn.started_at, turn.completed_at
         FROM (
           SELECT started_at, completed_at, run_attempt_id, status
           FROM orchestration_v2_projection_provider_turns
           WHERE provider_thread_id=?1 AND thread_id=?2 ORDER BY ordinal DESC LIMIT 1
         ) AS turn
         JOIN orchestration_v2_projection_run_attempts AS attempt ON attempt.attempt_id=turn.run_attempt_id
         JOIN orchestration_v2_projection_runs AS run ON run.run_id=attempt.run_id
         WHERE turn.status='completed' AND turn.completed_at IS NOT NULL
           AND CASE WHEN json_valid(run.payload_json) THEN json_extract(run.payload_json, '$.modelSelection.model') END = ?3",
        params![provider_thread, conversation.thread_id, model],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    let Some((start, end)) = window else {
        return Ok(None);
    };
    let (Ok(start), Ok(end)) = (
        DateTime::parse_from_rfc3339(&start),
        DateTime::parse_from_rfc3339(&end),
    ) else {
        return Ok(None);
    };
    let turn_window = start.timestamp_millis()..=end.timestamp_millis();
    let path = antigravity_profile_dir(state_dir, &conversation.provider_instance_id)
        .join("antigravity-acp")
        .join("conversations")
        .join(format!("{native_session}.db"));
    if !path.is_file() {
        return Ok(None);
    }

    let native = open_read_only(&path)?;
    let mut samples = native.prepare(
        "SELECT CASE WHEN length(data)<=?1 THEN data END FROM gen_metadata ORDER BY idx DESC LIMIT 8",
    )?;
    // Only associate a generator record with a completed native PLANNER_RESPONSE (15 / 3).
    let mut reply_step =
        native.prepare("SELECT metadata FROM steps WHERE idx=?1 AND step_type=15 AND status=3")?;
    let mut rows = samples.query([4 * 1024 * 1024])?;
    while let Some(row) = rows.next()? {
        let ValueRef::Blob(data) = row.get_ref(0)? else {
            continue;
        };
        let sample = parse_gen_metadata_proto(data);
        for index in &sample.step_indices {
            let started_at = reply_step
                .query_row([index], |row| {
                    Ok(match row.get_ref(0)? {
                        ValueRef::Blob(data) => parse_step_metadata_slim(data).timestamp_ms,
                        _ => None,
                    })
                })
                .optional()?
                .flatten();
            if started_at.is_some_and(|timestamp| turn_window.contains(&timestamp)) {
                return Ok(AntigravitySpeed::from_metadata(&sample));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn varint(mut value: u64) -> Vec<u8> {
        let mut encoded = Vec::new();
        while value >= 128 {
            encoded.push((value as u8 & 0x7f) | 0x80);
            value >>= 7;
        }
        encoded.push(value as u8);
        encoded
    }

    fn write_varint(data: &mut Vec<u8>, field: u64, value: u64) {
        data.extend(varint(field << 3));
        data.extend(varint(value));
    }

    fn write_bytes(data: &mut Vec<u8>, field: u64, bytes: &[u8]) {
        data.extend(varint((field << 3) | 2));
        data.extend(varint(bytes.len() as u64));
        data.extend(bytes);
    }

    fn insert_model_call(db: &Connection, index: u64, timestamp: u64, status: u64, nanos: u64) {
        let mut time = Vec::new();
        write_varint(&mut time, 1, timestamp);
        let mut step = Vec::new();
        write_bytes(&mut step, 1, &time);
        db.execute(
            "INSERT INTO steps VALUES (?1, 15, ?2, ?3)",
            params![index, status, step],
        )
        .unwrap();

        let mut usage = Vec::new();
        write_varint(&mut usage, 3, 328);
        write_varint(&mut usage, 9, 107);
        write_varint(&mut usage, 10, 221);
        let mut first_response = Vec::new();
        write_varint(&mut first_response, 1, 2);
        write_varint(&mut first_response, 2, 816_600_167);
        let mut streaming = Vec::new();
        write_varint(&mut streaming, 2, nanos);
        let mut model = Vec::new();
        write_bytes(&mut model, 4, &usage);
        write_bytes(&mut model, 11, &first_response);
        write_bytes(&mut model, 12, &streaming);
        let mut metadata = Vec::new();
        write_bytes(&mut metadata, 1, &model);
        write_bytes(&mut metadata, 2, &varint(index));
        db.execute(
            "INSERT INTO gen_metadata VALUES (?1, ?2)",
            params![index, metadata],
        )
        .unwrap();
    }

    #[test]
    fn antigravity_speed_uses_the_latest_completed_model_call_of_the_selected_run() {
        let dir = tempfile::tempdir().unwrap();
        let conn = Connection::open(dir.path().join("statev2.sqlite")).unwrap();
        conn.execute_batch(r#"
            CREATE TABLE orchestration_v2_projection_provider_threads (
                provider_thread_id TEXT, thread_id TEXT, provider_instance_id TEXT, driver TEXT,
                owner_node_id TEXT, updated_at TEXT, payload_json TEXT
            );
            CREATE TABLE orchestration_v2_projection_provider_turns (
                provider_thread_id TEXT, thread_id TEXT, ordinal INTEGER, status TEXT,
                started_at TEXT, completed_at TEXT, run_attempt_id TEXT, payload_json TEXT
            );
            CREATE TABLE orchestration_v2_projection_run_attempts (attempt_id TEXT PRIMARY KEY, run_id TEXT);
            CREATE TABLE orchestration_v2_projection_runs (run_id TEXT PRIMARY KEY, payload_json TEXT);
            INSERT INTO orchestration_v2_projection_provider_threads VALUES
                ('active', 'chat', 'parallel', 'antigravity', NULL, '1',
                '{"nativeThreadRef":{"driver":"antigravity","nativeId":"native-session","strength":"strong"},"contextUsage":{"usedTokens":40,"maxTokens":200}}');
            INSERT INTO orchestration_v2_projection_provider_turns VALUES
                ('active', 'chat', 1, 'completed', '1970-01-01T00:01:40Z', '1970-01-01T00:01:50Z', 'attempt', '{}');
            INSERT INTO orchestration_v2_projection_run_attempts VALUES ('attempt', 'run');
            INSERT INTO orchestration_v2_projection_runs VALUES ('run', '{"modelSelection":{"model":"gemini-4-argon"}}');
        "#).unwrap();
        let profile =
            antigravity_profile_dir(dir.path(), "parallel").join("antigravity-acp/conversations");
        std::fs::create_dir_all(&profile).unwrap();
        let native = Connection::open(profile.join("native-session.db")).unwrap();
        native.execute_batch("CREATE TABLE gen_metadata (idx INTEGER PRIMARY KEY, data BLOB); CREATE TABLE steps (idx INTEGER PRIMARY KEY, step_type INTEGER, status INTEGER, metadata BLOB);").unwrap();
        insert_model_call(&native, 1, 91, 3, 100_000_000);
        insert_model_call(&native, 2, 101, 3, 653_278_667);
        insert_model_call(&native, 3, 102, 2, 100_000_000);
        insert_model_call(&native, 4, 111, 3, 100_000_000);

        let mut conversation = Conversation {
            environment_id: "local".into(),
            thread_id: "chat".into(),
            provider_instance_id: "parallel".into(),
            driver: "antigravity".into(),
            context_provider_thread_id: Some("active".into()),
        };
        let read = |conversation: &Conversation, model| {
            read_thread_usage(dir.path(), conversation, Some(model)).unwrap()
        };
        let speed = read(&conversation, "gemini-4-argon")
            .antigravity_speed
            .unwrap();
        assert_eq!(speed.output_tokens, 221);
        assert_eq!(format!("{:.1}", speed.rate), "63.7");
        assert_eq!(format!("{:.1}", speed.overall_rate.unwrap()), "94.5");
        assert_eq!(format!("{:.1}", speed.streaming_rate.unwrap()), "502.1");
        assert!((speed.elapsed_seconds - 3.469_878_834).abs() < 1e-9);
        assert!((speed.first_response_ms - 2_816.600_167).abs() < 1e-6);
        assert!(read(&conversation, "gemini-4-neon")
            .antigravity_speed
            .is_none());

        insert_model_call(&native, 5, 109, 3, 39_647_417);
        let short = read(&conversation, "gemini-4-argon");
        assert!(short.antigravity_speed.is_none());
        assert_eq!(short.context.unwrap().used_tokens, 40);

        conn.execute(
            "UPDATE orchestration_v2_projection_provider_turns SET status='running'",
            [],
        )
        .unwrap();
        native
            .execute("DELETE FROM gen_metadata WHERE idx=5", [])
            .unwrap();
        assert!(read(&conversation, "gemini-4-argon")
            .antigravity_speed
            .is_none());
        conn.execute(
            "UPDATE orchestration_v2_projection_provider_turns SET status='completed'",
            [],
        )
        .unwrap();
        conversation.context_provider_thread_id = Some("previous".into());
        assert!(read(&conversation, "gemini-4-argon")
            .antigravity_speed
            .is_none());
    }

    #[test]
    fn context_uses_only_the_confirmed_active_provider_thread() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            r#"CREATE TABLE orchestration_v2_projection_provider_threads (
                provider_thread_id TEXT, thread_id TEXT, provider_instance_id TEXT,
                driver TEXT, owner_node_id TEXT, updated_at TEXT, payload_json TEXT
            );
            CREATE TABLE orchestration_v2_projection_provider_turns (
                provider_thread_id TEXT, thread_id TEXT, ordinal INTEGER, payload_json TEXT
            );
            INSERT INTO orchestration_v2_projection_provider_threads VALUES
                ('active', 'chat', 'provider', 'codex', NULL, '2026-11-25T00:00:00Z', '{}'),
                ('old', 'chat', 'provider', 'codex', NULL, '2026-11-25T01:00:00Z',
                    '{"contextUsage":{"usedTokens":90,"maxTokens":100}}');
            INSERT INTO orchestration_v2_projection_provider_turns VALUES
                ('old', 'chat', 1, '{"tokenUsage":{"usedTokens":90,"maxTokens":100}}'),
                ('active', 'chat', 2, '{"tokenUsage":{"usedTokens":40,"maxTokens":200}}');"#,
        )
        .unwrap();
        let mut conversation = Conversation {
            environment_id: "local".into(),
            thread_id: "chat".into(),
            provider_instance_id: "provider".into(),
            driver: "codex".into(),
            context_provider_thread_id: Some("active".into()),
        };
        let context = read_projection(&conn, &conversation)
            .unwrap()
            .context
            .unwrap();
        assert_eq!((context.used_tokens, context.max_tokens), (40, 200));

        conversation.context_provider_thread_id = Some("missing".into());
        assert!(read_projection(&conn, &conversation)
            .unwrap()
            .context
            .is_none());
        conversation.context_provider_thread_id = None;
        assert!(read_projection(&conn, &conversation)
            .unwrap()
            .context
            .is_none());
    }
}
