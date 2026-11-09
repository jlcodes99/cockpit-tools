#[test]
fn disable_restores_profiles_even_when_gateway_shutdown_fails() {
    let mut restored = false;
    let result = super::finish_local_access_disable(Err("port still occupied".to_string()), || {
        restored = true;
        Ok(())
    });
    assert!(restored);
    assert_eq!(result, Err("port still occupied".to_string()));
}

#[tokio::test]
async fn port_recovery_preserves_an_unrelated_listener_and_switches_to_a_new_port() {
    let _env_lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _env = LocalAccessTestDataGuard::new("local-access-port-recovery");
    let occupied = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let original_port = occupied.local_addr().unwrap().port();
    super::with_chatgpt_plan_test_runtime(|| async {
        let mut original = test_local_access_collection(Vec::new());
        original.port = original_port;
        super::save_collection_to_disk(&original).unwrap();
        {
            let mut runtime = super::gateway_runtime().lock().await;
            runtime.loaded = true;
            runtime.collection = Some(original.clone());
        }
        assert!(super::kill_local_access_port_processes().await.is_err());
        assert_eq!(super::load_collection_from_disk().unwrap().unwrap().port, original_port);

        let result = super::change_random_local_access_port().await;
        let running = super::gateway_runtime().lock().await.running;
        super::stop_gateway().await;

        let new = result.unwrap().collection.unwrap();
        assert_ne!(new.port, original_port);
        assert!(running);
        assert_eq!(new.api_key, original.api_key);
        assert_eq!(new.account_ids, original.account_ids);
        assert_eq!(super::load_collection_from_disk().unwrap().unwrap().port, new.port);
        assert!(std::net::TcpStream::connect(("127.0.0.1", original_port)).is_ok());
    }).await;
}

#[test]
fn disable_preserves_both_shutdown_and_profile_restore_failures() {
    let result = super::finish_local_access_disable(Err("port still occupied".to_string()), || {
        Err("profile restore failed".to_string())
    });
    let error = result.expect_err("both operations failed");
    assert!(error.contains("port still occupied"));
    assert!(error.contains("profile restore failed"));
}

#[test]
fn disable_reports_profile_restore_failure_after_successful_shutdown() {
    assert_eq!(
        super::finish_local_access_disable(Ok(()), || Err("profile restore failed".to_string())),
        Err("profile restore failed".to_string()),
    );
    assert_eq!(
        super::finish_local_access_disable(Ok(()), || Ok(())),
        Ok(())
    );
}

#[tokio::test]
async fn disable_waits_for_occupied_port_to_release() {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let release = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        drop(listener);
    });
    super::wait_for_gateway_port_release("127.0.0.1", port)
        .await
        .unwrap();
    release.await.unwrap();
    assert!(super::is_local_access_port_bindable("127.0.0.1", port).unwrap());
}

#[tokio::test]
async fn disable_reports_port_that_never_releases_with_bounded_wait() {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let result = tokio::time::timeout(
        Duration::from_secs(7),
        super::wait_for_gateway_port_release("127.0.0.1", port),
    )
    .await;
    let error = result
        .expect("port wait must be bounded")
        .expect_err("occupied port must fail");
    assert!(error.contains(&port.to_string()));
    drop(listener);
}

#[tokio::test]
async fn disable_times_out_while_another_lifecycle_operation_holds_lock() {
    let _env_lock = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let _lock = super::gateway_lifecycle_lock().lock().await;
    let result = tokio::time::timeout(
        Duration::from_secs(4),
        super::stop_gateway_and_wait_for_release("127.0.0.1", 0),
    )
    .await;
    let error = result
        .expect("lifecycle wait must be bounded")
        .expect_err("busy lifecycle must fail");
    assert!(error.contains("超时"));
}

#[test]
fn cancelled_disable_restoration_does_not_read_or_modify_profiles() {
    let _lock = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let _env = LocalAccessTestDataGuard::new("cancelled-disable-restore");
    let collection = test_local_access_collection(Vec::new());
    let result = super::restore_takeover_profiles_after_disable_checked(&collection, || {
        Err("cancelled".to_string())
    });
    assert_eq!(result, Err("cancelled".to_string()));
}

#[tokio::test]
async fn disable_rejects_duplicate_transition_without_waiting_on_profile_io() {
    let _env_lock = crate::modules::test_support::env_lock()
        .lock().unwrap_or_else(|error| error.into_inner());
    let _guard = super::local_access_enable_transition_lock().lock().await;
    let result = tokio::time::timeout(
        Duration::from_millis(200),
        super::set_local_access_enabled(false),
    )
    .await;
    let error = result
        .expect("duplicate transition must fail promptly")
        .expect_err("another transition is active");
    assert!(error.contains("正在启停"));
}
