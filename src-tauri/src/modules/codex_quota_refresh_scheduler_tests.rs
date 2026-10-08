use super::*;
use std::sync::Arc;

// Only these tests exercise the global scheduler. Operations are synthetic:
// no account files, tokens, network requests or desktop actions are involved.
static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .expect("scheduler stalled")
}

fn quota() -> CodexQuota {
    serde_json::from_value(serde_json::json!({"hourly_percentage": 80, "weekly_percentage": 70}))
        .unwrap()
}

async fn mock_refresh(_: String) -> RefreshResult {
    Ok(quota())
}

async fn never_refresh(_: String) -> RefreshResult {
    panic!("duplicate/cancelled worker must not execute")
}

#[test]
fn scheduler_limits_are_small_and_explicit() {
    assert_eq!(MANUAL_CONCURRENCY + BACKGROUND_CONCURRENCY, 2);
    assert_eq!(MAX_BACKGROUND_PENDING_ACCOUNTS, 16);
    assert_eq!(MAX_MANUAL_PENDING_ACCOUNTS, 16);
    assert_eq!(MAX_WAITERS_PER_ACCOUNT, 16);
    assert!(QUEUE_WAIT_TIMEOUT <= Duration::from_secs(120));
    assert!(REFRESH_TIMEOUT <= Duration::from_secs(120));
}

#[tokio::test]
async fn manual_lane_runs_while_background_is_active_and_same_account_joins() {
    let _guard = TEST_LOCK.lock().await;
    let (started, started_rx) = oneshot::channel();
    let (finish, finish_rx) = oneshot::channel();
    let mut background = Box::pin(refresh_with_operation(
        "running",
        RefreshPriority::Background,
        Some(background_epoch()),
        move |_| async move {
            started.send(()).unwrap();
            finish_rx.await.unwrap();
            Ok(quota())
        },
    ));
    assert!(futures_util::poll!(&mut background).is_pending());
    bounded(started_rx).await.unwrap();
    let mut duplicate = Box::pin(refresh_with_operation(
        "running",
        RefreshPriority::Manual,
        None,
        never_refresh,
    ));
    assert!(futures_util::poll!(&mut duplicate).is_pending());
    assert_eq!(
        bounded(refresh_with_operation(
            "manual",
            RefreshPriority::Manual,
            None,
            mock_refresh
        ))
        .await
        .unwrap()
        .hourly_percentage,
        80
    );
    assert_eq!(BACKGROUND_PERMIT.available_permits(), 0);
    cancel_queued_background_refreshes(None);
    assert!(IN_FLIGHT.lock().unwrap().get("running").unwrap().started);
    finish.send(()).unwrap();
    assert_eq!(bounded(background).await.unwrap().hourly_percentage, 80);
    assert_eq!(bounded(duplicate).await.unwrap().hourly_percentage, 80);
    assert!(IN_FLIGHT.lock().unwrap().is_empty());
    assert_eq!(MANUAL_PERMIT.available_permits(), 1);
    assert_eq!(BACKGROUND_PERMIT.available_permits(), 1);
}

#[tokio::test]
async fn promotion_cancels_other_pending_work_and_invalidates_old_batch() {
    let _guard = TEST_LOCK.lock().await;
    let background_permit = BACKGROUND_PERMIT.acquire().await.unwrap();
    let epoch = background_epoch();
    let mut promoted = Box::pin(refresh_with_operation(
        "promote",
        RefreshPriority::Background,
        Some(epoch),
        mock_refresh,
    ));
    let mut cancelled = Box::pin(refresh_with_operation(
        "cancel",
        RefreshPriority::Background,
        Some(epoch),
        never_refresh,
    ));
    assert!(futures_util::poll!(&mut promoted).is_pending());
    assert!(futures_util::poll!(&mut cancelled).is_pending());
    cancel_queued_background_refreshes(Some("promote"));
    assert!(bounded(cancelled).await.is_err());
    // A member not polled/enqueued before cancellation must not restart the batch.
    assert!(bounded(refresh_with_operation(
        "late",
        RefreshPriority::Background,
        Some(epoch),
        never_refresh
    ))
    .await
    .is_err());
    let result = bounded(refresh_with_operation(
        "promote",
        RefreshPriority::Manual,
        None,
        never_refresh,
    ))
    .await
    .unwrap();
    assert_eq!(result.hourly_percentage, 80);
    assert!(bounded(promoted).await.is_ok());
    assert!(IN_FLIGHT.lock().unwrap().is_empty());
    drop(background_permit);
    assert!(bounded(refresh_with_operation(
        "next-round",
        RefreshPriority::Background,
        Some(background_epoch()),
        mock_refresh
    ))
    .await
    .is_ok());
}

#[tokio::test]
async fn queues_and_duplicate_waiters_are_bounded_and_drained() {
    let _guard = TEST_LOCK.lock().await;
    let manual_permit = MANUAL_PERMIT.acquire().await.unwrap();
    let background_permit = BACKGROUND_PERMIT.acquire().await.unwrap();
    let ids = (0..16).map(|n| format!("bounded-{n}")).collect::<Vec<_>>();
    let bg_ids = (0..16)
        .map(|n| format!("background-{n}"))
        .collect::<Vec<_>>();
    let mut manual = Vec::new();
    let mut background = Vec::new();
    for (id, bg_id) in ids.iter().zip(bg_ids.iter()) {
        let mut request = Box::pin(refresh_with_operation(
            id,
            RefreshPriority::Manual,
            None,
            mock_refresh,
        ));
        assert!(futures_util::poll!(&mut request).is_pending());
        manual.push(request);
        let mut request = Box::pin(refresh_with_operation(
            bg_id,
            RefreshPriority::Background,
            Some(background_epoch()),
            never_refresh,
        ));
        assert!(futures_util::poll!(&mut request).is_pending());
        background.push(request);
    }
    for priority in [RefreshPriority::Manual, RefreshPriority::Background] {
        assert!(bounded(refresh_with_operation(
            "overflow",
            priority,
            Some(background_epoch()),
            never_refresh
        ))
        .await
        .is_err());
    }
    // Promotion cannot exceed the manual queue capacity.
    assert!(bounded(refresh_with_operation(
        &bg_ids[0],
        RefreshPriority::Manual,
        None,
        never_refresh
    ))
    .await
    .is_err());
    let mut duplicates = Vec::new();
    for _ in 1..MAX_WAITERS_PER_ACCOUNT {
        let mut request = Box::pin(refresh_with_operation(
            &ids[0],
            RefreshPriority::Manual,
            None,
            never_refresh,
        ));
        assert!(futures_util::poll!(&mut request).is_pending());
        duplicates.push(request);
    }
    assert!(bounded(refresh_with_operation(
        &ids[0],
        RefreshPriority::Manual,
        None,
        never_refresh
    ))
    .await
    .is_err());
    // A dropped caller must not permanently consume a waiter slot.
    duplicates.pop();
    let mut replacement = Box::pin(refresh_with_operation(
        &ids[0],
        RefreshPriority::Manual,
        None,
        never_refresh,
    ));
    assert!(futures_util::poll!(&mut replacement).is_pending());
    cancel_queued_background_refreshes(None);
    for request in background {
        assert!(bounded(request).await.is_err());
    }
    drop(background_permit);
    drop(manual_permit);
    for request in manual {
        assert!(bounded(request).await.is_ok());
    }
    for request in duplicates {
        assert!(bounded(request).await.is_ok());
    }
    assert!(bounded(replacement).await.is_ok());
    assert!(IN_FLIGHT.lock().unwrap().is_empty());
}

#[tokio::test]
async fn stale_worker_cannot_execute_or_remove_a_replacement() {
    let _guard = TEST_LOCK.lock().await;
    let permit = MANUAL_PERMIT.acquire().await.unwrap();
    let mut request = Box::pin(refresh_with_operation(
        "reused",
        RefreshPriority::Manual,
        None,
        mock_refresh,
    ));
    assert!(futures_util::poll!(&mut request).is_pending());
    let generation = IN_FLIGHT.lock().unwrap().get("reused").unwrap().generation;
    bounded(run_refresh(
        "reused".into(),
        generation.wrapping_sub(1),
        never_refresh,
    ))
    .await;
    assert_eq!(
        IN_FLIGHT.lock().unwrap().get("reused").unwrap().generation,
        generation
    );
    drop(permit);
    assert!(bounded(request).await.is_ok());
    assert!(IN_FLIGHT.lock().unwrap().is_empty());
}

#[tokio::test]
async fn panic_and_failure_release_entries_and_allow_retry() {
    let _guard = TEST_LOCK.lock().await;
    assert!(bounded(refresh_with_operation(
        "panic",
        RefreshPriority::Manual,
        None,
        never_refresh
    ))
    .await
    .is_err());
    assert!(IN_FLIGHT.lock().unwrap().is_empty());
    assert!(bounded(refresh_with_operation(
        "error",
        RefreshPriority::Manual,
        None,
        |_| async { Err("mock failure".into()) }
    ))
    .await
    .is_err());
    assert!(IN_FLIGHT.lock().unwrap().is_empty());
    assert!(bounded(refresh_with_operation(
        "panic",
        RefreshPriority::Manual,
        None,
        mock_refresh
    ))
    .await
    .is_ok());
    assert_eq!(MANUAL_PERMIT.available_permits(), 1);
}

#[tokio::test]
async fn timed_out_operation_drops_its_owned_resources() {
    let resource = Arc::new(());
    let owned = resource.clone();
    let operation = async move {
        let _keep_alive = owned;
        std::future::pending::<RefreshResult>().await
    };
    assert!(execute_operation(operation, Duration::from_millis(1))
        .await
        .is_err());
    assert_eq!(Arc::strong_count(&resource), 1);
}
