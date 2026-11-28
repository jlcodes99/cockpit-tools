// Test-only observer decoder. It never changes bytes sent through the TCP tap.
// Include as a module, then call public_wire_decoder::decode_public_wire(&wire).

use std::io::Read;

const MAX_WIRE_BYTES: usize = 8 * 1024 * 1024;
const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_TRAILER_BYTES: usize = 64 * 1024;
const MAX_CHUNKS: usize = 65_536;
const MAX_DECODED_BYTES: usize = 4 * 1024 * 1024;
const MAX_CONTENT_ENCODINGS: usize = 4;

pub(super) fn decode_public_wire(wire: &[u8]) -> Result<String, String> {
    if wire.len() > MAX_WIRE_BYTES {
        return Err("HTTP wire size limit exceeded".into());
    }
    let header_end = wire
        .windows(4)
        .position(|bytes| bytes == b"\r\n\r\n")
        .ok_or("HTTP headers are truncated")?;
    if header_end > MAX_HEADER_BYTES {
        return Err("HTTP header size limit exceeded".into());
    }
    let headers = std::str::from_utf8(&wire[..header_end])
        .map_err(|error| format!("invalid HTTP header text: {error}"))?;
    let mut lines = headers.split("\r\n");
    let status = lines.next().ok_or("missing HTTP status line")?;
    if !status.starts_with("HTTP/1.") {
        return Err("expected an HTTP/1.x response".into());
    }
    let mut transfer_encodings = Vec::new();
    let mut content_encodings = Vec::new();
    let mut content_length = None;
    for line in lines {
        let (name, value) = line.split_once(':').ok_or("malformed HTTP header")?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err("invalid HTTP header name".into());
        }
        match name.to_ascii_lowercase().as_str() {
            "transfer-encoding" => append_codings(value, &mut transfer_encodings)?,
            "content-encoding" => append_codings(value, &mut content_encodings)?,
            "content-length" => {
                let length = value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| "invalid Content-Length")?;
                if content_length
                    .replace(length)
                    .is_some_and(|previous| previous != length)
                {
                    return Err("conflicting Content-Length headers".into());
                }
            }
            _ => {}
        }
    }
    if content_encodings.len() > MAX_CONTENT_ENCODINGS {
        return Err("too many Content-Encoding layers".into());
    }
    let body = &wire[header_end + 4..];
    let mut payload = match transfer_encodings.as_slice() {
        [] => {
            if content_length.is_some_and(|length| length != body.len()) {
                return Err("Content-Length does not match complete HTTP body".into());
            }
            body.to_vec()
        }
        [coding] if coding == "chunked" => {
            if content_length.is_some() {
                return Err("ambiguous chunked body with Content-Length".into());
            }
            decode_chunked(body)?
        }
        _ => return Err("unsupported Transfer-Encoding".into()),
    };
    for coding in content_encodings.iter().rev() {
        payload = match coding.as_str() {
            "identity" => payload,
            "gzip" => read_limited(flate2::read::MultiGzDecoder::new(payload.as_slice()))?,
            "zstd" => {
                let decoder = zstd::stream::read::Decoder::new(payload.as_slice())
                    .map_err(|error| format!("invalid zstd body: {error}"))?;
                read_limited(decoder)?
            }
            _ => return Err(format!("unsupported Content-Encoding: {coding}")),
        };
        if payload.len() > MAX_DECODED_BYTES {
            return Err("decoded body size limit exceeded".into());
        }
    }
    if payload.len() > MAX_DECODED_BYTES {
        return Err("decoded body size limit exceeded".into());
    }
    String::from_utf8(payload).map_err(|error| format!("decoded body is not UTF-8: {error}"))
}

fn append_codings(value: &str, output: &mut Vec<String>) -> Result<(), String> {
    for coding in value.split(',') {
        let coding = coding.trim();
        if coding.is_empty() {
            return Err("empty HTTP encoding token".into());
        }
        output.push(coding.to_ascii_lowercase());
    }
    Ok(())
}

fn read_limited(reader: impl Read) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    reader
        .take((MAX_DECODED_BYTES + 1) as u64)
        .read_to_end(&mut output)
        .map_err(|error| format!("compressed body decoding failed: {error}"))?;
    if output.len() > MAX_DECODED_BYTES {
        return Err("decoded body size limit exceeded".into());
    }
    Ok(output)
}

fn decode_chunked(body: &[u8]) -> Result<Vec<u8>, String> {
    let mut position = 0;
    let mut output = Vec::new();
    for _ in 0..MAX_CHUNKS {
        let line = take_line(body, &mut position)?;
        if line.len() > MAX_HEADER_BYTES {
            return Err("chunk header size limit exceeded".into());
        }
        // Chunk extensions are framing metadata; only the size controls decoding.
        let size = line.split(|byte| *byte == b';').next().unwrap();
        if size.is_empty() || !size.iter().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("invalid hexadecimal chunk size".into());
        }
        let size = std::str::from_utf8(size).map_err(|_| "invalid chunk size text")?;
        let size = usize::from_str_radix(size, 16).map_err(|_| "chunk size overflow")?;
        if size == 0 {
            let trailer_start = position;
            loop {
                let trailer = take_line(body, &mut position)?;
                if position - trailer_start > MAX_TRAILER_BYTES {
                    return Err("HTTP trailer size limit exceeded".into());
                }
                if trailer.is_empty() {
                    if position != body.len() {
                        return Err("unexpected bytes after chunked body".into());
                    }
                    return Ok(output);
                }
                if !trailer.contains(&b':') {
                    return Err("malformed HTTP trailer".into());
                }
            }
        }
        let end = position
            .checked_add(size)
            .ok_or("chunk boundary overflow")?;
        let framed_end = end.checked_add(2).ok_or("chunk boundary overflow")?;
        if framed_end > body.len() {
            return Err("chunk data is truncated".into());
        }
        if &body[end..framed_end] != b"\r\n" {
            return Err("missing CRLF after chunk data".into());
        }
        if output.len().checked_add(size).ok_or("body size overflow")? > MAX_WIRE_BYTES {
            return Err("chunked body size limit exceeded".into());
        }
        output.extend_from_slice(&body[position..end]);
        position = framed_end;
    }
    Err("HTTP chunk count limit exceeded".into())
}

fn take_line<'a>(body: &'a [u8], position: &mut usize) -> Result<&'a [u8], String> {
    let remaining = body
        .get(*position..)
        .ok_or("HTTP framing position overflow")?;
    let length = remaining
        .windows(2)
        .position(|bytes| bytes == b"\r\n")
        .ok_or("HTTP framing line is truncated")?;
    let line = &remaining[..length];
    *position += length + 2;
    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const SSE: &str = "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_一\"}}\n\n";

    fn chunked_wire(encoding: &str, body: &[u8]) -> Vec<u8> {
        let mut wire = format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Encoding: {encoding}\r\n\r\n"
        )
        .into_bytes();
        // Deliberately split compressed frames and UTF-8 characters across chunks.
        for chunk in body.chunks(7) {
            wire.extend_from_slice(format!("{:x};fixture=\"yes\"\r\n", chunk.len()).as_bytes());
            wire.extend_from_slice(chunk);
            wire.extend_from_slice(b"\r\n");
        }
        wire.extend_from_slice(b"0;done=yes\r\nX-Fixture: complete\r\n\r\n");
        wire
    }

    fn gzip(body: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(body).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn chunked_zstd_extensions_and_trailers_decode() {
        let compressed = zstd::stream::encode_all(SSE.as_bytes(), 1).unwrap();
        assert_eq!(
            decode_public_wire(&chunked_wire("zstd", &compressed)).unwrap(),
            SSE
        );
    }

    #[test]
    fn chunked_gzip_and_identity_decode() {
        assert_eq!(
            decode_public_wire(&chunked_wire("gzip", &gzip(SSE.as_bytes()))).unwrap(),
            SSE
        );
        assert_eq!(
            decode_public_wire(&chunked_wire("identity", SSE.as_bytes())).unwrap(),
            SSE
        );
    }

    #[test]
    fn truncated_headers_chunks_trailers_and_compression_are_rejected() {
        for wire in [
            b"HTTP/1.1 200 OK\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nabc".as_slice(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n0\r\nX-Trailer: unfinished\r\n"
                .as_slice(),
        ] {
            assert!(decode_public_wire(wire).is_err());
        }
        let mut compressed = gzip(SSE.as_bytes());
        compressed.truncate(compressed.len() - 4);
        assert!(decode_public_wire(&chunked_wire("gzip", &compressed)).is_err());
        let mut compressed = zstd::stream::encode_all(SSE.as_bytes(), 1).unwrap();
        compressed.truncate(compressed.len() - 1);
        assert!(decode_public_wire(&chunked_wire("zstd", &compressed)).is_err());
    }

    #[test]
    fn decompression_size_limit_is_enforced_for_gzip_and_zstd() {
        let oversized = vec![b'x'; MAX_DECODED_BYTES + 1];
        let compressed = gzip(&oversized);
        let error = decode_public_wire(&chunked_wire("gzip", &compressed)).unwrap_err();
        assert!(error.contains("size limit"), "{error}");
        let compressed = zstd::stream::encode_all(oversized.as_slice(), 1).unwrap();
        let error = decode_public_wire(&chunked_wire("zstd", &compressed)).unwrap_err();
        assert!(error.contains("size limit"), "{error}");
    }
}
