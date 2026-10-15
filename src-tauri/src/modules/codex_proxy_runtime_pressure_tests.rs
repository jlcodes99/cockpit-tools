use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[test]
fn inactive_records_are_reclaimed_beyond_the_lifetime_account_count_limit() {
    let registry = Mutex::new(HashMap::new());
    for index in 0..(MAX_RUNTIMES + 32) {
        drop(slot_in(&registry, &format!("account-{index}"), MAX_RUNTIMES).unwrap());
        assert!(registry.lock().unwrap().len() <= MAX_RUNTIMES);
    }
    assert!(registry
        .lock()
        .unwrap()
        .contains_key(&format!("account-{}", MAX_RUNTIMES + 31)));
}

#[test]
fn reclaim_preserves_existing_leases_and_starting_single_flight_slots() {
    let registry = Mutex::new(HashMap::new());
    let transaction = slot_in(&registry, "transaction", 2).unwrap();
    let starting = slot_in(&registry, "starting", 2).unwrap();
    starting.try_lock().unwrap().account_starting = true;
    drop(starting);
    assert_eq!(
        slot_in(&registry, "next", 2).err().unwrap(),
        "PROXY_RUNTIME_CAPACITY"
    );
    assert!(Arc::ptr_eq(
        &transaction,
        &slot_in(&registry, "transaction", 2).unwrap()
    ));
    let starting = slot_in(&registry, "starting", 2).unwrap();
    assert!(starting.try_lock().unwrap().account_starting);
    drop(transaction);
    let next = slot_in(&registry, "next", 2).unwrap();
    assert!(!registry.lock().unwrap().contains_key("transaction"));
    assert!(Arc::ptr_eq(
        &starting,
        &slot_in(&registry, "starting", 2).unwrap()
    ));
    assert!(Arc::ptr_eq(&next, &slot_in(&registry, "next", 2).unwrap()));
}

#[test]
fn locked_records_are_not_evicted_and_failed_admission_recovers() {
    let registry = Mutex::new(HashMap::new());
    let shared = slot_in(&registry, "busy", 1).unwrap();
    let guard = shared.try_lock().unwrap();
    assert_eq!(
        slot_in(&registry, "next", 1).err().unwrap(),
        "PROXY_RUNTIME_CAPACITY"
    );
    drop(guard);
    drop(shared);
    assert!(slot_in(&registry, "next", 1).is_ok());
}

#[test]
fn active_unknown_and_externally_leased_tunnels_are_never_reclaimable() {
    let tunnel = Arc::new(());
    assert!(!tunnel_is_reclaimable(Some(&tunnel), |_| Some(true)));
    assert!(!tunnel_is_reclaimable(Some(&tunnel), |_| None));
    assert!(tunnel_is_reclaimable(Some(&tunnel), |_| Some(false)));
    let request_lease = tunnel.clone();
    assert!(!tunnel_is_reclaimable(Some(&tunnel), |_| Some(false)));
    drop(request_lease);
    assert!(tunnel_is_reclaimable(Some(&tunnel), |_| Some(false)));
    assert!(tunnel_is_reclaimable::<()>(None, |_| panic!(
        "no child to probe"
    )));
}

#[test]
fn concurrent_capacity_requests_cannot_replace_a_leased_slot_identity() {
    let registry = Arc::new(Mutex::new(HashMap::new()));
    let original = slot_in(&registry, "shared", 1).unwrap();
    let workers = (0..8)
        .map(|_| {
            let registry = registry.clone();
            let original = original.clone();
            std::thread::spawn(move || {
                for _ in 0..32 {
                    assert!(Arc::ptr_eq(
                        &original,
                        &slot_in(&registry, "shared", 1).unwrap()
                    ));
                    assert_eq!(
                        slot_in(&registry, "other", 1).err().unwrap(),
                        "PROXY_RUNTIME_CAPACITY"
                    );
                }
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().unwrap();
    }
}

#[tokio::test]
async fn ninth_read_waits_for_a_slot_instead_of_failing_immediately() {
    let reads = Arc::new(tokio::sync::Semaphore::new(8));
    let permits = reads.clone().acquire_many_owned(8).await.unwrap();
    let started = Arc::new(AtomicBool::new(false));
    let started_work = started.clone();
    let task = tokio::spawn(read_with_limit(
        reads.clone(),
        Duration::from_secs(1),
        Duration::from_secs(2),
        move || {
            started_work.store(true, Ordering::SeqCst);
            9
        },
    ));
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(!task.is_finished());
    assert!(!started.load(Ordering::SeqCst));
    drop(permits);
    assert_eq!(task.await.unwrap().unwrap(), 9);
    assert_eq!(reads.available_permits(), 8);
}

#[tokio::test]
async fn queue_timeout_does_not_dispatch_work_and_next_read_recovers() {
    let reads = Arc::new(tokio::sync::Semaphore::new(1));
    let permit = reads.clone().acquire_owned().await.unwrap();
    let error = read_with_limit(
        reads.clone(),
        Duration::from_millis(20),
        Duration::from_secs(1),
        || panic!("timed-out queue must not dispatch disk work"),
    )
    .await
    .unwrap_err();
    assert_eq!(error, "PROXY_RUNTIME_BUSY");
    drop(permit);
    assert_eq!(
        read_with_limit(
            reads.clone(),
            Duration::from_secs(1),
            Duration::from_secs(1),
            || 7
        )
        .await
        .unwrap(),
        7
    );
    assert_eq!(reads.available_permits(), 1);
}

#[tokio::test]
async fn timed_out_disk_read_retains_its_permit_until_work_finishes() {
    let reads = Arc::new(tokio::sync::Semaphore::new(1));
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let task = tokio::spawn(read_with_limit(
        reads.clone(),
        Duration::from_secs(1),
        Duration::from_millis(150),
        move || {
            entered.send(()).unwrap();
            blocked.recv_timeout(Duration::from_secs(5)).unwrap();
        },
    ));
    waiting.await.unwrap();
    assert_eq!(
        task.await.unwrap().unwrap_err(),
        "PROXY_RUNTIME_READ_TIMEOUT"
    );
    assert_eq!(reads.available_permits(), 0);
    assert_eq!(
        read_with_limit(
            reads.clone(),
            Duration::from_millis(20),
            Duration::from_secs(1),
            || ()
        )
        .await
        .unwrap_err(),
        "PROXY_RUNTIME_BUSY"
    );
    release.send(()).unwrap();
    assert_eq!(
        read_with_limit(
            reads.clone(),
            Duration::from_secs(1),
            Duration::from_secs(1),
            || 42
        )
        .await
        .unwrap(),
        42
    );
}

#[tokio::test]
async fn cancelled_read_keeps_the_actual_io_bounded() {
    let reads = Arc::new(tokio::sync::Semaphore::new(1));
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let task = tokio::spawn(read_with_limit(
        reads.clone(),
        Duration::from_secs(1),
        Duration::from_secs(2),
        move || {
            entered.send(()).unwrap();
            blocked.recv_timeout(Duration::from_secs(5)).unwrap();
        },
    ));
    waiting.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(reads.available_permits(), 0);
    release.send(()).unwrap();
    let _permit = tokio::time::timeout(Duration::from_secs(1), reads.acquire())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn cancelled_binding_transaction_retains_reserved_capacity_until_write_finishes() {
    let registry = Mutex::new(HashMap::new());
    let reservation = slot_in(&registry, "binding", 1).unwrap();
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let task = tokio::spawn(persist_binding_mutation(lock.clone(), move || {
        let _reservation = reservation;
        entered.send(()).unwrap();
        blocked.recv_timeout(Duration::from_secs(5)).unwrap();
        Ok(())
    }));
    waiting.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(
        slot_in(&registry, "next", 1).err().unwrap(),
        "PROXY_RUNTIME_CAPACITY"
    );
    release.send(()).unwrap();
    let _guard = tokio::time::timeout(Duration::from_secs(1), lock.lock())
        .await
        .unwrap();
    assert!(slot_in(&registry, "next", 1).is_ok());
}

#[test]
fn concurrent_reclamation_does_not_lease_the_entire_registry() {
    const READERS: usize = 16;
    let registry = Arc::new(Mutex::new(HashMap::new()));
    let ready = Arc::new(std::sync::Barrier::new(READERS));
    let workers = (0..READERS)
        .map(|worker| {
            let registry = registry.clone();
            let ready = ready.clone();
            std::thread::spawn(move || {
                let mut failed_round = None;
                for round in 0..32 {
                    if worker == 0 {
                        let mut store = registry.lock().unwrap();
                        store.clear();
                        for index in 0..READERS {
                            store.insert(format!("old-{index}"), Slot::default());
                        }
                    }
                    ready.wait();
                    let result = slot_in(&registry, &format!("new-{worker}"), READERS);
                    // Keep each successful admission leased until every scanner finishes.
                    ready.wait();
                    let succeeded = result.is_ok();
                    drop(result);
                    ready.wait();
                    if !succeeded {
                        failed_round = Some(round);
                    }
                }
                assert!(failed_round.is_none(), "idle capacity must be available to worker {worker}, failed round {failed_round:?}");
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().unwrap();
    }
}
