//! Port-allocation regressions use only test-owned files and loopback listeners.
use super::*;

#[test]
fn scattered_candidates_preserve_the_old_order_and_are_bounded_and_unique() {
    let start = 59_980;
    let candidates: Vec<_> = candidate_ports("scatter-order", Some(start)).collect();
    let original: Vec<_> = (0..PORT_ATTEMPTS)
        .map(|step| PORT_RANGE_START + (start - PORT_RANGE_START + step) % PORT_RANGE_SIZE)
        .collect();
    assert_eq!(&candidates[..usize::from(PORT_ATTEMPTS)], &original);
    assert!(candidates.len() > usize::from(PORT_ATTEMPTS));
    assert!(candidates.len() <= usize::from(PORT_ATTEMPTS + PORT_SCATTER_ATTEMPTS));
    assert_eq!(
        candidates.iter().copied().collect::<HashSet<_>>().len(),
        candidates.len()
    );
    assert!(candidates.iter().all(|port| is_managed_port(*port)));
    assert!(!original.contains(&candidates[usize::from(PORT_ATTEMPTS)]));
}

#[tokio::test]
async fn a_large_contiguous_exclusion_falls_back_and_persists_the_bound_listener() {
    let temp = TempDir::new("large-exclusion");
    let path = temp.ports_file();
    let id = format!("large-exclusion-{}", uuid::Uuid::new_v4());
    let recorded = preferred_port(&id);
    let mut registry = PortRegistry::default();
    registry.record(&id, recorded);
    save_registry(&path, &registry).unwrap();
    let blocked: Vec<_> = (0..80u32)
        .map(|step| {
            PORT_RANGE_START
                + ((u32::from(recorded - PORT_RANGE_START) + step) % u32::from(PORT_RANGE_SIZE))
                    as u16
        })
        .collect();
    let mut blockers = Vec::new();
    for port in &blocked {
        // A port already excluded/held by the OS is unavailable too.
        if let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, *port)).await {
            blockers.push(listener);
        }
    }
    let (listener, port) = install(&id, Some(&path))
        .await
        .expect("escape the excluded range");
    assert!(!blocked.contains(&port));
    assert_eq!(listener.local_addr().unwrap().port(), port);
    assert_eq!(read_registry(&path).recorded(&id), Some(port));
    assert!(
        TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .await
            .is_err(),
        "the successful probe remains the installed listener"
    );
    drop(listener);
    drop(blockers);
    let (restarted, reused) = install(&id, Some(&path)).await.unwrap();
    assert_eq!(
        reused, port,
        "host restart prefers the saved scattered port"
    );
    drop(restarted);
}

#[tokio::test]
async fn simultaneous_colliding_accounts_get_distinct_persisted_ports() {
    let temp = TempDir::new("concurrent-ports");
    let path = temp.ports_file();
    // These IDs have the same deterministic legacy port.
    let first = "port-collision-164";
    let second = "port-collision-227";
    assert_eq!(preferred_port(first), preferred_port(second));
    let (first_result, second_result) =
        tokio::join!(install(first, Some(&path)), install(second, Some(&path)),);
    let (first_listener, first_port) = first_result.unwrap();
    let (second_listener, second_port) = second_result.unwrap();
    assert_ne!(first_port, second_port);
    let registry = read_registry(&path);
    assert_eq!(registry.recorded(first), Some(first_port));
    assert_eq!(registry.recorded(second), Some(second_port));
    assert_eq!(first_listener.local_addr().unwrap().port(), first_port);
    assert_eq!(second_listener.local_addr().unwrap().port(), second_port);
}

#[tokio::test]
async fn another_accounts_inactive_persisted_port_is_never_reassigned() {
    let temp = TempDir::new("reserved-ports");
    let path = temp.ports_file();
    let id = format!("reserved-ports-{}", uuid::Uuid::new_v4());
    let reserved = preferred_port(&id);
    let mut registry = PortRegistry::default();
    registry.record("inactive-owner", reserved);
    save_registry(&path, &registry).unwrap();
    let (listener, port) = install(&id, Some(&path)).await.unwrap();
    assert_ne!(port, reserved);
    let saved = read_registry(&path);
    assert_eq!(saved.recorded("inactive-owner"), Some(reserved));
    assert_eq!(saved.recorded(&id), Some(port));
    drop(listener);
}

#[tokio::test]
async fn unknown_registry_ownership_does_not_allocate_or_overwrite_the_file() {
    let temp = TempDir::new("unknown-ports");
    let path = temp.ports_file();
    fs::write(&path, "{broken").unwrap();
    let id = format!("unknown-ports-{}", uuid::Uuid::new_v4());
    assert_eq!(
        install(&id, Some(&path)).await.unwrap_err(),
        "PROXY_ENTRY_PORT_UNAVAILABLE"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), "{broken");
    assert!(!PORT_OWNERS.lock().await.contains_key(&id));
    // Recoverable once the file can be read; no poisoned permanent allocation state.
    save_registry(&path, &PortRegistry::default()).unwrap();
    let (listener, _) = install(&id, Some(&path)).await.unwrap();
    drop(listener);
}

#[tokio::test]
async fn exact_recovery_cannot_claim_another_accounts_in_memory_port() {
    let owner = format!("memory-owner-{}", uuid::Uuid::new_v4());
    let recovering = format!("memory-recovery-{}", uuid::Uuid::new_v4());
    let (listener, port) = install(&owner, None).await.unwrap();
    drop(listener);
    assert_eq!(
        restore_exact_port(&recovering, port).await.unwrap_err(),
        "PROXY_ENTRY_PORT_UNAVAILABLE"
    );
    assert_eq!(route_port(&recovering).await, None);
    let status = entry_status(&recovering).unwrap();
    assert_eq!(status.port, Some(port));
    assert_eq!(status.state, "failed");
    assert_eq!(status.last_error, Some("PROXY_ENTRY_PORT_UNAVAILABLE"));
    assert!(!PORT_OWNERS.lock().await.contains_key(&recovering));
}
