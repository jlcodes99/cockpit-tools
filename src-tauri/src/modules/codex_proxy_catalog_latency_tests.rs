use super::*;
use std::sync::atomic::AtomicBool;
use tokio::sync::oneshot;

#[tokio::test]
async fn latency_cancel_during_preflight_returns_without_starting_node_work() {
    let id = uuid::Uuid::new_v4().to_string();
    let (entered, ready) = oneshot::channel();
    let (finish, pending) = oneshot::channel();
    let ran = Arc::new(AtomicBool::new(false));
    let observed = ran.clone();
    let request_id = id.clone();
    let task = tokio::spawn(latency_with_preflight(
        request_id,
        Duration::from_secs(10),
        async move {
            let _ = entered.send(());
            pending.await.map_err(|_| "PRECHECK_DROPPED".to_string())
        },
        async move {
            observed.store(true, Ordering::Release);
            Ok(())
        },
    ));
    ready.await.unwrap();
    cancel(id.clone()).unwrap();
    let error = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("cancellation must not wait for the engine preflight")
        .unwrap()
        .unwrap_err();
    assert_eq!(error, "CATALOG_CANCELLED");
    assert!(
        finish.send(()).is_err(),
        "the preflight wait must be dropped"
    );
    assert!(!ran.load(Ordering::Acquire));
    assert!(!JOBS.lock().unwrap().contains_key(&id));
}

#[tokio::test]
async fn latency_deadline_includes_preflight_and_prevents_a_late_probe() {
    let id = uuid::Uuid::new_v4().to_string();
    let (finish, pending) = oneshot::channel();
    let ran = AtomicBool::new(false);
    let error = latency_with_preflight(
        id.clone(),
        Duration::from_millis(30),
        async { pending.await.map_err(|_| "PRECHECK_DROPPED".to_string()) },
        async {
            ran.store(true, Ordering::Release);
            Ok(())
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error, "PROXY_PROBE_TIMEOUT");
    assert!(
        finish.send(()).is_err(),
        "a late preflight cannot resume the probe"
    );
    assert!(!ran.load(Ordering::Acquire));
    assert!(!JOBS.lock().unwrap().contains_key(&id));
}

#[tokio::test]
async fn latency_cancellation_wins_when_preflight_completes_in_the_same_poll() {
    let id = uuid::Uuid::new_v4().to_string();
    let cancelled_id = id.clone();
    let ran = AtomicBool::new(false);
    let error = latency_with_preflight(
        id,
        Duration::from_secs(1),
        async move { cancel(cancelled_id) },
        async {
            ran.store(true, Ordering::Release);
            Ok(())
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error, "CATALOG_CANCELLED");
    assert!(!ran.load(Ordering::Acquire));
}

#[tokio::test]
async fn latency_expired_deadline_cannot_start_an_immediately_ready_probe() {
    let ran = AtomicBool::new(false);
    let error = latency_with_preflight(
        uuid::Uuid::new_v4().to_string(),
        Duration::ZERO,
        std::future::ready(Ok(())),
        async {
            ran.store(true, Ordering::Release);
            Ok(())
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error, "PROXY_PROBE_TIMEOUT");
    assert!(!ran.load(Ordering::Acquire));
}

#[tokio::test]
async fn latency_preflight_failure_is_preserved_and_success_starts_work_once() {
    let count = AtomicU8::new(0);
    let error = latency_with_preflight(
        uuid::Uuid::new_v4().to_string(),
        Duration::from_secs(1),
        std::future::ready(Err("ENGINE_INSTALL_VERIFY".into())),
        async {
            count.fetch_add(1, Ordering::Relaxed);
            Ok(216)
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error, "ENGINE_INSTALL_VERIFY");
    assert_eq!(count.load(Ordering::Relaxed), 0);
    let value = latency_with_preflight(
        uuid::Uuid::new_v4().to_string(),
        Duration::from_secs(1),
        std::future::ready(Ok(())),
        async {
            count.fetch_add(1, Ordering::Relaxed);
            Ok(216)
        },
    )
    .await
    .unwrap();
    assert_eq!(value, 216);
    assert_eq!(count.load(Ordering::Relaxed), 1);
}
