//! Lazy upstream decoding: plain SSE must not wait for a four-byte sniff.
use super::super::contract::ServiceError;
use axum::http::{header, HeaderMap};
use futures_util::{stream, StreamExt, TryStreamExt};
use std::{io, pin::Pin};
use tokio::io::{AsyncRead, AsyncReadExt, BufReader};
use tokio_util::io::{ReaderStream, StreamReader};

type Reader = Pin<Box<dyn AsyncRead + Send>>;
type ByteStream = futures_util::stream::BoxStream<'static, Result<axum::body::Bytes, io::Error>>;

pub(super) fn response(response: reqwest::Response) -> Result<reqwest::Response, ServiceError> {
    let headers = response.headers();
    let encodings: Vec<_> = headers
        .get_all(header::CONTENT_ENCODING)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty() && v != "identity")
        .collect();
    if encodings.len() > 8
        || encodings
            .iter()
            .any(|v| !matches!(v.as_str(), "gzip" | "zstd" | "br" | "deflate"))
    {
        return Err(ServiceError::new(
            502,
            "unsupported_upstream_encoding",
            "Unsupported upstream content encoding",
        ));
    }
    let sniff = !headers.contains_key(header::CONTENT_ENCODING) && textual(headers);
    if encodings.is_empty() && !sniff {
        return Ok(response);
    }
    let status = response.status();
    let version = response.version();
    let mut headers = headers.clone();
    headers.remove(header::CONTENT_ENCODING);
    headers.remove(header::CONTENT_LENGTH);
    let source = response
        .bytes_stream()
        .map(|chunk| chunk.map_err(io::Error::other))
        .boxed();
    if sniff {
        let body = reqwest::Body::wrap_stream(sniff_stream(source));
        let mut response = axum::http::Response::new(body);
        *response.status_mut() = status;
        *response.version_mut() = version;
        *response.headers_mut() = headers;
        return Ok(reqwest::Response::from(response));
    }
    let mut reader: Reader = Box::pin(StreamReader::new(source));
    for encoding in encodings.iter().rev() {
        reader = decoder(reader, encoding);
    }
    let mut response = axum::http::Response::new(reqwest::Body::wrap_stream(
        ReaderStream::with_capacity(reader, 64 * 1024),
    ));
    *response.status_mut() = status;
    *response.version_mut() = version;
    *response.headers_mut() = headers;
    Ok(reqwest::Response::from(response))
}

fn sniff_stream(mut source: ByteStream) -> ByteStream {
    stream::once(async move {
        let mut chunks = Vec::new();
        let mut prefix = Vec::new();
        while let Some(chunk) = source.next().await {
            let chunk = chunk?;
            if chunk.is_empty() {
                continue;
            }
            prefix.extend(chunk.iter().copied().take(4 - prefix.len()));
            chunks.push(chunk);
            let needed = match prefix.first() {
                Some(0x1f) => 2,
                Some(0x28) => 4,
                Some(_) => 1,
                None => continue,
            };
            if prefix.len() >= needed {
                break;
            }
        }
        let source = stream::iter(chunks.into_iter().map(Ok::<_, io::Error>))
            .chain(source)
            .boxed();
        let encoding = if prefix.starts_with(&[0x1f, 0x8b]) {
            "gzip"
        } else if prefix.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
            "zstd"
        } else {
            return Ok::<ByteStream, io::Error>(source);
        };
        let reader = decoder(Box::pin(StreamReader::new(source)), encoding);
        Ok(ReaderStream::with_capacity(reader, 64 * 1024).boxed())
    })
    .try_flatten()
    .boxed()
}

fn textual(headers: &HeaderMap) -> bool {
    // A .gz file downloaded as a binary attachment is data, not an unlabelled
    // HTTP encoding. Responses/Chat error JSON and SSE are safe sniff targets.
    let Some(value) = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
    else {
        return true;
    };
    let media = value
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    media.starts_with("text/") || media == "application/json" || media.ends_with("+json")
}

fn decoder(reader: Reader, encoding: &str) -> Reader {
    use async_compression::tokio::bufread::*;
    match encoding {
        "gzip" => {
            let mut decoder = GzipDecoder::new(BufReader::new(reader));
            decoder.multiple_members(true);
            Box::pin(decoder)
        }
        "zstd" => {
            let mut decoder = ZstdDecoder::new(BufReader::new(reader));
            decoder.multiple_members(true);
            Box::pin(decoder)
        }
        "br" => Box::pin(BrotliDecoder::new(BufReader::new(reader))),
        "zlib" => Box::pin(ZlibDecoder::new(BufReader::new(reader))),
        "raw-deflate" => Box::pin(DeflateDecoder::new(BufReader::new(reader))),
        "deflate" => detect_deflate(reader),
        _ => reader,
    }
}

fn detect_deflate(mut reader: Reader) -> Reader {
    let initialization = async move {
        let mut prefix = [0_u8; 2];
        let mut used = 0;
        while used < prefix.len() {
            let count = reader.read(&mut prefix[used..]).await?;
            if count == 0 {
                break;
            }
            used += count;
        }
        let prefix = &prefix[..used];
        let zlib = prefix.len() == 2
            && prefix[0] & 15 == 8
            && prefix[0] >> 4 <= 7
            && u16::from_be_bytes([prefix[0], prefix[1]]) % 31 == 0;
        let encoding = if zlib { "zlib" } else { "raw-deflate" };
        let source: Reader = Box::pin(io::Cursor::new(prefix.to_vec()).chain(reader));
        Ok::<_, io::Error>(ReaderStream::with_capacity(
            decoder(source, encoding),
            64 * 1024,
        ))
    };
    // Initialization is polled only when the caller requests body bytes; its
    // existing connect/idle/total deadlines and cancellation still apply.
    Box::pin(StreamReader::new(
        stream::once(initialization).try_flatten(),
    ))
}
