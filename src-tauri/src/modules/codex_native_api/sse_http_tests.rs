#[tokio::test]
async fn repaired_sse_terminal_matches_history_ledger_for_streaming_and_json_clients() {
    for stream in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("identity.sqlite");
        let payload = concat!(
            "event: response.created\ndata: {\"response\":{\"id\":\"framed-response\",\"status\":\"in_progress\"}}\n\n",
            "event: response.output_item.done\ndata: {\"output_index\":1,\"item\":{\"type\":\"reasoning\",\"encrypted_content\":\"framed-cipher\",\"extension\":true}}\n\n",
            "event: response.completed\ndata: {\"response\":{\"id\":\"framed-response\",\"status\":\"completed\",\"output\":[],\"extension\":17,\"usage\":{\"input_tokens\":2,\"output_tokens\":3}}}"
        );
        let (host, _, task) = fixture(None, payload.into()).await;
        let mut service = native::start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            path.clone(),
            Options::default(),
        )
        .await
        .unwrap();
        let mut body = request();
        body["stream"] = json!(stream);
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(format!("http://{}/v1/responses", service.addr))
            .bearer_auth("key")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let bytes = response.bytes().await.unwrap();
        let final_response = if stream {
            let mut decoder = super::super::events::SseDecoder::new(4096);
            let events = decoder.push(&bytes).unwrap();
            assert_eq!(events.len(), 3);
            events[2]["response"].clone()
        } else {
            serde_json::from_slice::<Value>(&bytes).unwrap()
        };
        assert_eq!(
            final_response["output"][0]["encrypted_content"],
            "framed-cipher"
        );
        assert_eq!(final_response["output"][0]["extension"], true);
        assert_eq!(final_response["extension"], 17);
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        let connection = rusqlite::Connection::open(path).unwrap();
        for kind in ["response", "parent_response", "encrypted"] {
            let count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM native_opaque_v1 WHERE kind=?1",
                    [kind],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1);
        }
        let reports = host.reports.lock().unwrap();
        assert_eq!(reports.len(), 1);
        assert!(reports[0].success);
        assert_eq!(reports[0].usage.as_ref().unwrap()["input_tokens"], 2);
        task.abort();
    }
}

#[tokio::test]
async fn automatic_sse_repaired_terminal_never_adopts_unproven_input_history() {
    for successful in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("identity.sqlite");
        let end = if successful {
            "event: response.completed\ndata: {\"response\":{\"id\":\"recovered-response\",\"status\":\"completed\",\"output\":[]}}"
        } else {
            "event: error\ndata: {\"error\":{\"code\":\"invalid_grant\",\"message\":\"fixture credential rejected\"}}\n\nevent: response.completed\ndata: {\"response\":{\"id\":\"recovered-response\",\"status\":\"completed\",\"output\":[]}}\n\n"
        };
        let payload = format!("event: response.output_item.done\ndata: {{\"item\":{{\"type\":\"reasoning\",\"encrypted_content\":\"new-cipher\"}}}}\n\n{end}");
        let (host, source, task) = fixture(Some("text/event-stream"), payload).await;
        let mut service = native::start(
            "127.0.0.1:0".parse().unwrap(),
            host.clone(),
            path.clone(),
            Options::default(),
        )
        .await
        .unwrap();
        let mut body = request();
        body["input"] = json!([{ "type":"reasoning", "encrypted_content":"old-cipher" }]);
        let response = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .post(format!("http://{}/v1/responses", service.addr))
            .bearer_auth("key")
            .json(&body)
            .send()
            .await
            .unwrap();
        let bytes = response.bytes().await.unwrap();
        let mut decoder = super::super::events::SseDecoder::new(4096);
        let events = decoder.push(&bytes).unwrap();
        assert_eq!(
            events.last().unwrap()["type"],
            if successful {
                "response.completed"
            } else {
                "error"
            }
        );
        assert_eq!(source.requests.lock().unwrap().len(), 1);
        service.shutdown(Duration::from_secs(2)).await.unwrap();
        let connection = rusqlite::Connection::open(&path).unwrap();
        use sha2::{Digest, Sha256};
        let digest = format!("{:x}", Sha256::digest(b"old-cipher"));
        let adopted: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM native_opaque_v1 WHERE kind='encrypted' AND digest=?1",
                [digest],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            adopted, 0,
            "presentation repair cannot establish input ownership"
        );
        let reports = host.reports.lock().unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].success, successful);
        if !successful {
            assert!(reports[0].credential_scope);
            assert!(reports[0]
                .error_message
                .as_ref()
                .unwrap()
                .contains("fixture credential rejected"));
        }
        task.abort();
    }
}
