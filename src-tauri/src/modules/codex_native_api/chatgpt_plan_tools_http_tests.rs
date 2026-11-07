//! Real Responses wire shapes across discover → invoke local tools → continue.
//! No tool is executed and every HTTP endpoint is a temporary loopback server.
use super::*;

fn search() -> Value {
    json!({"type":"tool_search","execution":"client","parameters":{"type":"object","properties":{"goal":{"type":"string"}},"required":["goal"]}})
}

fn discovered() -> Value {
    json!([
        {"type":"namespace","name":"mcp__workspace","description":"Client MCP tools","tools":[{"type":"function","name":"read_file","defer_loading":true,"parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}]},
        {"type":"namespace","name":"collaboration","description":"Client agents","tools":[{"type":"function","name":"spawn_agent","defer_loading":true,"parameters":{"type":"object","properties":{"message":{"type":"string"}},"required":["message"]}}]},
        {"type":"function","name":"shell_command","defer_loading":true,"parameters":{"type":"object","properties":{"command":{"type":"string"}},"required":["command"]}},
        {"type":"custom","name":"apply_patch","defer_loading":true,"format":{"type":"grammar","syntax":"lark","definition":"start: /(.|\\n)+/"}}
    ])
}

fn tool_declarations(body: &Value) -> Vec<Value> {
    let mut tools = body
        .get("tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for item in body["input"].as_array().unwrap() {
        if item["type"] == "additional_tools" {
            tools.extend(item["tools"].as_array().unwrap().iter().cloned());
        }
    }
    tools
}

fn response_events(round: usize, body: &Value) -> Vec<Value> {
    let items = match round {
        1 => {
            let tool = tool_declarations(body)
                .into_iter()
                .find(|tool| {
                    tool["name"]
                        .as_str()
                        .is_some_and(|name| name.starts_with("cockpit_client_tool_search"))
                })
                .unwrap();
            vec![
                json!({"type":"function_call","id":"fc_find_local","call_id":"call_find_local","name":tool["name"],"arguments":"{\"goal\":\"local files, shell and agents\"}","status":"completed"}),
            ]
        }
        2 => vec![
            json!({"type":"function_call","id":"fc_read","call_id":"call_read","namespace":"mcp__workspace","name":"read_file","arguments":"{\"path\":\"README.md\"}","status":"completed"}),
            json!({"type":"function_call","id":"fc_shell","call_id":"call_shell","name":"shell_command","arguments":"{\"command\":\"git status --short\"}","status":"completed"}),
            json!({"type":"custom_tool_call","id":"ctc_patch","call_id":"call_patch","name":"apply_patch","input":"*** Begin Patch\n*** End Patch","status":"completed"}),
            json!({"type":"function_call","id":"fc_agent","call_id":"call_agent","namespace":"collaboration","name":"spawn_agent","arguments":"{\"message\":\"Review the local result\"}","status":"completed"}),
        ],
        _ => vec![
            json!({"type":"message","id":"msg_final","role":"assistant","status":"completed","content":[{"type":"output_text","text":"Local tools completed.","annotations":[]}]}),
        ],
    };
    let mut events = vec![
        json!({"type":"response.created","response":{"id":format!("resp_{round}"),"status":"in_progress","output":[]}}),
    ];
    for (index, item) in items.iter().enumerate() {
        let mut added = item.clone();
        let custom = item["type"] == "custom_tool_call";
        let argument_key = if custom { "input" } else { "arguments" };
        let arguments = item[argument_key].as_str();
        if arguments.is_some() {
            added[argument_key] = json!("");
        }
        added["status"] = json!("in_progress");
        events.push(json!({"type":"response.output_item.added","output_index":index,"item":added}));
        if let Some(arguments) = arguments {
            let prefix = if custom {
                "response.custom_tool_call_input"
            } else {
                "response.function_call_arguments"
            };
            let split = arguments.len() / 2;
            for delta in [&arguments[..split], &arguments[split..]] {
                events.push(json!({"type":format!("{prefix}.delta"),"item_id":item["id"],"output_index":index,"delta":delta}));
            }
            let mut done =
                json!({"type":format!("{prefix}.done"),"item_id":item["id"],"output_index":index});
            done[argument_key] = json!(arguments);
            events.push(done);
        }
        events.push(json!({"type":"response.output_item.done","output_index":index,"item":item}));
    }
    events.push(json!({"type":"response.completed","response":{"id":format!("resp_{round}"),"status":"completed","output":items,"usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15}}}));
    events
}

async fn send(
    client: &reqwest::Client,
    url: &str,
    body: &Value,
    streaming: bool,
) -> (Value, String) {
    let response = client
        .post(url)
        .bearer_auth("local-test")
        .json(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let raw = response.text().await.unwrap();
    let response = if streaming {
        raw.lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find(|event| event["type"] == "response.completed")
            .expect("completed inference")["response"]
            .clone()
    } else {
        serde_json::from_str(&raw).unwrap()
    };
    (response, raw)
}

#[tokio::test]
async fn chatgpt_plan_client_discovery_and_local_tools_complete_three_http_rounds_json_and_sse() {
    for streaming in [false, true] {
        let captures: Arc<Mutex<Vec<Value>>> = Default::default();
        let seen = captures.clone();
        let upstream = Router::new().route(
            "/v1/responses",
            post(move |Json(body): Json<Value>| {
                let seen = seen.clone();
                async move {
                    if body["input"].as_array().unwrap().iter().any(|item| {
                        item["type"] == "additional_tools" && item["role"] != "developer"
                    }) {
                        return Response::builder()
                            .status(StatusCode::BAD_REQUEST)
                            .header("content-type", "application/json")
                            .body(Body::from(json!({"error":{"code":"missing_required_parameter","param":"input.role","message":"Additional tools require role developer"}}).to_string()))
                            .unwrap();
                    }
                    let round = {
                        let mut captures = seen.lock().unwrap();
                        captures.push(body.clone());
                        captures.len()
                    };
                    let events = response_events(round, &body);
                    let payload = events
                        .iter()
                        .map(|event| {
                            format!(
                                "event: {}\ndata: {event}\n\n",
                                event["type"].as_str().unwrap()
                            )
                        })
                        .collect::<String>();
                    Response::builder()
                        .header("content-type", "text/event-stream")
                        .body(Body::from(payload))
                        .unwrap()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let task = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
        let host = Arc::new(PlanHost {
            base,
            refreshes: AtomicUsize::new(0),
            reports: Mutex::new(Vec::new()),
        });
        let root = tempfile::tempdir().unwrap();
        let mut service = start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            root.path().join("identity.sqlite"),
            Options::default(),
        )
        .await
        .unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("http://{}/v1/responses", service.addr);
        let mut body = json!({"model":"plan-model","stream":streaming,"store":false,"tools":[search()],"input":[{"role":"user","content":"Find and run the local tools."}]});
        let (first, first_raw) = send(&client, &url, &body, streaming).await;
        let search_call = &first["output"][0];
        assert_eq!(search_call["type"], "tool_search_call");
        assert_eq!(search_call["execution"], "client");
        assert_eq!(search_call["id"], "tsc_find_local");
        assert!(!first_raw.contains("cockpit_client_tool_search"));
        if streaming {
            assert!(!first_raw.contains("response.function_call_arguments"));
        }
        body["input"].as_array_mut().unwrap().extend([
            search_call.clone(),
            json!({"type":"tool_search_output","execution":"client","status":"completed","call_id":"call_find_local","tools":discovered()}),
        ]);
        let (second, second_raw) = send(&client, &url, &body, streaming).await;
        assert_eq!(second["output"][0]["namespace"], "mcp__workspace");
        assert_eq!(second["output"][0]["name"], "read_file");
        assert_eq!(second["output"][1]["name"], "shell_command");
        assert_eq!(second["output"][2]["type"], "custom_tool_call");
        assert_eq!(
            second["output"][2]["input"],
            "*** Begin Patch\n*** End Patch"
        );
        assert_eq!(second["output"][3]["namespace"], "collaboration");
        assert_eq!(second["output"][3]["encrypted_function_args"], json!([]));
        assert!(!second_raw.contains("cockpit_client_tool_search"));
        body["input"]
            .as_array_mut()
            .unwrap()
            .extend(second["output"].as_array().unwrap().iter().cloned());
        body["input"].as_array_mut().unwrap().extend([
            json!({"type":"function_call_output","call_id":"call_read","output":"README contents"}),
            json!({"type":"function_call_output","call_id":"call_shell","output":"working tree changes"}),
            json!({"type":"custom_tool_call_output","call_id":"call_patch","output":"Patch applied"}),
            json!({"type":"function_call_output","call_id":"call_agent","output":"{\"agent_id\":\"worker\"}"}),
        ]);
        let (third, _) = send(&client, &url, &body, streaming).await;
        assert_eq!(
            third["output"][0]["content"][0]["text"],
            "Local tools completed."
        );
        {
            let captures = captures.lock().unwrap();
            assert_eq!(captures.len(), 3);
            for request in captures.iter() {
                assert_eq!(request["stream"], true);
                assert_eq!(request["store"], false);
                for item in request["input"].as_array().unwrap() {
                    if item["type"] == "additional_tools" {
                        assert_eq!(item["role"], "developer");
                    }
                }
                assert!(!request["input"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|item| matches!(
                        item["type"].as_str(),
                        Some("tool_search_call" | "tool_search_output")
                    )));
                assert!(!tool_declarations(request)
                    .iter()
                    .any(|tool| tool["type"] == "tool_search"));
            }
            assert_eq!(captures[1]["input"][1]["call_id"], "call_find_local");
            assert_eq!(captures[1]["input"][1]["type"], "function_call");
            assert_eq!(captures[1]["input"][2]["type"], "function_call_output");
            let tools = tool_declarations(&captures[2]);
            assert!(tools
                .iter()
                .any(|tool| tool["type"] == "custom" && tool["name"] == "apply_patch"));
            let input = captures[2]["input"].as_array().unwrap();
            assert!(input
                .iter()
                .any(|item| item["type"] == "custom_tool_call_output"
                    && item["call_id"] == "call_patch"));
        }
        assert_eq!(host.reports.lock().unwrap().len(), 3);
        assert!(host
            .reports
            .lock()
            .unwrap()
            .iter()
            .all(|report| report.success));
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
}

#[tokio::test]
async fn chatgpt_plan_conflicting_search_terminal_never_reports_success_or_saves_a_checkpoint() {
    const PAYLOAD: &str = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_conflict\",\"status\":\"in_progress\",\"output\":[]}}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"id\":\"fc_search\",\"call_id\":\"call_search\",\"name\":\"cockpit_client_tool_search\",\"arguments\":\"{\\\"goal\\\":\\\"files\\\"}\",\"status\":\"completed\"}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_conflict\",\"status\":\"completed\",\"output\":[{\"type\":\"function_call\",\"id\":\"fc_search\",\"call_id\":\"call_search\",\"name\":\"cockpit_client_tool_search\",\"arguments\":\"{\\\"goal\\\":\\\"private replacement\\\"}\",\"status\":\"completed\"}],\"usage\":{\"input_tokens\":2,\"output_tokens\":1}}}\n\n",
    );
    for streaming in [false, true] {
        let (host, _, task, dir, mut service) = fixture(PAYLOAD, false).await;
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(format!("http://{}/v1/responses", service.addr))
            .bearer_auth("local-test")
            .json(&json!({"model":"plan-model","stream":streaming,
                "client_metadata":{"thread_id":"search-integrity","session_id":"search-integrity"},
                "tools":[search()],"input":[{"role":"user","content":"Find local tools."}]}))
            .send()
            .await
            .unwrap();
        if !streaming {
            assert_eq!(response.status(), 502);
        }
        let raw = response.text().await.unwrap();
        assert!(!raw.contains("response.completed"), "{raw}");
        assert!(!raw.contains("private replacement"), "{raw}");
        let events: Vec<Value> = if streaming {
            raw.lines()
                .filter_map(|line| line.strip_prefix("data: "))
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect()
        } else {
            vec![serde_json::from_str(&raw).unwrap()]
        };
        assert!(
            events
                .iter()
                .any(|event| event.get("error").is_some()
                    || event.pointer("/response/error").is_some()),
            "{raw}"
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            while host.reports.lock().unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!host.reports.lock().unwrap()[0].success);
        let connection = rusqlite::Connection::open(dir.path().join("identity.sqlite")).unwrap();
        for query in [
            "SELECT COUNT(*) FROM native_opaque_v1 WHERE kind='response'",
            "SELECT COUNT(*) FROM native_rebuild_checkpoints_v1",
            "SELECT COUNT(*) FROM native_rebuild_portable_v1",
        ] {
            let count: i64 = connection.query_row(query, [], |row| row.get(0)).unwrap();
            assert_eq!(count, 0, "streaming={streaming}: {query}");
        }
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
}

#[tokio::test]
async fn chatgpt_plan_empty_search_terminal_restores_client_output_and_complete_checkpoint() {
    const PAYLOAD: &str = concat!(
        "data: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"type\":\"function_call\",\"id\":\"fc_search\",\"call_id\":\"call_search\",\"name\":\"cockpit_client_tool_search\",\"arguments\":\"{\\\"goal\\\":\\\"files\\\"}\",\"status\":\"completed\"}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_sparse_search\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":2,\"output_tokens\":1}}}\n\n",
    );
    for streaming in [false, true] {
        let (host, _, task, dir, mut service) = fixture(PAYLOAD, false).await;
        let request = json!({"model":"plan-model","stream":streaming,
            "client_metadata":{"thread_id":"search-snapshot","session_id":"search-snapshot"},
            "tools":[search()],"input":[{"role":"user","content":"Find local tools."}]});
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let (response, raw) = send(
            &client,
            &format!("http://{}/v1/responses", service.addr),
            &request,
            streaming,
        )
        .await;
        let call = &response["output"][0];
        assert_eq!(call["type"], "tool_search_call");
        assert_eq!(call["id"], "tsc_search");
        assert_eq!(call["call_id"], "call_search");
        assert_eq!(call["execution"], "client");
        assert_eq!(call["arguments"], json!({"goal":"files"}));
        assert!(!raw.contains("cockpit_client_tool_search"));
        tokio::time::timeout(Duration::from_secs(5), async {
            while host.reports.lock().unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(host.reports.lock().unwrap()[0].success);
        let connection = rusqlite::Connection::open(dir.path().join("identity.sqlite")).unwrap();
        let transcript: String = connection
            .query_row(
                "SELECT transcript FROM native_rebuild_checkpoints_v1 WHERE caller='plan-caller' AND response='resp_sparse_search'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let transcript: Vec<Value> = serde_json::from_str(&transcript).unwrap();
        let stored = transcript
            .iter()
            .find(|item| item["type"] == "tool_search_call")
            .expect("checkpoint retains the completed client search");
        assert_eq!(stored, call);
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        task.abort();
    }
}
