use crate::modules::codex_session_rollout_records::Records;

#[derive(Default, Deserialize)]
#[serde(default)]
struct VisibilityRolloutRecord {
    #[serde(rename = "type")]
    kind: JsonValue,
    #[serde(deserialize_with = "present_rollout_field")]
    id: Option<JsonValue>,
    session_id: JsonValue,
    #[serde(deserialize_with = "rollout_object_payload")]
    payload: Option<VisibilityRolloutPayload>,
    #[serde(deserialize_with = "present_rollout_field")]
    timestamp: Option<JsonValue>,
    #[serde(deserialize_with = "present_rollout_field")]
    time: Option<JsonValue>,
    #[serde(deserialize_with = "present_rollout_field")]
    created_at: Option<JsonValue>,
    #[serde(rename = "createdAt", deserialize_with = "present_rollout_field")]
    created_at_camel: Option<JsonValue>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct VisibilityRolloutPayload {
    #[serde(deserialize_with = "present_rollout_field")]
    id: Option<JsonValue>,
    session_id: JsonValue,
    source: JsonValue,
    model_provider: JsonValue,
    cwd: JsonValue,
    #[serde(deserialize_with = "present_rollout_field")]
    timestamp: Option<JsonValue>,
    #[serde(deserialize_with = "present_rollout_field")]
    time: Option<JsonValue>,
    #[serde(deserialize_with = "present_rollout_field")]
    created_at: Option<JsonValue>,
    #[serde(rename = "createdAt", deserialize_with = "present_rollout_field")]
    created_at_camel: Option<JsonValue>,
}

fn present_rollout_field<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<JsonValue>, D::Error> {
    JsonValue::deserialize(deserializer).map(Some)
}

fn rollout_object_payload<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<VisibilityRolloutPayload>, D::Error> {
    use serde::de::{value::MapAccessDeserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};

    struct PayloadVisitor;
    impl<'de> Visitor<'de> for PayloadVisitor {
        type Value = Option<VisibilityRolloutPayload>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a rollout payload")
        }

        fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
            VisibilityRolloutPayload::deserialize(MapAccessDeserializer::new(map)).map(Some)
        }

        fn visit_seq<S: SeqAccess<'de>>(self, mut seq: S) -> Result<Self::Value, S::Error> {
            while seq.next_element::<IgnoredAny>()?.is_some() {}
            Ok(None)
        }

        fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Self::Value, E> {
            Ok(None)
        }
    }

    deserializer.deserialize_any(PayloadVisitor)
}

impl VisibilityRolloutPayload {
    fn thread_id(&self) -> Option<&str> {
        self.id.as_ref().unwrap_or(&self.session_id).as_str()
    }

    fn is_non_root(&self) -> bool {
        source_value_marks_non_root_agent(&self.source)
    }

    fn provider(&self) -> Option<&str> {
        self.model_provider
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    fn activity_ms(&self) -> Option<i128> {
        self.timestamp
            .as_ref()
            .or(self.time.as_ref())
            .or(self.created_at.as_ref())
            .or(self.created_at_camel.as_ref())
            .and_then(parse_timestamp_ms)
    }
}

impl VisibilityRolloutRecord {
    fn activity_ms(&self) -> Option<i128> {
        self.timestamp
            .as_ref()
            .or(self.time.as_ref())
            .or(self.created_at.as_ref())
            .or(self.created_at_camel.as_ref())
            .and_then(parse_timestamp_ms)
            .or_else(|| self.payload.as_ref()?.activity_ms())
    }
}

const MARKER_TAIL_BYTES: usize = "encrypted_content".len() - 1;

#[derive(Default)]
struct RolloutMarkers {
    has_user_event: bool,
    contains_encrypted_content: bool,
    tail: [u8; MARKER_TAIL_BYTES],
    tail_len: usize,
}

impl RolloutMarkers {
    fn observe(&mut self, chunk: &[u8]) {
        let mut boundary = [0; MARKER_TAIL_BYTES * 2];
        let prefix_len = chunk.len().min(MARKER_TAIL_BYTES);
        boundary[..self.tail_len].copy_from_slice(&self.tail[..self.tail_len]);
        boundary[self.tail_len..self.tail_len + prefix_len].copy_from_slice(&chunk[..prefix_len]);
        let boundary = &boundary[..self.tail_len + prefix_len];
        let text = std::str::from_utf8(chunk).expect("rollout chunks are validated UTF-8");
        let contains = |needle: &str| {
            text.contains(needle)
                || boundary
                    .windows(needle.len())
                    .any(|window| window == needle.as_bytes())
        };
        self.has_user_event |= contains("\"user_message\"") || contains("\"user_input\"");
        self.contains_encrypted_content |= contains("encrypted_content");
        let tail = if chunk.len() >= MARKER_TAIL_BYTES {
            &chunk[chunk.len() - MARKER_TAIL_BYTES..]
        } else {
            &boundary[boundary.len().saturating_sub(MARKER_TAIL_BYTES)..]
        };
        self.tail[..tail.len()].copy_from_slice(tail);
        self.tail_len = tail.len();
    }
}

#[derive(Default)]
struct RolloutVisibilityScan {
    metadata: Vec<VisibilityRolloutRecord>,
    markers: RolloutMarkers,
    activity_ms: Option<i128>,
}

impl RolloutVisibilityScan {
    fn provider_facts(&self) -> RolloutProviderRewrite {
        let mut facts = RolloutProviderRewrite::default();
        for record in &self.metadata {
            let Some(payload) = &record.payload else {
                continue;
            };
            facts.session_meta_count += 1;
            facts.non_root_agent |= payload.is_non_root();
            if let Some(provider) = payload.provider() {
                facts.providers.insert(provider.to_owned());
            }
            if facts.thread_id.is_none() {
                facts.thread_id = payload
                    .thread_id()
                    .or_else(|| record.id.as_ref().unwrap_or(&record.session_id).as_str())
                    .map(str::to_owned);
            }
        }
        facts
    }
}

fn scan_rollout_summary(input: impl std::io::Read) -> std::io::Result<RolloutVisibilityScan> {
    let mut scan = RolloutVisibilityScan::default();
    let mut records = Records::new(input, |chunk: &[u8]| {
        scan.markers.observe(chunk);
        std::thread::yield_now();
        Ok(())
    });
    while let Some(record) = records.next::<VisibilityRolloutRecord>()? {
        if let Ok(record) = record.value {
            scan.activity_ms = scan.activity_ms.max(record.activity_ms());
            if record.kind.as_str() == Some("session_meta") {
                scan.metadata.push(record);
            }
        }
    }
    Ok(scan)
}

fn read_rollout_summary(path: &Path) -> Result<Option<RolloutVisibilityScan>, String> {
    let result = fs::File::open(path).and_then(scan_rollout_summary);
    match result {
        Ok(scan) => Ok(Some(scan)),
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
            modules::logger::log_warn(&format!(
                "跳过非 UTF-8 Codex rollout 文件 ({}): {}",
                path.display(),
                error
            ));
            Ok(None)
        }
        Err(error) => Err(format!(
            "读取 Codex rollout 文件失败 ({}): {}",
            path.display(),
            error
        )),
    }
}
