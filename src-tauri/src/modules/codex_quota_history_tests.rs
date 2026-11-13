use super::*;

#[test]
fn history_keeps_real_zero_and_finds_a_weekly_window_in_the_primary_slot() {
    let quota: CodexQuota = serde_json::from_value(serde_json::json!({
        "hourly_percentage": 0,
        "hourly_window_minutes": 10_080,
        "hourly_window_present": true,
        "weekly_percentage": 100,
        "weekly_window_present": false,
    }))
    .unwrap();
    assert_eq!(window_values(&quota), (None, Some(0)));
}

#[test]
fn history_preserves_newest_samples_per_interval_and_separates_identities() {
    let mut conn = Connection::open_in_memory().unwrap();
    initialize(&conn).unwrap();
    let sample =
        |id: &str, identity: &str, observed_at, short_remaining, weekly_remaining| Sample {
            account_id: id.into(),
            identity: identity.into(),
            observed_at,
            short_remaining,
            weekly_remaining,
        };
    let from = 10 * MINUTE;
    store_batch(
        &mut conn,
        &[
            sample("a", "person", from - 1, Some(100), None),
            sample("a", "person", from + 200, Some(4), Some(62)),
            sample("a", "person", from + 100, Some(99), None),
            sample("a", "person", from + 3 * MINUTE, None, Some(0)),
            sample("a", "person", from + 4 * MINUTE, None, Some(100)),
            sample("a", "person", from + 10 * MINUTE, Some(85), Some(100)),
            sample("a", "person", from + 15 * MINUTE, Some(99), Some(100)),
            sample("a", "other-workspace", from + 4 * MINUTE, Some(73), None),
            sample("other-account", "person", from + 4 * MINUTE, Some(81), None),
        ],
    )
    .unwrap();
    let until = from + 15 * MINUTE;
    let minute = read_history(&conn, "a", "person", from, until, MINUTE)
        .unwrap()
        .points;
    assert_eq!(minute.len(), 4);
    assert_eq!(minute[0].short_remaining, Some(4));
    assert_eq!(minute[0].observed_at, from + 200);
    let five_minutes = read_history(&conn, "a", "person", from, until, 5 * MINUTE).unwrap();
    assert_eq!(five_minutes.short_consumed, None);
    assert_eq!(five_minutes.weekly_consumed, Some(62));
    assert_eq!(
        five_minutes.points,
        [
            QuotaHistoryPoint {
                observed_at: from + 4 * MINUTE,
                bucket_start: from,
                short_remaining: None,
                weekly_remaining: Some(100),
            },
            QuotaHistoryPoint {
                observed_at: from + 10 * MINUTE,
                bucket_start: from + 10 * MINUTE,
                short_remaining: Some(85),
                weekly_remaining: Some(100),
            },
        ]
    );
    assert_eq!(
        read_history(&conn, "a", "other-workspace", from, until, MINUTE)
            .unwrap()
            .points
            .len(),
        1
    );
    assert!(
        read_history(&conn, "a", "new-identity", from, until, MINUTE)
            .unwrap()
            .points
            .is_empty()
    );
}

#[test]
fn recorded_samples_survive_reopening_the_database() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history.sqlite");
    let observed_at = 60_100;
    {
        let mut writer = Connection::open(&path).unwrap();
        initialize(&writer).unwrap();
        store_batch(
            &mut writer,
            &[Sample {
                account_id: "account".into(),
                identity: "workspace".into(),
                observed_at,
                short_remaining: Some(76),
                weekly_remaining: None,
            }],
        )
        .unwrap();
    }
    let reader =
        Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let version: i64 = reader
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 2);
    assert_eq!(
        read_history(&reader, "account", "workspace", 0, 120_000, MINUTE)
            .unwrap()
            .points,
        [QuotaHistoryPoint {
            observed_at,
            bucket_start: MINUTE,
            short_remaining: Some(76),
            weekly_remaining: None,
        }]
    );
}

#[test]
fn history_settings_filter_by_account_interval_and_actual_changes() {
    let conn = Connection::open_in_memory().unwrap();
    initialize(&conn).unwrap();
    let mut settings = HistorySettings::default();
    settings.interval_minutes = 5;
    settings.account_ids = Some(vec!["a".into()]);
    let sample = |id: &str, minute, short| Sample {
        account_id: id.into(),
        identity: "identity".into(),
        observed_at: minute * MINUTE,
        short_remaining: short,
        weekly_remaining: Some(0),
    };
    let ids = store_samples(
        &conn,
        &[
            sample("other", 1, Some(80)),
            sample("a", 1, Some(80)),
            sample("a", 3, Some(75)),
            sample("a", 6, Some(80)),
            sample("a", 7, Some(75)),
            sample("a", 12, None),
        ],
        &settings,
    )
    .unwrap();
    assert_eq!(ids, ["a", "a", "a"]);
    let history = read_history(&conn, "a", "identity", 0, 20 * MINUTE, MINUTE).unwrap();
    assert_eq!(
        history
            .points
            .iter()
            .map(|row| row.observed_at / MINUTE)
            .collect::<Vec<_>>(),
        [1, 7, 12]
    );
    assert!(
        read_history(&conn, "other", "identity", 0, 20 * MINUTE, MINUTE)
            .unwrap()
            .points
            .is_empty()
    );
    settings.enabled = false;
    assert!(
        store_samples(&conn, &[sample("a", 20, Some(10))], &settings)
            .unwrap()
            .is_empty()
    );
}
