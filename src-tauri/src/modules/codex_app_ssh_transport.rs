//! Evaluate once and await the promise; never register a navigation hook.
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::{connect_async, tungstenite::Message};

pub(super) async fn evaluate(
    url: &str,
    script: &str,
    cancelled: impl std::future::Future<Output = ()>,
) -> Result<Value, String> {
    tokio::select! {
        result = evaluate_once(url, script) => result,
        _ = cancelled => Err("ssh_sync_superseded".into()),
    }
}

async fn evaluate_once(url: &str, script: &str) -> Result<Value, String> {
    let (mut socket, _) = connect_async(url).await.map_err(|_| "bridge_unavailable")?;
    socket
        .send(Message::Text(
            json!({"id":1,"method":"Runtime.evaluate","params":{
                "expression":script,"returnByValue":true,"awaitPromise":true
            }})
            .to_string()
            .into(),
        ))
        .await
        .map_err(|_| "bridge_failed")?;
    while let Some(message) = socket.next().await {
        let Ok(Message::Text(text)) = message else {
            continue;
        };
        let value: Value = serde_json::from_str(&text).map_err(|_| "bridge_invalid_response")?;
        if value["id"] != 1 {
            continue;
        }
        if value.get("error").is_some() || value.pointer("/result/exceptionDetails").is_some() {
            return Err("bridge_failed".into());
        }
        return value
            .pointer("/result/result/value")
            .cloned()
            .ok_or_else(|| "bridge_invalid_response".into());
    }
    Err("bridge_closed".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    async fn exchange(response: Value) -> Result<Value, String> {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let Message::Text(text) = socket.next().await.unwrap().unwrap() else {
                panic!("expected command");
            };
            let command: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(command["method"], "Runtime.evaluate");
            assert_eq!(command["params"]["awaitPromise"], true);
            assert_eq!(command["params"]["expression"], "fixture reconnect");
            socket
                .send(Message::Text(response.to_string().into()))
                .await
                .unwrap();
        });
        let result = evaluate(&url, "fixture reconnect", std::future::pending()).await;
        server.await.unwrap();
        result
    }

    #[tokio::test]
    async fn reconnect_uses_one_shot_promise_evaluation_without_installing_future_hooks() {
        let value = exchange(json!({"id":1,"result":{"result":{"value":{"ok":true}}}}))
            .await
            .unwrap();
        assert_eq!(value["ok"], true);
    }

    #[tokio::test]
    async fn native_script_exceptions_are_not_reported_as_success() {
        assert_eq!(
            exchange(json!({"id":1,"result":{"exceptionDetails":{"text":"fixture failure"}}}))
                .await
                .unwrap_err(),
            "bridge_failed"
        );
    }

    #[tokio::test]
    async fn superseded_account_cancels_a_pending_native_refresh() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let (sent, received) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            socket.next().await.unwrap().unwrap();
            sent.send(()).unwrap();
            // Dropping the cancelled request releases the socket as well.
            let _ = socket.next().await;
        });
        let value = evaluate(&url, "fixture reconnect", async {
            received.await.unwrap();
        })
        .await;
        assert_eq!(value.unwrap_err(), "ssh_sync_superseded");
        tokio::time::timeout(std::time::Duration::from_secs(1), server)
            .await
            .unwrap()
            .unwrap();
    }
}
