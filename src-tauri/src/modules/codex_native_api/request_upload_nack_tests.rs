//! Preserve the reusable reqwest body's protocol-only NACK recovery after
//! installing progress observation. All peers are loopback h2 servers.
use super::*;
use crate::modules::codex_native_api::{deadline::Deadline, request_upload::Opening};
use std::sync::atomic::AtomicUsize;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    task::JoinSet,
    time::Instant,
};

#[derive(Clone, Copy)]
enum Failure {
    Reset(h2::Reason),
    DelayedReset(h2::Reason, Duration),
    GoAway,
    Disconnect,
}

struct Peer {
    addr: SocketAddr,
    task: JoinHandle<()>,
    calls: Arc<AtomicUsize>,
    completed: Arc<AtomicUsize>,
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Peer {
    async fn start(failure: Failure, failures: usize, response_delay: Duration) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let completed = Arc::new(AtomicUsize::new(0));
        let (seen, done) = (calls.clone(), completed.clone());
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (mut socket, _) = accepted.unwrap();
                        let (seen, done) = (seen.clone(), done.clone());
                        // A GOAWAY with last-stream-id 0 proves the peer did not
                        // process the first request. Frame only this exact case;
                        // the next connection uses the real h2 implementation.
                        if matches!(failure, Failure::GoAway) && seen.load(Ordering::SeqCst) < failures {
                            connections.spawn(async move {
                                let mut preface = [0; 24];
                                if socket.read_exact(&mut preface).await.is_err() { return; }
                                assert_eq!(&preface, b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n");
                                socket.write_all(&[0,0,0,4,0,0,0,0,0]).await.unwrap();
                                loop {
                                    let mut header = [0; 9];
                                    if socket.read_exact(&mut header).await.is_err() { return; }
                                    let len = ((header[0] as usize)<<16) | ((header[1] as usize)<<8) | header[2] as usize;
                                    let mut body = vec![0; len];
                                    if socket.read_exact(&mut body).await.is_err() { return; }
                                    if header[3] == 4 && header[4] & 1 == 0 {
                                        socket.write_all(&[0,0,0,4,1,0,0,0,0]).await.unwrap();
                                    }
                                    if header[3] == 1 {
                                        seen.fetch_add(1, Ordering::SeqCst);
                                        let mut frame = vec![0,0,8,7,0,0,0,0,0];
                                        frame.extend_from_slice(&[0;8]);
                                        socket.write_all(&frame).await.unwrap();
                                        socket.flush().await.unwrap();
                                        tokio::time::sleep(Duration::from_millis(30)).await;
                                        return;
                                    }
                                }
                            });
                        } else {
                            connections.spawn(async move {
                                let Ok(mut connection) = h2::server::handshake(socket).await else { return; };
                                let mut streams = JoinSet::new();
                                loop {
                                    tokio::select! {
                                        stream = connection.accept() => {
                                            let Some(Ok((request, mut respond))) = stream else { break; };
                                            let index = seen.fetch_add(1, Ordering::SeqCst);
                                            if index < failures {
                                                match failure {
                                                    Failure::Reset(reason) => { respond.send_reset(reason); continue; },
                                                    Failure::DelayedReset(reason, delay) => {
                                                        streams.spawn(async move {
                                                            tokio::time::sleep(delay).await;
                                                            respond.send_reset(reason);
                                                        });
                                                        continue;
                                                    },
                                                    Failure::Disconnect => break,
                                                    Failure::GoAway => unreachable!("handled before handshake"),
                                                }
                                            }
                                            let done = done.clone();
                                            streams.spawn(async move {
                                                let expected = request.headers().get("content-length").map(|v|v.to_str().unwrap().parse::<usize>().unwrap());
                                                let mut body = request.into_body();
                                                let mut bytes = 0;
                                                while let Some(data) = body.data().await {
                                                    let Ok(data) = data else { return; };
                                                    bytes += data.len();
                                                    body.flow_control().release_capacity(data.len()).unwrap();
                                                }
                                                assert_eq!(Some(bytes), expected);
                                                done.fetch_add(1, Ordering::SeqCst);
                                                tokio::time::sleep(response_delay).await;
                                                let response = axum::http::Response::builder().status(200).body(()).unwrap();
                                                let _ = respond.send_response(response, true);
                                            });
                                        },
                                        _ = streams.join_next(), if !streams.is_empty() => {},
                                    }
                                }
                            });
                        }
                    },
                    _ = connections.join_next(), if !connections.is_empty() => {},
                }
            }
        });
        Self {
            addr,
            task,
            calls,
            completed,
        }
    }

    async fn send(
        &self,
        wrapped: bool,
        size: usize,
        timeout: Option<Duration>,
    ) -> Result<reqwest::Response, reqwest::Error> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .http2_prior_knowledge()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let mut request = client
            .post(format!("http://{}/responses", self.addr))
            .body(vec![b'x'; size])
            .build()
            .unwrap();
        *request.timeout_mut() = timeout;
        let opening = Opening::new(
            Instant::now() + Duration::from_secs(5),
            Deadline::NONE,
            Duration::from_secs(5),
        );
        let result = if wrapped {
            opening
                .run(opening.execute(&client, request, true))
                .await
                .unwrap()
        } else {
            client.execute(request).await
        };
        result
    }
}

#[tokio::test]
async fn upload_nack_refused_stream_and_goaway_preserve_reqwest_baseline() {
    for failure in [Failure::Reset(h2::Reason::REFUSED_STREAM), Failure::GoAway] {
        for wrapped in [false, true] {
            let peer = Peer::start(failure, 2, Duration::ZERO).await;
            let response = tokio::time::timeout(
                Duration::from_secs(8),
                peer.send(wrapped, 8 * 1024 * 1024, None),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(peer.calls.load(Ordering::SeqCst), 3);
            assert_eq!(peer.completed.load(Ordering::SeqCst), 1);
        }
    }
}

#[tokio::test]
async fn upload_nack_three_refusals_stop_and_other_failures_never_replay() {
    for (failure, expected) in [
        (Failure::Reset(h2::Reason::REFUSED_STREAM), 3),
        (Failure::Reset(h2::Reason::INTERNAL_ERROR), 1),
        (Failure::Disconnect, 1),
    ] {
        for wrapped in [false, true] {
            let peer = Peer::start(failure, 10, Duration::ZERO).await;
            assert!(tokio::time::timeout(
                Duration::from_secs(8),
                peer.send(wrapped, 8 * 1024 * 1024, None)
            )
            .await
            .unwrap()
            .is_err());
            assert_eq!(peer.calls.load(Ordering::SeqCst), expected);
            assert_eq!(peer.completed.load(Ordering::SeqCst), 0);
        }
    }
}

#[tokio::test]
async fn upload_nack_small_body_does_not_stack_an_extra_retry_budget() {
    let peer = Peer::start(
        Failure::Reset(h2::Reason::REFUSED_STREAM),
        10,
        Duration::ZERO,
    )
    .await;
    assert!(peer.send(true, 1024, None).await.is_err());
    assert_eq!(peer.calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn upload_nack_explicit_request_timeout_still_bounds_headers() {
    for wrapped in [false, true] {
        let peer = Peer::start(
            Failure::DelayedReset(h2::Reason::REFUSED_STREAM, Duration::from_millis(150)),
            1,
            Duration::from_millis(150),
        )
        .await;
        let started = Instant::now();
        let error = peer
            .send(wrapped, 8 * 1024 * 1024, Some(Duration::from_millis(220)))
            .await
            .unwrap_err();
        assert!(error.is_timeout());
        assert!(started.elapsed() < Duration::from_millis(500));
        assert_eq!(peer.calls.load(Ordering::SeqCst), 2);
    }
}
