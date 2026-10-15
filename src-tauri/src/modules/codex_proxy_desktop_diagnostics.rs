//! Transport metadata only: never log payloads, destinations or error display strings.
use super::{entry, logger, protocol};
use std::{
    io,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    task::{Context, Poll},
    time::Instant,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

tokio::task_local! { static CURRENT: uuid::Uuid; }

fn tls_cause(error: &io::Error) -> &'static str {
    let mut source = error
        .get_ref()
        .map(|value| value as &(dyn std::error::Error + 'static));
    for _ in 0..8 {
        let Some(current) = source else {
            break;
        };
        if let Some(tls) = current.downcast_ref::<rustls::Error>() {
            return match tls {
                rustls::Error::InvalidCertificate(_) => "invalid_certificate",
                rustls::Error::InvalidMessage(_) => "invalid_message",
                rustls::Error::AlertReceived(_) => "peer_alert",
                rustls::Error::PeerIncompatible(_) => "peer_incompatible",
                rustls::Error::PeerMisbehaved(_) => "peer_misbehaved",
                rustls::Error::NoCertificatesPresented => "no_certificates",
                _ => "other_tls_error",
            };
        }
        source = if let Some(nested) = current.downcast_ref::<io::Error>() {
            nested
                .get_ref()
                .map(|value| value as &(dyn std::error::Error + 'static))
        } else {
            current.source()
        };
    }
    "not_exposed"
}

pub(super) fn upstream_socket(port: u16) {
    let id = CURRENT.try_with(|id| *id).ok();
    logger::log_info(&format!(
        "[CodexProxy] upstream_socket: connection_id={id:?}, upstream_source_port={port}"
    ));
}

pub(super) fn io_failed(stage: &'static str, error: &io::Error) -> &'static str {
    let id = CURRENT.try_with(|id| *id).ok();
    logger::log_warn(&format!(
        "[CodexProxy] transport_io_error: connection_id={id:?}, stage={stage}, io_kind={:?}, os_code={:?}, tls_cause={}",
        error.kind(), error.raw_os_error(), tls_cause(error)
    ));
    "PROXY_CONNECT_FAILED"
}

pub(super) fn protocol_reply(stage: &'static str, status: Option<u16>) {
    let id = CURRENT.try_with(|id| *id).ok();
    logger::log_info(&format!(
        "[CodexProxy] upstream_reply: connection_id={id:?}, stage={stage}, status={status:?}"
    ));
}

pub(super) struct Connection {
    id: uuid::Uuid,
    started: Instant,
    entry_port: u16,
    source_port: u16,
    stage: Mutex<&'static str>,
    finished: AtomicBool,
}

impl Connection {
    pub(super) fn new(entry_port: u16, source_port: u16) -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            started: Instant::now(),
            entry_port,
            source_port,
            stage: Mutex::new("accepted"),
            finished: AtomicBool::new(false),
        }
    }

    pub(super) fn phase(&self, stage: &'static str) {
        if let Ok(mut current) = self.stage.lock() {
            *current = stage;
        }
        logger::log_info(&format!(
            "[CodexProxy] connection_stage: {}",
            self.context(None, stage)
        ));
    }

    pub(super) async fn run(
        &self,
        work: impl std::future::Future<Output = Result<(), String>>,
    ) -> Result<(), String> {
        logger::log_info(&format!("[CodexProxy] diagnostic_session: schema=2, host_pid={}, app_version={}, connection_id={}", std::process::id(), env!("CARGO_PKG_VERSION"), self.id));
        self.phase("accepted");
        let result = CURRENT.scope(self.id, work).await;
        self.finished.store(true, Ordering::Relaxed);
        let stage = self.stage.lock().map(|value| *value).unwrap_or("unknown");
        logger::log_info(&format!(
            "[CodexProxy] connection_end: {}, outcome={}, code={}",
            self.context(None, stage),
            if result.is_ok() {
                "returned_ok"
            } else {
                "returned_error"
            },
            result
                .as_ref()
                .err()
                .map(|error| entry::safe_error(error))
                .unwrap_or_default()
        ));
        result
    }

    fn context(&self, protocol: Option<protocol::Protocol>, stage: &str) -> String {
        format!(
            "connection_id={}, entry_port={}, source_port={}, protocol={}, stage={}, elapsed_ms={}",
            self.id,
            self.entry_port,
            self.source_port,
            protocol.map(|value| value.label()).unwrap_or("unknown"),
            stage,
            self.started.elapsed().as_millis()
        )
    }

    pub(super) fn route(&self, tunnel_id: Option<uuid::Uuid>, upstream_source_port: Option<u16>) {
        logger::log_info(&format!(
            "[CodexProxy] relay_route: connection_id={}, tunnel_id={tunnel_id:?}, upstream_source_port={upstream_source_port:?}", self.id
        ));
    }

    pub(super) fn failed(&self, protocol: Option<protocol::Protocol>, stage: &str, error: &str) {
        logger::log_warn(&format!(
            "[CodexProxy] 桌面代理连接失败: {}, error={}",
            self.context(protocol, stage),
            entry::safe_error(error)
        ));
    }

    pub(super) async fn relay<A, B>(
        &self,
        protocol: protocol::Protocol,
        client: &mut A,
        upstream: &mut B,
    ) -> io::Result<()>
    where
        A: AsyncRead + AsyncWrite + Unpin,
        B: AsyncRead + AsyncWrite + Unpin,
    {
        logger::log_info(&format!(
            "[CodexProxy] relay_start: {}",
            self.context(Some(protocol), "relay")
        ));
        let mut client = ObservedIo::new(client, self.started).with_context(self.id, "client");
        let mut upstream =
            ObservedIo::new(upstream, self.started).with_context(self.id, "upstream");
        let result = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
        client.finished = true;
        upstream.finished = true;
        // EOF is a TCP observation, not confirmation of a TLS close_notify or a complete response.
        let detail = format!(
            "{}, client={:?}, upstream={:?}",
            self.context(Some(protocol), "relay"),
            client.stats,
            upstream.stats
        );
        match &result {
            Ok(_) => logger::log_info(&format!(
                "[CodexProxy] relay_end: outcome=tcp_eof, {detail}"
            )),
            Err(error) => {
                logger::log_warn(&format!(
                "[CodexProxy] relay_end: outcome=io_error, io_kind={:?}, os_code={:?}, {detail}",
                error.kind(), error.raw_os_error()))
            }
        }
        result.map(|_| ())
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        if !self.finished.load(Ordering::Relaxed) {
            let stage = self.stage.lock().map(|value| *value).unwrap_or("unknown");
            logger::log_warn(&format!(
                "[CodexProxy] connection_end: {}, outcome=task_dropped",
                self.context(None, stage)
            ));
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct IoStats {
    read_bytes: u64,
    written_bytes: u64,
    first_read_ms: Option<u128>,
    last_read_ms: Option<u128>,
    last_write_ms: Option<u128>,
    eof_ms: Option<u128>,
    error_operation: Option<&'static str>,
}

struct ObservedIo<S> {
    inner: S,
    started: Instant,
    stats: IoStats,
    context: Option<(uuid::Uuid, &'static str)>,
    finished: bool,
}

impl<S> ObservedIo<S> {
    fn new(inner: S, started: Instant) -> Self {
        Self {
            inner,
            started,
            stats: IoStats::default(),
            context: None,
            finished: false,
        }
    }
}

impl<S> ObservedIo<S> {
    fn with_context(mut self, id: uuid::Uuid, side: &'static str) -> Self {
        self.context = Some((id, side));
        self
    }

    fn event(&self, operation: &'static str, error: Option<&io::Error>) {
        if let Some((id, side)) = self.context {
            // Fixed metadata only: an arbitrary io::Error message can contain credentials.
            logger::log_info(&format!(
                "[CodexProxy] relay_event: connection_id={id}, side={side}, operation={operation}, elapsed_ms={}, read_idle_ms={:?}, io_kind={:?}, os_code={:?}, tls_cause={}, stats={:?}",
                self.started.elapsed().as_millis(),
                self.stats.last_read_ms.map(|last| self.started.elapsed().as_millis().saturating_sub(last)),
                error.map(io::Error::kind), error.and_then(io::Error::raw_os_error), error.map(tls_cause).unwrap_or("none"), self.stats
            ));
        }
    }

    fn failed(&mut self, operation: &'static str, error: &io::Error) {
        if self.stats.error_operation.is_none() {
            self.stats.error_operation = Some(operation);
            self.event(operation, Some(error));
        }
    }
}

impl<S> Drop for ObservedIo<S> {
    fn drop(&mut self) {
        if !self.finished {
            self.event("relay_dropped_snapshot", None);
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for ObservedIo<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let before = buf.filled().len();
        let remaining = buf.remaining();
        let result = Pin::new(&mut this.inner).poll_read(cx, buf);
        match &result {
            Poll::Ready(Ok(())) => {
                let bytes = buf.filled().len() - before;
                this.stats.read_bytes += bytes as u64;
                if bytes > 0 {
                    let now = this.started.elapsed().as_millis();
                    this.stats.first_read_ms.get_or_insert(now);
                    this.stats.last_read_ms = Some(now);
                }
                if bytes == 0 && remaining > 0 && this.stats.eof_ms.is_none() {
                    this.stats.eof_ms = Some(this.started.elapsed().as_millis());
                    this.event("read_eof", None);
                }
            }
            Poll::Ready(Err(error)) => this.failed("read", error),
            Poll::Pending => {}
        }
        result
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for ObservedIo<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let result = Pin::new(&mut this.inner).poll_write(cx, buf);
        match &result {
            Poll::Ready(Ok(bytes)) => {
                this.stats.written_bytes += *bytes as u64;
                if *bytes > 0 {
                    this.stats.last_write_ms = Some(this.started.elapsed().as_millis());
                }
            }
            Poll::Ready(Err(error)) => this.failed("write", error),
            Poll::Pending => {}
        }
        result
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let result = Pin::new(&mut this.inner).poll_flush(cx);
        if let Poll::Ready(Err(error)) = &result {
            this.failed("flush", error);
        }
        result
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let result = Pin::new(&mut this.inner).poll_shutdown(cx);
        if let Poll::Ready(Err(error)) = &result {
            this.failed("shutdown", error);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[derive(Clone, Default)]
    struct Capture(std::sync::Arc<Mutex<Vec<u8>>>);
    impl io::Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl Capture {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
        fn subscriber(&self) -> impl tracing::Subscriber + Send + Sync + 'static {
            let writer = self.clone();
            tracing_subscriber::fmt()
                .without_time()
                .with_ansi(false)
                .with_writer(move || writer.clone())
                .finish()
        }
    }

    #[test]
    fn nested_tls_causes_are_classified_without_raw_details() {
        let tls = io::Error::new(
            io::ErrorKind::InvalidData,
            rustls::Error::NoCertificatesPresented,
        );
        assert_eq!(tls_cause(&tls), "no_certificates");
        let nested = io::Error::new(io::ErrorKind::Other, tls);
        assert_eq!(tls_cause(&nested), "no_certificates");
        let secret = io::Error::new(
            io::ErrorKind::Other,
            rustls::Error::General("secret-host-token".into()),
        );
        assert_eq!(tls_cause(&secret), "other_tls_error");
    }

    #[tokio::test]
    async fn cancelled_relay_logs_both_snapshots_and_task_end_without_payload() {
        let capture = Capture::default();
        let _guard = tracing::subscriber::set_default(capture.subscriber());
        let connection = Connection::new(100, 101);
        let (mut client, _client_peer) = tokio::io::duplex(8);
        let (mut upstream, _upstream_peer) = tokio::io::duplex(8);
        let work = connection.run(async {
            connection.phase("relay");
            connection
                .relay(protocol::Protocol::HttpConnect, &mut client, &mut upstream)
                .await
                .map_err(|_| "PROXY_CONNECT_FAILED".to_owned())
        });
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), work)
                .await
                .is_err()
        );
        drop(connection);
        let logs = capture.text();
        assert_eq!(logs.matches("operation=relay_dropped_snapshot").count(), 2);
        assert!(logs.contains("side=client") && logs.contains("side=upstream"));
        assert!(logs.contains("stage=relay") && logs.contains("outcome=task_dropped"));
        assert!(!logs.contains("outcome=tcp_eof"));
    }

    #[tokio::test]
    async fn concurrent_scopes_keep_error_ids_and_completed_tasks_are_not_cancelled() {
        let capture = Capture::default();
        let _guard = tracing::subscriber::set_default(capture.subscriber());
        let first = Connection::new(100, 101);
        let second = Connection::new(100, 102);
        let (one, two) = tokio::join!(
            first.run(async {
                tokio::task::yield_now().await;
                Err(io_failed(
                    "first",
                    &io::Error::new(io::ErrorKind::Other, "secret-token"),
                )
                .to_owned())
            }),
            second.run(async {
                tokio::task::yield_now().await;
                io_failed("second", &io::Error::from_raw_os_error(123));
                Ok(())
            })
        );
        assert!(one.is_err() && two.is_ok());
        let ids = (first.id, second.id);
        drop(first);
        drop(second);
        let logs = capture.text();
        assert!(logs.contains(&format!("connection_id=Some({}), stage=first", ids.0)));
        assert!(logs.contains(&format!("connection_id=Some({}), stage=second", ids.1)));
        assert!(logs.contains("os_code=Some(123)"));
        assert!(!logs.contains("secret-token") && !logs.contains("outcome=task_dropped"));
    }

    #[tokio::test]
    async fn observed_relay_preserves_bidirectional_data_and_half_close() {
        let (mut left, client) = tokio::io::duplex(8);
        let (upstream, mut right) = tokio::io::duplex(8);
        let relay = tokio::spawn(async move {
            let mut client = ObservedIo::new(client, Instant::now());
            let mut upstream = ObservedIo::new(upstream, Instant::now());
            let counts = tokio::io::copy_bidirectional(&mut client, &mut upstream)
                .await
                .unwrap();
            (counts, client.stats, upstream.stats)
        });
        let peers = async {
            left.write_all(b"request").await.unwrap();
            left.shutdown().await.unwrap();
            let mut input = Vec::new();
            right.read_to_end(&mut input).await.unwrap();
            assert_eq!(input, b"request");
            right.write_all(b"response").await.unwrap();
            right.shutdown().await.unwrap();
            let mut output = Vec::new();
            left.read_to_end(&mut output).await.unwrap();
            assert_eq!(output, b"response");
            let (counts, client, upstream) = relay.await.unwrap();
            assert_eq!(counts, (7, 8));
            assert_eq!((client.read_bytes, client.written_bytes), (7, 8));
            assert_eq!((upstream.read_bytes, upstream.written_bytes), (8, 7));
            assert!(client.eof_ms.is_some() && upstream.eof_ms.is_some());
            assert!(client.first_read_ms <= client.last_read_ms);
            assert!(client.last_read_ms.is_some() && upstream.last_write_ms.is_some());
            assert!(client.error_operation.is_none() && upstream.error_operation.is_none());
        };
        tokio::time::timeout(std::time::Duration::from_secs(2), peers)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn upstream_eof_is_observed_before_client_closes() {
        let (stream, mut peer) = tokio::io::duplex(8);
        let mut observed = ObservedIo::new(stream, Instant::now());
        peer.write_all(b"partial").await.unwrap();
        peer.shutdown().await.unwrap();
        let mut bytes = Vec::new();
        observed.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(bytes, b"partial");
        assert!(observed.stats.eof_ms.is_some());
        assert!(observed.stats.first_read_ms.is_some());
        // Read-side EOF must not prevent writing the remaining opposite direction.
        observed.write_all(b"tail").await.unwrap();
        let mut tail = [0; 4];
        peer.read_exact(&mut tail).await.unwrap();
        assert_eq!(&tail, b"tail");
    }

    #[tokio::test]
    async fn read_reset_preserves_os_error() {
        struct Reset;
        impl AsyncRead for Reset {
            fn poll_read(
                self: Pin<&mut Self>,
                _: &mut Context<'_>,
                _: &mut ReadBuf<'_>,
            ) -> Poll<io::Result<()>> {
                Poll::Ready(Err(io::Error::from_raw_os_error(123)))
            }
        }
        let mut observed = ObservedIo::new(Reset, Instant::now());
        let error = observed.read(&mut [0; 1]).await.unwrap_err();
        assert_eq!(error.raw_os_error(), Some(123));
        assert_eq!(observed.stats.error_operation, Some("read"));
        assert!(observed.stats.eof_ms.is_none());
    }

    #[tokio::test]
    async fn failed_write_retains_error_and_does_not_log_payload() {
        let (stream, peer) = tokio::io::duplex(8);
        drop(peer);
        let mut observed = ObservedIo::new(stream, Instant::now());
        let error = observed.write_all(b"secret-token").await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(observed.stats.error_operation, Some("write"));
        assert_eq!(observed.stats.written_bytes, 0);
        assert!(!format!("{:?}", observed.stats).contains("secret-token"));
    }
}
