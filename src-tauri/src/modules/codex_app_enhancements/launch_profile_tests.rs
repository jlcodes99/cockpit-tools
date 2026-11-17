use super::*;

#[tokio::test]
async fn client_launch_waits_for_the_initial_profile_publication() {
    let (published, prepared) = oneshot::channel();
    let (started, starting) = oneshot::channel();
    let launched = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed = launched.clone();
    let launch = tokio::spawn(async move {
        started.send(()).unwrap();
        assert!(wait_for_launch_profile(Some(prepared), Duration::from_secs(2)).await);
        observed.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    starting.await.unwrap();
    tokio::task::yield_now().await;
    assert!(!launched.load(std::sync::atomic::Ordering::SeqCst));
    // run_profile signals only after the read worker published its result.
    published.send(()).unwrap();
    launch.await.unwrap();
    assert!(launched.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn stalled_profile_read_does_not_prevent_native_startup() {
    let (_published, prepared) = oneshot::channel();
    let ready = tokio::time::timeout(
        Duration::from_secs(1),
        wait_for_launch_profile(Some(prepared), Duration::from_millis(20)),
    )
    .await
    .expect("optional preparation must have a bounded wait");
    assert!(!ready);
}

#[tokio::test]
async fn stopped_profile_worker_does_not_hold_up_native_startup() {
    let (published, prepared) = oneshot::channel();
    drop(published);
    let ready = tokio::time::timeout(
        Duration::from_secs(1),
        wait_for_launch_profile(Some(prepared), Duration::from_secs(10)),
    )
    .await
    .expect("worker failure must release the launch immediately");
    assert!(!ready);
}

#[tokio::test]
async fn profiles_without_optional_preparation_start_normally() {
    assert!(wait_for_launch_profile(None, Duration::ZERO).await);
}
