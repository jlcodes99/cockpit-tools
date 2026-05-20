use base64::{engine::general_purpose, Engine as _};

/// Protobuf Varint 编码
pub fn encode_varint(mut value: u64) -> Vec<u8> {
    let mut buf = Vec::new();
    while value >= 0x80 {
        buf.push((value & 0x7F | 0x80) as u8);
        value >>= 7;
    }
    buf.push(value as u8);
    buf
}

/// 编码长度分隔字段 (wire_type = 2)
pub fn encode_len_delim_field(field_num: u32, data: &[u8]) -> Vec<u8> {
    let tag = (field_num << 3) | 2;
    let mut f = encode_varint(tag as u64);
    f.extend(encode_varint(data.len() as u64));
    f.extend_from_slice(data);
    f
}

/// 编码字符串字段 (wire_type = 2)
pub fn encode_string_field(field_num: u32, value: &str) -> Vec<u8> {
    encode_len_delim_field(field_num, value.as_bytes())
}

/// 读取 Protobuf Varint
pub fn read_varint(data: &[u8], offset: usize) -> Result<(u64, usize), String> {
    let mut result = 0u64;
    let mut shift = 0;
    let mut pos = offset;

    loop {
        if pos >= data.len() {
            return Err("数据不完整".to_string());
        }
        let byte = data[pos];
        result |= ((byte & 0x7F) as u64) << shift;
        pos += 1;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
    }

    Ok((result, pos))
}

/// 跳过 Protobuf 字段
pub fn skip_field(data: &[u8], offset: usize, wire_type: u8) -> Result<usize, String> {
    match wire_type {
        0 => {
            // Varint
            let (_, new_offset) = read_varint(data, offset)?;
            Ok(new_offset)
        }
        1 => {
            // 64-bit
            Ok(offset + 8)
        }
        2 => {
            // Length-delimited
            let (length, content_offset) = read_varint(data, offset)?;
            Ok(content_offset + length as usize)
        }
        5 => {
            // 32-bit
            Ok(offset + 4)
        }
        _ => Err(format!("未知 wire_type: {}", wire_type)),
    }
}

/// 创建 OAuthTokenInfo 消息
pub fn create_oauth_info(access_token: &str, refresh_token: &str, expiry: i64) -> Vec<u8> {
    // Field 1: access_token (string, wire_type = 2)
    let field1 = encode_string_field(1, access_token);

    // Field 2: token_type (string, fixed value "Bearer", wire_type = 2)
    let field2 = encode_string_field(2, "Bearer");

    // Field 3: refresh_token (string, wire_type = 2)
    let field3 = encode_string_field(3, refresh_token);

    // Field 4: expiry (嵌套的 Timestamp 消息, wire_type = 2)
    let timestamp_tag = (1 << 3) | 0;
    let mut timestamp_msg = encode_varint(timestamp_tag);
    timestamp_msg.extend(encode_varint(expiry as u64));

    let field4 = encode_len_delim_field(4, &timestamp_msg);

    // 合并所有字段为 OAuthTokenInfo 消息
    [field1, field2, field3, field4].concat()
}

/// 从 Topic.data 中移除指定 map entry，保留同 topic 下其他 sentinel row。
pub fn remove_unified_topic_entry(data: &[u8], target_key: &str) -> Result<Vec<u8>, String> {
    let mut result = Vec::new();
    let mut offset = 0;

    while offset < data.len() {
        let start_offset = offset;
        let (tag, new_offset) = read_varint(data, offset)?;
        let wire_type = (tag & 7) as u8;
        let field_num = (tag >> 3) as u32;
        let next_offset = skip_field(data, new_offset, wire_type)?;

        let should_remove = if field_num == 1 && wire_type == 2 {
            let (length, content_offset) = read_varint(data, new_offset)?;
            let length = length as usize;
            if content_offset + length > data.len() {
                return Err("Topic.data entry 数据不完整".to_string());
            }
            let entry = &data[content_offset..content_offset + length];
            unified_topic_entry_key(entry) == Some(target_key)
        } else {
            false
        };

        if !should_remove {
            result.extend_from_slice(&data[start_offset..next_offset]);
        }
        offset = next_offset;
    }

    Ok(result)
}

fn unified_topic_entry_key(data: &[u8]) -> Option<&str> {
    let mut offset = 0;
    while offset < data.len() {
        let (tag, new_offset) = read_varint(data, offset).ok()?;
        let wire_type = (tag & 7) as u8;
        let field_num = (tag >> 3) as u32;

        if field_num == 1 && wire_type == 2 {
            let (length, content_offset) = read_varint(data, new_offset).ok()?;
            let length = length as usize;
            if content_offset + length > data.len() {
                return None;
            }
            return std::str::from_utf8(&data[content_offset..content_offset + length]).ok();
        }

        offset = skip_field(data, new_offset, wire_type).ok()?;
    }

    None
}

/// 从 antigravityUnifiedStateSync.oauthToken 中提取 refresh_token。
/// 结构: Topic.data[oauthTokenInfoSentinelKey].Row.value = base64(OAuthTokenInfo)，再取 Field 3。
pub fn extract_refresh_token_from_unified_oauth_token(data: &[u8]) -> Option<String> {
    let mut offset = 0;
    while offset < data.len() {
        let (tag, new_offset) = read_varint(data, offset).ok()?;
        let wire_type = (tag & 7) as u8;
        let field_num = (tag >> 3) as u32;

        if field_num == 1 && wire_type == 2 {
            let (length, content_offset) = read_varint(data, new_offset).ok()?;
            let length = length as usize;
            if content_offset + length > data.len() {
                return None;
            }
            let entry = &data[content_offset..content_offset + length];
            if let Some(refresh_token) = extract_refresh_token_from_unified_entry(entry) {
                return Some(refresh_token);
            }
        }

        offset = skip_field(data, new_offset, wire_type).ok()?;
    }

    None
}

fn extract_refresh_token_from_unified_entry(data: &[u8]) -> Option<String> {
    let mut offset = 0;
    let mut sentinel_matched = false;
    let mut row_data: Option<Vec<u8>> = None;

    while offset < data.len() {
        let (tag, new_offset) = read_varint(data, offset).ok()?;
        let wire_type = (tag & 7) as u8;
        let field_num = (tag >> 3) as u32;

        if wire_type == 2 {
            let (length, content_offset) = read_varint(data, new_offset).ok()?;
            let length = length as usize;
            if content_offset + length > data.len() {
                return None;
            }
            let value = &data[content_offset..content_offset + length];
            if field_num == 1 {
                sentinel_matched = std::str::from_utf8(value).ok()? == "oauthTokenInfoSentinelKey";
            } else if field_num == 2 {
                row_data = Some(value.to_vec());
            }
        }

        offset = skip_field(data, new_offset, wire_type).ok()?;
    }

    if !sentinel_matched {
        return None;
    }

    let row_data = row_data?;
    let oauth_info_b64 = extract_string_field(&row_data, 1)?;
    let oauth_info = general_purpose::STANDARD.decode(oauth_info_b64).ok()?;
    extract_string_field(&oauth_info, 3)
}

/// 从 protobuf 消息中提取指定字段的字符串
fn extract_string_field(data: &[u8], target_field: u32) -> Option<String> {
    let mut offset = 0;
    while offset < data.len() {
        let (tag, new_offset) = read_varint(data, offset).ok()?;
        let wire_type = (tag & 7) as u8;
        let field_num = (tag >> 3) as u32;

        if field_num == target_field && wire_type == 2 {
            let (length, content_offset) = read_varint(data, new_offset).ok()?;
            let length = length as usize;
            if content_offset + length > data.len() {
                return None;
            }
            let value = &data[content_offset..content_offset + length];
            return String::from_utf8(value.to_vec()).ok();
        }

        offset = skip_field(data, new_offset, wire_type).ok()?;
    }

    None
}
