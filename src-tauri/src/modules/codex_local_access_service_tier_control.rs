// Only the main API service accepts this control. Instance provider gateways
// retain their own profile defaults and lifecycle.
const API_SERVICE_SPEED_SYNC_TIMEOUT: Duration = Duration::from_secs(3);
const API_SERVICE_SPEED_CONTROL_TIMEOUT: Duration = Duration::from_secs(2);
const API_SERVICE_SPEED_SYNC_FAILED: &str = "codex_api_service_speed_sync_failed";
static API_SERVICE_SPEED_CONTROL_REVISION: AtomicU64 = AtomicU64::new(0);

pub(crate) fn api_service_speed_save_lock() -> &'static TokioMutex<()> {
    static LOCK: OnceLock<TokioMutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| TokioMutex::new(()))
}

/// The preference is already persisted. A failed acknowledgement is surfaced
/// to the caller, and retrying the same value is safe. Never start or restart a
/// gateway here, and never hold the runtime state lock across local HTTP I/O.
pub async fn sync_running_api_service_speed(speed: CodexAppSpeed) -> Result<(), String> {
    let result = timeout(API_SERVICE_SPEED_SYNC_TIMEOUT, async {
        let _lifecycle_guard = gateway_lifecycle_lock().lock().await;
        // The lifecycle counter can advance while the existing process is
        // reused. Compare it against this operation's snapshot, not the
        // process's launch generation; still detect stop requests during I/O.
        let lifecycle_generation = current_gateway_lifecycle_generation();
        let (port, sidecar_generation) = {
            let runtime = gateway_runtime().lock().await;
            if !runtime.running {
                return Ok(());
            }
            // Native requests read the persisted preference when selecting their route.
            if runtime.native_handle.is_some() { return Ok(()); }
            let port = runtime.actual_port.ok_or("running gateway has no port")?;
            (port, runtime.sidecar_generation)
        };
        send_api_service_speed_to_sidecar(port, &speed, API_SERVICE_SPEED_CONTROL_TIMEOUT).await?;
        let runtime = gateway_runtime().lock().await;
        if !runtime.running
            || runtime.actual_port != Some(port)
            || runtime.sidecar_generation != sidecar_generation
            || gateway_lifecycle_generation_changed(lifecycle_generation)
        {
            return Err("gateway changed during speed update".to_string());
        }
        Ok(())
    })
    .await;
    match result {
        Ok(Ok(())) => Ok(()),
        failure => {
            let reason = match failure {
                Ok(Err(reason)) => reason,
                Err(_) => "speed update timed out".to_string(),
                Ok(Ok(())) => unreachable!(),
            };
            logger::log_codex_api_warn(&format!(
                "[CodexLocalAccess] API 服务速度已保存，但运行网关更新未确认，可重试: {reason}"
            ));
            Err(API_SERVICE_SPEED_SYNC_FAILED.to_string())
        }
    }
}

async fn send_api_service_speed_to_sidecar(
    port: u16,
    speed: &CodexAppSpeed,
    request_timeout: Duration,
) -> Result<(), String> {
    // Revisions also protect against an old timed-out HTTP handler resuming
    // after a newer save was acknowledged. The sidecar rejects stale revisions.
    let revision = API_SERVICE_SPEED_CONTROL_REVISION
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
            current.checked_add(1)
        })
        .map_err(|_| "speed control revision exhausted".to_string())?
        + 1;
    let client = build_localhost_http_client(request_timeout, "API 服务速度设置")?;
    let response = client
        .post(format!(
            "http://{CODEX_LOCAL_ACCESS_LOCALHOST_BIND_HOST}:{port}/v1/cockpit/service-tier-default"
        ))
        .bearer_auth(internal_api_service_key())
        .json(&json!({"speed": speed, "revision": revision}))
        .send()
        .await
        .map_err(|_| "speed control connection failed".to_string())?;
    if !response.status().is_success() {
        return Err(format!("speed control HTTP {}", response.status()));
    }
    let acknowledgement: Value = response
        .json()
        .await
        .map_err(|_| "invalid speed control acknowledgement".to_string())?;
    if acknowledgement.get("speed") != Some(&json!(speed))
        || acknowledgement.get("revision").and_then(Value::as_u64) != Some(revision)
    {
        return Err("speed control acknowledgement mismatch".to_string());
    }
    Ok(())
}
