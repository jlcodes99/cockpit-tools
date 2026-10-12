mod service_tier_control_tests {
    use super::super::*;

    fn speed_request_body(request: &str) -> Value {
        serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap()
    }

    async fn speed_control_server(
        status: u16,
        body: Value,
        delay: Duration,
    ) -> (u16, tokio::task::JoinHandle<String>) {
        let (port, task, _) = speed_control_server_with_signal(status, body, delay).await;
        (port, task)
    }

    async fn speed_control_server_with_signal(
        status: u16,
        mut body: Value,
        delay: Duration,
    ) -> (
        u16,
        tokio::task::JoinHandle<String>,
        tokio::sync::oneshot::Receiver<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (received, request_received) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut buffer = [0; 1024];
                let count = socket.read(&mut buffer).await.unwrap();
                if count == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            line.split_once(':')
                                .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                                .map(|(_, value)| value.trim().parse().unwrap())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let request = String::from_utf8(bytes).unwrap();
            if body.get("revision").is_none() {
                body["revision"] = speed_request_body(&request)["revision"].clone();
            }
            let _ = received.send(());
            tokio::time::sleep(delay).await;
            let body = body.to_string();
            let response = format!("HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = socket.write_all(response.as_bytes()).await;
            request
        });
        (port, task, request_received)
    }

    #[tokio::test]
    async fn service_tier_control_sends_private_auth_and_checks_acknowledgement() {
        let (port, server) =
            speed_control_server(200, json!({"speed":"fast"}), Duration::ZERO).await;
        send_api_service_speed_to_sidecar(port, &CodexAppSpeed::Fast, Duration::from_secs(1))
            .await
            .unwrap();
        let request = server.await.unwrap();
        assert!(request.starts_with("POST /v1/cockpit/service-tier-default HTTP/1.1\r\n"));
        assert!(request
            .lines()
            .any(|line| line.eq_ignore_ascii_case(&format!(
                "authorization: Bearer {}",
                internal_api_service_key()
            ))));
        assert_eq!(speed_request_body(&request)["speed"], "fast");
        assert!(speed_request_body(&request)["revision"].as_u64().unwrap() > 0);
        for (status, response) in [
            (200, json!({"speed":"standard"})),
            (200, json!({})),
            (200, json!({"speed":"fast", "revision":0})),
            (200, json!({"speed":"fast", "revision":null})),
            (403, json!({"speed":"fast"})),
        ] {
            let (port, server) = speed_control_server(status, response, Duration::ZERO).await;
            assert!(send_api_service_speed_to_sidecar(
                port,
                &CodexAppSpeed::Fast,
                Duration::from_secs(1)
            )
            .await
            .is_err());
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn service_tier_control_slow_or_closed_server_has_bounded_failure() {
        let (port, server) =
            speed_control_server(200, json!({"speed":"fast"}), Duration::from_millis(200)).await;
        let started = Instant::now();
        assert!(send_api_service_speed_to_sidecar(
            port,
            &CodexAppSpeed::Fast,
            Duration::from_millis(30)
        )
        .await
        .is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
        server.abort();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert!(send_api_service_speed_to_sidecar(
            port,
            &CodexAppSpeed::Standard,
            Duration::from_millis(100)
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn service_tier_control_only_updates_running_main_service_without_holding_runtime_lock() {
        let _env_lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let (port, server) =
            speed_control_server(200, json!({"speed":"fast"}), Duration::from_millis(80)).await;
        let previous = {
            let mut runtime = gateway_runtime().lock().await;
            let previous = std::mem::take(&mut *runtime);
            runtime.running = true;
            runtime.actual_port = Some(port);
            runtime.sidecar_generation = Some(current_gateway_lifecycle_generation());
            previous
        };
        let update = tokio::spawn(sync_running_api_service_speed(CodexAppSpeed::Fast));
        tokio::time::sleep(Duration::from_millis(20)).await;
        let state = timeout(Duration::from_millis(20), gateway_runtime().lock())
            .await
            .expect("runtime remains readable during control request");
        assert!(state.running);
        drop(state);
        update.await.unwrap().unwrap();
        server.await.unwrap();
        {
            let mut runtime = gateway_runtime().lock().await;
            assert_eq!(runtime.actual_port, Some(port));
            assert!(runtime.task.is_none());
            assert!(runtime.sidecar_child.is_none());
            runtime.running = false;
        }
        // A stopped service must not connect to its stale port or start a process.
        sync_running_api_service_speed(CodexAppSpeed::Standard)
            .await
            .unwrap();
        let mut runtime = gateway_runtime().lock().await;
        assert!(!runtime.running);
        *runtime = previous;
    }

    #[test]
    fn service_tier_control_manifest_is_enabled_only_for_main_api_service() {
        let _env_lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let env = super::LocalAccessTestDataGuard::new("service-tier-control-manifest");
        let collection = super::test_local_access_collection(Vec::new());
        for enabled in [true, false] {
            let config = prepare_sidecar_launch_config_in_dir_sync(
                &collection,
                env.data_dir.join(format!("sidecar-{enabled}")),
                HashMap::new(),
                Some("priority"),
                HashMap::new(),
                enabled,
                None,
            )
            .unwrap();
            let manifest: Value =
                serde_json::from_slice(&fs::read(config.manifest_path).unwrap()).unwrap();
            assert_eq!(manifest["serviceTierControlEnabled"], enabled);
        }
    }

    #[tokio::test]
    async fn service_tier_control_waits_for_startup_then_updates_current_port() {
        let _env_lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let lifecycle = gateway_lifecycle_lock().lock().await;
        let previous = std::mem::take(&mut *gateway_runtime().lock().await);
        let update = tokio::spawn(sync_running_api_service_speed(CodexAppSpeed::Standard));
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(!update.is_finished(), "must serialize with startup");
        let (port, server) =
            speed_control_server(200, json!({"speed":"standard"}), Duration::ZERO).await;
        {
            let mut runtime = gateway_runtime().lock().await;
            runtime.running = true;
            runtime.actual_port = Some(port);
            runtime.sidecar_generation = Some(current_gateway_lifecycle_generation());
        }
        drop(lifecycle);
        update.await.unwrap().unwrap();
        assert_eq!(
            speed_request_body(&server.await.unwrap())["speed"],
            "standard"
        );
        *gateway_runtime().lock().await = previous;
    }

    #[tokio::test]
    async fn service_tier_control_accepts_existing_process_after_lifecycle_generation_advanced() {
        let _env_lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let previous = std::mem::take(&mut *gateway_runtime().lock().await);
        let process_generation = current_gateway_lifecycle_generation();
        // Enabling an already running service advances the lifecycle counter
        // without replacing its process. These are different identities.
        advance_gateway_lifecycle_generation();
        for speed in [
            CodexAppSpeed::Standard,
            CodexAppSpeed::Fast,
            CodexAppSpeed::Standard,
        ] {
            let (port, server) =
                speed_control_server(200, json!({"speed":speed}), Duration::ZERO).await;
            {
                let mut runtime = gateway_runtime().lock().await;
                runtime.running = true;
                runtime.actual_port = Some(port);
                runtime.sidecar_generation = Some(process_generation);
            }
            let result = sync_running_api_service_speed(speed.clone()).await;
            let request = server.await.unwrap();
            assert_eq!(speed_request_body(&request)["speed"], json!(speed));
            if let Err(error) = result {
                *gateway_runtime().lock().await = previous;
                panic!("acknowledged update to unchanged existing process must succeed: {error}");
            }
        }
        assert_eq!(
            gateway_runtime().lock().await.sidecar_generation,
            Some(process_generation)
        );
        *gateway_runtime().lock().await = previous;
    }

    #[tokio::test]
    async fn service_tier_control_rejects_lifecycle_or_process_change_during_acknowledgement() {
        let _env_lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let previous = std::mem::take(&mut *gateway_runtime().lock().await);
        for lifecycle_change in [true, false] {
            let (port, server, request_received) = speed_control_server_with_signal(
                200,
                json!({"speed":"fast"}),
                Duration::from_millis(50),
            )
            .await;
            {
                let mut runtime = gateway_runtime().lock().await;
                runtime.running = true;
                runtime.actual_port = Some(port);
                runtime.sidecar_generation = Some(current_gateway_lifecycle_generation());
            }
            let update = tokio::spawn(sync_running_api_service_speed(CodexAppSpeed::Fast));
            timeout(Duration::from_secs(1), request_received)
                .await
                .unwrap()
                .unwrap();
            if lifecycle_change {
                // Stop requests invalidate the operation before obtaining the
                // lifecycle lock; successful HTTP alone must not hide them.
                advance_gateway_lifecycle_generation();
            } else {
                // A replacement on the same port must also be rejected.
                gateway_runtime().lock().await.sidecar_generation = None;
            }
            let result = update.await.unwrap();
            server.await.unwrap();
            assert_eq!(result, Err(API_SERVICE_SPEED_SYNC_FAILED.to_string()));
        }
        *gateway_runtime().lock().await = previous;
    }

    #[tokio::test]
    async fn service_tier_control_failure_preserves_preference_for_idempotent_retry() {
        let _env_lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let _env = super::LocalAccessTestDataGuard::new("service-tier-control-failure");
        let (port, server) = speed_control_server(503, json!({}), Duration::ZERO).await;
        let previous = {
            let mut runtime = gateway_runtime().lock().await;
            let previous = std::mem::take(&mut *runtime);
            runtime.running = true;
            runtime.actual_port = Some(port);
            previous
        };
        crate::modules::codex_speed::save_api_service_app_speed(CodexAppSpeed::Fast).unwrap();
        assert_eq!(
            sync_running_api_service_speed(CodexAppSpeed::Fast).await,
            Err(API_SERVICE_SPEED_SYNC_FAILED.to_string())
        );
        let failed_request = speed_request_body(&server.await.unwrap());
        assert_eq!(
            crate::modules::codex_speed::get_api_service_app_speed_config()
                .unwrap()
                .speed,
            CodexAppSpeed::Fast
        );
        let (retry_port, retry_server) =
            speed_control_server(200, json!({"speed":"fast"}), Duration::ZERO).await;
        gateway_runtime().lock().await.actual_port = Some(retry_port);
        sync_running_api_service_speed(CodexAppSpeed::Fast)
            .await
            .unwrap();
        let retry_request = speed_request_body(&retry_server.await.unwrap());
        assert!(
            retry_request["revision"].as_u64().unwrap()
                > failed_request["revision"].as_u64().unwrap()
        );
        *gateway_runtime().lock().await = previous;
    }

    #[tokio::test]
    async fn service_tier_control_stopped_service_uses_saved_speed_for_next_launch() {
        let _env_lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let env = super::LocalAccessTestDataGuard::new("service-tier-control-next-launch");
        let previous = std::mem::take(&mut *gateway_runtime().lock().await);
        let collection = super::test_local_access_collection(Vec::new());
        for speed in [CodexAppSpeed::Fast, CodexAppSpeed::Standard] {
            crate::modules::codex_speed::save_api_service_app_speed(speed.clone()).unwrap();
            sync_running_api_service_speed(speed.clone()).await.unwrap();
            let launch = prepare_sidecar_launch_config_in_dir_sync(
                &collection,
                env.data_dir.join("sidecar"),
                HashMap::new(),
                api_service_default_service_tier().unwrap(),
                HashMap::new(),
                true,
                None,
            )
            .unwrap();
            let config: Value =
                serde_json::from_slice(&fs::read(launch.config_path).unwrap()).unwrap();
            if speed == CodexAppSpeed::Fast {
                assert_eq!(
                    config["payload"]["default"][0]["params"]["service_tier"],
                    "priority"
                );
            } else {
                assert!(config.get("payload").is_none());
            }
            let runtime = gateway_runtime().lock().await;
            assert!(!runtime.running);
            assert!(runtime.sidecar_child.is_none());
        }
        *gateway_runtime().lock().await = previous;
    }

    #[tokio::test]
    async fn service_tier_control_lifecycle_contention_is_bounded() {
        let _env_lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let _lifecycle = gateway_lifecycle_lock().lock().await;
        let started = Instant::now();
        let result = timeout(
            Duration::from_secs(4),
            sync_running_api_service_speed(CodexAppSpeed::Fast),
        )
        .await
        .expect("lifecycle wait must be bounded");
        assert_eq!(result, Err(API_SERVICE_SPEED_SYNC_FAILED.to_string()));
        assert!(started.elapsed() >= API_SERVICE_SPEED_SYNC_TIMEOUT);
        assert!(
            gateway_runtime().try_lock().is_ok(),
            "timeout must not occupy the runtime lock"
        );
    }
}
