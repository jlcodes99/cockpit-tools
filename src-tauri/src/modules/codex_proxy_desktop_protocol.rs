//! Ingress negotiation only; both protocols share the account's existing routing and lease.
use super::{diagnostics, read_socks_request, Destination};
use std::net::Ipv6Addr;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

const MAX_CONNECT_HEADER_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy)]
pub(super) enum Protocol {
    Socks5,
    HttpConnect,
}

pub(super) enum Reply {
    Established,
    BadRequest,
    BadGateway,
}

impl Protocol {
    pub(super) async fn detect(stream: &TcpStream) -> Result<Self, String> {
        let mut first = [0];
        match stream.peek(&mut first).await {
            Ok(1) if first[0] == 5 => Ok(Self::Socks5),
            Ok(1) if first[0].is_ascii_alphabetic() => Ok(Self::HttpConnect),
            Err(error) => Err(diagnostics::io_failed("protocol_peek", &error).into()),
            _ => Err("PROXY_CONNECT_FAILED".into()),
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Socks5 => "socks5",
            Self::HttpConnect => "http_connect",
        }
    }

    pub(super) async fn read_destination<S>(self, stream: &mut S) -> Result<Destination, String>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        match self {
            Self::Socks5 => read_socks_request(stream).await,
            Self::HttpConnect => read_connect_request(stream).await,
        }
    }

    pub(super) async fn reply<S>(self, stream: &mut S, reply: Reply) -> Result<(), String>
    where
        S: AsyncWrite + Unpin,
    {
        let bytes: &[u8] = match (self, reply) {
            (Self::Socks5, Reply::Established) => &[5, 0, 0, 1, 0, 0, 0, 0, 0, 0],
            (Self::Socks5, _) => &[5, 1, 0, 1, 0, 0, 0, 0, 0, 0],
            (Self::HttpConnect, Reply::Established) => {
                b"HTTP/1.1 200 Connection Established\r\n\r\n"
            }
            (Self::HttpConnect, Reply::BadRequest) => {
                b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            }
            (Self::HttpConnect, Reply::BadGateway) => {
                b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            }
        };
        stream
            .write_all(bytes)
            .await
            .map_err(|error| diagnostics::io_failed("ingress_reply", &error).into())
    }
}

async fn read_connect_request<S: AsyncRead + Unpin>(stream: &mut S) -> Result<Destination, String> {
    let mut header = Vec::with_capacity(512);
    // Read exactly through the header, retaining pipelined TLS bytes in the socket.
    while header.len() < MAX_CONNECT_HEADER_BYTES {
        let byte = stream
            .read_u8()
            .await
            .map_err(|error| diagnostics::io_failed("ingress_http_header", &error))?;
        header.push(byte);
        if header.ends_with(b"\r\n\r\n") {
            let text = std::str::from_utf8(&header).map_err(|_| "PROXY_CONNECT_FAILED")?;
            let line = text.split("\r\n").next().ok_or("PROXY_CONNECT_FAILED")?;
            let parts: Vec<_> = line.split(' ').collect();
            if parts.len() != 3
                || parts[0] != "CONNECT"
                || !matches!(parts[2], "HTTP/1.0" | "HTTP/1.1")
            {
                return Err("PROXY_CONNECT_FAILED".into());
            }
            return parse_authority(parts[1]);
        }
    }
    Err("PROXY_CONNECT_FAILED".into())
}

fn parse_authority(authority: &str) -> Result<Destination, String> {
    let (host, port) = authority.rsplit_once(':').ok_or("PROXY_CONNECT_FAILED")?;
    if port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("PROXY_CONNECT_FAILED".into());
    }
    let port = port.parse::<u16>().map_err(|_| "PROXY_CONNECT_FAILED")?;
    if port == 0 {
        return Err("PROXY_CONNECT_FAILED".into());
    }
    let host = if let Some(ip) = host.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        ip.parse::<Ipv6Addr>()
            .map_err(|_| "PROXY_CONNECT_FAILED")?
            .to_string()
    } else {
        if host.is_empty()
            || host.len() > 253
            || !host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        {
            return Err("PROXY_CONNECT_FAILED".into());
        }
        host.to_owned()
    };
    Ok(Destination { host, port })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_authorities_require_an_explicit_valid_port_and_no_url_credentials() {
        for (input, host, port) in [
            ("chatgpt.com:443", "chatgpt.com", 443),
            ("127.0.0.1:80", "127.0.0.1", 80),
            ("[::1]:443", "::1", 443),
        ] {
            let target = parse_authority(input).unwrap();
            assert_eq!((target.host.as_str(), target.port), (host, port));
        }
        for invalid in [
            "chatgpt.com",
            "host:0",
            "host:65536",
            "host:+443",
            ":443",
            "user:secret@host:443",
            "host/path:443",
            "host?query:443",
            "host\r\nInjected:443",
            "::1:443",
            "[host]:443",
        ] {
            assert!(parse_authority(invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[tokio::test]
    async fn header_limit_and_truncated_header_fail_without_waiting_for_more_input() {
        for input in [
            vec![b'C'; MAX_CONNECT_HEADER_BYTES + 1],
            b"CONNECT host:443 HTTP/1.1\r\n".to_vec(),
        ] {
            let (mut writer, mut reader) = tokio::io::duplex(MAX_CONNECT_HEADER_BYTES * 2);
            writer.write_all(&input).await.unwrap();
            writer.shutdown().await.unwrap();
            assert!(read_connect_request(&mut reader).await.is_err());
        }
    }

    #[tokio::test]
    async fn split_headers_preserve_pipelined_tunnel_data() {
        let (mut writer, mut reader) = tokio::io::duplex(128);
        let task = tokio::spawn(async move {
            for part in [
                b"CON".as_slice(),
                b"NECT [::1]:443 HTTP/1.1\r\nHost: [::1]:443\r\n",
                b"\r\n\x16\x03\x01",
            ] {
                writer.write_all(part).await.unwrap();
                tokio::task::yield_now().await;
            }
        });
        let target = read_connect_request(&mut reader).await.unwrap();
        assert_eq!((target.host.as_str(), target.port), ("::1", 443));
        let mut tls = [0; 3];
        reader.read_exact(&mut tls).await.unwrap();
        assert_eq!(tls, [0x16, 3, 1]);
        task.await.unwrap();
    }
}
