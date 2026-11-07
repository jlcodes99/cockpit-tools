//! Transport compression never changes the canonical request used for replay.
use super::{contract::ServiceError, server::AppState};
use axum::{
    body::{Body, Bytes, HttpBody},
    extract::{Request, State},
    http::{header, HeaderMap, HeaderValue, Method},
    middleware::Next,
    response::Response,
};
use futures_util::{stream, StreamExt};
use std::{io::Write, sync::Arc};

const MIN_BYTES: usize = 1024;

pub(super) fn codex_endpoint(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some("chatgpt.com")
        && url.path().starts_with("/backend-api/codex/")
}

pub(super) async fn request(
    state: &AppState,
    mut request: reqwest::Request,
    codex: bool,
) -> Result<(reqwest::Request, Option<reqwest::Request>), ServiceError> {
    request.headers_mut().remove(header::CONTENT_ENCODING);
    request.headers_mut().remove(header::CONTENT_LENGTH);
    if !codex || !codex_endpoint(request.url()) {
        return Ok((request, None));
    }
    let Some(plain) = request
        .body()
        .and_then(reqwest::Body::as_bytes)
        .filter(|bytes| bytes.len() >= MIN_BYTES)
    else {
        return Ok((request, None));
    };
    let plain = plain.to_vec();
    let compressed = super::blocking::run_prepared(state, move || {
        zstd::stream::encode_all(plain.as_slice(), 3).map_err(compression_error)
    })
    .await?;
    if compressed.len()
        >= request
            .body()
            .and_then(reqwest::Body::as_bytes)
            .map_or(0, <[u8]>::len)
    {
        return Ok((request, None));
    }
    let plain = request.try_clone();
    *request.body_mut() = Some(compressed.into());
    request
        .headers_mut()
        .insert(header::CONTENT_ENCODING, HeaderValue::from_static("zstd"));
    Ok((request, plain))
}

fn compression_error(error: std::io::Error) -> ServiceError {
    ServiceError::new(
        500,
        "compression_failed",
        format!("Transport compression failed: {error}"),
    )
}

#[path = "response_decode.rs"]
mod decode;
#[path = "compression_rejection.rs"]
mod rejection;

pub(super) async fn rejected_before_execution(response: reqwest::Response) -> Result<(reqwest::Response, bool), ServiceError> {
    rejection::inspect(response).await
}

pub(super) fn decode_response(response: reqwest::Response) -> Result<reqwest::Response, ServiceError> {
    decode::response(response)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Encoding {
    Zstd,
    Gzip,
}
impl Encoding {
    fn name(self) -> &'static str {
        match self {
            Self::Zstd => "zstd",
            Self::Gzip => "gzip",
        }
    }
}

fn negotiate(headers: &HeaderMap) -> (Option<Encoding>, bool) {
    let mut qualities = std::collections::HashMap::new();
    for value in headers.get_all(header::ACCEPT_ENCODING) {
        for token in value.to_str().unwrap_or_default().split(',') {
            let mut parts = token.split(';');
            let name = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
            let mut quality = 1.0_f32;
            for part in parts {
                if let Some((key, value)) = part.split_once('=') {
                    if key.trim().eq_ignore_ascii_case("q") {
                        quality = value
                            .trim()
                            .parse::<f32>()
                            .ok()
                            .filter(|v| (0.0..=1.0).contains(v))
                            .unwrap_or(0.0);
                    }
                }
            }
            qualities.insert(name, quality);
        }
    }
    let wildcard = qualities.get("*").copied().unwrap_or(0.0);
    let identity = qualities.get("identity").copied().unwrap_or_else(|| {
        if qualities.contains_key("*") {
            wildcard
        } else {
            1.0
        }
    }) > 0.0;
    let mut chosen = None;
    let mut highest = 0.0;
    for encoding in [Encoding::Zstd, Encoding::Gzip] {
        let quality = qualities.get(encoding.name()).copied().unwrap_or(wildcard);
        if quality > highest {
            chosen = Some(encoding);
            highest = quality;
        }
    }
    (chosen, identity)
}

enum Encoder {
    Zstd(zstd::stream::write::Encoder<'static, Vec<u8>>),
    Gzip(flate2::write::GzEncoder<Vec<u8>>),
}
impl Encoder {
    fn new(encoding: Encoding) -> std::io::Result<Self> {
        Ok(match encoding {
            Encoding::Zstd => {
                let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 3)?;
                encoder.window_log(20)?;
                Self::Zstd(encoder)
            }
            Encoding::Gzip => Self::Gzip(flate2::write::GzEncoder::new(
                Vec::new(),
                flate2::Compression::fast(),
            )),
        })
    }
    fn chunk(&mut self, bytes: &[u8]) -> std::io::Result<Bytes> {
        let output = match self {
            Self::Zstd(encoder) => {
                encoder.write_all(bytes)?;
                encoder.flush()?;
                encoder.get_mut()
            }
            Self::Gzip(encoder) => {
                encoder.write_all(bytes)?;
                encoder.flush()?;
                encoder.get_mut()
            }
        };
        Ok(std::mem::take(output).into())
    }
    fn finish(self) -> std::io::Result<Bytes> {
        match self {
            Self::Zstd(encoder) => encoder.finish(),
            Self::Gzip(encoder) => encoder.finish(),
        }
        .map(Bytes::from)
    }
}

pub(super) async fn response(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let head = request.method() == Method::HEAD;
    let upgrade = request.headers().contains_key(header::UPGRADE);
    let (encoding, identity) = negotiate(request.headers());
    let mut response = next.run(request).await;
    if upgrade {
        return response;
    }
    let vary = response
        .headers()
        .get_all(header::VARY)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .any(|v| {
            v.split(',')
                .any(|s| s.trim() == "*" || s.trim().eq_ignore_ascii_case("accept-encoding"))
        });
    if !vary {
        response
            .headers_mut()
            .append(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    }
    let status = response.status().as_u16();
    let media = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim();
    let compressible = media.starts_with("text/")
        || media == "application/json"
        || media.ends_with("+json")
        || media == "application/javascript"
        || media == "image/svg+xml";
    let no_transform = response
        .headers()
        .get_all(header::CACHE_CONTROL)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .any(|v| {
            v.split(',')
                .any(|s| s.trim().eq_ignore_ascii_case("no-transform"))
        });
    let small = identity
        && media != "text/event-stream"
        && response
            .body()
            .size_hint()
            .exact()
            .is_some_and(|n| n < MIN_BYTES as u64);
    let Some(encoding) = encoding.filter(|_| {
        !head
            && status >= 200
            && !matches!(status, 204 | 206 | 304)
            && !response.headers().contains_key(header::CONTENT_ENCODING)
            && !no_transform
            && compressible
            && !small
    }) else {
        return response;
    };
    let encoder = match Encoder::new(encoding) {
        Ok(encoder) => encoder,
        Err(_) => return response,
    };
    response.headers_mut().remove(header::CONTENT_LENGTH);
    response.headers_mut().insert(
        header::CONTENT_ENCODING,
        HeaderValue::from_static(encoding.name()),
    );
    let (parts, body) = response.into_parts();
    let stream = stream::try_unfold(
        (body.into_data_stream(), Some(encoder), state),
        |(mut source, encoder, state)| async move {
            let Some(mut encoder) = encoder else {
                return Ok(None);
            };
            let next = source
                .next()
                .await
                .transpose()
                .map_err(std::io::Error::other)?;
            let end = next.is_none();
            let (bytes, encoder) = super::blocking::run_tracked(&state, move || {
                if let Some(bytes) = next {
                    Ok((
                        encoder.chunk(&bytes).map_err(compression_error)?,
                        Some(encoder),
                    ))
                } else {
                    Ok((encoder.finish().map_err(compression_error)?, None))
                }
            })
            .await
            .map_err(|error| std::io::Error::other(error.message))?;
            Ok::<_, std::io::Error>(Some((
                bytes,
                (source, if end { None } else { encoder }, state),
            )))
        },
    );
    Response::from_parts(parts, Body::from_stream(stream))
}
