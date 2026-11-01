use super::*;

fn observation<'a>(
    subject: &'a str,
    scope: &'a str,
    started_at: i64,
    restricted: bool,
) -> Observation<'a> {
    Observation {
        subject,
        scope,
        source: "test",
        instance_id: Some("default"),
        started_at,
        restricted,
    }
}

#[test]
fn device_restriction_scope_recovery_and_manual_retry_are_independent() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = open(&dir.path().join("state.sqlite")).unwrap();
    observe(&mut db, observation("a", "desktop", 10, true)).unwrap();
    observe(&mut db, observation("a", "quota", 20, false)).unwrap();
    assert!(read(&db, "a", "desktop").unwrap().unwrap().suspended);
    allow_retry(&mut db, "a", "desktop", 10, 30).unwrap();
    let state = read(&db, "a", "desktop").unwrap().unwrap();
    assert_eq!(state.status, "restricted");
    assert!(!state.suspended);
    assert!(!observe(&mut db, observation("a", "desktop", 25, true)).unwrap());
    observe(&mut db, observation("a", "desktop", 40, false)).unwrap();
    assert_eq!(
        read(&db, "a", "desktop").unwrap().unwrap().status,
        "recovered"
    );
}

#[test]
fn device_restriction_persists_deduplicates_and_rejects_stale_results() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite");
    let mut db = open(&path).unwrap();
    assert!(observe(&mut db, observation("a", "api", 10, true)).unwrap());
    assert!(!observe(&mut db, observation("a", "api", 11, true)).unwrap());
    assert!(!observe(&mut db, observation("a", "api", 9, false)).unwrap());
    drop(db);
    let mut db = open(&path).unwrap();
    let state = read(&db, "a", "api").unwrap().unwrap();
    assert_eq!(state.occurrences, 2);
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM events", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(allow_retry(&mut db, "a", "api", 10, 12).is_err());
    assert!(read(&db, "a", "api").unwrap().unwrap().suspended);
}

#[test]
fn device_restriction_delete_fences_late_responses_and_allows_new_import() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = open(&dir.path().join("state.sqlite")).unwrap();
    observe(&mut db, observation("a", "api", 10, true)).unwrap();
    retire(&mut db, "a", 20).unwrap();
    assert!(!observe(&mut db, observation("a", "api", 15, true)).unwrap());
    assert!(list(&db, "").unwrap().is_empty());
    assert!(observe(&mut db, observation("a", "api", 30, true)).unwrap());
}

#[test]
fn device_restriction_paging_keeps_scopes_together_and_bounds_history() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = open(&dir.path().join("state.sqlite")).unwrap();
    for n in 0..105 {
        let id = format!("account-{n:03}");
        for scope in ["api", "quota", "desktop"] {
            observe(&mut db, observation(&id, scope, 10, true)).unwrap();
        }
    }
    let first = list(&db, "").unwrap();
    assert_eq!(first.len(), 300);
    assert_eq!(list(&db, &first.last().unwrap().subject).unwrap().len(), 15);
    for n in 11..111 {
        observe(&mut db, observation("account-000", "api", n, n % 2 == 0)).unwrap();
    }
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM events WHERE subject='account-000'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        32
    );
}

#[test]
fn device_restriction_lock_contention_does_not_erase_prior_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.sqlite");
    let mut db = open(&path).unwrap();
    observe(&mut db, observation("a", "api", 10, true)).unwrap();
    let mut other = open(&path).unwrap();
    let transaction = db
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let started = std::time::Instant::now();
    assert!(observe(&mut other, observation("a", "api", 20, false)).is_err());
    assert!(started.elapsed() < Duration::from_secs(1));
    drop(transaction); // Simulates interruption/rollback, not a committed recovery.
    assert!(read(&other, "a", "api").unwrap().unwrap().suspended);
    observe(&mut other, observation("a", "api", 20, false)).unwrap();
    assert!(!read(&other, "a", "api").unwrap().unwrap().suspended);
}

#[test]
fn device_restriction_retention_keeps_unresolved_accounts_and_uses_identity_index() {
    let dir = tempfile::tempdir().unwrap();
    let mut db = open(&dir.path().join("state.sqlite")).unwrap();
    observe(&mut db, observation("active", "api", 1, true)).unwrap();
    observe(&mut db, observation("resolved", "api", 1, true)).unwrap();
    observe(&mut db, observation("resolved", "api", 2, false)).unwrap();
    observe(&mut db, observation("login:old", "oauth", 1, true)).unwrap();
    observe(&mut db, observation("new", "api", 31 * 86400 * 1000, true)).unwrap();
    assert!(read(&db, "active", "api").unwrap().unwrap().suspended);
    assert!(read(&db, "resolved", "api").unwrap().is_none());
    assert!(read(&db, "login:old", "oauth").unwrap().is_none());
    let plan: String = db
        .query_row(
            "EXPLAIN QUERY PLAN SELECT * FROM state WHERE subject='active' AND scope='api'",
            [],
            |r| r.get(3),
        )
        .unwrap();
    assert!(plan.contains("INDEX"), "{plan}");
}
