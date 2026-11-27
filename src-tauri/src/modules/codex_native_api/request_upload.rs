//! Give a large native request time to make bounded local send progress.
//!
//! Body polling is not a remote acknowledgement or evidence about execution.
//! It changes only the opening timer, never retry/replay eligibility. Small
//! bodies and other providers retain their reusable reqwest bodies unchanged.
use std::{
    error::Error as _,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};

use axum::http::{header, HeaderValue};
use hyper::body::{Body as HttpBody, Bytes, Frame, SizeHint};
use tokio::time::Instant;

use super::deadline::Deadline;

pub(super) const MIN_BYTES: usize = 8 * 1024 * 1024;
const CHUNK_BYTES: usize = 64 * 1024;

struct Credit {
    used: Duration,
    limit: Duration,
    base: Instant,
    hard: Instant,
}

impl Credit {
    fn deadline(&self) -> Instant {
        self.base
            .checked_add(self.used)
            .unwrap_or(self.hard)
            .min(self.hard)
    }
}

/// One attempt owns this budget, including a validated compression fallback.
/// The original total/overload deadline and outer network recovery fence win.
pub(super) struct Opening {
    base: Instant,
    credit: Arc<Mutex<Credit>>,
}

impl Opening {
    pub(super) fn new(base: Instant, total: Deadline, extra: Duration) -> Self {
        let hard = total.cap(base.checked_add(extra).unwrap_or(base));
        Self {
            base: total.cap(base),
            credit: Arc::new(Mutex::new(Credit {
                used: Duration::ZERO,
                limit: extra,
                base: total.cap(base),
                hard,
            })),
        }
    }

    pub(super) fn eligible(request: &reqwest::Request, native: bool) -> bool {
        native
            && request
                .body()
                .and_then(reqwest::Body::as_bytes)
                .is_some_and(|bytes| bytes.len() >= MIN_BYTES)
    }

    /// Preserve reqwest 0.12's default, protocol-proven safe H2 NACK recovery.
    /// Streaming bodies cannot be cloned by reqwest itself. Only wrapped bodies
    /// use this adapter; all other requests retain their original client policy.
    pub(super) async fn execute(
        &self,
        client: &reqwest::Client,
        mut request: reqwest::Request,
        native: bool,
    ) -> Result<reqwest::Response, reqwest::Error> {
        if !Self::eligible(&request, native) {
            return client.execute(request).await;
        }
        let original = request
            .try_clone()
            .expect("reusable large body before wrapping");
        let timeout = request.timeout().copied();
        let started = Instant::now();
        for retry in 0..=2 {
            if retry > 0 {
                // Reqwest's explicit timeout covers its internal retries and
                // the eventual response body. Do not renew it on this clone.
                *request.timeout_mut() =
                    timeout.map(|timeout| timeout.saturating_sub(started.elapsed()));
            }
            match client.execute(self.track(request, true)).await {
                Err(error) if retry < 2 && protocol_nack(&error) => {
                    request = original.try_clone().expect("unchanged reusable backup");
                }
                result => return result,
            }
        }
        unreachable!("the last send always returns")
    }

    pub(super) fn track(&self, mut request: reqwest::Request, native: bool) -> reqwest::Request {
        let Some(bytes) = request.body().and_then(reqwest::Body::as_bytes) else {
            return request;
        };
        if !native || bytes.len() < MIN_BYTES {
            return request;
        }
        let len = bytes.len() as u64;
        let body = request.body_mut().take().expect("in-memory request body");
        *request.body_mut() = Some(reqwest::Body::wrap(ProgressBody {
            inner: body,
            pending: Bytes::new(),
            remaining: len,
            previous_poll: None,
            credit: self.credit.clone(),
        }));
        // A streaming wrapper must not turn the original fixed-length request
        // into chunked encoding. Content-Encoding and the actual bytes stay put.
        request.headers_mut().remove(header::TRANSFER_ENCODING);
        request.headers_mut().insert(
            header::CONTENT_LENGTH,
            HeaderValue::from_str(&len.to_string()).expect("u64 content length"),
        );
        request
    }

    pub(super) async fn run_if<F: Future>(
        &self,
        eligible: bool,
        future: F,
    ) -> Result<F::Output, ()> {
        if eligible {
            self.run(future).await
        } else {
            // Keep the original timeout_at readiness semantics for small and
            // non-native requests, not only their original body representation.
            super::deadline::timeout_at(self.base, future)
                .await
                .map_err(|_| ())
        }
    }

    fn deadline(&self) -> Instant {
        self.credit
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .deadline()
    }

    pub(super) async fn run<F: Future>(&self, future: F) -> Result<F::Output, ()> {
        let mut future = Box::pin(future);
        loop {
            let deadline = self.deadline();
            if Instant::now() >= deadline {
                return Err(());
            }
            tokio::select! {
                biased;
                _ = tokio::time::sleep_until(deadline) => {
                    // Progress only extends this deadline. Recheck at the old
                    // deadline instead of waking the timer for every 64 KiB.
                    if Instant::now() >= self.deadline() {
                        return Err(());
                    }
                },
                result = &mut future => {
                    // A long synchronous poll cannot outrun the hard fence.
                    return if Instant::now() >= self.deadline() { Err(()) } else { Ok(result) };
                },
            }
        }
    }
}

fn protocol_nack(error: &reqwest::Error) -> bool {
    // Walk the typed source chain instead of hard-coding reqwest 0.12's
    // wrapper depth. Still require an exact typed remote H2 protocol NACK
    // (REFUSED_STREAM or GOAWAY(NO_ERROR)); never infer safety from text,
    // timeouts, local resets, HTTP statuses, or partial body writes.
    let mut current = error.source();
    while let Some(cause) = current {
        if let Some(h2) = cause.downcast_ref::<h2::Error>() {
            return h2.is_remote()
                && ((h2.is_reset() && h2.reason() == Some(h2::Reason::REFUSED_STREAM))
                    || (h2.is_go_away() && h2.reason() == Some(h2::Reason::NO_ERROR)));
        }
        current = cause.source();
    }
    false
}

struct ProgressBody {
    inner: reqwest::Body,
    pending: Bytes,
    remaining: u64,
    previous_poll: Option<Instant>,
    credit: Arc<Mutex<Credit>>,
}

impl ProgressBody {
    fn data(&mut self) -> Frame<Bytes> {
        let now = Instant::now();
        {
            let mut credit = self.credit.lock().unwrap_or_else(|e| e.into_inner());
            // A late poll cannot revive an expired opening. Do not park the
            // body here: early response headers can legitimately precede the
            // last upload frame, after the opening future has already returned.
            if let Some(previous) = self
                .previous_poll
                .replace(now)
                .filter(|_| now < credit.deadline())
            {
                credit.used = credit.used.saturating_add(now - previous).min(credit.limit);
            }
        }
        let bytes = self.pending.split_to(self.pending.len().min(CHUNK_BYTES));
        self.remaining = self.remaining.saturating_sub(bytes.len() as u64);
        Frame::data(bytes)
    }
}

impl HttpBody for ProgressBody {
    type Data = Bytes;
    type Error = reqwest::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        if !self.pending.is_empty() {
            return Poll::Ready(Some(Ok(self.data())));
        }
        match Pin::new(&mut self.inner).poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => match frame.into_data() {
                Ok(bytes) if !bytes.is_empty() => {
                    self.pending = bytes;
                    Poll::Ready(Some(Ok(self.data())))
                }
                Ok(bytes) => Poll::Ready(Some(Ok(Frame::data(bytes)))),
                Err(frame) => Poll::Ready(Some(Ok(frame))),
            },
            result => result,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.pending.is_empty() && self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::with_exact(self.remaining)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    fn request(bytes: usize) -> reqwest::Request {
        reqwest::Client::new()
            .post("https://example.invalid/responses")
            .header(header::CONTENT_ENCODING, "zstd")
            .body(vec![b'x'; bytes])
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn upload_progress_preserves_exact_bytes_encoding_and_plain_backup() {
        let opening = Opening::new(
            Instant::now() + Duration::from_secs(1),
            Deadline::NONE,
            Duration::from_secs(1),
        );
        let original = request(MIN_BYTES);
        let plain_backup = original.try_clone().unwrap();
        let mut tracked = opening.track(original, true);
        assert_eq!(tracked.headers()[header::CONTENT_ENCODING], "zstd");
        assert_eq!(
            tracked.headers()[header::CONTENT_LENGTH],
            MIN_BYTES.to_string()
        );
        assert!(
            tracked.try_clone().is_none(),
            "transport polling is intentionally single-use"
        );
        assert!(plain_backup.try_clone().is_some());
        let body = tracked.body_mut().take().unwrap();
        assert_eq!(body.size_hint().exact(), Some(MIN_BYTES as u64));
        let bytes = body.collect().await.unwrap().to_bytes();
        assert_eq!(
            bytes.as_ref(),
            plain_backup.body().unwrap().as_bytes().unwrap()
        );
    }

    #[test]
    fn upload_progress_leaves_small_other_routes_and_streams_unmodified() {
        let opening = Opening::new(Instant::now(), Deadline::NONE, Duration::from_secs(1));
        for (size, native) in [(MIN_BYTES - 1, true), (MIN_BYTES, false)] {
            let tracked = opening.track(request(size), native);
            assert!(tracked.try_clone().is_some());
            assert!(!tracked.headers().contains_key(header::CONTENT_LENGTH));
        }
        let mut streaming = request(1);
        *streaming.body_mut() = Some(reqwest::Body::wrap_stream(futures_util::stream::once(
            async { Ok::<_, std::io::Error>(Bytes::from_static(b"stream")) },
        )));
        let tracked = opening.track(streaming, true);
        assert!(!tracked.headers().contains_key(header::CONTENT_LENGTH));
        assert_eq!(opening.deadline(), opening.base);
    }

    #[tokio::test]
    async fn upload_progress_does_not_credit_connect_time_or_unpolled_stalls() {
        let opening = Opening::new(
            Instant::now() + Duration::from_millis(80),
            Deadline::NONE,
            Duration::from_millis(80),
        );
        let mut request = opening.track(request(MIN_BYTES), true);
        tokio::time::sleep(Duration::from_millis(20)).await;
        let body = request.body_mut().as_mut().unwrap();
        body.frame().await.unwrap().unwrap();
        assert_eq!(opening.deadline(), opening.base);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(
            opening.deadline(),
            opening.base,
            "a stalled body earns no new time"
        );
        body.frame().await.unwrap().unwrap();
        assert!(opening.deadline() >= opening.base + Duration::from_millis(20));
    }

    #[tokio::test]
    async fn upload_progress_shared_fallback_credit_and_total_cap_never_renew() {
        let base = Instant::now() + Duration::from_millis(300);
        let opening = Opening::new(
            base,
            Deadline::from(base + Duration::from_millis(15)),
            Duration::from_millis(25),
        );
        for _ in 0..2 {
            let mut request = opening.track(request(MIN_BYTES), true);
            let body = request.body_mut().as_mut().unwrap();
            body.frame().await.unwrap().unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
            body.frame().await.unwrap().unwrap();
        }
        assert_eq!(
            opening.credit.lock().unwrap().used,
            Duration::from_millis(25)
        );
        assert_eq!(opening.deadline(), base + Duration::from_millis(15));
        assert!(opening.run(std::future::pending::<()>()).await.is_err());
    }

    #[tokio::test]
    async fn upload_progress_expired_ready_result_and_late_poll_cannot_revive_attempt() {
        let expired = Opening::new(
            Instant::now() - Duration::from_millis(1),
            Deadline::NONE,
            Duration::from_secs(1),
        );
        assert!(expired.run(std::future::ready(42)).await.is_err());
        let opening = Opening::new(
            Instant::now() + Duration::from_millis(100),
            Deadline::NONE,
            Duration::from_secs(1),
        );
        let mut request = opening.track(request(MIN_BYTES), true);
        let body = request.body_mut().as_mut().unwrap();
        body.frame().await.unwrap().unwrap();
        tokio::time::sleep(Duration::from_millis(120)).await;
        let waker = futures_util::task::noop_waker();
        let mut context = Context::from_waker(&waker);
        assert!(Pin::new(body).poll_frame(&mut context).is_ready());
        assert_eq!(opening.credit.lock().unwrap().used, Duration::ZERO);
        assert!(opening.run(std::future::ready(42)).await.is_err());
    }
}
