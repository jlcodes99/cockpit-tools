use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::test]
async fn reused_socket_send_does_not_poll_after_stop_or_deadline() {
    for stopped in [false, true] {
        let (_keepalive, signal) = tokio::sync::watch::channel(stopped);
        let polls = AtomicUsize::new(0);
        let send = async {
            polls.fetch_add(1, Ordering::SeqCst);
            Ok::<_, std::io::Error>(())
        };
        let deadline = (!stopped).then(tokio::time::Instant::now);
        let error = timed_write(
            super::super::lifecycle::StopSignal(signal),
            std::time::Duration::from_secs(1),
            deadline,
            send,
        )
        .await
        .unwrap_err();
        assert_eq!(
            error.code,
            if stopped {
                "service_stopping"
            } else {
                "websocket_send_timeout"
            }
        );
        assert_eq!(polls.load(Ordering::SeqCst), 0);
    }
}
