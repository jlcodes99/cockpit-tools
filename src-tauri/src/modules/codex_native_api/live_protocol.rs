//! Opt-in real transport probes. Reports contain no credentials, prompts,
//! upstream response identifiers or account names. Output excerpts are limited
//! to this driver's independent synthetic prompts.
use super::{contract::*, server::Options};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
type Socket = yawc::WebSocket<yawc::MaybeTlsStream<tokio::net::TcpStream>>;

fn expected<'a>(model: &str, marker: &'a str) -> &'a str {
    if !model.starts_with("grok-") {
        return marker;
    }
    match marker {
        "LIVE_HTTP_OK" => "4",
        "LIVE_ZSTD_OK" => "6",
        "LIVE_WS_ONE" => "8",
        "LIVE_HTTP_BRIDGE" => "10",
        "LIVE_WS_TWO" => "12",
        "LIVE_AFTER_REJECTION" => "14",
        "LIVE_STEER_OK" => "16",
        _ => marker,
    }
}
fn request(model: &str, marker: &str, padding: usize) -> Value {
    let answer = expected(model, marker);
    let prompt = if let Ok(sum) = answer.parse::<u32>() {
        format!(
            "What is {} + {}? Answer with the number only.",
            sum / 2,
            sum / 2
        )
    } else {
        format!("Reply with exactly {marker}.")
    };
    json!({"model":model,"input":[{"role":"user","content":format!("{prompt}{}"," ".repeat(padding))}],"reasoning":{"effort":"low"},"stream":true})
}
fn response_text(event: &Value) -> String {
    event
        .pointer("/response/output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|item| item["content"].as_array().into_iter().flatten())
        .filter_map(|part| part["text"].as_str())
        .collect()
}
fn terminal(event: &Value) -> bool {
    matches!(
        event["type"].as_str(),
        Some(
            "response.completed"
                | "response.done"
                | "response.failed"
                | "response.incomplete"
                | "error"
        )
    )
}
fn error_code(event: &Value) -> Option<String> {
    event
        .pointer("/error/code")
        .or_else(|| event.pointer("/response/error/code"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}
async fn http(
    url: &str,
    key: &str,
    model: &str,
    encoding: &str,
    padding: usize,
) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .no_gzip()
        .no_zstd()
        .no_brotli()
        .no_deflate()
        .timeout(Duration::from_secs(80))
        .build()
        .map_err(|_| "client_init")?;
    let marker = if padding == 0 {
        "LIVE_HTTP_OK"
    } else {
        "LIVE_ZSTD_OK"
    };
    let body = request(model, marker, padding);
    let mut request = client
        .post(format!("{url}/v1/responses"))
        .bearer_auth(key)
        .header("user-agent", "codex_cli_rs/0.153.4")
        .header("accept-encoding", encoding);
    let input_encoding = if padding > 0 { encoding } else { "identity" };
    let serialized = serde_json::to_vec(&body).map_err(|_| "serialize")?;
    let encoded = match input_encoding {
        "gzip" => {
            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            encoder.write_all(&serialized).map_err(|_| "gzip_encode")?;
            encoder.finish().map_err(|_| "gzip_encode")?
        }
        "zstd" => zstd::stream::encode_all(serialized.as_slice(), 3).map_err(|_| "zstd_encode")?,
        _ => serialized,
    };
    if input_encoding != "identity" {
        request = request.header("content-encoding", input_encoding);
    }
    let response = request
        .header("content-type", "application/json")
        .body(encoded)
        .send()
        .await
        .map_err(|_| "http_transport")?;
    let status = response.status().as_u16();
    let media = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let cache_control = response
        .headers()
        .get("cache-control")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let actual = response
        .headers()
        .get("content-encoding")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("identity")
        .to_owned();
    let bytes = response.bytes().await.map_err(|_| "http_read")?;
    let wire_bytes = bytes.len();
    let mut decoded = Vec::new();
    match actual.as_str() {
        "gzip" => {
            flate2::read::GzDecoder::new(bytes.as_ref())
                .read_to_end(&mut decoded)
                .map_err(|_| "gzip_decode")?;
        }
        "zstd" => {
            decoded = zstd::stream::decode_all(bytes.as_ref()).map_err(|_| "zstd_decode")?;
        }
        "identity" => decoded.extend_from_slice(&bytes),
        _ => return Err("unexpected_response_encoding".into()),
    }
    let mut decoder = super::events::SseDecoder::new(4 * 1024 * 1024);
    let events = decoder
        .push(&decoded)
        .and_then(|mut events| {
            events.extend(decoder.finish()?);
            Ok(events)
        })
        .unwrap_or_default();
    let value = events
        .iter()
        .rev()
        .find(|event| terminal(event))
        .cloned()
        .or_else(|| serde_json::from_slice(&decoded).ok())
        .unwrap_or(Value::Null);
    Ok(
        json!({"status":status,"encoding":actual,"input_encoding":input_encoding,"content_type":media,"cache_control":cache_control,"wire_bytes":wire_bytes,"decoded_bytes":decoded.len(),"canonical_request_bytes":body.to_string().len(),"events":events.iter().filter_map(|v|v["type"].as_str()).collect::<Vec<_>>(),"terminal":value["type"],"error":error_code(&value),"error_detail":value.get("error").or_else(||value.pointer("/response/error")),"synthetic_output_excerpt":response_text(&value).chars().take(240).collect::<String>(),"protocol_ok":status==200&&matches!(value["type"].as_str(),Some("response.completed"|"response.done")),"pass":status==200&&response_text(&value).trim()==expected(model,marker)}),
    )
}
async fn socket(url: &str, key: &str, session: &str) -> Result<Socket, String> {
    socket_policy(url, key, session, false).await
}
async fn socket_policy(
    url: &str,
    key: &str,
    session: &str,
    disable_images: bool,
) -> Result<Socket, String> {
    yawc::WebSocket::connect(url.parse().map_err(|_| "ws_url")?)
        .with_options(super::websocket_io::options(4 * 1024 * 1024))
        .with_request(
            axum::http::Request::builder()
                .header("authorization", format!("Bearer {key}"))
                .header("user-agent", "codex_cli_rs/0.153.4")
                .header("session-id", session)
                .header("thread-id", session)
                .header(
                    "x-agtools-disable-image-generation",
                    if disable_images { "true" } else { "false" },
                ),
        )
        .await
        .map_err(|_| "ws_connect".into())
}
async fn next(socket: &mut Socket) -> Result<Value, String> {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(50), socket.next_frame())
            .await
            .map_err(|_| "ws_timeout")?
            .map_err(|_| "ws_read")?;
        match frame.opcode() {
            yawc::OpCode::Text => {
                return serde_json::from_slice(frame.payload()).map_err(|_| "ws_json".into())
            }
            yawc::OpCode::Close => return Err("ws_closed".into()),
            _ => {}
        }
    }
}
async fn send(socket: &mut Socket, body: Value) -> Result<(), String> {
    socket
        .send(yawc::Frame::text(body.to_string()))
        .await
        .map_err(|_| "ws_send".into())
}
async fn switching(
    url: &str,
    key: &str,
    model: &str,
    bridge: &AtomicBool,
) -> Result<Value, String> {
    let session = format!("live-switch-{}", uuid::Uuid::new_v4());
    let mut socket = socket(url, key, &session).await?;
    let mut results = Vec::new();
    for (http, marker) in [
        (false, "LIVE_WS_ONE"),
        (true, "LIVE_HTTP_BRIDGE"),
        (false, "LIVE_WS_TWO"),
    ] {
        bridge.store(http, Ordering::Release);
        let mut body = request(model, marker, 2048);
        body["type"] = json!("response.create");
        send(&mut socket, body).await?;
        let mut kinds = Vec::new();
        for _ in 0..256 {
            let event = next(&mut socket).await?;
            if let Some(kind) = event["type"].as_str() {
                kinds.push(kind.to_owned());
            }
            if terminal(&event) {
                let pass = response_text(&event).trim() == expected(model, marker);
                results.push(json!({"upstream_http":http,"pass":pass,"events":kinds,"error":error_code(&event),"synthetic_output_excerpt":response_text(&event).chars().take(240).collect::<String>()}));
                if !pass {
                    let _ = socket.close().await;
                    bridge.store(false, Ordering::Release);
                    return Ok(json!({"pass":false,"turns":results}));
                }
                break;
            }
        }
    }
    let _ = socket.close().await;
    bridge.store(false, Ordering::Release);
    Ok(
        json!({"pass":results.len()==3&&results.iter().all(|v|v["pass"]==true),"same_downstream_connection":true,"turns":results}),
    )
}
async fn steering(
    url: &str,
    key: &str,
    model: &str,
    disable_images: bool,
) -> Result<Value, String> {
    let session = format!("live-steer-{}", uuid::Uuid::new_v4());
    let mut socket = socket_policy(url, key, &session, disable_images).await?;
    let mut body = request(model, "LIVE_ORIGINAL", 0);
    body["type"] = json!("response.create");
    if !disable_images {
        body["tools"] = json!([{"type":"image_generation","output_format":"png"}]);
    }
    body["input"][0]["content"]=json!("Write a numbered list of the integers 1 through 80, each on a separate line. No other text.");
    send(&mut socket, body).await?;
    let mut parent = None;
    let mut accepted = false;
    let mut automatic = false;
    let mut kinds = Vec::new();
    let mut result: Option<Value> = None;
    let mut rejected = false;
    let mut untyped = Vec::new();
    for _ in 0..512 {
        let event = match next(&mut socket).await {
            Ok(event) => event,
            Err(code) if rejected => {
                result.as_mut().unwrap()["connection_outcome"] = json!(code);
                break;
            }
            Err(code) => return Err(code),
        };
        let kind = event["type"].as_str().unwrap_or("").to_owned();
        kinds.push(kind.clone());
        if kind.is_empty() {
            untyped.push(super::output_error::json_error(
                &ServiceError::new(502, "untyped_event", "Untyped upstream event")
                    .with_error_details(event.clone()),
            ));
        }
        if kind == "response.created" && parent.is_none() {
            let id = event
                .pointer("/response/id")
                .and_then(Value::as_str)
                .ok_or("missing_response_id")?
                .to_owned();
            parent = Some(id.clone());
            let input = if model.starts_with("grok-") {
                "Stop the list. What is 8 + 8? Answer with the number only."
            } else {
                "Stop the list. Reply with exactly LIVE_STEER_OK."
            };
            send(&mut socket,json!({"type":"response.steer","event_id":"live-steer-control","previous_response_id":id,"input":input})).await?;
        } else if kind == "response.created"
            && event.pointer("/response/id").and_then(Value::as_str) != parent.as_deref()
        {
            automatic = true;
        }
        if kind == "response.steer.accepted" {
            accepted = true;
        }
        if rejected && error_code(&event).as_deref() == Some("upstream_websocket_lost") {
            result.as_mut().unwrap()["connection_outcome"] = json!("upstream_websocket_lost");
            break;
        }
        if kind == "response.steer.failed" || kind == "error" {
            result = Some(
                json!({"pass":false,"accepted":accepted,"automatic_successor":automatic,"error":error_code(&event),"error_detail":event["error"],"event":kind}),
            );
            if matches!(
                error_code(&event).as_deref(),
                Some(
                    "unsupported_native_inflight_message"
                        | "steer_transport_unavailable"
                        | "steering_not_supported"
                )
            ) {
                result.as_mut().unwrap()["error_detail"] = event["error"].clone();
                rejected = true;
                continue;
            }
            break;
        }
        if rejected && terminal(&event) {
            result.as_mut().unwrap()["original_terminal"] = json!(kind);
            result.as_mut().unwrap()["original_completed"] = json!(matches!(
                kind.as_str(),
                "response.completed" | "response.done"
            ));
            break;
        }
        if terminal(&event) && automatic {
            result = Some(
                json!({"pass":accepted&&response_text(&event).trim()==expected(model,"LIVE_STEER_OK"),"accepted":accepted,"automatic_successor":automatic,"error":error_code(&event),"event":kind}),
            );
            break;
        }
    }
    if rejected {
        let mut body = request(model, "LIVE_AFTER_REJECTION", 0);
        body["type"] = json!("response.create");
        if send(&mut socket, body).await.is_ok() {
            for _ in 0..256 {
                let Ok(event) = next(&mut socket).await else {
                    break;
                };
                if terminal(&event) {
                    result.as_mut().unwrap()["next_independent_turn_pass"] = json!(
                        response_text(&event).trim() == expected(model, "LIVE_AFTER_REJECTION")
                    );
                    result.as_mut().unwrap()["next_independent_error"] = json!(error_code(&event));
                    break;
                }
            }
        }
    }
    let _ = socket.close().await;
    let mut result = result.unwrap_or(json!({"pass":false,"error":"no_steering_terminal"}));
    result["events"] = json!(kinds);
    result["untyped_events"] = json!(untyped);
    Ok(result)
}

pub(crate) async fn run(
    host: Arc<dyn Host>,
    key: &str,
    model: &str,
    output: &Path,
    bridge: Arc<AtomicBool>,
    phase: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut handle = super::start(
        "127.0.0.1:0".parse()?,
        host.clone(),
        output.join(format!("history-{}.sqlite", uuid::Uuid::new_v4())),
        Options::default(),
    )
    .await?;
    let http_url = format!("http://{}", handle.addr);
    let ws_url = format!("ws://{}/v1/responses", handle.addr);
    let mut evidence = json!({"model":model,"real_upstream":true,"cases":{}});
    let steering_only = matches!(phase, "steering" | "api-steering");
    let api = phase.starts_with("api");
    let save = |value: &Value| {
        std::fs::write(
            output.join("protocol.json"),
            serde_json::to_vec_pretty(value).unwrap(),
        )
    };
    evidence["cases"]["upstream_ws_negotiation"] =
        upstream_negotiation(host.clone(), key, model, phase == "wire")
            .await
            .unwrap_or_else(|code| json!({"pass":false,"error":code}));
    save(&evidence)?;
    if phase == "wire" {
        handle.shutdown(Duration::from_secs(10)).await?;
        return Ok(());
    }
    let baseline = if steering_only {
        true
    } else {
        let result = http(&http_url, key, model, "identity", 0)
            .await
            .unwrap_or_else(|code| json!({"pass":false,"error":code}));
        let baseline = result["protocol_ok"] == true;
        evidence["cases"]["http_baseline"] = result;
        save(&evidence)?;
        baseline
    };
    if baseline {
        if !steering_only {
            for encoding in ["gzip", "zstd"] {
                evidence["cases"][format!("http_compression_{encoding}")] =
                    http(&http_url, key, model, encoding, 4096)
                        .await
                        .unwrap_or_else(|code| json!({"pass":false,"error":code}));
                save(&evidence)?;
            }
            evidence["cases"]["websocket_http_switch"] = switching(&ws_url, key, model, &bridge)
                .await
                .unwrap_or_else(|code| json!({"pass":false,"error":code}));
            save(&evidence)?;
        }
        bridge.store(false, Ordering::Release);
        evidence["cases"]["steering"] = steering(&ws_url, key, model, api)
            .await
            .unwrap_or_else(|code| json!({"pass":false,"error":code}));
        save(&evidence)?;
        if evidence["cases"]["steering"]["pass"] == true && !api {
            evidence["cases"]["steering_without_image_tools"] = steering(&ws_url, key, model, true)
                .await
                .unwrap_or_else(|code| json!({"pass":false,"error":code}));
            save(&evidence)?;
            evidence["cases"]["steering_tool_result"] = steering_tool_result(&ws_url, key, model)
                .await
                .unwrap_or_else(|code| json!({"pass":false,"error":code}));
            save(&evidence)?;
        }
    }
    handle.shutdown(Duration::from_secs(10)).await?;
    Ok(())
}

async fn upstream_negotiation(
    host: Arc<dyn Host>,
    key: &str,
    model: &str,
    send_probe: bool,
) -> Result<Value, String> {
    let caller = host.authorize(key).await.map_err(|e| e.code.to_owned())?;
    let request = RouteRequest {
        model: model.to_owned(),
        session_id: None,
        thread_id: None,
        parent_thread_id: None,
        preferred_account_id: None,
        soft_preferred_account_id: None,
        target_account_id: None,
        force_refresh: false,
        image_generation_requested: false,
        image_input: false,
        excluded_account_ids: Vec::new(),
        requires_permit: true,
        auxiliary: Some("responses.websocket".into()),
        required_capabilities: Vec::new(),
        capability_fallback_from: None,
    };
    let route = host
        .route(&caller, request)
        .await
        .map_err(|e| e.code.to_owned())?;
    if route.wire_api != WireApi::Responses {
        return Ok(
            json!({"pass":false,"error":"upstream_protocol_has_no_responses_websocket","wire_api":format!("{:?}",route.wire_api),"inference_sent":false}),
        );
    }
    let mut headers = route.headers.clone();
    if route.chatgpt {
        headers.insert(
            "openai-beta",
            axum::http::HeaderValue::from_static("responses_websockets=2026-02-06"),
        );
    }
    let endpoint = route.websocket_endpoint.as_ref();
    let base = endpoint.map_or(route.base_url.as_str(), |endpoint| {
        endpoint.base_url.as_str()
    });
    if let Some(endpoint) = endpoint {
        for name in route.headers.keys() {
            headers.remove(name);
        }
        headers.extend(endpoint.headers.clone());
    }
    let (_sender, receiver) = tokio::sync::watch::channel(false);
    let connected = super::transport::connect_websocket(
        &route,
        &format!("{}/responses", base.trim_end_matches('/')),
        headers,
        4 * 1024 * 1024,
        Duration::from_secs(25),
        super::lifecycle::StopSignal(receiver),
    )
    .await;
    let (mut socket, headers) = match connected {
        Ok(value) => value,
        Err(error) => {
            return Ok(
                json!({"pass":false,"status":error.status,"error":error.code,"inference_sent":false}),
            )
        }
    };
    let extension = headers
        .get("sec-websocket-extensions")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    if send_probe {
        let mut result = raw_roundtrip(&mut socket, model).await?;
        result["negotiated_extensions"] = json!(extension);
        let _ = socket.close().await;
        return Ok(result);
    }
    let _ = socket.close().await;
    Ok(json!({"pass":true,"negotiated_extensions":extension,"inference_sent":false}))
}

async fn raw_roundtrip(
    socket: &mut super::websocket_io::UpstreamSocket,
    model: &str,
) -> Result<Value, String> {
    use tokio_tungstenite::tungstenite::Message as UpstreamMessage;
    let mut body = request(model, "LIVE_WIRE_OK", 0);
    body["type"] = json!("response.create");
    body["instructions"] = json!("Follow the user's request precisely.");
    body["store"] = json!(false);
    socket
        .send(UpstreamMessage::Text(body.to_string().into()))
        .await
        .map_err(|_| "probe_send")?;
    let mut kinds = Vec::new();
    for _ in 0..128 {
        let message = tokio::time::timeout(Duration::from_secs(35), socket.next())
            .await
            .map_err(|_| "probe_timeout")?;
        match message {
            Some(Ok(UpstreamMessage::Text(text))) => {
                let event: Value = serde_json::from_str(&text).map_err(|_| "probe_json")?;
                kinds.push(event["type"].clone());
                if terminal(&event) {
                    return Ok(
                        json!({"pass":response_text(&event).trim()=="LIVE_WIRE_OK","error":error_code(&event),"error_detail":event["error"],"events":kinds,"inference_sent":true}),
                    );
                }
            }
            Some(Ok(UpstreamMessage::Close(frame))) => {
                return Ok(
                    json!({"pass":false,"events":kinds,"close_code":frame.as_ref().map(|f|u16::from(f.code)),"close_reason":frame.map(|f|f.reason.to_string()),"inference_sent":true}),
                )
            }
            Some(Ok(UpstreamMessage::Ping(bytes))) => {
                socket
                    .send(UpstreamMessage::Pong(bytes))
                    .await
                    .map_err(|_| "probe_pong")?;
            }
            Some(Err(error)) => {
                return Ok(
                    json!({"pass":false,"events":kinds,"read_error":error.to_string(),"inference_sent":true}),
                )
            }
            None => {
                return Ok(
                    json!({"pass":false,"events":kinds,"error":"probe_eof","inference_sent":true}),
                )
            }
            _ => {}
        }
    }
    Err("probe_event_limit".into())
}

async fn steering_tool_result(url: &str, key: &str, model: &str) -> Result<Value, String> {
    let session = format!("live-tool-steer-{}", uuid::Uuid::new_v4());
    let mut socket = socket(url, key, &session).await?;
    let body = json!({"type":"response.create","model":model,"input":[{"role":"user","content":"Call live_gate exactly once with {} and wait for its result. Do not use image tools."}],"tools":[{"type":"function","name":"live_gate","description":"Synthetic protocol test; has no side effects.","parameters":{"type":"object","properties":{},"additionalProperties":false}},{"type":"image_generation","output_format":"png"}],"tool_choice":{"type":"function","name":"live_gate"},"reasoning":{"effort":"low"}});
    let tools = body["tools"].clone();
    send(&mut socket, body).await?;
    let mut parent = None;
    let mut call = None;
    let mut accepted = false;
    let mut submitted = false;
    let mut kinds = Vec::new();
    let result = loop {
        let event = next(&mut socket).await?;
        let kind = event["type"].as_str().unwrap_or("").to_owned();
        kinds.push(kind.clone());
        if kinds.len() > 512 {
            break json!({"pass":false,"error":"event_limit"});
        }
        if kind == "response.created" && parent.is_none() {
            let id = event
                .pointer("/response/id")
                .and_then(Value::as_str)
                .ok_or("missing_parent")?
                .to_owned();
            parent = Some(id.clone());
            send(&mut socket,json!({"type":"response.steer","event_id":"live-tool-steer","previous_response_id":id,"input":"After the pending tool result, reply with exactly LIVE_TOOL_STEER_OK."})).await?;
        }
        if kind == "response.steer.accepted" {
            accepted = true;
        }
        if event.pointer("/item/type").and_then(Value::as_str) == Some("function_call") {
            call = event
                .pointer("/item/call_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        if let Some(output) = event.pointer("/response/output").and_then(Value::as_array) {
            for item in output {
                if item["type"] == "function_call" {
                    call = item["call_id"].as_str().map(str::to_owned);
                }
            }
        }
        if kind == "response.steer.pending" && !submitted {
            if call.is_none() {
                call = event["required_input"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find_map(|v| v["call_id"].as_str())
                    .map(str::to_owned);
            }
            let call = call.as_ref().ok_or("missing_tool_call")?;
            let parent = event
                .pointer("/steer/previous_response_id")
                .and_then(Value::as_str)
                .or(parent.as_deref())
                .ok_or("missing_parent")?;
            send(&mut socket,json!({"type":"response.create","model":model,"previous_response_id":parent,"tools":tools,"tool_choice":"none","input":[{"type":"function_call_output","call_id":call,"output":"{\"ok\":true}"}]})).await?;
            submitted = true;
        }
        if kind == "error" || kind == "response.steer.failed" {
            break json!({"pass":false,"accepted":accepted,"tool_result_sent":submitted,"error":error_code(&event),"error_detail":event["error"]});
        }
        if submitted && terminal(&event) {
            break json!({"pass":accepted&&response_text(&event).trim()=="LIVE_TOOL_STEER_OK","accepted":accepted,"tool_result_sent":submitted,"error":error_code(&event)});
        }
    };
    let _ = socket.close().await;
    let mut result = result;
    result["events"] = json!(kinds);
    Ok(result)
}
