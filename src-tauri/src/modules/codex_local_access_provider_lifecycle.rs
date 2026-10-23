// Provider engine shutdown must finish before shared state is handed over.
async fn stop_provider_gateway_runtime(runtime_key: &str) -> Option<GatewayBindEndpoint> {
    stop_provider_gateway_runtime_for_reason(runtime_key, "profile_or_instance_stop").await
}

async fn stop_provider_gateway_runtime_for_reason(
    runtime_key: &str,
    reason: &str,
) -> Option<GatewayBindEndpoint> {
    let (endpoint, result) = stop_provider_gateway_runtime_checked(runtime_key, reason).await;
    if let Err(error) = result {
        logger::log_codex_api_warn(&format!("[CodexLocalAccess][provider-gateway] {error}"));
    }
    endpoint
}

async fn stop_provider_gateway_runtime_checked(
    runtime_key: &str,
    reason: &str,
) -> (Option<GatewayBindEndpoint>, Result<(), String>) {
    let mut runtime = {
        let mut runtimes = provider_gateway_runtime_store().lock().await;
        let Some(runtime) = runtimes.remove(runtime_key) else {
            return (None, Ok(()));
        };
        NATIVE_PROFILE_RUNNING.store(
            runtimes
                .values()
                .filter(|runtime| runtime.native_handle.is_some())
                .count(),
            Ordering::Release,
        );
        runtime
    };
    let endpoint = runtime
        .actual_port
        .zip(runtime.actual_bind_host.clone())
        .map(|(port, bind_host)| GatewayBindEndpoint { bind_host, port });
    logger::log_codex_api_info(&format!(
        "[CodexLocalAccess][provider-gateway] 停止 sidecar: runtime={} reason={} pid={:?} port={:?}",
        runtime_key.replace('\n', " / "), reason, runtime.sidecar_child.as_ref().and_then(Child::id),
        endpoint.as_ref().map(|endpoint| endpoint.port)
    ));
    let stopped = if let Some(handle) = runtime.native_handle.as_mut() {
        handle.shutdown(GATEWAY_SHUTDOWN_TIMEOUT).await
    } else if let Some(child) = runtime.sidecar_child.as_mut() {
        stop_gateway_sidecar_child(child).await
    } else {
        Ok(())
    };
    if let Err(error) = stopped {
        // Retain the writer handle after a failed stop. No new engine may export
        // or truncate the shared journal until termination is confirmed.
        let mut runtimes = provider_gateway_runtime_store().lock().await;
        runtimes.insert(runtime_key.to_string(), runtime);
        NATIVE_PROFILE_RUNNING.store(
            runtimes
                .values()
                .filter(|runtime| runtime.native_handle.is_some())
                .count(),
            Ordering::Release,
        );
        return (endpoint, Err(format!("NATIVE_API_SWITCH_FAILED: {error}")));
    }
    if let Some(mut task) = runtime.task.take() {
        tokio::select! {
            _ = &mut task => {}
            _ = tokio::time::sleep(GATEWAY_SHUTDOWN_TIMEOUT) => {
                task.abort();
                let _ = task.await;
            }
        }
    }
    (endpoint, Ok(()))
}

#[cfg(all(test, unix))]
mod continuity_writer_shutdown_tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, BufReader};

    #[tokio::test]
    async fn handoff_shutdown_waits_for_writer_exit_and_final_flush() {
        let directory = tempfile::tempdir().unwrap();
        let journal = directory.path().join("writer-flushed");
        let mut child = TokioCommand::new("/bin/sh")
            .arg("-c")
            .arg("trap 'printf stopped > \"$1\"; exit 0' TERM; printf 'ready\\n'; while :; do sleep 0.05; done")
            .arg("continuity-test")
            .arg(&journal)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let ready = timeout(Duration::from_secs(5), lines.next_line())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ready.as_deref(), Some("ready"));
        stop_gateway_sidecar_child(&mut child).await.unwrap();
        assert!(child.try_wait().unwrap().is_some());
        assert_eq!(std::fs::read_to_string(journal).unwrap(), "stopped");
    }
}

// Called before changing a persisted endpoint. Losing the old port first would
// make a detached Go writer unreachable while a new engine reused its journal.
async fn stop_profile_writer_before_port_reassignment(
    profile_dir: &Path,
    runtime_id: &str,
) -> Result<(), String> {
    let runtime_key = provider_gateway_runtime_key(profile_dir, runtime_id);
    // Tracked handles remain authoritative and are stopped by the existing
    // activation transaction after its new configuration has been validated.
    if provider_gateway_runtime_store()
        .lock()
        .await
        .contains_key(&runtime_key)
    {
        return Ok(());
    }
    stop_persisted_profile_writer_checked(profile_dir, runtime_id).await?;
    Ok(())
}

async fn stop_persisted_profile_writer_checked(
    profile_dir: &Path,
    runtime_id: &str,
) -> Result<Option<GatewayBindEndpoint>, String> {
    // The state file can be removed while its process is still draining. Its
    // deterministic config path remains sufficient to identify that writer.
    let port = load_provider_gateway_profile_state(profile_dir, runtime_id)?
        .and_then(|state| state.port.filter(|port| *port > 0));
    let config_path = sidecar_config_path(&provider_gateway_sidecar_dir(profile_dir, runtime_id)?);
    let stopped = tokio::task::spawn_blocking(move || {
        process::stop_managed_sidecar_writer_for_handoff(
            CODEX_LOCAL_ACCESS_SIDECAR_BIN_NAME,
            &config_path,
            std::process::id(),
        )
    })
    .await
    .map_err(|error| format!("Cannot check persisted continuity writer: {error}"))??;
    if stopped == 0 {
        return Ok(None);
    }
    let Some(port) = port else {
        return Ok(None);
    };
    let endpoint = GatewayBindEndpoint {
        bind_host: CODEX_LOCAL_ACCESS_LOCALHOST_BIND_HOST.to_string(),
        port,
    };
    wait_for_gateway_port_release(&endpoint.bind_host, endpoint.port).await?;
    Ok(Some(endpoint))
}
