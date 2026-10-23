#[test]
fn config_readers_keep_cached_values_during_slow_persistence() {
    use std::sync::{Arc, RwLock, mpsc};
    use std::time::Duration;
    let state = Arc::new(RwLock::new(super::RuntimeState {
        user_config: super::UserConfig::default(), actual_port: None,
    }));
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let writer_state = Arc::clone(&state);
    let writer = std::thread::spawn(move || super::patch_runtime_state(
        &writer_state, |cached| Ok((cached.clone(), ())),
        |_| { started_tx.send(()).unwrap(); release_rx.recv_timeout(Duration::from_secs(3)).unwrap(); Ok(()) },
        |_| {}, |config| { config.floating_card_minimal = true; Ok(()) },
    ));
    started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    let cached = state.try_read().expect("slow disk must not hold the reader lock");
    assert!(!cached.user_config.floating_card_minimal);
    drop(cached);
    release_tx.send(()).unwrap();
    assert!(writer.join().unwrap().unwrap().floating_card_minimal);
    assert!(state.read().unwrap().user_config.floating_card_minimal);
}

#[test]
fn config_failed_persistence_keeps_last_committed_appearance() {
    let state = std::sync::RwLock::new(super::RuntimeState {
        user_config: super::UserConfig::default(), actual_port: None,
    });
    let result = super::patch_runtime_state(&state, |cached| Ok((cached.clone(), ())),
        |_| Err("disk unavailable".into()), |_| panic!("failed save must not commit"),
        |config| { config.floating_card_minimal = true; Ok(()) });
    assert!(result.is_err());
    assert!(!state.read().unwrap().user_config.floating_card_minimal);
}

#[test]
fn config_cross_process_lock_wait_is_bounded_and_retryable() {
    let dir = std::env::temp_dir().join(format!("cockpit-config-lock-timeout-{}-{}", std::process::id(), uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.lock");
    let held = super::acquire_config_file_lock(&path).unwrap();
    let started = std::time::Instant::now();
    let error = super::acquire_config_file_lock(&path).unwrap_err();
    assert_eq!(error, "common.configSaveTimeout");
    assert!(started.elapsed() < std::time::Duration::from_secs(7));
    drop(held);
    drop(super::acquire_config_file_lock(&path).unwrap());
    std::fs::remove_dir_all(dir).unwrap();
}
