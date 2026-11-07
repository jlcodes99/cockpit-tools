// Request identity metadata is independent of optional body snapshots. Never
// manufacture a wire ID from an ownership ledger or silently truncate one.
fn normalize_diagnostic_identity(
    captured: &mut bool,
    session: &mut Option<String>,
    thread: &mut Option<String>,
) -> bool {
    if !*captured {
        *session = None;
        *thread = None;
        return false;
    }
    let invalid = [session.as_deref(), thread.as_deref()]
        .into_iter()
        .flatten()
        .any(|id| id.len() > 4096 || id.chars().any(char::is_control));
    if invalid {
        *captured = false;
        *session = None;
        *thread = None;
        return true;
    }
    for id in [session, thread] {
        if id.as_deref() == Some("") {
            *id = None;
        }
    }
    false
}

fn ensure_request_identity_columns(conn: &Connection) -> Result<(), SqliteError> {
    let columns: std::collections::HashSet<String> = {
        let mut statement = conn.prepare("PRAGMA table_info(request_diagnostics)")?;
        let columns = statement
            .query_map([], |row| row.get(1))?
            .collect::<Result<_, _>>()?;
        columns
    };
    // Adding nullable/default columns is a metadata-only migration. Existing
    // records retain their original bodies and explicitly remain uncaptured.
    for (name, definition) in [
        ("client_identity_captured", "INTEGER NOT NULL DEFAULT 0"),
        ("client_session_id", "TEXT"),
        ("client_thread_id", "TEXT"),
        (
            "client_identity_metadata_captured",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("client_installation_id", "TEXT"),
        ("client_chatgpt_account_id", "TEXT"),
    ] {
        if !columns.contains(name) {
            conn.execute(
                &format!("ALTER TABLE request_diagnostics ADD COLUMN {name} {definition}"),
                [],
            )?;
        }
    }
    Ok(())
}

fn normalize_diagnostic_account_profile(
    profile: &mut Option<crate::modules::codex_api_identity::InstallationProfile>,
) -> bool {
    let Some(value) = profile.as_ref() else {
        return false;
    };
    let valid = value.version == 1
        && value.revision > 0
        && value.scope.len() == 64
        && value
            .scope
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && uuid::Uuid::parse_str(&value.installation_id).is_ok_and(|id| {
            id.get_version_num() == 4
                && id.get_variant() == uuid::Variant::RFC4122
                && id.to_string() == value.installation_id
        });
    if !valid {
        *profile = None;
    }
    !valid
}

fn diagnostic_identity_bool_column(row: &rusqlite::Row<'_>, name: &str) -> rusqlite::Result<bool> {
    match row.as_ref().column_index(name) {
        Ok(index) => Ok(row.get::<_, i64>(index)? != 0),
        Err(SqliteError::InvalidColumnName(_)) => Ok(false),
        Err(error) => Err(error),
    }
}

fn diagnostic_identity_text_column(
    row: &rusqlite::Row<'_>,
    name: &str,
) -> rusqlite::Result<Option<String>> {
    match row.as_ref().column_index(name) {
        Ok(index) => row.get(index),
        Err(SqliteError::InvalidColumnName(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod request_identity_persistence_tests {
    use super::*;

    fn with_account_identity() -> CodexLocalAccessRequestDetail {
        let mut detail = captured_detail();
        detail.client_identity_metadata_captured = true;
        detail.client_installation_id = Some("client-installation".into());
        detail.client_chatgpt_account_id = Some("client-workspace".into());
        for attempt in &mut detail.attempts {
            let id = if attempt.sequence == 1 {
                "11111111-1111-4111-8111-111111111111"
            } else {
                "22222222-2222-4222-8222-222222222222"
            };
            attempt.upstream_identity_metadata_captured = true;
            attempt.upstream_installation_id = Some(id.into());
            attempt.upstream_chatgpt_account_id = Some(format!("workspace-{}", attempt.account_id));
            attempt.account_identity =
                Some(crate::modules::codex_api_identity::InstallationProfile {
                    version: 1,
                    scope: attempt.account_id.repeat(64),
                    installation_id: id.into(),
                    revision: 1,
                });
        }
        detail
    }

    #[test]
    fn account_profiles_and_wire_headers_survive_migration_reopen_and_body_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diagnostics.sqlite");
        let expected = with_account_identity();
        {
            let conn = Connection::open(&path).unwrap();
            insert_request_diagnostic(&conn, expected.clone()).unwrap();
            clear_request_payloads_from_db(&conn).unwrap();
        }
        let conn = Connection::open(path).unwrap();
        assert_eq!(
            read_request_diagnostic(&conn, "new").unwrap().unwrap(),
            expected
        );
        let wire = serde_json::to_value(&expected).unwrap();
        assert_eq!(wire["clientIdentityMetadataCaptured"], true);
        assert_eq!(wire["clientInstallationId"], "client-installation");
        assert_eq!(
            wire["attempts"][0]["accountIdentity"]["installationId"],
            expected.attempts[0]
                .upstream_installation_id
                .as_deref()
                .unwrap()
        );
        assert_ne!(
            wire["attempts"][0]["accountIdentity"],
            wire["attempts"][1]["accountIdentity"]
        );
        let legacy = legacy_database();
        let old = read_request_diagnostic(&legacy, "old").unwrap().unwrap();
        assert!(!old.client_identity_metadata_captured);
        assert!(old.client_installation_id.is_none());
        create_request_diagnostics_table(&legacy).unwrap();
        create_request_diagnostics_table(&legacy).unwrap();
        assert_eq!(
            read_request_diagnostic(&legacy, "old").unwrap().unwrap(),
            old
        );
    }

    #[test]
    fn metadata_merge_never_fills_identity_from_another_attempt_or_account() {
        let original = with_account_identity();
        let mut summary = captured_detail();
        summary.captured_at_ms += 1;
        let merged = merge_request_diagnostics(original.clone(), summary.clone());
        assert_eq!(
            merged,
            CodexLocalAccessRequestDetail {
                captured_at_ms: summary.captured_at_ms,
                ..original.clone()
            }
        );
        summary.attempts[0].account_id = "different-account".into();
        let changed = merge_request_diagnostics(original.clone(), summary.clone());
        assert!(changed.attempts[0].account_identity.is_none());
        assert!(!changed.attempts[0].upstream_identity_metadata_captured);
        summary.attempts[0].account_id = original.attempts[0].account_id.clone();
        summary.attempts[0].started_at_ms += 1;
        let different_attempt = merge_request_diagnostics(original, summary);
        assert!(different_attempt.attempts[0].account_identity.is_none());
    }

    #[test]
    fn invalid_identity_metadata_is_omitted_without_destroying_session_headers() {
        let mut detail = with_account_identity();
        detail.client_installation_id = Some("x".repeat(4097));
        detail.attempts[0].upstream_chatgpt_account_id = Some("bad\nworkspace".into());
        detail.attempts[0].account_identity.as_mut().unwrap().scope = "not-a-scope".into();
        let normalized = normalize_request_diagnostics(detail);
        assert!(normalized.truncated);
        assert!(!normalized.client_identity_metadata_captured);
        assert!(normalized.client_identity_captured);
        assert!(normalized.client_installation_id.is_none());
        assert!(normalized.attempts[0].account_identity.is_none());
        assert!(normalized.attempts[0].upstream_identity_captured);
        assert!(!normalized.attempts[0].upstream_identity_metadata_captured);
        assert!(normalized.attempts[1].account_identity.is_some());
    }

    fn legacy_database() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE request_diagnostics (
            request_id TEXT PRIMARY KEY, captured_at_ms INTEGER NOT NULL,
            first_response_ms INTEGER, failure_phase TEXT,
            attempts_json TEXT NOT NULL DEFAULT '[]',payloads_json TEXT NOT NULL DEFAULT '[]',
            payload_bytes INTEGER NOT NULL DEFAULT 0,truncated INTEGER NOT NULL DEFAULT 0);
            INSERT INTO request_diagnostics(request_id,captured_at_ms) VALUES ('old',1);",
        )
        .unwrap();
        conn
    }

    fn captured_detail() -> CodexLocalAccessRequestDetail {
        CodexLocalAccessRequestDetail {
            request_id: "new".into(),
            captured_at_ms: 2,
            client_identity_captured: true,
            client_session_id: Some("client-session".into()),
            client_thread_id: None,
            attempts: vec![
                CodexLocalAccessRequestAttempt {
                    sequence: 1,
                    account_id: "a".into(),
                    started_at_ms: 1,
                    upstream_identity_captured: true,
                    upstream_session_id: Some("upstream-session-a".into()),
                    upstream_thread_id: Some("upstream-thread-a".into()),
                    ..Default::default()
                },
                CodexLocalAccessRequestAttempt {
                    sequence: 2,
                    account_id: "b".into(),
                    started_at_ms: 2,
                    upstream_identity_captured: true,
                    upstream_session_id: Some("upstream-session-b".into()),
                    upstream_thread_id: Some("upstream-thread-b".into()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn legacy_read_and_idempotent_migration_preserve_uncaptured_history() {
        let conn = legacy_database();
        let before = read_request_diagnostic(&conn, "old").unwrap().unwrap();
        assert!(!before.client_identity_captured);
        create_request_diagnostics_table(&conn).unwrap();
        create_request_diagnostics_table(&conn).unwrap();
        assert_eq!(
            read_request_diagnostic(&conn, "old").unwrap().unwrap(),
            before
        );
        insert_request_diagnostic(&conn, captured_detail()).unwrap();
        let stored = read_request_diagnostic(&conn, "new").unwrap().unwrap();
        assert!(stored.client_identity_captured);
        assert_eq!(stored.client_session_id.as_deref(), Some("client-session"));
        assert_eq!(stored.client_thread_id, None);
        assert_eq!(stored.attempts.len(), 2);
        assert_eq!(
            stored.attempts[0].upstream_thread_id.as_deref(),
            Some("upstream-thread-a")
        );
        assert_eq!(
            stored.attempts[1].upstream_thread_id.as_deref(),
            Some("upstream-thread-b")
        );
        assert!(stored.payloads.is_empty());
        clear_request_payloads_from_db(&conn).unwrap();
        assert_eq!(
            read_request_diagnostic(&conn, "new").unwrap().unwrap(),
            stored
        );
    }

    #[test]
    fn identities_survive_late_legacy_events_but_never_cross_attempt_accounts() {
        let mut latest = captured_detail();
        let mut legacy = latest.clone();
        legacy.captured_at_ms = 3;
        legacy.client_identity_captured = false;
        legacy.client_session_id = None;
        for attempt in &mut legacy.attempts {
            attempt.upstream_identity_captured = false;
            attempt.upstream_session_id = None;
            attempt.upstream_thread_id = None;
        }
        let merged = merge_request_diagnostics(latest.clone(), legacy.clone());
        assert_eq!(merged.client_session_id, latest.client_session_id);
        assert_eq!(
            merged.attempts[0].upstream_thread_id,
            latest.attempts[0].upstream_thread_id
        );
        legacy.attempts[0].account_id = "different-account".into();
        latest = merge_request_diagnostics(latest, legacy);
        assert!(!latest.attempts[0].upstream_identity_captured);
        assert_eq!(latest.attempts[0].upstream_thread_id, None);
    }

    #[test]
    fn identity_metadata_survives_reopen_and_disabled_or_cleared_body_logging() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("request-diagnostics.sqlite");
        let mut expected = captured_detail();
        expected.payloads.push(CodexLocalAccessRequestPayload {
            stage: "client".into(),
            body: "{}".into(),
            ..Default::default()
        });
        prepare_request_diagnostic_payloads(&mut expected, false, 0);
        assert!(expected.payloads.is_empty());
        {
            let connection = Connection::open(&path).unwrap();
            insert_request_diagnostic(&connection, expected.clone()).unwrap();
        }
        let reopened = Connection::open(&path).unwrap();
        let stored = read_request_diagnostic(&reopened, "new").unwrap().unwrap();
        assert_eq!(stored, expected);
        let wire = serde_json::to_value(&stored).unwrap();
        assert_eq!(wire["clientIdentityCaptured"], true);
        assert_eq!(wire["clientSessionId"], "client-session");
        assert!(wire.get("clientThreadId").is_none());
        assert_eq!(wire["attempts"][1]["upstreamIdentityCaptured"], true);
        assert_eq!(
            wire["attempts"][1]["upstreamSessionId"],
            "upstream-session-b"
        );
        assert_eq!(wire["attempts"][1]["upstreamThreadId"], "upstream-thread-b");
        clear_request_payloads_from_db(&reopened).unwrap();
        drop(reopened);
        let reopened = Connection::open(&path).unwrap();
        assert_eq!(
            read_request_diagnostic(&reopened, "new").unwrap().unwrap(),
            expected
        );
    }

    #[test]
    fn untrusted_or_oversized_ids_are_omitted_instead_of_truncated() {
        let mut detail = captured_detail();
        detail.client_session_id = Some("x".repeat(4097));
        detail.attempts[0].upstream_thread_id = Some("invalid\nthread".into());
        let clean = normalize_request_diagnostics(detail);
        assert!(clean.truncated);
        assert!(!clean.client_identity_captured);
        assert_eq!(clean.client_session_id, None);
        assert!(!clean.attempts[0].upstream_identity_captured);
        assert_eq!(clean.attempts[0].upstream_session_id, None);
        assert_eq!(
            clean.attempts[1].upstream_thread_id.as_deref(),
            Some("upstream-thread-b")
        );
    }
}
