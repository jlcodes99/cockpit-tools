//! Buffered and streaming auxiliary failures retain classification without body logs.
use super::*;

#[tokio::test]
async fn auxiliary_audio_and_realtime_errors_preserve_request_scope_and_auth_type() {
    for operation in ["oauth_audio", "api_key_audio", "realtime_call"] {
        for (status,raw,request_scoped,credential_scope,challenge) in [
            (401,r#"{"error":{"code":"invalid_request_error","message":"Malformed request"}}"#,true,false,false),
            (403,r#"{"error":{"code":"cyber_policy","type":"invalid_request_error","message":"Rejected prompt"}}"#,true,false,false),
            (401,r#"{"error":{"code":"invalid_request_error","type":"authentication_error","message":"Expired credential"}}"#,false,true,false),
            (429,r#"{"error":{"code":"invalid_request_error","message":"Quota exhausted"}}"#,false,false,false),
            (403,r#"{"error":{"code":"invalid_request_error","message":"Rejected request"}}"#,false,false,true),
            (404,"Item with id fixture-item not found. Items are not persisted when `store` is set to false.",true,false,false),
        ] {
            let path=match operation{"oauth_audio"=>"/v1/transcribe","api_key_audio"=>"/v1/audio/speech",_=>"/v1/realtime/calls"};
            let app=Router::new().route(path,post(move||async move{
                // Split inside the error identifier: EOF classification must
                // examine the full bounded error body, not individual chunks.
                let midpoint=raw.len()/2;
                let chunks=vec![Ok::<_,std::io::Error>(axum::body::Bytes::copy_from_slice(&raw.as_bytes()[..midpoint])),Ok(axum::body::Bytes::copy_from_slice(&raw.as_bytes()[midpoint..]))];
                let mut response=axum::http::Response::builder().status(status).header("content-type","application/json").header("x-request-id","auxiliary-error-fixture").header("retry-after","7");
                if challenge{response=response.header("cf-mitigated","challenge");}
                response.body(Body::from_stream(stream::iter(chunks))).unwrap()
            }));
            let oauth=operation!="api_key_audio";
            let (handle,host,upstream,root)=fixture(app,oauth,Options::default()).await;
            let request=client().post(format!("http://{}{}",handle.addr,match operation{"oauth_audio"=>"/v1/audio/transcriptions","api_key_audio"=>"/v1/audio/speech",_=>"/v1/realtime/calls"})).bearer_auth("local-a");
            let request=match operation{
                "oauth_audio"=>request.multipart(reqwest::multipart::Form::new().text("model","client-model").part("file",reqwest::multipart::Part::bytes(vec![0,255]).file_name("voice.wav"))),
                "api_key_audio"=>request.json(&json!({"model":"client-model","input":"speech fixture"})),
                _=>request.json(&json!({"sdp":"v=0","session":{"model":"client-model"}})),
            };
            let response=request.send().await.unwrap();
            assert_eq!(response.status().as_u16(),status,"{operation}");
            assert_eq!(response.headers()["x-request-id"],"auxiliary-error-fixture");
            assert_eq!(response.headers()["retry-after"],"7");
            assert_eq!(response.bytes().await.unwrap().as_ref(),raw.as_bytes());
            tokio::time::timeout(Duration::from_secs(2),async{while host.reports.lock().unwrap().is_empty(){tokio::task::yield_now().await;}}).await.unwrap();
            wait_free(&host).await;
            let reports=host.reports.lock().unwrap();let report=reports.last().unwrap();
            assert_eq!(report.status,status);
            assert!(!report.success);
            assert_eq!(report.credential_scope,credential_scope,"{operation} status={status} raw={raw}");
            assert_eq!(report.retry_after,Some(Duration::from_secs(7)));
            assert_eq!(report.diagnostics.as_ref().unwrap().upstream_request_scoped,request_scoped,"{operation} status={status} raw={raw}");
            assert!(report.diagnostics.as_ref().unwrap().payloads.is_empty());
            if challenge{assert_eq!(report.error_message.as_deref(),Some("cloudflare challenge"));}
            drop(reports);
            assert_eq!(host.selections.lock().unwrap().len(),1);
            cleanup(handle,upstream,root).await;
        }
    }
}
