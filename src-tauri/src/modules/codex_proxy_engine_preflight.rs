//! Explicit operation prerequisite. Reads never download an engine or open an upstream connection.
use super::codex_proxy_engine as engine;
use std::{
    future::Future,
    sync::{LazyLock, Mutex},
    time::Duration,
};
use tokio::sync::watch;

type CheckResult = Result<(), String>;

/// Concurrent clicks share one bounded disk/version check. Completed results are
/// not cached: removal, damage, or a completed repair is observed on the next operation.
#[derive(Default)]
struct Checks(Mutex<Option<watch::Receiver<Option<CheckResult>>>>);
static CHECKS: LazyLock<Checks> = LazyLock::new(Checks::default);

impl Checks {
    async fn run(
        &self,
        work: impl Future<Output = CheckResult> + Send + 'static,
        timeout: Duration,
    ) -> CheckResult {
        let mut receiver = {
            let mut current = self.0.lock().map_err(|_| "PROXY_ENGINE_START_FAILED")?;
            if let Some(receiver) = current
                .as_ref()
                .filter(|rx| rx.has_changed().is_ok() && rx.borrow().is_none())
            {
                receiver.clone()
            } else {
                let (sender, receiver) = watch::channel(None);
                *current = Some(receiver.clone());
                // The supervisor stays alive after a caller cancels, so the shared
                // slot always completes. It cannot mutate catalog/account state.
                tokio::spawn(async move {
                    let result = tokio::time::timeout(timeout, work)
                        .await
                        .unwrap_or_else(|_| Err("PROXY_ENGINE_TIMEOUT".into()));
                    let _ = sender.send(Some(result));
                });
                receiver
            }
        };
        loop {
            if let Some(result) = receiver.borrow().clone() {
                return result;
            }
            receiver
                .changed()
                .await
                .map_err(|_| "PROXY_ENGINE_START_FAILED")?;
        }
    }
}

/// Call only before an explicit action, never for a page, account list, or status poll.
pub async fn require() -> CheckResult {
    super::codex_proxy_engine_install::check_not_installing()?;
    CHECKS
        .run(
            async {
                let binary = engine::engine_path_async().await?;
                let lease_binary = binary.clone();
                let _lease = tokio::time::timeout(
                    Duration::from_secs(3),
                    tokio::task::spawn_blocking(move || {
                        super::codex_proxy_engine_install::lease_managed(&lease_binary)
                    }),
                )
                .await
                .map_err(|_| "PROXY_ENGINE_TIMEOUT")?
                .map_err(|_| "PROXY_ENGINE_START_FAILED")??;
                // Never execute a managed binary before verifying its recorded digest.
                super::codex_proxy_engine_install::verify_managed(&binary).await?;
                engine::verify_version(&binary).await
            },
            Duration::from_secs(15),
        )
        .await?;
    // A repair can start while the shared hash/version check is in flight.
    super::codex_proxy_engine_install::check_not_installing()
}

/// These prerequisite failures must not be reclassified as account authorization errors.
pub(crate) fn is_prerequisite_error(error: &str) -> bool {
    matches!(
        error,
        "PROXY_ENGINE_MISSING"
            | "PROXY_ENGINE_TIMEOUT"
            | "PROXY_ENGINE_VERSION"
            | "PROXY_ENGINE_START_FAILED"
            | "ENGINE_INSTALL_VERIFY"
            | "ENGINE_INSTALL_IO"
            | "ENGINE_INSTALL_TIMEOUT"
            | "ENGINE_INSTALL_BUSY"
            | "ENGINE_INSTALL_UNSUPPORTED"
            | "ENGINE_INSTALL_ARCHIVE"
            | "ENGINE_INSTALL_TOO_LARGE"
    )
}

#[derive(Clone, Copy)]
pub(crate) enum Usage {
    AccountRequest,
    Desktop,
}

pub(crate) fn requires_engine(value: &str, usage: Usage) -> bool {
    match usage {
        Usage::AccountRequest => !super::codex_proxy_runtime::is_direct(value),
        Usage::Desktop => super::codex_proxy_runtime::desktop_engine_required(value),
    }
}

pub(crate) async fn for_url(value: &str, usage: Usage) -> CheckResult {
    if requires_engine(value, usage) {
        require().await?;
    }
    Ok(())
}

pub(crate) async fn for_account(account_id: &str, usage: Usage) -> CheckResult {
    let account = super::codex_proxy_runtime::load(account_id).await?;
    if !super::codex_account_proxy::eligible(&account) {
        return Ok(());
    }
    if let Some(value) = super::codex_account_proxy::configured_url(&account)? {
        for_url(value.as_ref(), usage).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[test]
    fn raw_request_paths_are_exempt_but_catalog_and_desktop_auth_are_not() {
        for value in [
            "http://proxy.example:8080",
            "https://user:secret@proxy.example:443",
            "socks5://user:secret@proxy.example:1080",
            "socks5h://proxy.example:1080",
        ] {
            assert!(!requires_engine(value, Usage::AccountRequest));
        }
        assert!(!requires_engine(
            "http://proxy.example:8080",
            Usage::Desktop
        ));
        assert!(requires_engine(
            "http://user:secret@proxy.example:8080",
            Usage::Desktop
        ));
        for value in [
            "cockpit-proxy://snapshot",
            "trojan://secret@node.example:443",
        ] {
            assert!(requires_engine(value, Usage::AccountRequest));
            assert!(requires_engine(value, Usage::Desktop));
        }
    }

    #[tokio::test]
    async fn concurrent_checks_are_single_flight_and_a_repair_is_not_cached_out() {
        let checks = Checks::default();
        let runs = Arc::new(AtomicUsize::new(0));
        let work = || {
            let runs = runs.clone();
            async move {
                runs.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(20)).await;
                Err("PROXY_ENGINE_MISSING".to_string())
            }
        };
        let (a, b) = tokio::join!(
            checks.run(work(), Duration::from_secs(1)),
            checks.run(work(), Duration::from_secs(1))
        );
        assert_eq!(a.unwrap_err(), "PROXY_ENGINE_MISSING");
        assert_eq!(b.unwrap_err(), "PROXY_ENGINE_MISSING");
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert!(checks
            .run(async { Ok(()) }, Duration::from_secs(1))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn cancelled_or_timed_out_preflight_never_reaches_dependent_mutation() {
        let checks = Checks::default();
        let writes = AtomicUsize::new(0);
        let blocked = async {
            checks
                .run(std::future::pending(), Duration::from_millis(30))
                .await?;
            writes.fetch_add(1, Ordering::SeqCst);
            Ok::<_, String>(())
        };
        assert!(tokio::time::timeout(Duration::from_millis(5), blocked)
            .await
            .is_err());
        let error = checks
            .run(async { Ok(()) }, Duration::from_secs(1))
            .await
            .unwrap_err();
        assert_eq!(error, "PROXY_ENGINE_TIMEOUT");
        assert_eq!(writes.load(Ordering::SeqCst), 0);
        assert!(checks
            .run(async { Ok(()) }, Duration::from_secs(1))
            .await
            .is_ok());
    }
}
