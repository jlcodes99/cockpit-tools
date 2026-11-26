//! Zero-dependency binary Protobuf wire decoder for Antigravity telemetry blobs.
//!
//! Enforces the zero double-counting invariant on API usage messages (`steps.metadata.f9`
//! and `gen_metadata.data.f1.f4`):
//!   `f3` (`raw_combined_output`) = `f9` (`thinking_tokens`) + `f10` (`response_output_tokens`)
//!   `output_tokens = f10.unwrap_or_else(|| f3.unwrap_or(0).saturating_sub(thinking_tokens))`
//!   `total_tokens = input_tokens + output_tokens + thinking_tokens + cache_read_tokens + cache_write_tokens`

use chrono::DateTime;
use std::time::Duration;

/// Maximum positive 64-bit signed integer in two's-complement varint representation.
/// Values above this threshold represent negative `int32`/`int64` values encoded as 10-byte varints.
pub const MAX_POSITIVE_INT64: u64 = 0x7fff_ffff_ffff_ffff;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtoWireValue<'a> {
    Varint(u64),
    Fixed64([u8; 8]),
    Bytes(&'a [u8]),
    Fixed32([u8; 4]),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtoField<'a> {
    pub field_number: u32,
    pub wire_type: u8,
    pub value: ProtoWireValue<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedUsageProto {
    pub model_enum: Option<u64>,
    pub input_tokens: u64,
    pub raw_combined_output: Option<u64>,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
    pub api_provider: Option<u64>,
    pub message_id: Option<String>,
    pub thinking_tokens: u64,
    pub response_output_tokens: Option<u64>,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedStepMetadataProto {
    pub timestamp: Option<String>,
    pub usage: Option<ParsedUsageProto>,
    pub fallback_model_enum: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedGenMetadataProto {
    pub step_indices: Vec<usize>,
    pub model_enum_numeric: Option<u64>,
    pub model_enum_placeholder: Option<String>,
    pub response_model: Option<String>,
    pub usage: Option<ParsedUsageProto>,
    pub time_to_first_token: Option<Duration>,
    pub streaming_duration: Option<Duration>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SlimUsageProto {
    pub model_enum: Option<u64>,
    pub api_provider: Option<u64>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub thinking_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SlimStepMetadataProto {
    pub timestamp_ms: Option<i64>,
    pub usage: Option<SlimUsageProto>,
    pub fallback_model_enum: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SlimGenMetadataProto {
    pub step_indices: Vec<usize>,
    pub model_enum_numeric: Option<u64>,
    pub model_enum_placeholder: Option<String>,
    pub response_model: Option<String>,
    pub usage: Option<SlimUsageProto>,
}

#[inline]
pub fn varint_to_token_count(val: u64) -> u64 {
    if val > MAX_POSITIVE_INT64 {
        0
    } else {
        val
    }
}

pub fn format_utc_ms(ms: i64) -> Option<String> {
    if ms <= 0 {
        return None;
    }
    let secs = ms.div_euclid(1_000);
    let millis = ms.rem_euclid(1_000) as u32;
    let dt = DateTime::from_timestamp(secs, millis * 1_000_000)?;
    Some(format!("{}.{:03}Z", dt.format("%Y-%m-%dT%H:%M:%S"), millis))
}

pub fn format_utc_secs_nanos(secs: i64, nanos: u64) -> Option<String> {
    if secs <= 0 {
        return None;
    }
    let extra_secs = i64::try_from(nanos / 1_000_000_000).ok()?;
    let rem_nanos = (nanos % 1_000_000_000) as u32;
    let total_secs = secs.checked_add(extra_secs)?;
    let dt = DateTime::from_timestamp(total_secs, rem_nanos)?;
    Some(format!(
        "{}.{:03}Z",
        dt.format("%Y-%m-%dT%H:%M:%S"),
        rem_nanos / 1_000_000
    ))
}

pub fn read_varint(buf: &[u8], offset: usize) -> Option<(u64, usize)> {
    if offset >= buf.len() {
        return None;
    }

    let mut result: u64 = 0;
    for i in 0..10 {
        let idx = offset.checked_add(i)?;
        if idx >= buf.len() {
            return None;
        }
        let byte = buf[idx];
        if i == 9 {
            if byte > 0x01 {
                return None;
            }
            result |= (byte as u64) << 63;
            return Some((result, idx + 1));
        }

        result |= ((byte & 0x7f) as u64) << (i * 7);
        if (byte & 0x80) == 0 {
            return Some((result, idx + 1));
        }
    }

    None
}

pub fn parse_proto_fields(buf: &[u8]) -> Vec<ProtoField<'_>> {
    parse_proto_fields_internal(buf).0
}

fn parse_proto_fields_internal(buf: &[u8]) -> (Vec<ProtoField<'_>>, usize) {
    let mut fields = Vec::new();
    let mut offset = 0;

    while offset < buf.len() {
        let Some((tag, after_tag)) = read_varint(buf, offset) else {
            break;
        };
        let field_number_u64 = tag >> 3;
        let wire_type = (tag & 0x07) as u8;
        if field_number_u64 == 0 || field_number_u64 > u32::MAX as u64 {
            break;
        }
        let field_number = field_number_u64 as u32;

        match wire_type {
            0 => {
                let Some((val, next_offset)) = read_varint(buf, after_tag) else {
                    break;
                };
                fields.push(ProtoField {
                    field_number,
                    wire_type,
                    value: ProtoWireValue::Varint(val),
                });
                offset = next_offset;
            }
            1 => {
                let Some(end) = after_tag.checked_add(8) else {
                    break;
                };
                if end > buf.len() {
                    break;
                }
                let mut fixed = [0u8; 8];
                fixed.copy_from_slice(&buf[after_tag..end]);
                fields.push(ProtoField {
                    field_number,
                    wire_type,
                    value: ProtoWireValue::Fixed64(fixed),
                });
                offset = end;
            }
            2 => {
                let Some((raw_len, after_len)) = read_varint(buf, after_tag) else {
                    break;
                };
                let Ok(len) = usize::try_from(raw_len) else {
                    break;
                };
                let Some(end) = after_len.checked_add(len) else {
                    break;
                };
                if end > buf.len() {
                    break;
                }
                fields.push(ProtoField {
                    field_number,
                    wire_type,
                    value: ProtoWireValue::Bytes(&buf[after_len..end]),
                });
                offset = end;
            }
            5 => {
                let Some(end) = after_tag.checked_add(4) else {
                    break;
                };
                if end > buf.len() {
                    break;
                }
                let mut fixed = [0u8; 4];
                fixed.copy_from_slice(&buf[after_tag..end]);
                fields.push(ProtoField {
                    field_number,
                    wire_type,
                    value: ProtoWireValue::Fixed32(fixed),
                });
                offset = end;
            }
            _ => {
                break;
            }
        }
    }

    (fields, offset)
}

pub fn parse_packed_varints(buf: &[u8]) -> Vec<u64> {
    let mut out = Vec::new();
    let mut offset = 0;
    while offset < buf.len() {
        let Some((val, next)) = read_varint(buf, offset) else {
            break;
        };
        out.push(val);
        offset = next;
    }
    out
}

pub fn visit_proto_fields<'a, F>(buf: &'a [u8], mut f: F) -> usize
where
    F: FnMut(u32, ProtoWireValue<'a>),
{
    let mut offset = 0;
    while offset < buf.len() {
        let Some((tag, after_tag)) = read_varint(buf, offset) else {
            break;
        };
        let field_number_u64 = tag >> 3;
        let wire_type = (tag & 0x07) as u8;
        if field_number_u64 == 0 || field_number_u64 > u32::MAX as u64 {
            break;
        }
        let field_number = field_number_u64 as u32;

        match wire_type {
            0 => {
                let Some((val, next_offset)) = read_varint(buf, after_tag) else {
                    break;
                };
                f(field_number, ProtoWireValue::Varint(val));
                offset = next_offset;
            }
            1 => {
                let Some(end) = after_tag.checked_add(8) else {
                    break;
                };
                if end > buf.len() {
                    break;
                }
                let mut fixed = [0u8; 8];
                fixed.copy_from_slice(&buf[after_tag..end]);
                f(field_number, ProtoWireValue::Fixed64(fixed));
                offset = end;
            }
            2 => {
                let Some((raw_len, after_len)) = read_varint(buf, after_tag) else {
                    break;
                };
                let Ok(len) = usize::try_from(raw_len) else {
                    break;
                };
                let Some(end) = after_len.checked_add(len) else {
                    break;
                };
                if end > buf.len() {
                    break;
                }
                f(field_number, ProtoWireValue::Bytes(&buf[after_len..end]));
                offset = end;
            }
            5 => {
                let Some(end) = after_tag.checked_add(4) else {
                    break;
                };
                if end > buf.len() {
                    break;
                }
                let mut fixed = [0u8; 4];
                fixed.copy_from_slice(&buf[after_tag..end]);
                f(field_number, ProtoWireValue::Fixed32(fixed));
                offset = end;
            }
            _ => break,
        }
    }
    offset
}

pub fn parse_proto_timestamp_ms(buf: &[u8]) -> Option<i64> {
    if buf.is_empty() {
        return None;
    }
    let mut secs: Option<u64> = None;
    let mut nanos: u64 = 0;

    visit_proto_fields(buf, |field_number, value| {
        if let ProtoWireValue::Varint(v) = value {
            match field_number {
                1 => {
                    if v > 0 && v <= MAX_POSITIVE_INT64 {
                        secs = Some(v);
                    }
                }
                2 => {
                    if v <= MAX_POSITIVE_INT64 {
                        nanos = v;
                    }
                }
                _ => {}
            }
        }
    });

    let raw_secs = i64::try_from(secs?).ok()?;
    if raw_secs <= 0 {
        return None;
    }
    let extra_ms = i64::try_from(nanos / 1_000_000).ok()?;
    raw_secs.checked_mul(1_000)?.checked_add(extra_ms)
}

pub fn parse_proto_timestamp(buf: &[u8]) -> Option<String> {
    parse_proto_timestamp_ms(buf).and_then(format_utc_ms)
}

pub fn try_parse_usage_proto(buf: &[u8]) -> Option<ParsedUsageProto> {
    if buf.is_empty() {
        return None;
    }
    let mut model_enum: Option<u64> = None;
    let mut input_tokens: u64 = 0;
    let mut raw_combined_output: Option<u64> = None;
    let mut cache_write_tokens: u64 = 0;
    let mut cache_read_tokens: u64 = 0;
    let mut api_provider: Option<u64> = None;
    let mut message_id: Option<String> = None;
    let mut thinking_tokens: u64 = 0;
    let mut response_output_tokens: Option<u64> = None;
    let mut saw_recognized = false;

    for field in parse_proto_fields(buf) {
        match (field.field_number, field.value) {
            (1, ProtoWireValue::Varint(v)) => {
                saw_recognized = true;
                if v > 0 && v <= MAX_POSITIVE_INT64 {
                    model_enum = Some(v);
                }
            }
            (2, ProtoWireValue::Varint(v)) => {
                saw_recognized = true;
                input_tokens = varint_to_token_count(v);
            }
            (3, ProtoWireValue::Varint(v)) => {
                saw_recognized = true;
                raw_combined_output = Some(varint_to_token_count(v));
            }
            (4, ProtoWireValue::Varint(v)) => {
                saw_recognized = true;
                cache_write_tokens = varint_to_token_count(v);
            }
            (5, ProtoWireValue::Varint(v)) => {
                saw_recognized = true;
                cache_read_tokens = varint_to_token_count(v);
            }
            (6, ProtoWireValue::Varint(v)) => {
                saw_recognized = true;
                if v > 0 && v <= MAX_POSITIVE_INT64 {
                    api_provider = Some(v);
                }
            }
            (7, ProtoWireValue::Bytes(b)) => {
                saw_recognized = true;
                if let Ok(s) = std::str::from_utf8(b) {
                    let trimmed = s.trim();
                    if !trimmed.is_empty() {
                        message_id = Some(trimmed.to_string());
                    }
                }
            }
            (9, ProtoWireValue::Varint(v)) => {
                saw_recognized = true;
                thinking_tokens = varint_to_token_count(v);
            }
            (10, ProtoWireValue::Varint(v)) => {
                saw_recognized = true;
                response_output_tokens = Some(varint_to_token_count(v));
            }
            _ => {}
        }
    }

    if !saw_recognized {
        return None;
    }

    let output_tokens = response_output_tokens.unwrap_or_else(|| {
        raw_combined_output
            .unwrap_or(0)
            .saturating_sub(thinking_tokens)
    });

    let total_tokens = input_tokens
        .saturating_add(output_tokens)
        .saturating_add(thinking_tokens)
        .saturating_add(cache_read_tokens)
        .saturating_add(cache_write_tokens);

    Some(ParsedUsageProto {
        model_enum,
        input_tokens,
        raw_combined_output,
        cache_write_tokens,
        cache_read_tokens,
        api_provider,
        message_id,
        thinking_tokens,
        response_output_tokens,
        output_tokens,
        total_tokens,
    })
}

pub fn parse_usage_proto(buf: &[u8]) -> ParsedUsageProto {
    try_parse_usage_proto(buf).unwrap_or(ParsedUsageProto {
        model_enum: None,
        input_tokens: 0,
        raw_combined_output: None,
        cache_write_tokens: 0,
        cache_read_tokens: 0,
        api_provider: None,
        message_id: None,
        thinking_tokens: 0,
        response_output_tokens: None,
        output_tokens: 0,
        total_tokens: 0,
    })
}

pub fn parse_step_metadata_proto(buf: &[u8]) -> ParsedStepMetadataProto {
    let mut timestamp: Option<String> = None;
    let mut usage: Option<ParsedUsageProto> = None;
    let mut fallback_model_enum: Option<u64> = None;

    for field in parse_proto_fields(buf) {
        match (field.field_number, field.value) {
            (1, ProtoWireValue::Bytes(b)) => {
                if let Some(ts) = parse_proto_timestamp(b) {
                    timestamp = Some(ts);
                }
            }
            (9, ProtoWireValue::Bytes(b)) => {
                if let Some(parsed_usage) = try_parse_usage_proto(b) {
                    usage = Some(parsed_usage);
                }
            }
            (11, ProtoWireValue::Varint(v)) => {
                if v > 0 && v <= MAX_POSITIVE_INT64 {
                    fallback_model_enum = Some(v);
                }
            }
            _ => {}
        }
    }

    ParsedStepMetadataProto {
        timestamp,
        usage,
        fallback_model_enum,
    }
}

fn parse_proto_duration(buf: &[u8]) -> Option<Duration> {
    let mut seconds = 0;
    let mut nanos = 0;
    visit_proto_fields(buf, |number, value| match (number, value) {
        (1, ProtoWireValue::Varint(value)) => seconds = value,
        (2, ProtoWireValue::Varint(value)) => nanos = value,
        _ => {}
    });
    (seconds <= MAX_POSITIVE_INT64 && nanos < 1_000_000_000)
        .then(|| Duration::new(seconds, nanos as u32))
}

pub fn parse_gen_metadata_proto(buf: &[u8]) -> ParsedGenMetadataProto {
    let mut step_indices = Vec::new();
    let mut model_enum_numeric: Option<u64> = None;
    let mut model_enum_placeholder: Option<String> = None;
    let mut response_model: Option<String> = None;
    let mut usage: Option<ParsedUsageProto> = None;
    let mut time_to_first_token = None;
    let mut streaming_duration = None;

    for field in parse_proto_fields(buf) {
        match (field.field_number, field.value) {
            (2, ProtoWireValue::Bytes(b)) => {
                for v in parse_packed_varints(b) {
                    if v <= MAX_POSITIVE_INT64 {
                        if let Ok(idx) = usize::try_from(v) {
                            step_indices.push(idx);
                        }
                    }
                }
            }
            (2, ProtoWireValue::Varint(v)) => {
                if v <= MAX_POSITIVE_INT64 {
                    if let Ok(idx) = usize::try_from(v) {
                        step_indices.push(idx);
                    }
                }
            }
            (1, ProtoWireValue::Bytes(inner)) => {
                for inner_field in parse_proto_fields(inner) {
                    match (inner_field.field_number, inner_field.value) {
                        (3, ProtoWireValue::Varint(v)) => {
                            if v > 0 && v <= MAX_POSITIVE_INT64 {
                                model_enum_numeric = Some(v);
                            }
                        }
                        (4, ProtoWireValue::Bytes(u_buf)) => {
                            if !u_buf.is_empty() {
                                usage = Some(parse_usage_proto(u_buf));
                            }
                        }
                        (11, ProtoWireValue::Bytes(duration)) => {
                            time_to_first_token = parse_proto_duration(duration);
                        }
                        (12, ProtoWireValue::Bytes(duration)) => {
                            streaming_duration = parse_proto_duration(duration);
                        }
                        (19, ProtoWireValue::Bytes(rm_buf)) => {
                            if let Ok(s) = std::str::from_utf8(rm_buf) {
                                let trimmed = s.trim();
                                if !trimmed.is_empty() {
                                    response_model = Some(trimmed.to_string());
                                }
                            }
                        }
                        (20, ProtoWireValue::Bytes(kv_buf)) => {
                            let mut key: Option<&str> = None;
                            let mut val: Option<&str> = None;
                            for kv_field in parse_proto_fields(kv_buf) {
                                match (kv_field.field_number, kv_field.value) {
                                    (1, ProtoWireValue::Bytes(kb)) => {
                                        key = std::str::from_utf8(kb).ok();
                                    }
                                    (2, ProtoWireValue::Bytes(vb)) => {
                                        val = std::str::from_utf8(vb).ok();
                                    }
                                    _ => {}
                                }
                            }
                            if key == Some("model_enum") {
                                if let Some(v) = val.map(str::trim).filter(|s| !s.is_empty()) {
                                    model_enum_placeholder = Some(v.to_string());
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    ParsedGenMetadataProto {
        step_indices,
        model_enum_numeric,
        model_enum_placeholder,
        response_model,
        usage,
        time_to_first_token,
        streaming_duration,
    }
}

pub fn try_parse_slim_usage_proto(buf: &[u8]) -> Option<SlimUsageProto> {
    if buf.is_empty() {
        return None;
    }
    let mut model_enum: Option<u64> = None;
    let mut input_tokens: u64 = 0;
    let mut raw_combined_output: Option<u64> = None;
    let mut cache_write_tokens: u64 = 0;
    let mut cache_read_tokens: u64 = 0;
    let mut api_provider: Option<u64> = None;
    let mut thinking_tokens: u64 = 0;
    let mut response_output_tokens: Option<u64> = None;
    let mut saw_recognized = false;

    visit_proto_fields(buf, |field_number, value| match (field_number, value) {
        (1, ProtoWireValue::Varint(v)) => {
            saw_recognized = true;
            if v > 0 && v <= MAX_POSITIVE_INT64 {
                model_enum = Some(v);
            }
        }
        (2, ProtoWireValue::Varint(v)) => {
            saw_recognized = true;
            input_tokens = varint_to_token_count(v);
        }
        (3, ProtoWireValue::Varint(v)) => {
            saw_recognized = true;
            raw_combined_output = Some(varint_to_token_count(v));
        }
        (4, ProtoWireValue::Varint(v)) => {
            saw_recognized = true;
            cache_write_tokens = varint_to_token_count(v);
        }
        (5, ProtoWireValue::Varint(v)) => {
            saw_recognized = true;
            cache_read_tokens = varint_to_token_count(v);
        }
        (6, ProtoWireValue::Varint(v)) => {
            saw_recognized = true;
            if v > 0 && v <= MAX_POSITIVE_INT64 {
                api_provider = Some(v);
            }
        }
        (7, ProtoWireValue::Bytes(_)) => {
            saw_recognized = true;
        }
        (9, ProtoWireValue::Varint(v)) => {
            saw_recognized = true;
            thinking_tokens = varint_to_token_count(v);
        }
        (10, ProtoWireValue::Varint(v)) => {
            saw_recognized = true;
            response_output_tokens = Some(varint_to_token_count(v));
        }
        _ => {}
    });

    if !saw_recognized {
        return None;
    }

    let output_tokens = response_output_tokens.unwrap_or_else(|| {
        raw_combined_output
            .unwrap_or(0)
            .saturating_sub(thinking_tokens)
    });
    let total_tokens = input_tokens
        .saturating_add(output_tokens)
        .saturating_add(thinking_tokens)
        .saturating_add(cache_read_tokens)
        .saturating_add(cache_write_tokens);

    Some(SlimUsageProto {
        model_enum,
        api_provider,
        input_tokens,
        output_tokens,
        thinking_tokens,
        cache_read_tokens,
        cache_write_tokens,
        total_tokens,
    })
}

pub fn parse_step_metadata_slim(buf: &[u8]) -> SlimStepMetadataProto {
    let mut timestamp_ms: Option<i64> = None;
    let mut usage: Option<SlimUsageProto> = None;
    let mut fallback_model_enum: Option<u64> = None;

    visit_proto_fields(buf, |field_number, value| match (field_number, value) {
        (1, ProtoWireValue::Bytes(b)) => {
            if let Some(ms) = parse_proto_timestamp_ms(b) {
                timestamp_ms = Some(ms);
            }
        }
        (9, ProtoWireValue::Bytes(b)) => {
            if let Some(parsed_usage) = try_parse_slim_usage_proto(b) {
                usage = Some(parsed_usage);
            }
        }
        (11, ProtoWireValue::Varint(v)) => {
            if v > 0 && v <= MAX_POSITIVE_INT64 {
                fallback_model_enum = Some(v);
            }
        }
        _ => {}
    });

    SlimStepMetadataProto {
        timestamp_ms,
        usage,
        fallback_model_enum,
    }
}

pub fn parse_gen_metadata_slim(buf: &[u8], need_usage: bool) -> SlimGenMetadataProto {
    let mut step_indices = Vec::new();
    let mut model_enum_numeric: Option<u64> = None;
    let mut model_enum_placeholder: Option<String> = None;
    let mut response_model: Option<String> = None;
    let mut usage: Option<SlimUsageProto> = None;

    visit_proto_fields(buf, |field_number, value| match (field_number, value) {
        (2, ProtoWireValue::Bytes(b)) => {
            let mut offset = 0;
            while offset < b.len() {
                let Some((v, next)) = read_varint(b, offset) else {
                    break;
                };
                if v <= MAX_POSITIVE_INT64 {
                    if let Ok(idx) = usize::try_from(v) {
                        step_indices.push(idx);
                    }
                }
                offset = next;
            }
        }
        (2, ProtoWireValue::Varint(v)) => {
            if v <= MAX_POSITIVE_INT64 {
                if let Ok(idx) = usize::try_from(v) {
                    step_indices.push(idx);
                }
            }
        }
        (1, ProtoWireValue::Bytes(inner)) => {
            visit_proto_fields(inner, |inner_num, inner_val| match (inner_num, inner_val) {
                (3, ProtoWireValue::Varint(v)) => {
                    if v > 0 && v <= MAX_POSITIVE_INT64 {
                        model_enum_numeric = Some(v);
                    }
                }
                (4, ProtoWireValue::Bytes(u_buf)) if need_usage && !u_buf.is_empty() => {
                    usage = Some(try_parse_slim_usage_proto(u_buf).unwrap_or_default());
                }
                (19, ProtoWireValue::Bytes(rm_buf)) => {
                    if let Ok(s) = std::str::from_utf8(rm_buf) {
                        let trimmed = s.trim();
                        if !trimmed.is_empty() {
                            response_model = Some(trimmed.to_string());
                        }
                    }
                }
                (20, ProtoWireValue::Bytes(kv_buf)) => {
                    let mut key: Option<&str> = None;
                    let mut val: Option<&str> = None;
                    visit_proto_fields(kv_buf, |kv_num, kv_val| match (kv_num, kv_val) {
                        (1, ProtoWireValue::Bytes(kb)) => key = std::str::from_utf8(kb).ok(),
                        (2, ProtoWireValue::Bytes(vb)) => val = std::str::from_utf8(vb).ok(),
                        _ => {}
                    });
                    if key == Some("model_enum") {
                        if let Some(v) = val.map(str::trim).filter(|s| !s.is_empty()) {
                            model_enum_placeholder = Some(v.to_string());
                        }
                    }
                }
                _ => {}
            });
        }
        _ => {}
    });

    SlimGenMetadataProto {
        step_indices,
        model_enum_numeric,
        model_enum_placeholder,
        response_model,
        usage,
    }
}

fn is_printable_utf8_text(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    !s.bytes()
        .any(|b| matches!(b, 0x00..=0x08 | 0x0b | 0x0c | 0x0e..=0x1f))
}

fn walk_readable_utf8_bytes(buf: &[u8], depth: usize) -> usize {
    if buf.is_empty() || depth > 6 {
        return 0;
    }

    let (fields, consumed) = parse_proto_fields_internal(buf);
    let looks_like_complete_proto = !fields.is_empty()
        && consumed == buf.len()
        && fields.iter().all(|f| f.field_number <= 4096);

    if looks_like_complete_proto {
        let has_bytes_children = fields
            .iter()
            .any(|f| matches!(f.value, ProtoWireValue::Bytes(_)));
        if has_bytes_children && depth < 6 {
            let mut nested_sum = 0usize;
            for f in &fields {
                if let ProtoWireValue::Bytes(child) = f.value {
                    if let Ok(s) = std::str::from_utf8(child) {
                        if is_printable_utf8_text(s) {
                            nested_sum = nested_sum.saturating_add(s.len());
                            continue;
                        }
                    }
                    nested_sum =
                        nested_sum.saturating_add(walk_readable_utf8_bytes(child, depth + 1));
                }
            }
            if nested_sum > 0 {
                return nested_sum;
            }
        }
    }

    if let Ok(s) = std::str::from_utf8(buf) {
        if is_printable_utf8_text(s) {
            return s.len();
        }
    }

    if depth < 6 {
        let mut partial_sum = 0usize;
        for f in &fields {
            if let ProtoWireValue::Bytes(child) = f.value {
                partial_sum =
                    partial_sum.saturating_add(walk_readable_utf8_bytes(child, depth + 1));
            }
        }
        if partial_sum > 0 {
            return partial_sum;
        }
    }

    0
}

pub fn extract_readable_utf8_bytes(buf: &[u8], fallback_len: usize) -> usize {
    if buf.is_empty() {
        return fallback_len;
    }
    let walked = walk_readable_utf8_bytes(buf, 0);
    if walked > 0 {
        walked
    } else if fallback_len > 0 {
        fallback_len
    } else {
        buf.len()
    }
}

#[inline]
pub fn estimate_tokens_from_bytes(readable_bytes: usize) -> u64 {
    (readable_bytes as u64).div_ceil(4).max(1)
}

#[cfg(test)]
pub fn encode_varint(mut value: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(10);
    loop {
        if value < 0x80 {
            out.push(value as u8);
            break;
        }
        out.push(((value & 0x7f) as u8) | 0x80);
        value >>= 7;
    }
    out
}

#[cfg(test)]
pub fn encode_varint_field(field_number: u32, value: u64) -> Vec<u8> {
    let tag = ((field_number as u64) << 3) | 0;
    let mut out = encode_varint(tag);
    out.extend_from_slice(&encode_varint(value));
    out
}

#[cfg(test)]
pub fn encode_bytes_field(field_number: u32, payload: &[u8]) -> Vec<u8> {
    let tag = ((field_number as u64) << 3) | 2;
    let mut out = encode_varint(tag);
    out.extend_from_slice(&encode_varint(payload.len() as u64));
    out.extend_from_slice(payload);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_usage_proto_enforces_zero_thinking_double_counting() {
        // Case A: f10 present (f3 = 386 = 329 thinking + 57 response)
        let mut usage_a = Vec::new();
        usage_a.extend_from_slice(&encode_varint_field(1, 1427));
        usage_a.extend_from_slice(&encode_varint_field(2, 33_202));
        usage_a.extend_from_slice(&encode_varint_field(3, 386));
        usage_a.extend_from_slice(&encode_varint_field(4, 100));
        usage_a.extend_from_slice(&encode_varint_field(5, 10_000));
        usage_a.extend_from_slice(&encode_varint_field(6, 24));
        usage_a.extend_from_slice(&encode_bytes_field(7, b"bot-msg-1"));
        usage_a.extend_from_slice(&encode_varint_field(9, 329));
        usage_a.extend_from_slice(&encode_varint_field(10, 57));

        let parsed_a = parse_usage_proto(&usage_a);
        assert_eq!(parsed_a.model_enum, Some(1427));
        assert_eq!(parsed_a.input_tokens, 33_202);
        assert_eq!(parsed_a.thinking_tokens, 329);
        assert_eq!(parsed_a.output_tokens, 57);
        assert_eq!(parsed_a.cache_write_tokens, 100);
        assert_eq!(parsed_a.cache_read_tokens, 10_000);
        assert_eq!(parsed_a.total_tokens, 33_202 + 57 + 329 + 10_000 + 100);

        // Case B: proto3 omits f10 when response_output_tokens == 0
        let mut usage_b = Vec::new();
        usage_b.extend_from_slice(&encode_varint_field(2, 1_000));
        usage_b.extend_from_slice(&encode_varint_field(3, 500));
        usage_b.extend_from_slice(&encode_varint_field(9, 500));

        let parsed_b = parse_usage_proto(&usage_b);
        assert_eq!(parsed_b.response_output_tokens, None);
        assert_eq!(parsed_b.thinking_tokens, 500);
        assert_eq!(parsed_b.output_tokens, 0);
        assert_eq!(parsed_b.total_tokens, 1_500);
    }

    #[test]
    fn parse_step_and_gen_metadata_and_nested_utf8_estimation() {
        let mut ts_buf = Vec::new();
        ts_buf.extend_from_slice(&encode_varint_field(1, 1_700_000_000));
        ts_buf.extend_from_slice(&encode_varint_field(2, 248_000_000));
        assert_eq!(
            parse_proto_timestamp(&ts_buf).as_deref(),
            Some("2023-11-14T22:13:20.248Z")
        );

        let mut leaf = Vec::new();
        leaf.extend_from_slice(&encode_bytes_field(1, "Hello from Rust".as_bytes()));
        leaf.extend_from_slice(&encode_bytes_field(2, b"12345"));
        let outer = encode_bytes_field(15, &leaf);
        let readable = extract_readable_utf8_bytes(&outer, 999);
        assert_eq!(readable, 20);
        assert_eq!(estimate_tokens_from_bytes(readable), 5);
    }
}
