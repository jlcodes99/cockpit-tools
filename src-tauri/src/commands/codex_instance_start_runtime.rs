//! Reuse the stop completed before a prepared profile was written.
//! Only internal callers holding the profile mutation lease may pass `Stopped`.

#[derive(Clone, Copy)]
pub(super) enum StartRuntimeState {
    NeedsStop,
    Stopped,
}

pub(super) async fn stop_runtime_for_start<F>(
    state: StartRuntimeState,
    stop: F,
) -> Result<&'static str, String>
where
    F: FnOnce() -> Result<(), String> + Send + 'static,
{
    if matches!(state, StartRuntimeState::Stopped) {
        return Ok("already-stopped");
    }
    tauri::async_runtime::spawn_blocking(stop)
        .await
        .map_err(|error| format!("Codex stop worker failed: {error}"))??;
    Ok("closed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[test]
    fn prepared_profile_does_not_stop_the_runtime_again() {
        let calls = Arc::new(AtomicUsize::new(0));
        let stops = calls.clone();
        let result = tauri::async_runtime::block_on(stop_runtime_for_start(
            StartRuntimeState::Stopped,
            move || {
                stops.fetch_add(1, Ordering::SeqCst);
                Err("must not stop".into())
            },
        ));
        assert_eq!(result.unwrap(), "already-stopped");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn ordinary_start_stops_once_and_preserves_stop_failure() {
        let calls = Arc::new(AtomicUsize::new(0));
        let stops = calls.clone();
        let error = tauri::async_runtime::block_on(stop_runtime_for_start(
            StartRuntimeState::NeedsStop,
            move || {
                stops.fetch_add(1, Ordering::SeqCst);
                Err("still running".into())
            },
        ))
        .unwrap_err();
        assert_eq!(error, "still running");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
