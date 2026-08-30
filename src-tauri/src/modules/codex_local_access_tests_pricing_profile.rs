// Codex Local Access 测试：Pricing, profile takeover and local configuration behavior。
// 测试与生产实现共享 super 作用域，验证真实网关、持久化和请求协议行为。
    #[test]
    fn sidecar_auth_json_marks_personal_access_token_accounts() {
        let mut account = CodexAccount::new(
            "account-at".to_string(),
            "at@example.com".to_string(),
            CodexTokens {
                id_token: String::new(),
                access_token: "at-cockpit-team-token".to_string(),
                refresh_token: None,
            },
        );
        let collection = test_local_access_collection(vec![account.id.clone()]);

        let auth_json = sidecar_auth_json_for_account(&account, &collection, None);

        assert_eq!(
            auth_json.get("auth_mode").and_then(Value::as_str),
            Some("personal_access_token")
        );
        assert_eq!(
            auth_json.get("openai_auth_mode").and_then(Value::as_str),
            Some("personal_access_token")
        );
        assert_eq!(
            auth_json.get("token_type").and_then(Value::as_str),
            Some("Bearer")
        );
        assert_eq!(
            auth_json
                .get("personal_access_token")
                .and_then(Value::as_str),
            Some("at-cockpit-team-token")
        );
        assert_eq!(
            auth_json.get("at_token").and_then(Value::as_str),
            Some("at-cockpit-team-token")
        );
        assert_eq!(
            auth_json.get("refresh_token").and_then(Value::as_str),
            Some("")
        );
        assert!(
            auth_json.get("account_id").is_none(),
            "Cockpit storage id must not be used as ChatGPT account id"
        );

        account.account_id = Some("workspace-at".to_string());
        let auth_json = sidecar_auth_json_for_account(&account, &collection, None);
        assert_eq!(
            auth_json.get("account_id").and_then(Value::as_str),
            Some("workspace-at")
        );
    }

    #[test]
    fn sidecar_account_manifest_marks_access_token_only_auth() {
        let mut account = CodexAccount::new(
            "account-at".to_string(),
            "at@example.com".to_string(),
            CodexTokens {
                id_token: String::new(),
                access_token: "at-cockpit-team-token".to_string(),
                refresh_token: None,
            },
        );
        account.account_id = Some("chatgpt-account-at".to_string());
        let collection = test_local_access_collection(vec![account.id.clone()]);

        let manifest_value =
            sidecar_account_manifest_value(&account, Some("account-at.json"), &collection);

        assert_eq!(
            manifest_value.get("authKind").and_then(Value::as_str),
            Some("access_token")
        );
        assert_eq!(
            manifest_value
                .get("accessTokenOnly")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            manifest_value
                .get("chatgptAccountId")
                .and_then(Value::as_str),
            Some("chatgpt-account-at")
        );
    }

    #[tokio::test]
    async fn prepare_sidecar_config_prunes_stale_auth_files_incrementally() {
        let dir = make_temp_dir("codex-sidecar-incremental-auth-files");
        let account = CodexAccount::new(
            "oauth-incremental".to_string(),
            "incremental@example.com".to_string(),
            CodexTokens {
                id_token: String::new(),
                access_token: make_test_jwt(json!({
                    "sub": "access-incremental",
                    "exp": 4_102_444_800i64,
                })),
                refresh_token: Some("refresh-incremental".to_string()),
            },
        );
        let collection = test_local_access_collection(vec![account.id.clone()]);
        let overrides = HashMap::from([(account.id.clone(), account.clone())]);

        prepare_sidecar_launch_config_in_dir(
            &collection,
            dir.clone(),
            HashMap::new(),
            None,
            overrides.clone(),
        )
        .await
        .expect("initial sidecar config should build");

        let auths_dir = sidecar_auths_dir(&dir);
        let auth_path = auths_dir.join(sidecar_auth_file_name(&account.id));
        let initial_auth_content = fs::read_to_string(&auth_path).expect("read auth file");
        let stale_path = auths_dir.join("stale.json");
        fs::write(&stale_path, "{}").expect("write stale auth file");

        prepare_sidecar_launch_config_in_dir(
            &collection,
            dir.clone(),
            HashMap::new(),
            None,
            overrides,
        )
        .await
        .expect("second sidecar config should build");

        assert!(!stale_path.exists(), "stale auth file should be removed");
        assert_eq!(
            fs::read_to_string(&auth_path).expect("read retained auth file"),
            initial_auth_content
        );

        fs::remove_dir_all(&dir).expect("cleanup temp dir");
    }

    #[test]
    fn sidecar_auth_scope_includes_collection_bound_oauth_account() {
        let mut collection = test_local_access_collection(Vec::new());
        collection.bound_oauth_account_id = Some(" oauth-bound ".to_string());

        assert!(sidecar_auth_account_is_scoped(&collection, "oauth-bound"));
    }

    #[test]
    fn sidecar_background_refresh_only_selects_expired_refreshable_oauth_accounts() {
        let expired_account = CodexAccount::new(
            "account-expired".to_string(),
            "expired@example.com".to_string(),
            CodexTokens {
                id_token: String::new(),
                access_token: make_test_jwt(json!({
                    "sub": "access-expired",
                    "exp": 1i64,
                })),
                refresh_token: None,
            },
        );
        let expired_refreshable_account = CodexAccount::new(
            "account-expired-refreshable".to_string(),
            "expired-refreshable@example.com".to_string(),
            CodexTokens {
                id_token: String::new(),
                access_token: make_test_jwt(json!({
                    "sub": "access-expired-refreshable",
                    "exp": 1i64,
                })),
                refresh_token: Some("refresh-token".to_string()),
            },
        );
        let valid_account = CodexAccount::new(
            "account-valid".to_string(),
            "valid@example.com".to_string(),
            CodexTokens {
                id_token: String::new(),
                access_token: make_test_jwt(json!({
                    "sub": "access-valid",
                    "exp": 4_102_444_800i64,
                })),
                refresh_token: None,
            },
        );
        let expired_id_refreshable_account = CodexAccount::new(
            "account-expired-id-refreshable".to_string(),
            "expired-id-refreshable@example.com".to_string(),
            CodexTokens {
                id_token: make_test_jwt(json!({
                    "sub": "id-expired-refreshable",
                    "exp": 1i64,
                })),
                access_token: make_test_jwt(json!({
                    "sub": "access-fresh-refreshable",
                    "exp": 4_102_444_800i64,
                })),
                refresh_token: Some("refresh-token".to_string()),
            },
        );

        assert!(!sidecar_account_needs_background_refresh(&expired_account));
        assert!(sidecar_account_needs_background_refresh(
            &expired_refreshable_account
        ));
        assert!(!sidecar_account_needs_background_refresh(&valid_account));
        assert!(!sidecar_account_needs_background_refresh(
            &expired_id_refreshable_account
        ));
        assert!(!sidecar_local_account_usable_for_start(&expired_account));
        assert!(sidecar_local_account_usable_for_start(
            &expired_refreshable_account
        ));
        assert!(sidecar_local_account_usable_for_start(&valid_account));

        let mut reauth_account = expired_refreshable_account;
        reauth_account.requires_reauth = true;
        assert!(!sidecar_local_account_usable_for_start(&reauth_account));

        let mut api_only_reauth_account = valid_account;
        api_only_reauth_account.requires_reauth = true;
        assert!(sidecar_local_account_usable_for_start(
            &api_only_reauth_account
        ));
    }

    fn test_instance(
        id: &str,
        profile_dir: &str,
        bind_account_id: Option<&str>,
    ) -> InstanceProfile {
        InstanceProfile {
            id: id.to_string(),
            name: id.to_string(),
            user_data_dir: profile_dir.to_string(),
            working_dir: None,
            extra_args: String::new(),
            bind_account_id: bind_account_id.map(str::to_string),
            model_routing: None,
            launch_mode: InstanceLaunchMode::App,
            app_speed: CodexAppSpeed::Standard,
            created_at: 0,
            last_launched_at: None,
            last_pid: None,
        }
    }

    fn has_image_generation_tool(body: &Value) -> bool {
        body.get("tools")
            .and_then(Value::as_array)
            .map(|tools| {
                tools.iter().any(|tool| {
                    tool.get("type").and_then(Value::as_str) == Some("image_generation")
                })
            })
            .unwrap_or(false)
    }

    async fn accept_raw_upstream_websocket(listener: TcpListener) -> TcpStream {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            socket.read_exact(&mut byte).await.unwrap();
            request.push(byte[0]);
            if request.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let request_text = String::from_utf8_lossy(&request);
        let sec_key = request_text
            .lines()
            .find_map(|line| {
                line.split_once(':').and_then(|(name, value)| {
                    name.eq_ignore_ascii_case("sec-websocket-key")
                        .then(|| value.trim().to_string())
                })
            })
            .expect("client handshake should include sec-websocket-key");
        let response = format!(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
            websocket_accept_value(&sec_key)
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        socket
    }

    async fn read_raw_client_websocket_frame(socket: &mut TcpStream) -> (u8, Vec<u8>) {
        let mut header = [0u8; 2];
        socket.read_exact(&mut header).await.unwrap();
        let opcode = header[0] & 0x0f;
        let masked = header[1] & 0x80 != 0;
        let len = match header[1] & 0x7f {
            n @ 0..=125 => n as usize,
            126 => {
                let mut ext = [0u8; 2];
                socket.read_exact(&mut ext).await.unwrap();
                u16::from_be_bytes(ext) as usize
            }
            127 => {
                let mut ext = [0u8; 8];
                socket.read_exact(&mut ext).await.unwrap();
                u64::from_be_bytes(ext) as usize
            }
            _ => unreachable!(),
        };
        let mut mask = [0u8; 4];
        if masked {
            socket.read_exact(&mut mask).await.unwrap();
        }
        let mut payload = vec![0u8; len];
        if len > 0 {
            socket.read_exact(&mut payload).await.unwrap();
        }
        if masked {
            for (index, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask[index % 4];
            }
        }
        (opcode, payload)
    }

    #[tokio::test]
    async fn bridge_flushes_upstream_pong_when_downstream_is_silent() {
        let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream_listener.local_addr().unwrap();
        let (pong_tx, pong_rx) = oneshot::channel();
        let upstream_server = tokio::spawn(async move {
            let (socket, _) = upstream_listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();

            let first = tokio::time::timeout(Duration::from_secs(1), ws.next())
                .await
                .expect("mock upstream should receive the initial payload")
                .expect("mock upstream stream should stay open")
                .expect("mock upstream should read a valid message");
            assert!(matches!(first, Message::Text(_)));

            ws.send(Message::Ping(b"probe".to_vec().into()))
                .await
                .unwrap();
            let pong_result = tokio::time::timeout(Duration::from_millis(250), async {
                loop {
                    let message = ws
                        .next()
                        .await
                        .expect("mock upstream stream should stay open")
                        .expect("mock upstream should read a valid message");
                    if let Message::Pong(payload) = message {
                        return payload;
                    }
                }
            })
            .await;
            let _ = pong_tx.send(pong_result);

            let _ = tokio::time::timeout(Duration::from_secs(1), async {
                while let Some(message) = ws.next().await {
                    if matches!(message, Ok(Message::Close(_)) | Err(_)) {
                        break;
                    }
                }
            })
            .await;
        });

        let upstream_socket = TcpStream::connect(upstream_addr).await.unwrap();
        let upstream_request = format!("ws://{upstream_addr}/responses")
            .into_client_request()
            .unwrap();
        let (downstream_listener, downstream_accept) = {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let accept = tokio::spawn(async move {
                let (socket, _) = listener.accept().await.unwrap();
                tokio_tungstenite::accept_async(socket).await.unwrap()
            });
            (addr, accept)
        };
        let (client_ws, _) =
            tokio_tungstenite::connect_async(format!("ws://{downstream_listener}/responses"))
                .await
                .unwrap();
        let (mut downstream_write, downstream_read) = client_ws.split();
        drop(downstream_read);
        let downstream = downstream_accept.await.unwrap();
        let (upstream, _) = tokio_tungstenite::client_async_tls_with_config(
            upstream_request,
            upstream_socket,
            None,
            None,
        )
        .await
        .unwrap();

        let bridge_task = tokio::spawn(bridge_websocket_streams(
            downstream,
            upstream,
            br#"{"type":"response.create","payload":{}}"#.to_vec(),
            CodexLocalAccessTimeouts::default(),
            None,
        ));
        let pong_result = pong_rx.await.unwrap();

        let _ = downstream_write.send(Message::Close(None)).await;
        tokio::time::timeout(Duration::from_secs(1), bridge_task)
            .await
            .expect("bridge should exit after downstream close")
            .expect("bridge task should not panic")
            .expect("bridge cleanup should succeed");
        upstream_server.await.unwrap();

        let payload = pong_result.expect("bridge should flush Pong back to the mock upstream");
        assert_eq!(payload.as_ref(), b"probe");
    }

    #[tokio::test]
    async fn bridge_sends_heartbeat_when_both_peers_are_quiet() {
        let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream_listener.local_addr().unwrap();
        let upstream_socket = TcpStream::connect(upstream_addr).await.unwrap();
        let upstream_request = format!("ws://{upstream_addr}/responses")
            .into_client_request()
            .unwrap();
        let (mut raw_upstream, upstream_result) = tokio::join!(
            accept_raw_upstream_websocket(upstream_listener),
            tokio_tungstenite::client_async_tls_with_config(
                upstream_request,
                upstream_socket,
                None,
                None,
            ),
        );
        let (upstream, _) = upstream_result.unwrap();

        let downstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let downstream_addr = downstream_listener.local_addr().unwrap();
        let downstream_accept = tokio::spawn(async move {
            let (socket, _) = downstream_listener.accept().await.unwrap();
            tokio_tungstenite::accept_async(socket).await.unwrap()
        });
        let (client_ws, _) =
            tokio_tungstenite::connect_async(format!("ws://{downstream_addr}/responses"))
                .await
                .unwrap();
        let (mut downstream_write, downstream_read) = client_ws.split();
        drop(downstream_read);
        let downstream = downstream_accept.await.unwrap();

        let bridge_task = tokio::spawn(bridge_websocket_streams(
            downstream,
            upstream,
            br#"{"type":"response.create","payload":{}}"#.to_vec(),
            CodexLocalAccessTimeouts::default(),
            None,
        ));
        let (first_opcode, first_payload) =
            read_raw_client_websocket_frame(&mut raw_upstream).await;
        assert_eq!(first_opcode, 0x1);
        assert_eq!(first_payload, br#"{"type":"response.create","payload":{}}"#);
        tokio::task::yield_now().await;
        assert!(
            !bridge_task.is_finished(),
            "bridge exited before the quiet heartbeat window"
        );
        let (heartbeat_opcode, heartbeat_payload) = tokio::time::timeout(
            Duration::from_secs(1),
            read_raw_client_websocket_frame(&mut raw_upstream),
        )
        .await
        .expect("bridge should send heartbeat Ping while both peers are quiet");
        assert_eq!(heartbeat_opcode, 0x9);
        assert!(heartbeat_payload.is_empty());

        let _ = downstream_write.send(Message::Close(None)).await;
        tokio::time::timeout(Duration::from_secs(1), bridge_task)
            .await
            .expect("bridge should exit after downstream close")
            .expect("bridge task should not panic")
            .expect("bridge cleanup should succeed");
    }

    #[test]
    fn takeover_dirs_skip_default_profile_when_default_not_bound_to_api_service() {
        let mut store = InstanceStore::new();
        store.default_settings = DefaultInstanceSettings {
            bind_account_id: Some("regular-oauth".to_string()),
            ..DefaultInstanceSettings::default()
        };
        store.instances = vec![
            test_instance("regular", "/tmp/codex-regular", Some("regular-oauth")),
            test_instance(
                "api-service",
                "/tmp/codex-api-service",
                Some(crate::modules::codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID),
            ),
        ];

        let dirs = collect_local_access_profile_takeover_dirs_from_store(
            store,
            PathBuf::from("/tmp/default-codex"),
            true,
        );

        assert_eq!(dirs, vec![PathBuf::from("/tmp/codex-api-service")]);
    }

    #[test]
    fn takeover_dirs_include_default_profile_only_when_bound_to_api_service() {
        let mut store = InstanceStore::new();
        store.default_settings = DefaultInstanceSettings {
            bind_account_id: Some(
                crate::modules::codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID.to_string(),
            ),
            ..DefaultInstanceSettings::default()
        };
        store.instances = vec![test_instance(
            "api-service",
            "/tmp/default-codex",
            Some(crate::modules::codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID),
        )];

        let dirs = collect_local_access_profile_takeover_dirs_from_store(
            store,
            PathBuf::from("/tmp/default-codex"),
            true,
        );

        assert_eq!(dirs, vec![PathBuf::from("/tmp/default-codex")]);
    }

    #[test]
    fn takeover_dirs_skip_default_profile_when_default_takeover_is_disabled() {
        let mut store = InstanceStore::new();
        store.default_settings = DefaultInstanceSettings {
            bind_account_id: Some(
                crate::modules::codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID.to_string(),
            ),
            ..DefaultInstanceSettings::default()
        };
        store.instances = vec![test_instance(
            "api-service",
            "/tmp/codex-api-service",
            Some(crate::modules::codex_instance::CODEX_API_SERVICE_BIND_ACCOUNT_ID),
        )];

        let dirs = collect_local_access_profile_takeover_dirs_from_store(
            store,
            PathBuf::from("/tmp/default-codex"),
            false,
        );

        assert_eq!(dirs, vec![PathBuf::from("/tmp/codex-api-service")]);
    }

    #[test]
    fn oauth_runtime_prevents_automatic_default_profile_takeover() {
        assert!(!super::should_include_default_profile_for_takeover(
            false, true
        ));
        assert!(!super::should_include_default_profile_for_takeover(
            true, true
        ));
        assert!(!super::should_include_default_profile_for_takeover(
            true, false
        ));
        assert!(super::should_include_default_profile_for_takeover(
            false, false
        ));
    }

    #[test]
    fn calculates_usage_cost_with_cached_input_price() {
        let usage = UsageCapture {
            input_tokens: 1_000,
            output_tokens: 2_000,
            total_tokens: 3_000,
            cached_tokens: 400,
            reasoning_tokens: 0,
            token_breakdown: None,
        };
        let pricing = model_pricing(
            "gpt-5.4",
            None,
            codex_price(1.25, 0.125, 10.0),
            None,
            None,
            None,
        );
        let cost = calculate_usage_cost_usd(Some(&usage), Some(&pricing));
        let expected = ((600.0 * 1.25) + (400.0 * 0.125) + (2_000.0 * 10.0)) / 1_000_000.0;
        assert!((cost - expected).abs() < 0.000000001);
    }

    #[test]
    fn request_log_db_preserves_api_key_label_error_and_pricing_version() {
        let dir = make_temp_dir("codex-local-access-logs");
        let db_path = dir.join("request_logs.sqlite");
        let conn = open_local_access_logs_db_once(&db_path, true).expect("open logs db");
        let long_error = format!(
            "upstream failed: {} tail-marker",
            "provider diagnostic ".repeat(128)
        );
        let mut events = Vec::new();
        let event = append_usage_event(
            &mut events,
            1_700_000_000_000,
            Some("req-long-error"),
            Some("acc-1"),
            Some("user@example.com"),
            Some("key-1"),
            Some("Production Key"),
            None,
            Some("gpt-5.4"),
            Some(CodexLocalAccessGatewayMode::Sidecar),
            CodexLocalAccessRequestKind::Text,
            None,
            None,
            false,
            Some(502),
            Some("upstream_bad_gateway"),
            Some(&long_error),
            42,
            None,
            Some(&model_pricing(
                "gpt-5.4",
                None,
                codex_price(1.0, 1.0, 2.0),
                None,
                None,
                None,
            )),
            7,
            0.0,
        );

        insert_local_access_usage_event(&conn, &event).expect("insert request log");
        let loaded = conn
            .query_row(
                "SELECT api_key_label, error_message, model_pricing_version FROM request_logs WHERE request_id = ?1",
                ["req-long-error"],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )
            .expect("read request log");

        assert_eq!(loaded.0, "Production Key");
        assert_eq!(loaded.1, long_error);
        assert!(loaded.1.contains("tail-marker"));
        assert_eq!(loaded.2, 7);

        drop(conn);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn request_log_db_persists_canonical_token_breakdown() {
        let dir = make_temp_dir("codex-local-access-token-breakdown");
        let db_path = dir.join("request_logs.sqlite");
        let conn = open_local_access_logs_db_once(&db_path, true).expect("open logs db");
        let mut breakdown = CodexTokenBreakdown::default();
        breakdown.schema_version = 2;
        breakdown.quality = "complete".to_string();
        breakdown.total_tokens = 1_200;
        breakdown.input.total_tokens = 1_000;
        breakdown.input.uncached_tokens = 600;
        breakdown.input.cache_read_tokens = 300;
        breakdown.input.cache_write_tokens = 100;
        breakdown.output.total_tokens = 200;
        breakdown.output.non_reasoning_tokens = 150;
        breakdown.output.reasoning_tokens = 50;
        let usage = UsageCapture {
            input_tokens: 1_000,
            output_tokens: 200,
            total_tokens: 1_200,
            cached_tokens: 300,
            reasoning_tokens: 50,
            token_breakdown: Some(breakdown.clone()),
        };
        let mut events = Vec::new();
        let event = append_usage_event(
            &mut events,
            1_700_000_000_000,
            Some("req-token-breakdown"),
            Some("acc-1"),
            Some("user@example.com"),
            Some("key-1"),
            Some("Production Key"),
            None,
            Some("gpt-5.4"),
            Some(CodexLocalAccessGatewayMode::Sidecar),
            CodexLocalAccessRequestKind::Text,
            None,
            None,
            true,
            Some(200),
            None,
            None,
            42,
            Some(&usage),
            None,
            2,
            0.0,
        );
        insert_local_access_usage_event(&conn, &event).expect("insert request log");

        let loaded = conn
            .query_row(
                "SELECT * FROM request_logs WHERE request_id = ?1",
                ["req-token-breakdown"],
                usage_event_from_row,
            )
            .expect("read request log");
        let loaded_breakdown = loaded.token_breakdown.expect("token breakdown");
        assert_eq!(loaded_breakdown.schema_version, breakdown.schema_version);
        assert_eq!(loaded_breakdown.quality, breakdown.quality);
        assert_eq!(loaded_breakdown.total_tokens, breakdown.total_tokens);
        assert_eq!(
            loaded_breakdown.input.cache_read_tokens,
            breakdown.input.cache_read_tokens
        );
        assert_eq!(
            loaded_breakdown.input.cache_write_tokens,
            breakdown.input.cache_write_tokens
        );
        assert_eq!(
            loaded_breakdown.output.reasoning_tokens,
            breakdown.output.reasoning_tokens
        );

        drop(conn);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn request_log_db_adds_token_breakdown_to_existing_schema() {
        let dir = make_temp_dir("codex-local-access-token-breakdown-migration");
        let db_path = dir.join("request_logs.sqlite");
        let conn = Connection::open(&db_path).expect("open legacy logs db");
        conn.execute_batch(
            "CREATE TABLE request_logs (id INTEGER PRIMARY KEY AUTOINCREMENT, event_key TEXT NOT NULL DEFAULT '', timestamp INTEGER NOT NULL DEFAULT 0)",
        )
        .expect("create legacy request logs table");
        drop(conn);

        let conn = open_local_access_logs_db_once(&db_path, true).expect("migrate logs db");
        assert!(request_logs_has_column(&conn, "token_breakdown_json")
            .expect("inspect token breakdown column"));

        drop(conn);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn request_log_db_adds_service_tier_to_existing_schema() {
        let dir = make_temp_dir("codex-local-access-service-tier-migration");
        let db_path = dir.join("request_logs.sqlite");
        let conn = open_local_access_logs_db_once(&db_path, false).expect("open legacy logs db");
        conn.execute(
            "INSERT INTO request_logs (event_key, timestamp, request_id) VALUES (?1, ?2, ?3)",
            rusqlite::params!["legacy-event", 1_700_000_000_000_i64, "legacy-request"],
        )
        .expect("insert legacy request log");
        let legacy_column_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('request_logs') WHERE name = 'service_tier'",
                [],
                |row| row.get(0),
            )
            .expect("inspect legacy schema");
        assert_eq!(legacy_column_count, 0);
        drop(conn);

        let conn = open_local_access_logs_db_once(&db_path, true).expect("migrate logs db");
        let migrated: (String, String) = conn
            .query_row(
                "SELECT request_id, service_tier FROM request_logs WHERE event_key = ?1",
                ["legacy-event"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read migrated request log");
        assert_eq!(migrated, ("legacy-request".to_string(), String::new()));

        drop(conn);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn request_log_reprice_updates_cost_and_pricing_version() {
        let dir = make_temp_dir("codex-local-access-reprice");
        let db_path = dir.join("request_logs.sqlite");
        let mut conn = open_local_access_logs_db_once(&db_path, true).expect("open logs db");
        let mut events = Vec::new();
        let mut breakdown = CodexTokenBreakdown::default();
        breakdown.schema_version = 2;
        breakdown.quality = "complete".to_string();
        breakdown.total_tokens = 1_500_000;
        breakdown.input.total_tokens = 1_000_000;
        breakdown.input.uncached_tokens = 700_000;
        breakdown.input.cache_read_tokens = 200_000;
        breakdown.input.cache_write_tokens = 100_000;
        breakdown.output.total_tokens = 500_000;
        breakdown.output.non_reasoning_tokens = 500_000;
        let usage = UsageCapture {
            input_tokens: 1_000_000,
            output_tokens: 500_000,
            total_tokens: 1_500_000,
            cached_tokens: 400_000,
            reasoning_tokens: 0,
            token_breakdown: Some(breakdown),
        };
        let event = append_usage_event(
            &mut events,
            1_700_000_000_000,
            Some("req-reprice"),
            Some("acc-1"),
            Some("user@example.com"),
            Some("key-1"),
            Some("Production Key"),
            None,
            Some("custom-model"),
            Some(CodexLocalAccessGatewayMode::Sidecar),
            CodexLocalAccessRequestKind::Text,
            None,
            None,
            true,
            Some(200),
            None,
            None,
            42,
            Some(&usage),
            Some(&model_pricing(
                "custom-model",
                None,
                codex_price(1.0, 0.5, 2.0),
                None,
                None,
                None,
            )),
            2,
            1.0,
        );
        insert_local_access_usage_event(&conn, &event).expect("insert request log");

        let mut collection = test_local_access_collection(vec!["acc-1".to_string()]);
        collection.model_pricing_version = 8;
        collection.model_pricings = vec![model_pricing(
            "custom-model",
            None,
            codex_price(3.0, 0.25, 9.0),
            None,
            None,
            None,
        )];
        let updated =
            reprice_request_logs_for_collection(&mut conn, &collection).expect("reprice logs");
        assert_eq!(updated, 1);

        let loaded = conn
            .query_row(
                r#"
                SELECT
                    estimated_cost_usd,
                    model_pricing_version,
                    input_usd_per_million,
                    output_usd_per_million,
                    cached_input_usd_per_million
                FROM request_logs
                WHERE request_id = ?1
                "#,
                ["req-reprice"],
                |row| {
                    Ok((
                        row.get::<_, f64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, f64>(2)?,
                        row.get::<_, f64>(3)?,
                        row.get::<_, Option<f64>>(4)?,
                    ))
                },
            )
            .expect("read repriced request log");

        assert!((loaded.0 - 6.95).abs() < 0.000001);
        assert_eq!(loaded.1, 8);
        assert_eq!(loaded.2, 3.0);
        assert_eq!(loaded.3, 9.0);
        assert_eq!(loaded.4, Some(0.25));

        drop(conn);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn background_reprice_reads_only_stale_pricing_versions() {
        let dir = make_temp_dir("codex-local-access-background-reprice");
        let db_path = dir.join("request_logs.sqlite");
        let conn = open_local_access_logs_db_once(&db_path, true).expect("open logs db");
        let mut events = Vec::new();
        let usage = UsageCapture {
            input_tokens: 1_000,
            output_tokens: 500,
            total_tokens: 1_500,
            cached_tokens: 200,
            reasoning_tokens: 0,
            token_breakdown: None,
        };
        for (request_id, timestamp, pricing_version) in [
            ("req-stale", 1_700_000_000_000, 7),
            ("req-current", 1_700_000_000_001, 8),
        ] {
            let event = append_usage_event(
                &mut events,
                timestamp,
                Some(request_id),
                Some("acc-1"),
                Some("user@example.com"),
                Some("key-1"),
                Some("Production Key"),
                None,
                Some("gpt-5.4"),
                Some(CodexLocalAccessGatewayMode::Sidecar),
                CodexLocalAccessRequestKind::Text,
                None,
                None,
                true,
                Some(200),
                None,
                None,
                42,
                Some(&usage),
                Some(&model_pricing(
                    "gpt-5.4",
                    None,
                    codex_price(1.0, 0.5, 2.0),
                    None,
                    None,
                    None,
                )),
                pricing_version,
                1.0,
            );
            insert_local_access_usage_event(&conn, &event).expect("insert request log");
        }

        let model_ids = vec!["gpt-5.4".to_string()];
        assert_eq!(
            count_request_logs_for_model_ids(&conn, Some(&model_ids), Some(8))
                .expect("count stale rows"),
            1
        );
        let mut cursors = HashMap::from([("gpt-5.4".to_string(), 0_i64)]);
        let rows = read_request_log_reprice_batch(&conn, &mut cursors, 10, Some(8))
            .expect("read stale rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].previous_model_pricing_version, 7);

        drop(conn);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn resolves_default_model_pricing_by_tier_and_context_band() {
        let short_usage = UsageCapture {
            input_tokens: 272_000,
            output_tokens: 1,
            total_tokens: 272_001,
            cached_tokens: 0,
            reasoning_tokens: 0,
            token_breakdown: None,
        };
        let long_usage = UsageCapture {
            input_tokens: 272_001,
            ..short_usage.clone()
        };

        let short =
            resolve_effective_model_pricing(None, Some("gpt-5.5"), Some(&short_usage), None)
                .expect("gpt-5.5 standard short pricing");
        assert_eq!(short.input_usd_per_million, 5.0);
        assert_eq!(short.output_usd_per_million, 30.0);
        assert_eq!(short.cached_input_usd_per_million, Some(0.5));

        // session long-context (above-272k): input x2, cache x2, output x1.5
        let long = resolve_effective_model_pricing(None, Some("gpt-5.5"), Some(&long_usage), None)
            .expect("gpt-5.5 standard long pricing");
        assert_eq!(long.input_usd_per_million, 10.0);
        assert_eq!(long.output_usd_per_million, 45.0);
        assert_eq!(long.cached_input_usd_per_million, Some(1.0));

        // priority absolute rates then long multipliers (10*2 / 1*2 / 60*1.5)
        let priority = resolve_effective_model_pricing(
            None,
            Some("gpt-5.5"),
            Some(&long_usage),
            Some("priority"),
        )
        .expect("gpt-5.5 priority long pricing");
        assert_eq!(priority.input_usd_per_million, 20.0);
        assert_eq!(priority.output_usd_per_million, 90.0);
        assert_eq!(priority.cached_input_usd_per_million, Some(2.0));

        // non-5.4/5.5 models do not apply long-context multipliers
        let mini_long =
            resolve_effective_model_pricing(None, Some("gpt-5.4-mini"), Some(&long_usage), None)
                .expect("gpt-5.4-mini standard long pricing");
        assert_eq!(mini_long.input_usd_per_million, 0.75);
        assert_eq!(mini_long.output_usd_per_million, 4.5);

        // Available 5.6 models keep default book prices.
        let sol =
            resolve_effective_model_pricing(None, Some("gpt-5.6-sol"), Some(&short_usage), None)
                .expect("gpt-5.6-sol pricing");
        assert_eq!(sol.input_usd_per_million, 5.0);
        assert_eq!(sol.output_usd_per_million, 30.0);
        let terra =
            resolve_effective_model_pricing(None, Some("gpt-5.6-terra"), Some(&short_usage), None)
                .expect("gpt-5.6-terra pricing");
        assert_eq!(terra.input_usd_per_million, 2.0);
        assert_eq!(terra.output_usd_per_million, 12.0);
        assert_eq!(terra.cached_input_usd_per_million, Some(0.2));
        let luna =
            resolve_effective_model_pricing(None, Some("gpt-5.6-luna"), Some(&short_usage), None)
                .expect("gpt-5.6-luna pricing");
        assert_eq!(luna.input_usd_per_million, 0.2);
        assert_eq!(luna.output_usd_per_million, 1.2);
        assert_eq!(luna.cached_input_usd_per_million, Some(0.02));
    }

    #[test]
    fn drops_legacy_default_56_overrides_but_keeps_custom_rates() {
        let kept = super::drop_superseded_default_56_model_pricings(vec![
            model_pricing(
                "gpt-5.6-terra",
                Some(272_000),
                codex_price(2.5, 0.25, 15.0),
                None,
                Some(codex_price(5.0, 0.5, 30.0)),
                None,
            ),
            model_pricing(
                "gpt-5.6-luna",
                Some(272_000),
                codex_price(1.0, 0.1, 6.0),
                None,
                Some(codex_price(2.0, 0.2, 12.0)),
                None,
            ),
            model_pricing(
                "gpt-5.6-luna",
                Some(272_000),
                codex_price(9.9, 0.9, 19.0),
                None,
                None,
                None,
            ),
            model_pricing(
                "gpt-5.5",
                Some(272_000),
                codex_price(5.0, 0.5, 30.0),
                None,
                Some(codex_price(10.0, 1.0, 60.0)),
                None,
            ),
        ]);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].model_id, "gpt-5.6-luna");
        assert_eq!(kept[0].input_usd_per_million, 9.9);
        assert_eq!(kept[1].model_id, "gpt-5.5");

        let mut collection = test_local_access_collection(vec!["account-a".to_string()]);
        collection.model_pricings = vec![model_pricing(
            "gpt-5.6-luna",
            Some(272_000),
            codex_price(1.0, 0.1, 6.0),
            None,
            Some(codex_price(2.0, 0.2, 12.0)),
            None,
        )];
        let usage = UsageCapture {
            input_tokens: 100,
            output_tokens: 10,
            total_tokens: 110,
            cached_tokens: 0,
            reasoning_tokens: 0,
            token_breakdown: None,
        };
        collection.model_pricings =
            super::drop_superseded_default_56_model_pricings(collection.model_pricings);
        let luna = resolve_effective_model_pricing(
            Some(&collection),
            Some("gpt-5.6-luna"),
            Some(&usage),
            None,
        )
        .expect("legacy snapshot should fall back to new book");
        assert_eq!(luna.input_usd_per_million, 0.2);
        assert_eq!(luna.output_usd_per_million, 1.2);
    }

    #[test]
    fn resolves_pricing_service_tier_flex_and_priority_fallback() {
        let usage = UsageCapture {
            input_tokens: 100,
            output_tokens: 50,
            total_tokens: 150,
            cached_tokens: 20,
            reasoning_tokens: 0,
            token_breakdown: None,
        };

        let flex =
            resolve_effective_model_pricing(None, Some("gpt-5.4"), Some(&usage), Some("flex"))
                .expect("flex pricing");
        assert!((flex.input_usd_per_million - 1.25).abs() < 1e-9);
        assert!((flex.output_usd_per_million - 7.5).abs() < 1e-9);
        assert_eq!(flex.cached_input_usd_per_million, Some(0.125));

        let nano_priority = resolve_effective_model_pricing(
            None,
            Some("gpt-5.4-nano"),
            Some(&usage),
            Some("priority"),
        )
        .expect("nano priority falls back to x2");
        assert!((nano_priority.input_usd_per_million - 0.4).abs() < 1e-9);
        assert!((nano_priority.output_usd_per_million - 2.5).abs() < 1e-9);

        let aliased = resolve_effective_model_pricing(
            None,
            Some("openai/gpt-5.4-2026-03-05"),
            Some(&usage),
            None,
        )
        .expect("date suffix alias");
        assert_eq!(aliased.input_usd_per_million, 2.5);
        assert_eq!(aliased.output_usd_per_million, 15.0);
    }

    #[test]
    fn calculates_usage_cost_for_long_context_session() {
        // gpt-5.4 long-context session multipliers
        let usage = UsageCapture {
            input_tokens: 300_000,
            output_tokens: 4_000,
            total_tokens: 304_000,
            cached_tokens: 0,
            reasoning_tokens: 0,
            token_breakdown: None,
        };
        let pricing = resolve_effective_model_pricing(None, Some("gpt-5.4"), Some(&usage), None)
            .expect("pricing");
        let cost = calculate_usage_cost_usd(Some(&usage), Some(&pricing));
        let expected_input = 300_000.0 * 2.5 * 2.0 / 1_000_000.0;
        let expected_output = 4_000.0 * 15.0 * 1.5 / 1_000_000.0;
        assert!((cost - (expected_input + expected_output)).abs() < 1e-10);
    }

    #[test]
    fn removes_only_codex_local_access_provider_config() {
        let input = r#"model_provider = "codex_local_access"
model_catalog_json = "cockpit-local-access-model-catalog.json"
model_context_window = 1000000

[model_providers.codex_local_access]
name = "Codex API Service"
base_url = "http://127.0.0.1:57391/v1"
wire_api = "responses"
requires_openai_auth = false
experimental_bearer_token = "agt_codex_test"
supports_websockets = false
custom_user_option = "keep-me"
http_headers = { "x-openai-actor-authorization" = "cockpit-tools", "x-agtools-disable-image-generation" = "chat", "x-cockpit-instance-id" = "default", "X-Custom" = "keep-me" }

[model_providers.manual]
name = "Manual"
base_url = "https://manual.example.com/v1"
wire_api = "responses"
"#;

        let output = remove_codex_local_access_config(input).expect("cleanup config");
        let parsed = output
            .parse::<toml_edit::Document>()
            .expect("parse cleaned toml");

        assert!(parsed.get("model_provider").is_none());
        assert!(parsed.get("model_catalog_json").is_none());
        assert_eq!(
            parsed
                .get("model_context_window")
                .and_then(|item| item.as_integer()),
            Some(1_000_000)
        );
        let providers = parsed
            .get("model_providers")
            .and_then(|item| item.as_table())
            .expect("model_providers should remain");
        let local_provider = providers
            .get("codex_local_access")
            .and_then(|item| item.as_table())
            .expect("unknown user fields should keep the provider table");
        assert_eq!(
            local_provider
                .get("custom_user_option")
                .and_then(|item| item.as_str()),
            Some("keep-me")
        );
        let headers = local_provider
            .get("http_headers")
            .and_then(|item| item.as_inline_table())
            .expect("custom header should remain");
        assert_eq!(
            headers.get("X-Custom").and_then(|value| value.as_str()),
            Some("keep-me")
        );
        assert_eq!(headers.len(), 1);
        assert!(providers.get("manual").is_some());
    }

    #[test]
    fn local_access_config_detection_requires_matching_api_key() {
        let input = r#"model_provider = "codex_local_access"

[model_providers.codex_local_access]
name = "Custom API Provider"
base_url = "https://custom.example.com/v1"
wire_api = "responses"
requires_openai_auth = true
experimental_bearer_token = "sk-user-custom"
"#;

        assert!(is_codex_local_access_config_for_api_key(
            input,
            "sk-user-custom"
        ));
        assert!(!is_codex_local_access_config_for_api_key(
            input,
            "local-api-key"
        ));
        assert!(!is_cockpit_managed_local_access_config(input));
        assert!(is_cockpit_managed_local_access_config(
            &input.replace("sk-user-custom", "agt_codex_managed")
        ));
    }

    #[test]
    fn takeover_cleanup_keeps_non_matching_codex_local_access_provider() {
        let dir = make_temp_dir("codex-local-access-custom-provider");
        let config_path = dir.join(CODEX_PROFILE_CONFIG_FILE);
        let auth_path = dir.join(CODEX_PROFILE_AUTH_FILE);
        let config = r#"model_provider = "codex_local_access"

[model_providers.codex_local_access]
name = "Custom API Provider"
base_url = "https://custom.example.com/v1"
wire_api = "responses"
requires_openai_auth = true
experimental_bearer_token = "agt_codex_provider_gateway"
"#;

        fs::write(&config_path, config).expect("write config");
        fs::write(
            &auth_path,
            r#"{"auth_mode":"apikey","OPENAI_API_KEY":"agt_codex_provider_gateway"}"#,
        )
        .expect("write auth");

        let changed =
            cleanup_profile_takeover_without_backup(&dir, "local-api-key", false).expect("cleanup");
        let next_config = fs::read_to_string(&config_path).expect("read config");
        let next_auth = fs::read_to_string(&auth_path).expect("read auth");

        assert!(!changed);
        assert_eq!(next_config, config);
        assert!(next_auth.contains("agt_codex_provider_gateway"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn takeover_backup_restore_preserves_current_plugin_config() {
        let current = r#"model_provider = "codex_local_access"
model = "gpt-user-current"
model_catalog_json = "cockpit-local-access-model-catalog.json"

[plugins."browser@openai-bundled"]
enabled = true

[plugins."chrome@openai-bundled"]
enabled = true

[plugins."hyperframes@openai-curated"]
enabled = true

[model_providers.codex_local_access]
name = "Codex API Service"
base_url = "http://localhost:14998/v1"
wire_api = "responses"
requires_openai_auth = true
experimental_bearer_token = "agt_codex_test"
custom_user_option = "keep-current"
http_headers = { "x-openai-actor-authorization" = "cockpit-tools", "X-Custom" = "keep-current" }
"#;
        let backup = r#"model = "gpt-5"
model_provider = "manual"
model_catalog_json = "official-catalog.json"

[plugins."browser@openai-bundled"]
enabled = true

[model_providers.manual]
name = "Manual"
base_url = "https://manual.example.com/v1"
wire_api = "responses"
"#;

        let output = restore_config_toml_from_takeover_backup(Some(current), Some(backup))
            .expect("restore config")
            .expect("restored content");
        let parsed = output
            .parse::<toml_edit::Document>()
            .expect("parse restored toml");
        let plugins = parsed
            .get("plugins")
            .and_then(|item| item.as_table())
            .expect("plugins should remain");

        assert_eq!(
            parsed.get("model_provider").and_then(|item| item.as_str()),
            Some("manual")
        );
        assert_eq!(
            parsed.get("model").and_then(|item| item.as_str()),
            Some("gpt-user-current")
        );
        assert_eq!(
            parsed
                .get("model_catalog_json")
                .and_then(|item| item.as_str()),
            Some("official-catalog.json")
        );
        assert!(plugins.get("browser@openai-bundled").is_some());
        assert!(plugins.get("chrome@openai-bundled").is_some());
        assert!(plugins.get("hyperframes@openai-curated").is_some());
        let local_provider = parsed
            .get("model_providers")
            .and_then(|item| item.as_table())
            .and_then(|providers| providers.get("codex_local_access"))
            .and_then(|item| item.as_table())
            .expect("current unknown provider fields should remain");
        assert_eq!(
            local_provider
                .get("custom_user_option")
                .and_then(|item| item.as_str()),
            Some("keep-current")
        );
        assert!(local_provider.get("experimental_bearer_token").is_none());
        assert_eq!(
            local_provider
                .get("http_headers")
                .and_then(|item| item.as_inline_table())
                .and_then(|headers| headers.get("X-Custom"))
                .and_then(|value| value.as_str()),
            Some("keep-current")
        );
    }

    #[test]
    fn takeover_backup_restore_drops_stale_local_access_catalog() {
        let current = r#"model_provider = "codex_local_access"
model_catalog_json = "cockpit-local-access-model-catalog.json"

[model_providers.codex_local_access]
name = "Codex API Service"
base_url = "http://localhost:14998/v1"
wire_api = "responses"
requires_openai_auth = false
experimental_bearer_token = "agt_codex_test"
"#;
        let stale_backup = r#"model_provider = "openai"
model_catalog_json = "cockpit-local-access-model-catalog.json"
model_context_window = 1000000
"#;

        let output = restore_config_toml_from_takeover_backup(Some(current), Some(stale_backup))
            .expect("restore config")
            .expect("restored content");
        let parsed = output
            .parse::<toml_edit::Document>()
            .expect("parse restored toml");

        assert_eq!(
            parsed.get("model_provider").and_then(|item| item.as_str()),
            Some("openai")
        );
        assert!(parsed.get("model_catalog_json").is_none());
    }

    #[test]
    fn takeover_backup_restore_restores_auth_and_removes_managed_artifacts() {
        let dir = make_temp_dir("codex-local-access-restore-auth-artifacts");
        let current_config = r#"model_provider = "codex_local_access"
model = "gpt-current"
model_catalog_json = "cockpit-local-access-model-catalog.json"

[model_providers.codex_local_access]
name = "Codex API Service"
base_url = "http://localhost:14998/v1"
wire_api = "responses"
requires_openai_auth = false
experimental_bearer_token = "agt_codex_test"
supports_websockets = false
"#;
        fs::write(dir.join(CODEX_PROFILE_CONFIG_FILE), current_config).expect("write config");
        fs::write(
            dir.join(CODEX_PROFILE_AUTH_FILE),
            r#"{"auth_mode":"apikey","OPENAI_API_KEY":"agt_codex_test"}"#,
        )
        .expect("write auth");
        for file_name in [
            CODEX_LOCAL_ACCESS_AUTH_PROJECTION_FILE,
            CODEX_LOCAL_ACCESS_MODEL_CATALOG_FILE,
            CODEX_MODEL_CACHE_FILE,
        ] {
            fs::write(dir.join(file_name), "managed").expect("write managed artifact");
        }
        fs::write(dir.join("user-file.json"), "keep").expect("write user file");

        let backup = CodexLocalAccessProfileTakeoverBackup {
            profile_dir: dir.to_string_lossy().to_string(),
            auth_json: Some(
                r#"{"tokens":{"id_token":"official-id","access_token":"official-access","refresh_token":"official-refresh"}}"#
                    .to_string(),
            ),
            config_toml: Some("model = \"gpt-before\"\n".to_string()),
            created_at: 1,
            updated_at: 1,
        };

        assert!(
            restore_profile_takeover_backup(&backup, "agt_codex_rotated_new", true)
                .expect("restore takeover")
        );

        let restored_config =
            fs::read_to_string(dir.join(CODEX_PROFILE_CONFIG_FILE)).expect("read config");
        let restored_doc = restored_config
            .parse::<toml_edit::Document>()
            .expect("parse config");
        assert_eq!(
            restored_doc.get("model").and_then(|item| item.as_str()),
            Some("gpt-current")
        );
        assert!(restored_doc.get("model_provider").is_none());
        assert!(restored_doc.get("model_catalog_json").is_none());
        assert!(fs::read_to_string(dir.join(CODEX_PROFILE_AUTH_FILE))
            .expect("read auth")
            .contains("official-access"));
        for file_name in [
            CODEX_LOCAL_ACCESS_AUTH_PROJECTION_FILE,
            CODEX_LOCAL_ACCESS_MODEL_CATALOG_FILE,
            CODEX_MODEL_CACHE_FILE,
        ] {
            assert!(!dir.join(file_name).exists());
        }
        assert_eq!(
            fs::read_to_string(dir.join("user-file.json")).expect("read user file"),
            "keep"
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn mixed_model_gateway_restore_preserves_refreshed_oauth_auth() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let _env = LocalAccessTestDataGuard::new("codex-mixed-model-restore");
        let profile_dir = make_temp_dir("codex-mixed-model-profile");
        let api_key = "agt_codex_mixed_restore_test";
        let original_config = "model = \"gpt-5.5\"\ncustom_setting = \"keep\"\n";
        let original_auth = r#"{"tokens":{"id_token":"old-id","access_token":"old-access","refresh_token":"old-refresh"}}"#;
        let refreshed_auth = r#"{"tokens":{"id_token":"new-id","access_token":"new-access","refresh_token":"new-refresh"}}"#;

        fs::write(profile_dir.join(CODEX_PROFILE_CONFIG_FILE), original_config)
            .expect("write original config");
        fs::write(profile_dir.join(CODEX_PROFILE_AUTH_FILE), original_auth)
            .expect("write original auth");
        super::save_provider_gateway_profile_state(
            &profile_dir,
            super::MIXED_MODEL_ROUTING_RUNTIME_ID,
            &super::ProviderGatewayProfileState {
                api_key: api_key.to_string(),
                port: None,
                created_at: 1,
                updated_at: 1,
            },
        )
        .expect("save mixed gateway state");
        super::save_profile_takeover_backup(&profile_dir, api_key)
            .expect("save mixed takeover backup");

        let mixed_config = format!(
            r#"model = "gpt-5.5"
custom_setting = "keep"
model_provider = "codex_local_access"
model_catalog_json = "{}"

[model_providers.codex_local_access]
name = "Cockpit Mixed Model Routing"
base_url = "http://127.0.0.1:14998/v1"
wire_api = "responses"
requires_openai_auth = false
experimental_bearer_token = "{}"
supports_websockets = false
"#,
            CODEX_LOCAL_ACCESS_MODEL_CATALOG_FILE, api_key
        );
        fs::write(profile_dir.join(CODEX_PROFILE_CONFIG_FILE), mixed_config)
            .expect("write mixed config");
        fs::write(profile_dir.join(CODEX_PROFILE_AUTH_FILE), refreshed_auth)
            .expect("write refreshed OAuth auth");
        for file_name in [
            CODEX_LOCAL_ACCESS_AUTH_PROJECTION_FILE,
            CODEX_LOCAL_ACCESS_MODEL_CATALOG_FILE,
            CODEX_MODEL_CACHE_FILE,
        ] {
            fs::write(profile_dir.join(file_name), "managed").expect("write managed artifact");
        }

        assert!(super::restore_mixed_model_gateway_profile(&profile_dir).expect("restore mixed"));
        let restored_config = fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE))
            .expect("read restored config");
        let restored_doc = restored_config
            .parse::<toml_edit::Document>()
            .expect("parse restored config");
        assert_eq!(
            restored_doc.get("model").and_then(|item| item.as_str()),
            Some("gpt-5.5")
        );
        assert_eq!(
            restored_doc
                .get("custom_setting")
                .and_then(|item| item.as_str()),
            Some("keep")
        );
        assert!(restored_doc.get("model_provider").is_none());
        assert!(restored_doc.get("model_catalog_json").is_none());
        assert_eq!(
            fs::read_to_string(profile_dir.join(CODEX_PROFILE_AUTH_FILE))
                .expect("read refreshed auth"),
            refreshed_auth
        );
        for file_name in [
            CODEX_LOCAL_ACCESS_AUTH_PROJECTION_FILE,
            CODEX_LOCAL_ACCESS_MODEL_CATALOG_FILE,
            CODEX_MODEL_CACHE_FILE,
        ] {
            assert!(!profile_dir.join(file_name).exists());
        }
        assert!(!super::restore_mixed_model_gateway_profile(&profile_dir)
            .expect("second restore is idempotent"));

        fs::remove_dir_all(profile_dir).expect("cleanup mixed profile");
    }

    #[test]
    fn mixed_model_gateway_reuses_persisted_profile_port() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let _env = LocalAccessTestDataGuard::new("codex-mixed-model-port");
        let profile_dir = make_temp_dir("codex-mixed-model-port-profile");

        let first = super::provider_gateway_profile_port(
            &profile_dir,
            super::MIXED_MODEL_ROUTING_RUNTIME_ID,
        )
        .expect("allocate persistent port");
        let second = super::provider_gateway_profile_port(
            &profile_dir,
            super::MIXED_MODEL_ROUTING_RUNTIME_ID,
        )
        .expect("reuse persistent port");

        assert!(first > 0);
        assert_eq!(second, first);
        fs::remove_dir_all(profile_dir).expect("cleanup mixed port profile");
    }

    #[test]
    fn mixed_model_profile_detection_accepts_managed_auth_projection() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let _env = LocalAccessTestDataGuard::new("codex-mixed-model-profile-detection");
        let profile_dir = make_temp_dir("codex-mixed-model-detection-profile");
        let api_key = "agt_codex_mixed_detection";
        super::save_provider_gateway_profile_state(
            &profile_dir,
            super::MIXED_MODEL_ROUTING_RUNTIME_ID,
            &super::ProviderGatewayProfileState {
                api_key: api_key.to_string(),
                port: Some(14998),
                created_at: 1,
                updated_at: 1,
            },
        )
        .expect("save mixed gateway state");
        fs::write(
            profile_dir.join(CODEX_PROFILE_AUTH_FILE),
            format!(r#"{{"auth_mode":"apikey","OPENAI_API_KEY":"{}"}}"#, api_key),
        )
        .expect("write managed auth");

        assert!(super::profile_uses_mixed_model_gateway(&profile_dir)
            .expect("detect mixed profile"));
        fs::remove_dir_all(profile_dir).expect("cleanup mixed detection profile");
    }

    #[test]
    fn mixed_route_model_selection_merges_manual_models() {
        let catalog = vec!["gpt-5.5".to_string(), "grok-4.6".to_string()];
        let manual = vec!["custom-preview".to_string()];

        assert_eq!(
            super::mixed_route_upstream_models(&catalog, None, Some(&manual)),
            vec!["gpt-5.5", "grok-4.6", "custom-preview"]
        );
        assert_eq!(
            super::mixed_route_upstream_models(
                &catalog,
                Some(&["grok-4.6".to_string(), "custom-preview".to_string()]),
                Some(&manual),
            ),
            vec!["grok-4.6", "custom-preview"]
        );
    }

    #[test]
    fn mixed_model_sidecar_is_not_bound_to_cockpit_parent_lifetime() {
        assert_eq!(super::provider_gateway_sidecar_parent_pid(true), 0);
        assert_eq!(
            super::provider_gateway_sidecar_parent_pid(false),
            std::process::id()
        );
    }

    #[test]
    fn mixed_model_activation_snapshot_restores_partial_takeover_failure() {
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let _env = LocalAccessTestDataGuard::new("codex-mixed-model-start-rollback");
        let profile_dir = make_temp_dir("codex-mixed-model-start-profile");
        let api_key = "agt_codex_mixed_start_rollback";
        let original_config = "model = \"gpt-5.5\"\ncustom_setting = \"keep\"\n";
        let original_auth =
            r#"{"tokens":{"id_token":"id","access_token":"access","refresh_token":"refresh"}}"#;
        let provider_backup_path = super::provider_model_backup_path(&profile_dir);

        fs::write(profile_dir.join(CODEX_PROFILE_CONFIG_FILE), original_config)
            .expect("write original config");
        fs::write(profile_dir.join(CODEX_PROFILE_AUTH_FILE), original_auth)
            .expect("write original auth");
        fs::write(
            profile_dir.join(CODEX_LOCAL_ACCESS_MODEL_CATALOG_FILE),
            "original-catalog",
        )
        .expect("write original catalog");
        if let Some(parent) = provider_backup_path.parent() {
            fs::create_dir_all(parent).expect("create provider backup dir");
        }
        fs::write(&provider_backup_path, "original-provider-backup")
            .expect("write original provider backup");

        let snapshot = super::capture_mixed_model_profile_activation_snapshot(&profile_dir, api_key)
            .expect("capture activation snapshot");
        super::save_profile_takeover_backup(&profile_dir, api_key)
            .expect("save partial takeover backup");
        fs::write(
            profile_dir.join(CODEX_PROFILE_CONFIG_FILE),
            "model_provider = \"codex_local_access\"\n",
        )
        .expect("write partial config");
        fs::write(
            profile_dir.join(CODEX_PROFILE_AUTH_FILE),
            format!(r#"{{"auth_mode":"apikey","OPENAI_API_KEY":"{}"}}"#, api_key),
        )
        .expect("write partial auth");
        fs::write(
            profile_dir.join(CODEX_LOCAL_ACCESS_MODEL_CATALOG_FILE),
            "partial-catalog",
        )
        .expect("write partial catalog");
        fs::remove_file(&provider_backup_path).expect("remove provider backup");

        super::rollback_mixed_model_profile_after_start_failure(&profile_dir, Some(snapshot))
            .expect("rollback partial takeover");
        assert_eq!(
            fs::read_to_string(profile_dir.join(CODEX_PROFILE_CONFIG_FILE))
                .expect("read rolled back config"),
            original_config
        );
        assert_eq!(
            fs::read_to_string(profile_dir.join(CODEX_PROFILE_AUTH_FILE))
                .expect("read rolled back auth"),
            original_auth
        );
        assert_eq!(
            fs::read_to_string(profile_dir.join(CODEX_LOCAL_ACCESS_MODEL_CATALOG_FILE))
                .expect("read rolled back catalog"),
            "original-catalog"
        );
        assert_eq!(
            fs::read_to_string(provider_backup_path).expect("read rolled back provider backup"),
            "original-provider-backup"
        );
        assert!(super::load_takeover_backups()
            .expect("load takeover backups")
            .profiles
            .iter()
            .all(|backup| backup.profile_dir != super::normalize_profile_dir_key(&profile_dir)));

        fs::remove_dir_all(profile_dir).expect("cleanup mixed profile");
    }

    #[test]
    fn detects_only_matching_local_access_auth_key() {
        assert!(is_codex_local_access_auth_text(
            r#"{"auth_mode":"apikey","OPENAI_API_KEY":"local-key"}"#,
            "local-key"
        ));
        assert!(is_codex_local_access_auth_text(
            r#"{"auth_mode":"apikey","OPENAI_API_KEY":"agt_codex_generated"}"#,
            "local-key"
        ));
        assert!(!is_codex_local_access_auth_text(
            r#"{"auth_mode":"apikey","OPENAI_API_KEY":"other-key"}"#,
            "local-key"
        ));
        assert!(!is_codex_local_access_auth_text(
            r#"{"tokens":{"access_token":"official"}}"#,
            "local-key"
        ));
        assert!(is_codex_oauth_auth_text(
            r#"{"tokens":{"id_token":"official-id-token","access_token":"official"}}"#
        ));
        assert!(!is_codex_oauth_auth_text(
            r#"{"auth_mode":"apikey","OPENAI_API_KEY":"local-key"}"#
        ));
    }

    #[test]
    fn local_access_profile_config_requires_selected_matching_provider() {
        let config = r#"model_provider = "codex_local_access"

[model_providers.codex_local_access]
name = "Codex API Service"
base_url = "http://localhost:14998/v1"
wire_api = "responses"
requires_openai_auth = false
experimental_bearer_token = "agt_codex_test"
http_headers = { "x-openai-actor-authorization" = "cockpit-tools" }
supports_websockets = false
"#;

        let inspection = inspect_local_access_profile_config(
            config,
            "http://localhost:14998/v1",
            "agt_codex_test",
            false,
        )
        .expect("inspect config");

        assert!(inspection.config_attached);
        assert_eq!(
            inspection.model_provider.as_deref(),
            Some("codex_local_access")
        );
        assert_eq!(
            inspection.base_url.as_deref(),
            Some("http://localhost:14998/v1")
        );
    }

    #[test]
    fn local_access_profile_config_rejects_oauth_auth_gate_for_api_key_projection() {
        let config = r#"model_provider = "codex_local_access"

[model_providers.codex_local_access]
name = "Codex API Service"
base_url = "http://localhost:14998/v1"
wire_api = "responses"
requires_openai_auth = true
experimental_bearer_token = "agt_codex_test"
supports_websockets = false
"#;

        let inspection = inspect_local_access_profile_config(
            config,
            "http://localhost:14998/v1",
            "agt_codex_test",
            false,
        )
        .expect("inspect OAuth auth gate config");

        assert!(!inspection.config_attached);
        assert!(inspection.token_matched);
    }

    #[test]
    fn local_access_profile_config_accepts_bound_oauth_with_imagegen_headers() {
        let config = r#"model_provider = "codex_local_access"

[model_providers.codex_local_access]
name = "Codex API Service"
base_url = "http://localhost:14998/v1"
wire_api = "responses"
requires_openai_auth = true
experimental_bearer_token = "agt_codex_test"
http_headers = { "x-openai-actor-authorization" = "cockpit-tools", "x-agtools-disable-image-generation" = "chat" }
supports_websockets = false
"#;

        let inspection = inspect_local_access_profile_config(
            config,
            "http://localhost:14998/v1",
            "agt_codex_test",
            true,
        )
        .expect("inspect bound oauth with imagegen");

        assert!(inspection.config_attached);
        assert!(inspection.token_matched);
    }

    #[test]
    fn local_access_profile_attachment_accepts_bound_oauth_with_imagegen_projection() {
        let dir = make_temp_dir("codex-local-access-bound-oauth-attachment");
        let config = r#"model_provider = "codex_local_access"

[model_providers.codex_local_access]
name = "Codex API Service"
base_url = "http://localhost:14998/v1"
wire_api = "responses"
requires_openai_auth = true
experimental_bearer_token = "local-api-key"
http_headers = { "x-openai-actor-authorization" = "cockpit-tools", "x-agtools-disable-image-generation" = "chat" }
supports_websockets = false
"#;
        fs::write(dir.join(CODEX_PROFILE_CONFIG_FILE), config).expect("write config");
        fs::write(
            dir.join(CODEX_PROFILE_AUTH_FILE),
            r#"{"tokens":{"id_token":"official-id-token","access_token":"official","refresh_token":"refresh"}}"#,
        )
        .expect("write auth");

        let mut collection = test_local_access_collection(Vec::new());
        collection.bound_oauth_account_id = Some("oauth-account".to_string());
        let attachment = inspect_local_access_profile_attachment(&dir, Some(&collection));

        assert!(attachment.attached);
        assert!(attachment.config_attached);
        assert!(attachment.auth_attached);
        assert!(attachment.error.is_none());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn local_access_profile_config_rejects_stale_port_or_key() {
        let config = r#"model_provider = "codex_local_access"

[model_providers.codex_local_access]
name = "Codex API Service"
base_url = "http://127.0.0.1:14998/v1"
wire_api = "responses"
requires_openai_auth = false
experimental_bearer_token = "agt_codex_old"
http_headers = { "X-OpenAI-Actor-Authorization" = "cockpit-tools" }
supports_websockets = false
"#;

        let stale_port = inspect_local_access_profile_config(
            config,
            "http://127.0.0.1:14999/v1",
            "agt_codex_old",
            false,
        )
        .expect("inspect stale port");
        let stale_key = inspect_local_access_profile_config(
            config,
            "http://127.0.0.1:14998/v1",
            "agt_codex_new",
            false,
        )
        .expect("inspect stale key");

        assert!(!stale_port.config_attached);
        assert!(!stale_key.config_attached);
    }

    #[test]
    fn profile_base_url_matching_ignores_trailing_slash_and_host_case() {
        assert!(profile_base_url_matches(
            Some("HTTP://LOCALHOST:14998/v1/"),
            "http://localhost:14998/v1"
        ));
        assert!(!profile_base_url_matches(
            Some("http://127.0.0.1:14998/v1"),
            "http://localhost:14998/v1"
        ));
    }

    #[test]
    fn builds_client_base_url_with_selected_host() {
        assert_eq!(
            build_base_url_with_host(14998, CodexLocalAccessClientBaseUrlHost::Localhost),
            "http://localhost:14998/v1"
        );
        assert_eq!(
            build_base_url_with_host(14998, CodexLocalAccessClientBaseUrlHost::Ipv4Loopback),
            "http://127.0.0.1:14998/v1"
        );
    }

    #[test]
    fn collection_base_url_defaults_to_localhost_and_uses_saved_host() {
        let default_collection = test_local_access_collection(Vec::new());
        assert_eq!(
            build_collection_base_url(&default_collection),
            "http://localhost:14998/v1"
        );

        let mut loopback_collection = test_local_access_collection(Vec::new());
        loopback_collection.client_base_url_host = CodexLocalAccessClientBaseUrlHost::Ipv4Loopback;
        assert_eq!(
            build_collection_base_url(&loopback_collection),
            "http://127.0.0.1:14998/v1"
        );
    }

    #[test]
    fn local_access_chat_completions_url_preserves_existing_v1_prefix() {
        assert_eq!(
            local_access_chat_completions_url("http://localhost:11892/v1"),
            "http://localhost:11892/v1/chat/completions"
        );
        assert_eq!(
            local_access_chat_completions_url("http://localhost:11892/v1/"),
            "http://localhost:11892/v1/chat/completions"
        );
    }

    #[test]
    fn invalid_stats_file_is_quarantined_and_replaced_by_empty_stats() {
        let dir = make_temp_dir("codex-local-access-invalid-stats");
        let path = dir.join("codex_local_access_stats.json");
        fs::write(
            &path,
            b"{\"since\":1,\"accounts\":[{\"email\":\"bad\0value\"}]}",
        )
        .expect("write invalid stats");
        let content = fs::read_to_string(&path).expect("read invalid stats");
        let parse_error =
            serde_json::from_str::<CodexLocalAccessStats>(&content).expect_err("invalid json");

        let recovered = recover_invalid_stats_file(&path, &parse_error);

        assert_eq!(recovered.totals.request_count, 0);
        assert!(!path.exists());
        let backups = fs::read_dir(&dir)
            .expect("read temp dir")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("codex_local_access_stats.json.invalid-")
            })
            .count();
        assert_eq!(backups, 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn api_key_usage_stats_are_isolated_by_time_window() {
        use chrono::{Local, TimeZone};

        let local_timestamp = |year, month, day, hour, minute| {
            Local
                .with_ymd_and_hms(year, month, day, hour, minute, 0)
                .single()
                .expect("test local datetime should be unambiguous")
                .timestamp_millis()
        };
        let now = local_timestamp(2026, 5, 20, 12, 0);
        let event =
            |timestamp: i64, api_key_id: &str, total_tokens: u64| CodexLocalAccessUsageEvent {
                timestamp,
                api_key_id: api_key_id.to_string(),
                api_key_label: api_key_id.to_string(),
                request_kind: CodexLocalAccessRequestKind::Text,
                success: true,
                total_tokens,
                ..Default::default()
            };
        let mut stats = empty_stats_snapshot();
        stats.events = vec![
            event(local_timestamp(2026, 4, 30, 23, 59), "key-a", 500),
            event(local_timestamp(2026, 5, 1, 0, 1), "key-a", 400),
            event(local_timestamp(2026, 5, 17, 23, 59), "key-a", 300),
            event(local_timestamp(2026, 5, 19, 23, 59), "key-a", 200),
            event(local_timestamp(2026, 5, 20, 0, 1), "key-a", 100),
            event(local_timestamp(2026, 5, 20, 0, 1), "key-b", 50),
        ];

        recompute_time_windows(&mut stats, now);

        let usage = |window: &CodexLocalAccessStatsWindow, api_key_id: &str| {
            let usage = &window
                .api_keys
                .iter()
                .find(|item| item.api_key_id == api_key_id)
                .expect("API key stats should exist")
                .usage;
            (usage.request_count, usage.total_tokens)
        };
        assert_eq!(usage(&stats.daily, "key-a"), (1, 100));
        assert_eq!(usage(&stats.daily, "key-b"), (1, 50));
        assert_eq!(usage(&stats.weekly, "key-a"), (2, 300));
        assert_eq!(usage(&stats.monthly, "key-a"), (4, 1000));
        let starts = calendar_stats_window_starts(now);
        assert_eq!(stats.daily.since, local_timestamp(2026, 5, 20, 0, 0));
        assert_eq!(stats.weekly.since, local_timestamp(2026, 5, 18, 0, 0));
        assert_eq!(stats.monthly.since, local_timestamp(2026, 5, 1, 0, 0));
        assert_eq!(stats.daily.since, starts.day);
        assert_eq!(stats.weekly.since, starts.week);
        assert_eq!(stats.monthly.since, starts.month);

        recompute_time_windows(&mut stats, local_timestamp(2026, 5, 21, 0, 1));
        assert!(stats.daily.api_keys.is_empty());
        assert_eq!(usage(&stats.weekly, "key-a"), (2, 300));
        assert_eq!(usage(&stats.monthly, "key-a"), (4, 1000));
    }
