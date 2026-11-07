//! Content-free request sizing; large inputs are measured in the parsing worker.
use std::{
    io::Write,
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestSize {
    /// Decoded client JSON bytes, including whitespace. Field sizes use compact JSON.
    pub total_bytes: u64,
    pub input_bytes: u64,
    pub tools_bytes: u64,
    pub instructions_bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverloadRetry {
    pub retry: u32,
    pub wait_ms: u64,
    /// From the first capacity fallback until the first response bytes, or preparation.
    pub recovery_ms: u64,
}

#[derive(Default)]
struct ByteCounter(u64);
impl Write for ByteCounter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 += bytes.len() as u64;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub(super) fn measure(body: &Value, total_bytes: usize) -> RequestSize {
    fn field(body: &Value, key: &str) -> u64 {
        let Some(value) = body.get(key) else { return 0 };
        let mut counter = ByteCounter::default();
        // Serializing a Value to an infallible counting writer cannot fail.
        serde_json::to_writer(&mut counter, value).expect("count JSON bytes");
        counter.0
    }
    RequestSize {
        total_bytes: total_bytes as u64,
        input_bytes: field(body, "input"),
        tools_bytes: field(body, "tools"),
        instructions_bytes: field(body, "instructions"),
    }
}

#[derive(Default)]
struct Bucket {
    requests: AtomicU64,
    failures: AtomicU64,
    attempts: AtomicU64,
    rejected_attempts: AtomicU64,
}

#[derive(Default)]
pub(super) struct SizeBuckets([Bucket; 4]);

impl SizeBuckets {
    pub(super) fn record(
        &self,
        context: &super::diagnostics::DiagnosticContext,
        success: bool,
        status: u16,
    ) {
        let Some(size) = context
            .attempts
            .first()
            .and_then(|attempt| attempt.request_size.as_ref())
        else {
            return;
        };
        // Client cancellations are not evidence of upstream admission failure.
        if status == 499 {
            return;
        }
        let upstream_bytes = context
            .attempts
            .first()
            .and_then(|attempt| attempt.upstream_request_bytes)
            .unwrap_or(size.total_bytes);
        let (index, label) = match upstream_bytes {
            0..100_000 => (0, "<100KB"),
            100_000..200_000 => (1, "100-200KB"),
            200_000..500_000 => (2, "200-500KB"),
            _ => (3, ">=500KB"),
        };
        let bucket = &self.0[index];
        let requests = bucket.requests.fetch_add(1, Ordering::Relaxed) + 1;
        let failures = bucket
            .failures
            .fetch_add(u64::from(!success), Ordering::Relaxed)
            + u64::from(!success);
        let attempts = bucket
            .attempts
            .fetch_add(context.attempts.len() as u64, Ordering::Relaxed)
            + context.attempts.len() as u64;
        let rejected = context
            .attempts
            .iter()
            .filter(|attempt| !attempt.success)
            .count() as u64;
        let rejected = bucket
            .rejected_attempts
            .fetch_add(rejected, Ordering::Relaxed)
            + rejected;
        tracing::info!(target: "codex_api", request_id=%context.request_id,
            size_bucket=label, request_bytes=size.total_bytes, upstream_request_bytes=upstream_bytes, input_bytes=size.input_bytes,
            tools_bytes=size.tools_bytes, instructions_bytes=size.instructions_bytes,
            attempts=context.attempts.len(), success, bucket_requests=requests,
            bucket_failures=failures, bucket_attempts=attempts, bucket_rejected_attempts=rejected,
            bucket_failure_rate=failures as f64 / requests as f64,
            bucket_mean_attempts=attempts as f64 / requests as f64,
            "[NativeAPI][request_size]");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizing_counts_utf8_and_json_escaping_without_retaining_content() {
        let body = serde_json::json!({"instructions":"中文\n\"x\"", "tools":[{"name":"工具"}],
            "input":[{"role":"user","content":"秘密"}]});
        let size = measure(&body, 1234);
        assert_eq!(size.total_bytes, 1234);
        assert_eq!(
            size.input_bytes,
            serde_json::to_vec(&body["input"]).unwrap().len() as u64
        );
        assert_eq!(
            size.tools_bytes,
            serde_json::to_vec(&body["tools"]).unwrap().len() as u64
        );
        assert_eq!(
            size.instructions_bytes,
            serde_json::to_vec(&body["instructions"]).unwrap().len() as u64
        );
        assert!(!serde_json::to_string(&size).unwrap().contains("秘密"));
        assert_eq!(measure(&serde_json::json!({}), 2).input_bytes, 0);
    }

    #[test]
    fn buckets_distinguish_recovered_attempts_final_failures_and_cancellations() {
        use super::super::diagnostics::{DiagnosticAttempt, DiagnosticContext};
        let buckets = SizeBuckets::default();
        let context = DiagnosticContext {
            attempts: vec![
                DiagnosticAttempt {
                    request_size: Some(RequestSize {
                        total_bytes: 500_000,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                DiagnosticAttempt {
                    success: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        buckets.record(&context, true, 200);
        buckets.record(&context, false, 503);
        buckets.record(&context, false, 499);
        assert_eq!(buckets.0[3].requests.load(Ordering::Relaxed), 2);
        assert_eq!(buckets.0[3].failures.load(Ordering::Relaxed), 1);
        assert_eq!(buckets.0[3].attempts.load(Ordering::Relaxed), 4);
        assert_eq!(buckets.0[3].rejected_attempts.load(Ordering::Relaxed), 2);
        assert_eq!(buckets.0[2].requests.load(Ordering::Relaxed), 0);
        let mut rewritten = context;
        rewritten.attempts[0].upstream_request_bytes = Some(99_999);
        buckets.record(&rewritten, true, 200);
        assert_eq!(
            buckets.0[0].requests.load(Ordering::Relaxed),
            1,
            "bucket by the bytes actually sent upstream"
        );
    }
}
