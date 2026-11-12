use super::{current_due_at, quota_reset_timing, run_task};
use crate::models::codex::{CodexAccount, CodexAccountIndex, CodexAccountSummary, CodexTokens};
use crate::modules::{codex_account, codex_local_access, codex_wakeup};
use axum::{
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::{Local, TimeZone};
use serde_json::{json, Value};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

const CHILD: &str = "COCKPIT_TEST_SCHEDULED_WAKEUP";
const RESET_TASK: &str = "quota-reset-fixture";

#[test]
fn scheduled_wakeup_uses_selected_accounts_and_continues_after_a_failure() {
    if std::env::var_os(CHILD).is_some() {
        fixture_runtime().block_on(scheduled_dispatch());
        return;
    }
    let root = tempfile::tempdir().unwrap();
    run_isolated(
        root.path(),
        "scheduled_wakeup_uses_selected_accounts_and_continues_after_a_failure",
        "run",
    );
}

#[test]
fn quota_reset_wakeup_stops_on_disable_and_does_not_replay_after_restart() {
    match std::env::var(CHILD).as_deref() {
        Ok("run") => fixture_runtime().block_on(cancel_quota_reset_wakeup()),
        Ok("restart") => fixture_runtime().block_on(verify_claim_after_restart()),
        _ => {
            let root = tempfile::tempdir().unwrap();
            let name = "quota_reset_wakeup_stops_on_disable_and_does_not_replay_after_restart";
            run_isolated(root.path(), name, "run");
            run_isolated(root.path(), name, "restart");
        }
    }
}

#[test]
fn quota_reset_plan_prefers_refreshed_primary_after_seeing_only_weekly() {
    if std::env::var_os(CHILD).is_none() {
        let root = tempfile::tempdir().unwrap();
        run_isolated(
            root.path(),
            "quota_reset_plan_prefers_refreshed_primary_after_seeing_only_weekly",
            "run",
        );
        return;
    }

    let now = chrono::Local::now().timestamp();
    let expired = now - 60;
    let primary = now + 5 * 3600;
    let weekly = expired + 7 * 24 * 3600;
    save_accounts(&["first", "second"], Some(expired));
    let state = serde_json::from_value(json!({
        "enabled": true,
        "tasks": [{
            "id": RESET_TASK, "name": "Quota reset fixture", "enabled": true,
            "accountIds": ["first", "second"], "createdAt": 1, "updatedAt": 1,
            "lastRunAt": now,
            "schedule": {"kind": "quota_reset", "quotaResetWindow": "either"}
        }]
    })).unwrap();
    let saved = codex_wakeup::save_state(&state).unwrap();
    assert_eq!(saved.tasks[0].next_run_at, Some(weekly));

    for (id, reset_at) in [("first", primary + 60), ("second", primary)] {
        let mut account = codex_account::load_account(id).unwrap();
        let quota = account.quota.as_mut().unwrap();
        assert!(super::primary_reset_pending(quota, now));
        quota.hourly_reset_time = None;
        assert!(super::primary_reset_pending(quota, now));
        quota.hourly_reset_time = Some(reset_at);
        assert!(!super::primary_reset_pending(quota, now));
        codex_account::save_account(&account).unwrap();
    }

    let updated = codex_wakeup::load_state().unwrap();
    assert_eq!(updated.tasks[0].next_run_at, Some(primary));
    assert_eq!(current_due_at(&updated.tasks[0], chrono::Local::now()), None);
}

#[test]
fn delayed_quota_reset_survives_refresh_and_restart_without_replay() {
    const TEST_NAME: &str = "delayed_quota_reset_survives_refresh_and_restart_without_replay";
    const DELAY_SECONDS: i64 = 5 * 60;
    match std::env::var(CHILD).as_deref() {
        Ok("plan") => {
            let reset = Local::now().timestamp() - 600;
            save_accounts(&["first"], Some(reset));
            let state = serde_json::from_value(json!({
                "enabled": true,
                "tasks": [{
                    "id": RESET_TASK, "name": "Delayed reset", "enabled": true,
                    "accountIds": ["first"], "createdAt": reset - 120, "updatedAt": reset - 120,
                    "schedule": {"kind": "quota_reset", "quotaResetWindow": "primary_window", "quotaResetDelayMinutes": 5}
                }]
            })).unwrap();
            let saved = codex_wakeup::save_state(&state).unwrap();
            let task = &saved.tasks[0];
            let timing = quota_reset_timing(task, reset - 1);
            assert_eq!(timing.next, Some(reset + DELAY_SECONDS));
            assert_eq!(timing.due, None);
            codex_wakeup::save_quota_reset_plans(vec![(task, timing.plan)]).unwrap();

            let mut account = codex_account::load_account("first").unwrap();
            account.quota.as_mut().unwrap().hourly_reset_time = Some(reset + 5 * 3600);
            codex_account::save_account(&account).unwrap();
        }
        Ok("claim") => {
            let state = codex_wakeup::load_state_for_scheduler().unwrap();
            let task = &state.tasks[0];
            assert_eq!(task.schedule.quota_reset_delay_minutes, Some(5));
            let due = task.quota_reset_plan.as_ref().unwrap().reset_at + DELAY_SECONDS;
            let before = Local.timestamp_opt(due - 1, 0).unwrap();
            assert_eq!(current_due_at(task, before), None);
            assert_eq!(quota_reset_timing(task, before.timestamp()).next, Some(due));
            assert_eq!(current_due_at(task, Local.timestamp_opt(due, 0).unwrap()), Some(due));
            let _lease = codex_wakeup::try_task_run_lease(RESET_TASK).unwrap().unwrap();
            let claimed = codex_wakeup::claim_task_run(RESET_TASK, Some(due), true).unwrap().unwrap();
            assert!(claimed.quota_reset_plan.is_none());
        }
        Ok("reopen") => {
            let state = codex_wakeup::load_state_for_scheduler().unwrap();
            let task = &state.tasks[0];
            assert!(task.last_run_at.is_some());
            assert!(task.quota_reset_plan.is_none());
            assert_eq!(current_due_at(task, Local::now()), None);
        }
        _ => {
            let root = tempfile::tempdir().unwrap();
            for phase in ["plan", "claim", "reopen"] {
                run_isolated(root.path(), TEST_NAME, phase);
            }
        }
    }
}

fn fixture_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn run_isolated(root: &Path, test: &str, phase: &str) {
    let codex_home = root.join("codex-home");
    std::fs::create_dir_all(&codex_home).unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let status = command
        .env_clear()
        .env(CHILD, phase)
        // The unoptimized scheduler and HTTP futures share this test thread.
        .env("RUST_MIN_STACK", "16777216")
        .env("COCKPIT_TOOLS_DATA_DIR", root)
        .env("CODEX_HOME", codex_home)
        .arg("--exact")
        .arg(format!(
            "modules::codex_wakeup_scheduler::dispatch_tests::{test}"
        ))
        .arg("--test-threads=1")
        .status()
        .unwrap();
    assert!(status.success());
}

fn save_accounts(
    ids: &[&'static str],
    reset_at: Option<i64>,
) -> std::collections::HashMap<String, &'static str> {
    let mut index = CodexAccountIndex::new();
    let mut credentials = std::collections::HashMap::new();
    for &id in ids {
        let claims = URL_SAFE_NO_PAD.encode(
            json!({
                "sub": id, "exp": 4_102_444_800i64,
                "https://api.openai.com/auth": {"chatgpt_account_id":id, "chatgpt_user_id": id}
            })
            .to_string(),
        );
        let token = format!("e30.{claims}.fixture");
        credentials.insert(format!("Bearer {token}"), id);
        let mut account = CodexAccount::new(
            id.into(),
            format!("{id}@example.invalid"),
            CodexTokens {
                access_token: token.clone(),
                id_token: token,
                refresh_token: None,
            },
        );
        account.account_id = Some(id.into());
        account.user_id = Some(id.into());
        account.egress_proxy_disabled = true;
        account.quota = reset_at.map(|reset| {
            serde_json::from_value(json!({
                "hourly_percentage": 80, "hourly_reset_time": reset,
                "weekly_percentage": 90, "weekly_reset_time": reset + 604800,
            }))
            .unwrap()
        });
        codex_account::save_account(&account).unwrap();
        index.accounts.push(CodexAccountSummary {
            id: account.id,
            email: account.email,
            plan_type: account.plan_type,
            subscription_active_until: None,
            created_at: account.created_at,
            last_used: account.last_used,
        });
    }
    codex_account::save_account_index(&index).unwrap();
    credentials
}

async fn scheduled_dispatch() {
    let credentials = save_accounts(
        &["selected-a", "selected-b", "selected-c", "pool-only"],
        None,
    );
    let pool_path = crate::modules::account::get_data_dir()
        .unwrap()
        .join("codex_local_access.json");
    std::fs::write(
        pool_path,
        json!({
            "enabled": false, "port": 60000, "apiKey": "local-fixture",
            "accountIds": ["pool-only"], "createdAt": 1, "updatedAt": 1
        })
        .to_string(),
    )
    .unwrap();
    assert_eq!(
        codex_wakeup::resolve_pool_wakeup_account_ids().unwrap(),
        ["pool-only"]
    );

    let task_id = "scheduled-fixture";
    let state = serde_json::from_value(json!({
        "enabled":true, "tasks":[{
            "id":task_id,"name":"Scheduled fixture","enabled":true,
            "accountIds":["selected-a","selected-b","selected-c"],
            "useApiServicePool":false, "prompt":"Wake selected accounts",
            "schedule":{"kind":"interval","intervalHours":1}, "createdAt":1,"updatedAt":1
        }]
    }))
    .unwrap();
    let saved = codex_wakeup::save_state(&state).unwrap();
    let due = current_due_at(&saved.tasks[0], chrono::Local::now()).unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let server = Router::new().route("/responses", post(
        move |headers: HeaderMap, Json(body): Json<Value>| {
            let captured = captured.clone();
            let id = credentials[headers["authorization"].to_str().unwrap()];
            async move {
                assert_eq!(headers["chatgpt-account-id"], id);
                assert_eq!(body["input"][0]["content"][0]["text"], "Wake selected accounts");
                let mut requests = captured.lock().unwrap();
                let first_b = id == "selected-b" && !requests.contains(&id);
                requests.push(id);
                if first_b {
                    return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error":{"message":"fixture upstream failure"}})));
                }
                (StatusCode::OK, Json(json!({"output":[{"type":"message","content":[{"type":"output_text","text":format!("reply from {id}")}]}]})))
            }
        },
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = listener.local_addr().unwrap();
    let upstream = tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
    let scheduled = codex_local_access::TEST_DIRECT_CODEX_ORIGIN.scope(origin, async {
        let batch = run_task(None, task_id, "scheduled", None, Some(due))
            .await
            .unwrap()
            .unwrap();
        assert!(run_task(None, task_id, "scheduled", None, Some(due))
            .await
            .unwrap()
            .is_none());
        batch
    });
    let batch = tokio::time::timeout(Duration::from_secs(10), scheduled)
        .await
        .unwrap();
    assert_eq!(
        *requests.lock().unwrap(),
        ["selected-a", "selected-b", "selected-c"]
    );
    assert_eq!((batch.success_count, batch.failure_count), (2, 1));
    assert_eq!(
        batch
            .records
            .iter()
            .map(|record| (record.account_id.as_str(), record.success))
            .collect::<Vec<_>>(),
        [
            ("selected-a", true),
            ("selected-b", false),
            ("selected-c", true)
        ],
    );
    assert!(batch.records[1]
        .error
        .as_deref()
        .unwrap()
        .contains("fixture upstream failure"));
    assert_eq!(
        batch.records[2].reply.as_deref(),
        Some("reply from selected-c")
    );
    let state = codex_wakeup::load_state_for_scheduler().unwrap();
    assert_eq!(state.tasks[0].last_success_count, Some(2));
    assert_eq!(state.tasks[0].last_failure_count, Some(1));
    assert_eq!(state.tasks[0].last_message, batch.records[1].error);
    let history = codex_wakeup::load_history(false).unwrap().records;
    assert_eq!(
        history
            .iter()
            .filter(|record| record.run_id == batch.run_id)
            .count(),
        3
    );

    let retried = tokio::time::timeout(
        Duration::from_secs(10),
        codex_local_access::TEST_DIRECT_CODEX_ORIGIN.scope(
            origin,
            super::run_task_now(None, task_id, "manual_task", None),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    upstream.abort();
    assert_eq!((retried.success_count, retried.failure_count), (3, 0));
    assert_eq!(
        *requests.lock().unwrap(),
        [
            "selected-a",
            "selected-b",
            "selected-c",
            "selected-a",
            "selected-b",
            "selected-c"
        ],
    );
    let state = codex_wakeup::load_state_for_scheduler().unwrap();
    assert_eq!(state.tasks[0].last_success_count, Some(3));
    assert_eq!(state.tasks[0].last_failure_count, Some(0));
    assert_eq!(state.tasks[0].last_message, None);
}

async fn cancel_quota_reset_wakeup() {
    use crate::modules::account_groups_store::{store, CODEX};
    use axum::{
        body::{Body, Bytes},
        response::Response,
    };
    use std::convert::Infallible;
    use tokio::sync::Notify;

    let reset = chrono::Utc::now().timestamp() - 60;
    let credentials = save_accounts(&["first", "second"], Some(reset));
    // The synthetic quota snapshot stays fixed. No real quota endpoint is called.
    store(CODEX)
        .unwrap()
        .replace_json(
            &json!([{
                "id": "fixture", "name": "Fixture", "accountIds": ["first", "second"],
                "quotaRefreshEnabled": false
            }])
            .to_string(),
            None,
        )
        .unwrap();
    assert!(codex_account::filter_account_ids_by_quota_refresh_policy(&[
        "first".into(),
        "second".into()
    ])
    .is_empty());

    let state = serde_json::from_value(json!({
        "enabled":true, "tasks":[{
            "id": RESET_TASK, "name":"Quota reset fixture","enabled":true,
            "accountIds":["first","second"],"prompt":"hi","createdAt":1,"updatedAt":1,
            "schedule":{"kind":"quota_reset","quotaResetWindow":"primary_window"}
        }]
    }))
    .unwrap();
    let saved = codex_wakeup::save_state(&state).unwrap();
    assert_eq!(
        current_due_at(&saved.tasks[0], chrono::Local::now()),
        Some(reset)
    );
    let received = Arc::new(Notify::new());
    let disconnected = Arc::new(Notify::new());
    let requests = Arc::new(Mutex::new(Vec::new()));
    struct StreamClosed(Arc<Notify>);
    impl Drop for StreamClosed {
        fn drop(&mut self) {
            self.0.notify_one();
        }
    }
    let app = Router::new().route(
        "/responses",
        post({
            let received = received.clone();
            let disconnected = disconnected.clone();
            let requests = requests.clone();
            move |headers: HeaderMap, Json(body): Json<Value>| {
                let received = received.clone();
                let closed = StreamClosed(disconnected.clone());
                let id = credentials[headers["authorization"].to_str().unwrap()];
                let requests = requests.clone();
                async move {
                    assert_eq!(body["input"][0]["content"][0]["text"], "hi");
                    requests.lock().unwrap().push(id);
                    received.notify_one();
                    let response = futures_util::stream::once(async move {
                        let _closed = closed;
                        std::future::pending::<Result<Bytes, Infallible>>().await
                    });
                    Response::builder()
                        .header("content-type", "text/event-stream")
                        .body(Body::from_stream(response))
                        .unwrap()
                }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = listener.local_addr().unwrap();
    let upstream = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let scenario = codex_local_access::TEST_DIRECT_CODEX_ORIGIN.scope(origin, async {
        let run = run_task(None, RESET_TASK, "quota_reset", None, Some(reset));
        let disable = async {
            received.notified().await;
            // A tick while the first account is waiting must not open another request.
            assert!(run_task(None, RESET_TASK, "quota_reset", None, Some(reset))
                .await
                .unwrap()
                .is_none());
            let mut state = codex_wakeup::load_state().unwrap();
            assert_eq!(state.tasks[0].last_status.as_deref(), Some("running"));
            assert!(state.tasks[0].last_run_at.unwrap() >= reset);
            state.enabled = false;
            codex_wakeup::save_state(&state).unwrap();
        };
        let (batch, ()) = tokio::join!(run, disable);
        let batch = batch.unwrap().unwrap();
        assert_eq!((batch.success_count, batch.failure_count), (0, 2));
        assert!(batch.records.iter().all(|record| record
            .error
            .as_deref()
            .is_some_and(|error| error.contains("已取消"))));
        disconnected.notified().await;
        let mut state = codex_wakeup::load_state().unwrap();
        state.enabled = true;
        let reenabled = codex_wakeup::save_state(&state).unwrap();
        assert_eq!(
            current_due_at(&reenabled.tasks[0], chrono::Local::now()),
            None
        );
        assert!(run_task(None, RESET_TASK, "quota_reset", None, Some(reset))
            .await
            .unwrap()
            .is_none());
    });
    tokio::time::timeout(Duration::from_secs(8), scenario)
        .await
        .unwrap();
    assert_eq!(*requests.lock().unwrap(), ["first"]);
    upstream.abort();
}

async fn verify_claim_after_restart() {
    let state = codex_wakeup::load_state_for_scheduler().unwrap();
    assert!(state.enabled);
    assert_eq!(state.tasks[0].last_status.as_deref(), Some("error"));
    assert_eq!(state.tasks[0].last_failure_count, Some(2));
    assert_eq!(current_due_at(&state.tasks[0], chrono::Local::now()), None);
    let reset = codex_account::load_account("first")
        .unwrap()
        .quota
        .unwrap()
        .hourly_reset_time
        .unwrap();
    let unused = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let stale = codex_local_access::TEST_DIRECT_CODEX_ORIGIN.scope(
        unused.local_addr().unwrap(),
        run_task(None, RESET_TASK, "quota_reset", None, Some(reset)),
    );
    assert!(tokio::time::timeout(Duration::from_secs(3), stale)
        .await
        .unwrap()
        .unwrap()
        .is_none());
}
