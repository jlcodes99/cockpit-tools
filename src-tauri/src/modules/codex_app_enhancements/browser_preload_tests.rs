use super::*;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

struct Mock {
    port: u16,
    commands: Arc<Mutex<Vec<Value>>>,
    event: mpsc::UnboundedSender<Value>,
    task: tokio::task::JoinHandle<()>,
}
async fn mock(reject: Option<&'static str>, no_ack: bool, delay: Duration) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let commands = Arc::new(Mutex::new(Vec::new()));
    let captured = commands.clone();
    let (event, mut events) = mpsc::unbounded_channel::<Value>();
    let task = tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0u8; 2048];
        socket.read(&mut request).await.unwrap();
        let body =
            json!({"webSocketDebuggerUrl":format!("ws://127.0.0.1:{port}/devtools/browser/mock")})
                .to_string();
        let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}", body.len());
        socket.write_all(response.as_bytes()).await.unwrap();
        drop(socket);
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        loop {
            tokio::select! {
                event = events.recv() => {
                    let Some(event) = event else { break; };
                    if socket.send(Message::Text(event.to_string().into())).await.is_err() { break; }
                }
                command = socket.next() => {
                    let Some(Ok(Message::Text(text))) = command else { break; };
                    let command: Value = serde_json::from_str(&text).unwrap();
                    captured.lock().unwrap().push(command.clone());
                    if no_ack { continue; }
                    let method = command["method"].as_str().unwrap();
                    let result = if method == "Page.addScriptToEvaluateOnNewDocument" {
                        json!({"identifier":format!("script-{}",command["id"])})
                    } else { json!({}) };
                    let response = if reject == Some(method) {
                        json!({"id":command["id"],"error":{"code":-32000,"message":"mock denied"}})
                    } else { json!({"id":command["id"],"result":result}) };
                    if socket.send(Message::Text(response.to_string().into())).await.is_err() { break; }
                }
            }
        }
    });
    Mock {
        port,
        commands,
        event,
        task,
    }
}
fn attached(session: &str, kind: &str, url: &str, waiting: bool) -> Value {
    json!({"method":"Target.attachedToTarget","params":{"sessionId":session,"waitingForDebugger":waiting,
        "targetInfo":{"type":kind,"url":url,"targetId":format!("target-{session}")}}})
}
async fn wait_for(mock: &Mock, method: &str, count: usize) -> Vec<Value> {
    timeout(Duration::from_secs(3), async {
        loop {
            let found: Vec<_> = mock
                .commands
                .lock()
                .unwrap()
                .iter()
                .filter(|v| v["method"] == method)
                .cloned()
                .collect();
            if found.len() >= count {
                return found;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
fn spawn(
    mock: &Mock,
    source: watch::Receiver<Option<String>>,
    stop: watch::Receiver<bool>,
    window: Duration,
) -> tokio::task::JoinHandle<Result<(), String>> {
    let port = mock.port;
    tokio::spawn(run_bounded(
        port,
        source,
        stop,
        window,
        Duration::from_millis(300),
    ))
}
fn assert_never_paused_or_navigated(mock: &Mock) {
    for command in mock.commands.lock().unwrap().iter() {
        assert!(!matches!(
            command["method"].as_str(),
            Some("Page.reload" | "Page.navigate" | "Debugger.pause")
        ));
        if command["method"] == "Runtime.evaluate" {
            assert_eq!(
                command["params"]["expression"],
                revoke_source(&format!("preload-{}", mock.port)),
                "only the exact owner-guarded teardown is permitted in an existing document"
            );
            assert_eq!(command["params"]["timeout"], 250);
        }
        if command["method"] == "Target.setAutoAttach" {
            assert_eq!(command["params"]["waitForDebuggerOnStart"], false);
        }
    }
}

#[test]
fn endpoints_are_exact_instance_loopback_and_targets_are_local_documents() {
    assert!(safe_socket(
        "ws://127.0.0.1:9233/devtools/browser/one",
        9233
    ));
    for value in [
        "ws://localhost:9233/devtools/browser/one",
        "wss://127.0.0.1:9233/devtools/browser/one",
        "ws://127.0.0.1:9234/devtools/browser/one",
        "ws://user@127.0.0.1:9233/devtools/browser/one",
        "ws://127.0.0.1:9233/devtools/browser/one?q=1",
        "ws://127.0.0.1:9233/devtools/browser/one#x",
        "ws://127.0.0.1:9233/devtools/page/one",
        "ws://127.0.0.1:9233/devtools/browser/",
    ] {
        assert!(!safe_socket(value, 9233), "{value}");
    }
    for value in [
        "",
        "about:blank",
        "app://-/index.html",
        "app://-/index.html?windowId=one",
    ] {
        assert!(local_target(&json!({"type":"page","url":value})));
    }
    for value in [
        "https://example.test",
        "app://evil/index.html",
        "app://-/other.html",
        "file:///index.html",
    ] {
        assert!(!local_target(&json!({"type":"page","url":value})));
    }
    assert!(!local_target(
        &json!({"type":"worker","url":"app://-/index.html"})
    ));
}

#[tokio::test]
async fn waits_for_trusted_profile_without_blocking_target_and_registers_complete_source_once() {
    let mock = mock(None, false, Duration::ZERO).await;
    let (source, receiver) = watch::channel(None);
    let (stop, stopping) = watch::channel(false);
    let task = spawn(&mock, receiver, stopping, Duration::from_secs(5));
    wait_for(&mock, "Target.setAutoAttach", 1).await;
    mock.event
        .send(attached("one", "page", "about:blank", false))
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!mock
        .commands
        .lock()
        .unwrap()
        .iter()
        .any(|v| v["method"] == "Page.addScriptToEvaluateOnNewDocument"));
    let script =
        super::super::fast::browser_preload_source(true, false, true, true, "test-preload");
    source.send(Some(script.clone())).unwrap();
    let adds = wait_for(&mock, "Page.addScriptToEvaluateOnNewDocument", 1).await;
    let installed = adds[0]["params"]["source"].as_str().unwrap();
    assert!(installed.contains("__cockpitStartupPreloadStarted"));
    assert!(installed.contains("document-already-started"));
    assert!(installed.contains("location.hostname !== '-'"));
    assert!(installed.contains(&script));
    source.send(Some(script)).unwrap();
    mock.event
        .send(attached("one", "page", "about:blank", false))
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(
        mock.commands
            .lock()
            .unwrap()
            .iter()
            .filter(|v| v["method"] == "Page.addScriptToEvaluateOnNewDocument")
            .count(),
        1
    );
    stop.send(true).unwrap();
    task.await.unwrap().unwrap();
    wait_for(&mock, "Page.removeScriptToEvaluateOnNewDocument", 1).await;
    wait_for(&mock, "Runtime.evaluate", 1).await;
    assert_never_paused_or_navigated(&mock);
    mock.task.abort();
}

#[tokio::test]
async fn late_browser_and_already_loaded_target_never_trigger_reload_or_unprotected_evaluation() {
    let mock = mock(None, false, Duration::from_millis(80)).await;
    let (_source, receiver) =
        watch::channel(Some("/* trusted bootstrap and guarded preload */".into()));
    let (stop, stopping) = watch::channel(false);
    let task = spawn(&mock, receiver, stopping, Duration::from_secs(5));
    wait_for(&mock, "Target.setAutoAttach", 1).await;
    mock.event
        .send(attached("late", "page", "app://-/index.html", false))
        .unwrap();
    wait_for(&mock, "Page.addScriptToEvaluateOnNewDocument", 1).await;
    stop.send(true).unwrap();
    task.await.unwrap().unwrap();
    assert_never_paused_or_navigated(&mock);
    mock.task.abort();
}

#[tokio::test]
async fn rejects_unsupported_targets_and_resumes_unexpected_paused_workers() {
    let mock = mock(None, false, Duration::ZERO).await;
    let (_source, receiver) = watch::channel(Some("source".into()));
    let (stop, stopping) = watch::channel(false);
    let task = spawn(&mock, receiver, stopping, Duration::from_secs(5));
    wait_for(&mock, "Target.setAutoAttach", 1).await;
    mock.event
        .send(attached("worker", "worker", "app://-/index.html", true))
        .unwrap();
    mock.event
        .send(attached("remote", "page", "https://example.test", false))
        .unwrap();
    wait_for(&mock, "Runtime.runIfWaitingForDebugger", 1).await;
    wait_for(&mock, "Target.detachFromTarget", 2).await;
    assert!(!mock
        .commands
        .lock()
        .unwrap()
        .iter()
        .any(|v| v["method"] == "Page.addScriptToEvaluateOnNewDocument"));
    stop.send(true).unwrap();
    task.await.unwrap().unwrap();
    assert_never_paused_or_navigated(&mock);
    mock.task.abort();
}

#[tokio::test]
async fn source_revocation_removes_registration_without_installing_fake_full_configuration() {
    let mock = mock(None, false, Duration::ZERO).await;
    let (source, receiver) = watch::channel(Some("verified".into()));
    let (stop, stopping) = watch::channel(false);
    let task = spawn(&mock, receiver, stopping, Duration::from_secs(5));
    wait_for(&mock, "Target.setAutoAttach", 1).await;
    mock.event
        .send(attached("page", "page", "about:blank", false))
        .unwrap();
    wait_for(&mock, "Page.addScriptToEvaluateOnNewDocument", 1).await;
    source.send(None).unwrap();
    wait_for(&mock, "Page.removeScriptToEvaluateOnNewDocument", 1).await;
    let revoked = wait_for(&mock, "Runtime.evaluate", 1).await;
    assert!(revoked.iter().all(|command| command["sessionId"] == "page"));
    stop.send(true).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(
        mock.commands
            .lock()
            .unwrap()
            .iter()
            .filter(|v| v["method"] == "Page.addScriptToEvaluateOnNewDocument")
            .count(),
        1
    );
    mock.task.abort();
}

#[tokio::test]
async fn rejected_registration_exits_without_reload_or_client_pause() {
    let mock = mock(
        Some("Page.addScriptToEvaluateOnNewDocument"),
        false,
        Duration::ZERO,
    )
    .await;
    let (_source, receiver) = watch::channel(Some("verified".into()));
    let (_stop, stopping) = watch::channel(false);
    let task = spawn(&mock, receiver, stopping, Duration::from_secs(5));
    wait_for(&mock, "Target.setAutoAttach", 1).await;
    mock.event
        .send(attached("page", "page", "about:blank", false))
        .unwrap();
    assert!(timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err()
        .contains("rejected"));
    assert_never_paused_or_navigated(&mock);
    mock.task.abort();
}

#[tokio::test]
async fn unresponsive_cdp_commands_and_missing_endpoint_are_bounded() {
    let mock = mock(None, true, Duration::ZERO).await;
    let (_source, receiver) = watch::channel(None);
    let (_stop, stopping) = watch::channel(false);
    let task = spawn(&mock, receiver, stopping, Duration::from_secs(5));
    assert!(timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err()
        .contains("timed out"));
    assert_never_paused_or_navigated(&mock);
    mock.task.abort();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (_source, receiver) = watch::channel(None);
    let (_stop, stopping) = watch::channel(false);
    let result = timeout(
        Duration::from_secs(1),
        run_bounded(
            port,
            receiver,
            stopping,
            Duration::from_millis(100),
            COMMAND,
        ),
    )
    .await
    .unwrap();
    assert!(result.unwrap_err().contains("discovery timed out"));
}

#[tokio::test]
async fn stop_interrupts_discovery_and_initial_stopped_launch_does_no_work() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (_source, receiver) = watch::channel(None);
    let (stop, stopping) = watch::channel(false);
    let task = tokio::spawn(run(port, receiver, stopping, Duration::from_secs(5)));
    stop.send(true).unwrap();
    timeout(Duration::from_millis(500), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let (_source, receiver) = watch::channel(None);
    let (_stop, stopping) = watch::channel(true);
    timeout(
        Duration::from_millis(100),
        run(port, receiver, stopping, Duration::from_secs(5)),
    )
    .await
    .unwrap()
    .unwrap();
}

#[tokio::test]
async fn launch_window_expiry_cleans_registration_without_holding_client_open() {
    let mock = mock(None, false, Duration::ZERO).await;
    let (_source, receiver) = watch::channel(Some("verified".into()));
    let (_stop, stopping) = watch::channel(false);
    let task = spawn(&mock, receiver, stopping, Duration::from_millis(250));
    wait_for(&mock, "Target.setAutoAttach", 1).await;
    mock.event
        .send(attached("page", "page", "about:blank", false))
        .unwrap();
    wait_for(&mock, "Page.addScriptToEvaluateOnNewDocument", 1).await;
    timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    wait_for(&mock, "Page.removeScriptToEvaluateOnNewDocument", 1).await;
    assert_never_paused_or_navigated(&mock);
    wait_for(&mock, "Runtime.evaluate", 1).await;
    mock.task.abort();
}

#[tokio::test]
async fn verified_configuration_change_replaces_script_and_detached_session_is_not_reused() {
    let mock = mock(None, false, Duration::ZERO).await;
    let (source, receiver) = watch::channel(Some("verified-one".into()));
    let (stop, stopping) = watch::channel(false);
    let task = spawn(&mock, receiver, stopping, Duration::from_secs(5));
    wait_for(&mock, "Target.setAutoAttach", 1).await;
    mock.event
        .send(attached("page", "page", "about:blank", false))
        .unwrap();
    wait_for(&mock, "Page.addScriptToEvaluateOnNewDocument", 1).await;
    source.send(Some("verified-two".into())).unwrap();
    let adds = wait_for(&mock, "Page.addScriptToEvaluateOnNewDocument", 2).await;
    assert!(adds[1]["params"]["source"]
        .as_str()
        .unwrap()
        .contains("verified-two"));
    wait_for(&mock, "Page.removeScriptToEvaluateOnNewDocument", 1).await;
    mock.event
        .send(json!({"method":"Target.detachedFromTarget","params":{"sessionId":"page"}}))
        .unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    source.send(Some("verified-three".into())).unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(
        mock.commands
            .lock()
            .unwrap()
            .iter()
            .filter(|v| v["method"] == "Page.addScriptToEvaluateOnNewDocument")
            .count(),
        2
    );
    stop.send(true).unwrap();
    task.await.unwrap().unwrap();
    assert_never_paused_or_navigated(&mock);
    mock.task.abort();
}
