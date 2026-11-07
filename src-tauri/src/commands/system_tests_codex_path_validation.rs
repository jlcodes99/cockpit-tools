use super::run_codex_path_validation;

#[tokio::test]
async fn codex_path_validation_timeout_or_cancellation_keeps_slot_until_worker_really_exits() {
    for cancel_caller in [false, true] {
        let slot = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        let first_slot = slot.clone();
        let first = tokio::spawn(async move {
            run_codex_path_validation(
                first_slot,
                move || {
                    let _ = started_tx.send(());
                    // Dropping finish_tx on an assertion failure also releases this
                    // fixture worker; no real files or user settings are accessed.
                    let _ = finish_rx.recv();
                    Ok(())
                },
                if cancel_caller {
                    std::time::Duration::from_secs(5)
                } else {
                    std::time::Duration::from_millis(20)
                },
            )
            .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), started_rx)
            .await
            .expect("validation fixture started")
            .unwrap();
        if cancel_caller {
            first.abort();
            assert!(first.await.unwrap_err().is_cancelled());
        } else {
            assert_eq!(
                first.await.unwrap(),
                Err("CODEX_APP_PATH_VALIDATION_TIMEOUT".into())
            );
        }
        assert_eq!(slot.available_permits(), 0);
        assert_eq!(
            run_codex_path_validation(
                slot.clone(),
                || panic!("The second validation must never spawn while the first worker remains"),
                std::time::Duration::from_secs(1),
            )
            .await,
            Err("CODEX_APP_PATH_VALIDATION_BUSY".into())
        );
        finish_tx.send(()).unwrap();
        let permit = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            slot.clone().acquire_owned(),
        )
        .await
        .expect("worker released its slot")
        .unwrap();
        drop(permit);
        assert_eq!(
            run_codex_path_validation(slot, || Ok(()), std::time::Duration::from_secs(1)).await,
            Ok(())
        );
    }
}
