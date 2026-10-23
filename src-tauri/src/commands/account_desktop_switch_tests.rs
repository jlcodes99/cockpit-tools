use super::{complete_desktop_switch, try_begin_desktop_switch};
use std::cell::RefCell;
use std::sync::Arc;

#[test]
fn credential_failure_never_changes_binding_or_launches() {
    let effects = RefCell::new(Vec::new());
    let result = complete_desktop_switch(
        || Err("credential write failed".to_string()),
        || {
            effects.borrow_mut().push("commit");
            Ok(())
        },
        true,
        || {
            effects.borrow_mut().push("launch");
            Ok(())
        },
    );
    assert_eq!(result.unwrap_err(), "credential write failed");
    assert!(effects.borrow().is_empty());
}

#[test]
fn no_launch_still_writes_credentials_before_binding() {
    let effects = RefCell::new(Vec::new());
    let result = complete_desktop_switch(
        || {
            effects.borrow_mut().push("inject");
            Ok(())
        },
        || {
            effects.borrow_mut().push("commit");
            Ok(())
        },
        false,
        || {
            panic!("no-launch switch must not start a client");
        },
    );
    assert_eq!(result.unwrap(), None);
    assert_eq!(*effects.borrow(), ["inject", "commit"]);
}

#[test]
fn launch_failure_retains_successful_credential_switch_result() {
    let effects = RefCell::new(Vec::new());
    let result = complete_desktop_switch(
        || {
            effects.borrow_mut().push("inject");
            Ok(())
        },
        || {
            effects.borrow_mut().push("commit");
            Ok(())
        },
        true,
        || {
            effects.borrow_mut().push("launch");
            Err("launch failed".to_string())
        },
    );
    assert_eq!(result.unwrap(), Some("launch failed".to_string()));
    assert_eq!(*effects.borrow(), ["inject", "commit", "launch"]);
}

#[test]
fn identity_commit_failure_reports_that_credentials_have_already_changed() {
    let result = complete_desktop_switch(
        || Ok(()),
        || Err("binding write failed".to_string()),
        true,
        || {
            panic!("cannot launch after binding failure");
        },
    );
    assert_eq!(
        result.unwrap_err(),
        "ANTIGRAVITY_SWITCH_STATE_SAVE_FAILED: binding write failed"
    );
}

#[tokio::test]
async fn blocking_worker_keeps_single_flight_after_caller_is_dropped() {
    let gate = Arc::new(tokio::sync::Semaphore::new(1));
    let permit = try_begin_desktop_switch(&gate).unwrap();
    let (finish_tx, finish_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        finish_rx.recv().unwrap();
        drop(_permit);
        let _ = done_tx.send(());
    });
    drop(task);
    assert!(try_begin_desktop_switch(&gate).is_err());
    finish_tx.send(()).unwrap();
    done_rx.await.unwrap();
    assert!(try_begin_desktop_switch(&gate).is_ok());
}
