// Explicit checks reuse only the running account tunnel. No scheduler, engine
// startup, binding mutation or selector update is performed by this path.
static CURRENT_LATENCY_CHECKS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(3);
static LATENCY_ACCOUNTS: LazyLock<Mutex<std::collections::HashSet<String>>> =
    LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

struct LatencyGuard(String);
impl LatencyGuard {
    fn acquire(account_id: &str) -> Result<Self, String> {
        let mut accounts = LATENCY_ACCOUNTS.lock().map_err(|_| "PROXY_PROBE_FAILED")?;
        if !accounts.insert(account_id.to_owned()) {
            return Err("PROXY_PROBE_BUSY".into());
        }
        Ok(Self(account_id.to_owned()))
    }
}
impl Drop for LatencyGuard {
    fn drop(&mut self) {
        if let Ok(mut accounts) = LATENCY_ACCOUNTS.lock() {
            accounts.remove(&self.0);
        }
    }
}

fn same_latency_snapshot<T>(
    current_stamp: &str,
    current: &Arc<T>,
    stamp: &str,
    snapshot: &Arc<T>,
) -> bool {
    current_stamp == stamp && Arc::ptr_eq(current, snapshot)
}

fn latency_tunnel_matches(state: &Runtime, stamp: &str, tunnel: &Arc<DesktopTunnel>) -> bool {
    state
        .tunnel
        .as_ref()
        .is_some_and(|current| same_latency_snapshot(&state.signature, current, stamp, tunnel))
        && tunnel.is_running()
}

pub async fn measure_current_latency(account_id: &str) -> Result<RuntimeStatus, String> {
    let _permit = CURRENT_LATENCY_CHECKS
        .try_acquire()
        .map_err(|_| "PROXY_PROBE_BUSY")?;
    let _guard = LatencyGuard::acquire(account_id)?;
    let account = load(account_id).await?;
    if !codex_account_proxy::eligible(&account) {
        return Err("PROXY_ACCOUNT_UNSUPPORTED".into());
    }
    let value =
        codex_account_proxy::configured_url(&account)?.ok_or("PROXY_LATENCY_NOT_RUNNING")?;
    let stamp = signature(value.as_ref());
    let shared = RUNTIMES
        .lock()
        .map_err(|_| "PROXY_PROBE_FAILED")?
        .get(account_id)
        .cloned()
        .ok_or("PROXY_LATENCY_NOT_RUNNING")?;
    let tunnel = {
        let state = shared.try_lock().map_err(|_| "PROXY_PROBE_BUSY")?;
        state
            .tunnel
            .as_ref()
            .filter(|tunnel| state.signature == stamp && tunnel.is_running())
            .cloned()
            .ok_or("PROXY_LATENCY_NOT_RUNNING")?
    };
    let reader = tunnel
        .selection_reader()
        .ok_or("PROXY_LATENCY_NOT_RUNNING")?;
    let (target, result) = reader.measure_current_delay(&tunnel.controller()).await?;
    let token_lock = codex_account::codex_token_lock_for(account_id);
    let _token_guard = token_lock.try_lock().map_err(|_| "PROXY_PROBE_BUSY")?;
    let latest = load(account_id).await?;
    if codex_account_proxy::configured_url(&latest)?
        .map(|value| signature(value.as_ref()))
        .as_deref()
        != Some(&stamp)
    {
        return Err("PROXY_BINDING_CHANGED".into());
    }
    {
        let state = shared.try_lock().map_err(|_| "PROXY_PROBE_BUSY")?;
        if !latency_tunnel_matches(&state, &stamp, &tunnel) {
            return Err("PROXY_BINDING_CHANGED".into());
        }
        // A failed check is a real observation: retain its timestamp with no
        // delay, so future read-only refreshes cannot resurrect an old success.
        reader.record_latency(target, result.as_ref().ok().copied());
    }
    drop(_token_guard);
    result?;
    status(account_id).await
}

#[cfg(test)]
mod current_latency_tests {
    use super::*;

    #[test]
    fn current_latency_rejects_rebound_accounts_and_restarted_tunnels() {
        let original = Arc::new(1);
        let restarted = Arc::new(1);
        assert!(same_latency_snapshot(
            "first",
            &original.clone(),
            "first",
            &original
        ));
        assert!(!same_latency_snapshot(
            "new-binding",
            &original,
            "first",
            &original
        ));
        assert!(!same_latency_snapshot(
            "first", &restarted, "first", &original
        ));
    }

    #[test]
    fn current_latency_global_limit_does_not_queue_more_network_checks() {
        let permits = (0..3)
            .map(|_| CURRENT_LATENCY_CHECKS.try_acquire().unwrap())
            .collect::<Vec<_>>();
        assert!(CURRENT_LATENCY_CHECKS.try_acquire().is_err());
        drop(permits);
        assert!(CURRENT_LATENCY_CHECKS.try_acquire().is_ok());
    }

    #[test]
    fn current_latency_guard_deduplicates_accounts_and_releases_on_drop() {
        let id = format!("test-latency-{}", uuid::Uuid::new_v4());
        let guard = LatencyGuard::acquire(&id).unwrap();
        assert_eq!(
            LatencyGuard::acquire(&id).err().as_deref(),
            Some("PROXY_PROBE_BUSY")
        );
        let other = LatencyGuard::acquire(&(id.clone() + "-other")).unwrap();
        drop(guard);
        assert!(LatencyGuard::acquire(&id).is_ok());
        drop(other);
    }

    #[tokio::test]
    async fn current_latency_guard_releases_when_future_is_cancelled() {
        let id = format!("test-latency-{}", uuid::Uuid::new_v4());
        let task_id = id.clone();
        let (ready, received) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _guard = LatencyGuard::acquire(&task_id).unwrap();
            ready.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        received.await.unwrap();
        assert!(LatencyGuard::acquire(&id).is_err());
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(LatencyGuard::acquire(&id).is_ok());
    }
}
