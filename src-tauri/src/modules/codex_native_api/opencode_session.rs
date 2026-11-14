//! Caller-scoped fallback for OpenCode HTTP requests without native identity.
//! The session label is untrusted input, never a credential or account selector.
use axum::http::{HeaderMap, HeaderValue};
use serde::{
    de::{DeserializeSeed, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer,
};
use serde_json::{Map, Value};
use sha1::{Digest, Sha1};

const DOMAIN: &str = "cockpit:opencode-continuity:v1";

// WebSockets, compact and independent image/audio operations keep their native
// identity behavior. Only original authenticated conversation ingress calls this.
pub(super) fn conversation_path(path: &str) -> bool {
    matches!(
        path,
        "/v1/responses"
            | "/responses"
            | "/v1/chat/completions/v1/responses"
            | "/v1/chat/completions"
            | "/chat/completions"
            | "/v1/completions"
            | "/completions"
    )
}

fn candidate_session<'a>(caller: &str, headers: &'a HeaderMap) -> Option<&'a str> {
    if caller.is_empty() || caller.len() > 4096 || caller.chars().any(char::is_control) {
        return None;
    }
    // Presence, even empty/invalid, leaves explicit identity validation in charge.
    for name in [
        "session-id",
        "session_id",
        "thread-id",
        "thread_id",
        "conversation_id",
        "x-codex-parent-thread-id",
        "x-codex-turn-state",
        "x-codex-turn-metadata",
    ] {
        if headers.contains_key(name) {
            return None;
        }
    }
    let mut values = headers.get_all("x-opencode-session").iter();
    let session = values.next()?.as_bytes();
    if values.next().is_some()
        || session.is_empty()
        || session.len() > 1024
        || session
            .iter()
            .any(|b| !(b'!'..=b'~').contains(b) || *b == b',')
    {
        return None;
    }
    std::str::from_utf8(session).ok()
}

// Parse the native Value and fallback eligibility together. Parsing to Value
// first would lose duplicate-key evidence. Ineligible headers use ordinary serde;
// candidates get one duplicate-aware parse, under the existing ingress limits.
pub(super) fn parse(
    raw_body: &[u8],
    identity: Option<(&str, &HeaderMap)>,
) -> Result<(Value, Option<HeaderMap>), serde_json::Error> {
    let candidate = identity.and_then(|(caller, headers)| {
        candidate_session(caller, headers).map(|session| (caller, headers, session))
    });
    let Some((caller, original, session)) = candidate else {
        return serde_json::from_slice(raw_body).map(|body| (body, None));
    };
    let mut decoder = serde_json::Deserializer::from_slice(raw_body);
    let (body, eligible) = ObjectVisitor { metadata: true }.deserialize(&mut decoder)?;
    decoder.end()?;
    let headers = if eligible {
        derive(caller, session, original)
    } else {
        None
    };
    Ok((body, headers))
}

fn derive(caller: &str, session: &str, original: &HeaderMap) -> Option<HeaderMap> {
    // A domain-separated tuple prevents ambiguous caller/session concatenations.
    // Keep JSON's HTML/line-separator escaping deterministic across encoders.
    let tuple = serde_json::to_string(&[DOMAIN, caller, session])
        .ok()?
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    // UUID v5 using existing dependencies; this label does not authorize access.
    let mut hash = Sha1::new();
    hash.update(uuid::Uuid::NAMESPACE_OID.as_bytes());
    hash.update(tuple.as_bytes());
    let digest = hash.finalize();
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let value = HeaderValue::from_str(&uuid::Uuid::from_bytes(bytes).to_string()).ok()?;
    let mut headers = original.clone();
    headers.insert("session-id", value.clone());
    headers.insert("thread-id", value);
    Some(headers)
}

// Only top-level and direct client_metadata keys participate in eligibility.
// Duplicates suppress fallback, not native parsing: retain Value's last-key-wins
// semantics and deserialize unrelated nested values normally, without body copies.
struct ObjectVisitor {
    metadata: bool,
}

impl<'de> DeserializeSeed<'de> for ObjectVisitor {
    type Value = (Value, bool);
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for ObjectVisitor {
    type Value = (Value, bool);
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a JSON value")
    }
    fn visit_map<A: MapAccess<'de>>(self, mut input: A) -> Result<Self::Value, A::Error> {
        let mut object = Map::new();
        let mut eligible = true;
        while let Some(key) = input.next_key::<String>()? {
            let reserved = matches!(
                key.as_str(),
                "$serde_json::private::Number" | "$serde_json::private::RawValue"
            );
            eligible &= !reserved;
            // arbitrary_precision represents some numbers as a synthetic map.
            // Replay its first key through Value's protocol, never reparse a body
            // or mistake it for an eligible object. Literal reserved keys are
            // ambiguous too, including when they occur after an ordinary key.
            if object.is_empty() && reserved {
                let value =
                    Value::deserialize(serde::de::value::MapAccessDeserializer::new(FirstKey {
                        key: Some(key),
                        input,
                    }))?;
                return Ok((value, false));
            }
            if object.contains_key(&key) {
                eligible = false;
            }
            let explicit = if self.metadata {
                matches!(
                    key.as_str(),
                    "previous_response_id" | "conversation" | "conversation_id"
                )
            } else {
                matches!(
                    key.as_str(),
                    "session_id"
                        | "thread_id"
                        | "parent_response_id"
                        | "parent_thread_id"
                        | "turn_id"
                        | "x-codex-parent-thread-id"
                        | "x-codex-turn-state"
                        | "x-codex-turn-metadata"
                )
            };
            eligible &= !explicit;
            let value = if self.metadata && key == "client_metadata" {
                let (value, metadata_eligible) =
                    input.next_value_seed(ObjectVisitor { metadata: false })?;
                eligible &= metadata_eligible;
                value
            } else {
                input.next_value::<Value>()?
            };
            object.insert(key, value);
        }
        Ok((Value::Object(object), eligible))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut input: A) -> Result<Self::Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = input.next_element::<Value>()? {
            values.push(value);
        }
        Ok((Value::Array(values), false))
    }
    fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Self::Value, E> {
        Ok((Value::Bool(value), false))
    }
    fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
        Ok((Value::from(value), false))
    }
    fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
        Ok((Value::from(value), false))
    }
    fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
        Ok((Value::from(value), false))
    }
    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
        Ok((Value::String(value.to_owned()), false))
    }
    fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Self::Value, E> {
        Ok((Value::String(value), false))
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        Ok((Value::Null, false))
    }
}

struct FirstKey<A> {
    key: Option<String>,
    input: A,
}

impl<'de, A: MapAccess<'de>> MapAccess<'de> for FirstKey<A> {
    type Error = A::Error;
    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, A::Error> {
        match self.key.take() {
            Some(key) => seed
                .deserialize(serde::de::value::StringDeserializer::<A::Error>::new(key))
                .map(Some),
            None => self.input.next_key_seed(seed),
        }
    }
    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, A::Error> {
        self.input.next_value_seed(seed)
    }
}

#[cfg(test)]
#[path = "opencode_session_tests.rs"]
mod tests;
