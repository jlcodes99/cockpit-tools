// Retiring Go listeners stop accepting requests but retain their workers and
// diagnostic pipes until their accepted requests finish. New traffic uses the
// replacement listener. Explicit service shutdown still stops every child.
const MAX_RETIRED_SIDECARS: usize = 4;
const RETIRED_SIDECAR_LIMIT: Duration = Duration::from_secs(16 * 60);

struct RetiredSidecar {
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

fn retired_sidecars() -> &'static Mutex<Vec<RetiredSidecar>> {
    static VALUE: OnceLock<Mutex<Vec<RetiredSidecar>>> = OnceLock::new();
    VALUE.get_or_init(|| Mutex::new(Vec::new()))
}

async fn retire_gateway_sidecar() -> Result<Option<GatewayBindEndpoint>, String> {
    {
        let mut retired = retired_sidecars()
            .lock()
            .map_err(|_| "Sidecar retirement lock unavailable")?;
        retired.retain(|entry| !entry.task.is_finished());
        if retired.len() >= MAX_RETIRED_SIDECARS {
            return Err("API gateway is still draining earlier requests; retry shortly".into());
        }
    }
    let mut runtime = gateway_runtime().lock().await;
    let Some(child) = runtime.sidecar_child.as_mut() else {
        return Ok(None);
    };
    let Some(stdin) = child.stdin.as_mut() else {
        return Ok(None);
    };
    timeout(Duration::from_secs(1), stdin.write_all(b"drain\n"))
        .await
        .map_err(|_| "Sidecar reload control timed out")?
        .map_err(|_| "Sidecar reload control is unavailable")?;
    let endpoint = runtime
        .actual_port
        .zip(runtime.actual_bind_host.clone())
        .map(|(port, bind_host)| GatewayBindEndpoint { bind_host, port });
    let child = runtime.sidecar_child.take().expect("checked sidecar child");
    let output = runtime.task.take();
    let monitor = runtime.sidecar_monitor_task.take();
    runtime.running = false;
    runtime.actual_port = None;
    runtime.actual_bind_host = None;
    runtime.sidecar_config_fingerprint = None;
    runtime.applied_collection = None;
    runtime.applied_account_signatures.clear();
    runtime.sidecar_generation = None;
    drop(runtime);
    if let Some(monitor) = monitor {
        monitor.abort();
        let _ = monitor.await;
    }
    let retired = supervise_retired_sidecar(child, output);
    retired_sidecars()
        .lock()
        .map_err(|_| "Sidecar retirement lock unavailable")?
        .push(retired);
    Ok(endpoint)
}

fn supervise_retired_sidecar(
    mut child: Child,
    output: Option<tokio::task::JoinHandle<()>>,
) -> RetiredSidecar {
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(async move {
        tokio::select! {
            result = child.wait() => {
                if let Err(error) = result { logger::log_codex_api_warn(&format!("[CodexLocalAccess] Retired sidecar wait failed: {error}")); }
            }
            _ = stopped => { let _ = stop_gateway_sidecar_child(&mut child).await; }
            _ = tokio::time::sleep(RETIRED_SIDECAR_LIMIT) => {
                logger::log_codex_api_warn("[CodexLocalAccess] Retired sidecar exceeded its drain limit");
                let _ = stop_gateway_sidecar_child(&mut child).await;
            }
        }
        // Keep reading late completion, accounting and identity diagnostics.
        if let Some(mut output) = output {
            if timeout(GATEWAY_SHUTDOWN_TIMEOUT, &mut output)
                .await
                .is_err()
            {
                output.abort();
                let _ = output.await;
            }
        }
    });
    RetiredSidecar { stop, task }
}

async fn stop_retired_sidecars() {
    let retired = retired_sidecars()
        .lock()
        .map(|mut value| std::mem::take(&mut *value))
        .unwrap_or_default();
    // Signal all children first so their bounded shutdowns run concurrently.
    let tasks: Vec<_> = retired
        .into_iter()
        .map(|entry| {
            let _ = entry.stop.send(());
            entry.task
        })
        .collect();
    for task in tasks {
        let _ = task.await;
    }
}

#[cfg(all(test, unix))]
mod sidecar_retirement_tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::process::Command;

    #[tokio::test]
    async fn retired_process_finishes_work_and_flushes_late_diagnostics() {
        let mut child = Command::new("sh")
            .args([
                "-c",
                "read command; printf '%s\\n' \"$command\"; read release; printf 'completed\\n'",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        input.write_all(b"drain\n").await.unwrap();
        assert_eq!(lines.next_line().await.unwrap().as_deref(), Some("drain"));
        let (flush, flushed) = oneshot::channel();
        let (observed, observation) = oneshot::channel();
        let output = tokio::spawn(async move {
            assert_eq!(
                lines.next_line().await.unwrap().as_deref(),
                Some("completed")
            );
            let _ = observed.send(());
            let _ = flushed.await;
        });
        let retired = supervise_retired_sidecar(child, Some(output));
        assert!(!retired.task.is_finished(), "accepted work was killed");
        input.write_all(b"release\n").await.unwrap();
        timeout(Duration::from_secs(2), observation)
            .await
            .unwrap()
            .unwrap();
        assert!(
            !retired.task.is_finished(),
            "diagnostic reader dropped before flush"
        );
        let _ = flush.send(());
        timeout(Duration::from_secs(2), retired.task)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn explicit_stop_reaps_retired_process_and_its_reader() {
        // Shell builtins avoid leaving grandchildren behind in this fixture.
        let mut child = Command::new("sh")
            .args(["-c", "read release"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let (sent, received) = oneshot::channel();
        let output = tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut bytes = Vec::new();
            let _ = stdout.read_to_end(&mut bytes).await;
            let _ = sent.send(());
        });
        let retired = supervise_retired_sidecar(child, Some(output));
        let _ = retired.stop.send(());
        timeout(Duration::from_secs(4), retired.task)
            .await
            .unwrap()
            .unwrap();
        received.await.unwrap();
        assert_eq!(unsafe { libc::kill(pid as libc::pid_t, 0) }, -1);
    }
}
