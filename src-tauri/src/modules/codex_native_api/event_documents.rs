//! Narrow compatibility for coalesced complete event objects, never partial JSON.
use serde_json::Value;

pub(super) fn split(bytes: &[u8]) -> Option<Vec<&[u8]>> {
    if bytes.len() > 16 * 1024 * 1024 {
        return None;
    }
    let mut reader = serde_json::Deserializer::from_slice(bytes).into_iter::<Value>();
    let mut parts = Vec::new();
    let mut start = 0;
    while let Some(value) = reader.next() {
        let value = value.ok()?;
        let kind = value.get("type")?.as_str()?;
        if kind.trim().is_empty() || kind.contains(['\r', '\n']) || parts.len() >= 16 {
            return None;
        }
        let end = reader.byte_offset();
        parts.push(&bytes[start..end]);
        start = end;
    }
    (parts.len() > 1).then_some(parts)
}

pub(super) fn websocket(
    message: tokio_tungstenite::tungstenite::Message,
) -> Vec<tokio_tungstenite::tungstenite::Message> {
    use tokio_tungstenite::tungstenite::Message;
    if let Message::Text(text) = &message {
        if serde_json::from_str::<serde::de::IgnoredAny>(text).is_err() {
            if let Some(parts) = split(text.as_bytes()) {
                return parts
                    .into_iter()
                    .map(|part| {
                        Message::Text(String::from_utf8_lossy(part).trim().to_owned().into())
                    })
                    .collect();
            }
        }
    }
    vec![message]
}
