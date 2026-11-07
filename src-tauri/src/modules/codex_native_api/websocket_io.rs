//! RFC 7692 transport with the existing message API and explicit read errors.
//! TLS, proxies and HTTP authentication remain owned by the selected reqwest client.
use futures_util::{Sink, Stream, StreamExt};
use std::{
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::tungstenite::{Error, Message};
use yawc::{Frame, OpCode};

pub(super) const OFFER: &str = "permessage-deflate; client_no_context_takeover; server_no_context_takeover; client_max_window_bits=15";

pub(super) fn negotiated(
    headers: &axum::http::HeaderMap,
) -> Result<Option<String>, super::contract::ServiceError> {
    let invalid = || {
        super::contract::ServiceError::new(
            502,
            "invalid_websocket_handshake",
            "Invalid WebSocket compression negotiation",
        )
    };
    let values = headers
        .get_all(axum::http::header::SEC_WEBSOCKET_EXTENSIONS)
        .iter()
        .collect::<Vec<_>>();
    if values.is_empty() {
        return Ok(None);
    }
    if values.len() != 1 {
        return Err(invalid());
    }
    let value = values[0].to_str().map_err(|_| invalid())?;
    let mut tokens = value.split(';');
    if tokens.next().map(str::trim) != Some("permessage-deflate") {
        return Err(invalid());
    }
    let mut result = String::from("permessage-deflate");
    let mut seen = std::collections::HashSet::new();
    for token in tokens {
        let (key, value) = token
            .trim()
            .split_once('=')
            .map_or((token.trim(), None), |(key, value)| {
                (key.trim(), Some(value.trim().trim_matches('"')))
            });
        if !seen.insert(key) {
            return Err(invalid());
        }
        match key {
            "client_no_context_takeover" | "server_no_context_takeover" if value.is_none() => {
                result.push_str("; ");
                result.push_str(key);
            }
            "client_max_window_bits" | "server_max_window_bits" => {
                let bits = value
                    .and_then(|value| value.parse::<u8>().ok())
                    .filter(|v| (9..=15).contains(v))
                    .ok_or_else(invalid)?;
                result.push_str(&format!("; {key}={bits}"));
            }
            _ => return Err(invalid()),
        }
    }
    Ok(Some(result))
}

pub(super) type UpstreamSocket = Socket<reqwest::Upgraded, Message>;
pub(super) type ClientSocket = Socket<yawc::HttpStream, axum::extract::ws::Message>;

pub(super) fn options(max: usize) -> yawc::Options {
    yawc::Options::default()
        .with_low_latency_compression()
        .client_no_context_takeover()
        .server_no_context_takeover()
        // yawc derives a two-frame buffer eagerly even when a buffer limit is
        // supplied. isize::MAX is also Rust's largest realizable allocation.
        .with_compression_threshold(1024)
        .with_limits(max.min(isize::MAX as usize), max.saturating_add(1))
        .with_backpressure_boundary(128 * 1024)
        .with_utf8()
        .with_max_fragment_size(64 * 1024)
        .with_fragment_timeout(std::time::Duration::from_secs(60))
}

/// Codec CPU work must not stall the application's async workers. The desktop
/// runtime is multithreaded; small single-threaded embedding/test runtimes keep
/// the ordinary synchronous codec behavior.
fn codec_work<T>(work: impl FnOnce() -> T) -> T {
    if tokio::runtime::Handle::try_current()
        .is_ok_and(|handle| handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread)
    {
        tokio::task::block_in_place(work)
    } else {
        work()
    }
}

pub(super) struct Socket<S, M> {
    inner: yawc::WebSocket<S>,
    ended: bool,
    marker: PhantomData<M>,
}
impl<S, M> std::fmt::Debug for Socket<S, M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebSocket")
            .field("ended", &self.ended)
            .finish_non_exhaustive()
    }
}
impl<S, M> Socket<S, M> {
    pub(super) fn new(inner: yawc::WebSocket<S>) -> Self {
        Self {
            inner,
            ended: false,
            marker: PhantomData,
        }
    }
}
impl<S: AsyncRead + AsyncWrite + Unpin, M: MessageCodec + Unpin> Socket<S, M> {
    pub(super) async fn recv(&mut self) -> Option<Result<M, Error>> {
        self.next().await
    }
}
pub(super) trait MessageCodec: Sized {
    fn decode(frame: Frame) -> Result<Self, Error>;
    fn encode(self) -> Result<Frame, Error>;
}
impl MessageCodec for Message {
    fn decode(frame: Frame) -> Result<Self, Error> {
        let opcode = frame.opcode();
        let data = frame.into_payload();
        Ok(match opcode {
            OpCode::Text => Message::Text(
                String::from_utf8(data.to_vec())
                    .map_err(|_| {
                        Error::Io(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "Invalid WebSocket UTF-8",
                        ))
                    })?
                    .into(),
            ),
            OpCode::Binary => Message::Binary(data),
            OpCode::Ping => Message::Ping(data),
            OpCode::Pong => Message::Pong(data),
            OpCode::Close => Message::Close(if data.len() >= 2 {
                Some(tokio_tungstenite::tungstenite::protocol::CloseFrame {
                    code: u16::from_be_bytes([data[0], data[1]]).into(),
                    reason: String::from_utf8_lossy(&data[2..]).into_owned().into(),
                })
            } else {
                None
            }),
            OpCode::Continuation => {
                return Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Unassembled WebSocket continuation",
                )))
            }
        })
    }
    fn encode(self) -> Result<Frame, Error> {
        Ok(match self {
            Message::Text(text) => Frame::text(text.to_string()),
            Message::Binary(bytes) => Frame::binary(bytes),
            Message::Ping(bytes) => Frame::ping(bytes),
            Message::Pong(bytes) => Frame::pong(bytes),
            Message::Close(Some(frame)) => {
                Frame::close(u16::from(frame.code).into(), frame.reason.as_bytes())
            }
            Message::Close(None) => {
                let mut frame = Frame::close(yawc::close::CloseCode::Normal, b"");
                *frame.payload_mut() = Default::default();
                frame
            }
            Message::Frame(_) => {
                return Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Raw frames are not supported",
                )))
            }
        })
    }
}
impl MessageCodec for axum::extract::ws::Message {
    fn decode(frame: Frame) -> Result<Self, Error> {
        use axum::extract::ws::{CloseFrame, Message as Client};
        Ok(match Message::decode(frame)? {
            Message::Text(text) => Client::Text(text.to_string().into()),
            Message::Binary(bytes) => Client::Binary(bytes),
            Message::Ping(bytes) => Client::Ping(bytes),
            Message::Pong(bytes) => Client::Pong(bytes),
            Message::Close(frame) => Client::Close(frame.map(|frame| CloseFrame {
                code: frame.code.into(),
                reason: frame.reason.to_string().into(),
            })),
            Message::Frame(_) => unreachable!(),
        })
    }
    fn encode(self) -> Result<Frame, Error> {
        use axum::extract::ws::Message as Client;
        match self {
            Client::Text(text) => Message::Text(text.to_string().into()),
            Client::Binary(bytes) => Message::Binary(bytes),
            Client::Ping(bytes) => Message::Ping(bytes),
            Client::Pong(bytes) => Message::Pong(bytes),
            Client::Close(frame) => Message::Close(frame.map(|frame| {
                tokio_tungstenite::tungstenite::protocol::CloseFrame {
                    code: frame.code.into(),
                    reason: frame.reason.to_string().into(),
                }
            })),
        }
        .encode()
    }
}
fn error(error: yawc::WebSocketError) -> Error {
    match error {
        yawc::WebSocketError::ConnectionClosed => Error::ConnectionClosed,
        yawc::WebSocketError::FrameTooLarge => Error::Capacity(
            tokio_tungstenite::tungstenite::error::CapacityError::MessageTooLong {
                size: usize::MAX,
                max_size: 0,
            },
        ),
        error => Error::Io(std::io::Error::other(error)),
    }
}
impl<S: AsyncRead + AsyncWrite + Unpin, M: MessageCodec + Unpin> Stream for Socket<S, M> {
    type Item = Result<M, Error>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.ended {
            return Poll::Ready(None);
        }
        match std::task::ready!(codec_work(|| this.inner.poll_next_frame(cx))) {
            Ok(frame) => Poll::Ready(Some(M::decode(frame))),
            Err(yawc::WebSocketError::ConnectionClosed) => {
                this.ended = true;
                Poll::Ready(None)
            }
            Err(err) => {
                this.ended = true;
                Poll::Ready(Some(Err(error(err))))
            }
        }
    }
}
impl<S: AsyncRead + AsyncWrite + Unpin, M: MessageCodec + Unpin> Sink<M> for Socket<S, M> {
    type Error = Error;
    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        codec_work(|| {
            Pin::new(&mut self.get_mut().inner)
                .poll_ready(cx)
                .map_err(error)
        })
    }
    fn start_send(self: Pin<&mut Self>, message: M) -> Result<(), Error> {
        codec_work(|| {
            Pin::new(&mut self.get_mut().inner)
                .start_send(message.encode()?)
                .map_err(error)
        })
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        codec_work(|| {
            Pin::new(&mut self.get_mut().inner)
                .poll_flush(cx)
                .map_err(error)
        })
    }
    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
        Pin::new(&mut self.get_mut().inner)
            .poll_close(cx)
            .map_err(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn full_compat_codec_work_yields_the_only_async_worker() {
        let (send, receive) = std::sync::mpsc::channel();
        tokio::spawn(async move {
            let worker = tokio::spawn(async move {
                codec_work(|| receive.recv_timeout(std::time::Duration::from_secs(2))).unwrap()
            });
            tokio::task::yield_now().await;
            tokio::spawn(async move {
                send.send(()).unwrap();
            })
            .await
            .unwrap();
            worker.await.unwrap();
        })
        .await
        .unwrap();
    }
    #[test]
    fn full_compat_negotiated_extensions_validate_parameters_and_duplicates() {
        use axum::http::{header, HeaderMap, HeaderValue};
        for value in [
            "permessage-deflate",
            "permessage-deflate; server_max_window_bits=12; client_max_window_bits=15",
            "permessage-deflate; server_no_context_takeover",
        ] {
            let headers = HeaderMap::from_iter([(
                header::SEC_WEBSOCKET_EXTENSIONS,
                HeaderValue::from_str(value).unwrap(),
            )]);
            assert!(negotiated(&headers).unwrap().is_some());
        }
        for value in [
            "permessage-brotli",
            "permessage-deflate; x-unknown",
            "permessage-deflate; server_max_window_bits=20",
            "permessage-deflate; client_no_context_takeover=1",
            "permessage-deflate; server_max_window_bits=12; server_max_window_bits=12",
            "permessage-deflate, permessage-deflate",
        ] {
            let headers = HeaderMap::from_iter([(
                header::SEC_WEBSOCKET_EXTENSIONS,
                HeaderValue::from_str(value).unwrap(),
            )]);
            assert!(negotiated(&headers).is_err(), "{value}");
        }
    }
}
