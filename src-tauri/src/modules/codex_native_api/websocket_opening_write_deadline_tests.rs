use super::*;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

struct PendingWrite(Arc<AtomicBool>);
impl Drop for PendingWrite {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn opening_control_write_uses_explicit_turn_deadline_and_keeps_its_write_cap() {
    // No task-local scope is active here. Both pre-split and split opening
    // helpers pass their turn deadline explicitly to this shared write path.
    assert!(super::super::execution::current().is_none());
    for total_shorter in [true, false] {
        let (_stop, snapshot) = tokio::sync::watch::channel(false);
        let cancelled = Arc::new(AtomicBool::new(false));
        let guard = PendingWrite(cancelled.clone());
        let write = async move {
            let _guard = guard;
            futures_util::future::pending::<Result<(), std::io::Error>>().await
        };
        let started = tokio::time::Instant::now();
        let (write_cap, turn_remaining) = if total_shorter {
            (Duration::from_millis(500), Duration::from_millis(50))
        } else {
            (Duration::from_millis(50), Duration::from_millis(500))
        };
        let error = timed_write(
            super::super::lifecycle::StopSignal(snapshot),
            write_cap,
            Some(started + turn_remaining),
            write,
        )
        .await
        .unwrap_err();
        assert_eq!(error.status, 504);
        assert_eq!(error.code, "websocket_send_timeout");
        assert!(started.elapsed() < Duration::from_millis(300));
        assert!(cancelled.load(Ordering::SeqCst));
    }
}

#[tokio::test]
async fn opening_control_write_still_observes_stop_and_allows_successful_writes() {
    let (stop, snapshot) = tokio::sync::watch::channel(false);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    let value = timed_write(
        super::super::lifecycle::StopSignal(snapshot.clone()),
        Duration::from_secs(1),
        Some(deadline),
        async { Ok::<_, std::io::Error>(42) },
    )
    .await
    .unwrap();
    assert_eq!(value, 42);
    let cancelled = Arc::new(AtomicBool::new(false));
    let guard = PendingWrite(cancelled.clone());
    let write = async move {
        let _guard = guard;
        futures_util::future::pending::<Result<(), std::io::Error>>().await
    };
    let cancel = async {
        tokio::time::sleep(Duration::from_millis(10)).await;
        stop.send_replace(true);
    };
    let (error, ()) = tokio::join!(
        timed_write(
            super::super::lifecycle::StopSignal(snapshot),
            Duration::from_secs(1),
            Some(deadline),
            write
        ),
        cancel
    );
    assert_eq!(error.unwrap_err().code, "service_stopping");
    assert!(cancelled.load(Ordering::SeqCst));
}
