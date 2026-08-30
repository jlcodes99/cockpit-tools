// Codex Local Access：Image handling and Chat Completions/Responses request transformation。
// 通过 include! 保持原 modules::codex_local_access 作用域和私有调用关系。
fn remove_image_generation_capabilities_from_object(object: &mut Map<String, Value>) -> bool {
    let mut changed = false;
    if let Some(Value::Array(tools)) = object.get_mut("tools") {
        let before = tools.len();
        tools.retain(|tool| !tool_declares_image_generation_capability(tool));
        changed |= tools.len() != before;
    }

    if object
        .get("tool_choice")
        .is_some_and(tool_choice_selects_image_generation)
    {
        object.remove("tool_choice");
        changed = true;
    }

    if let Some(Value::Array(input)) = object.get_mut("input") {
        let before = input.len();
        input.retain_mut(|item| {
            let Some(item_object) = item.as_object_mut() else {
                return true;
            };
            if item_object.get("type").and_then(Value::as_str) != Some("additional_tools") {
                return true;
            }
            changed |= remove_image_generation_capabilities_from_object(item_object);
            !item_object
                .get("tools")
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty)
        });
        changed |= input.len() != before;
    }

    if let Some(Value::Object(response)) = object.get_mut("response") {
        changed |= remove_image_generation_capabilities_from_object(response);
    }

    changed
}

fn image_generation_tools_allowed(
    mode: CodexLocalAccessImageGenerationMode,
    request_kind: CodexLocalAccessRequestKind,
) -> bool {
    match mode {
        CodexLocalAccessImageGenerationMode::Enabled => true,
        CodexLocalAccessImageGenerationMode::ImagesOnly => request_kind_is_image(request_kind),
        CodexLocalAccessImageGenerationMode::Disabled => false,
    }
}

fn request_image_generation_mode(
    collection_mode: CodexLocalAccessImageGenerationMode,
    headers: &HashMap<String, String>,
) -> CodexLocalAccessImageGenerationMode {
    // Collection-level hard disable always wins.
    if collection_mode == CodexLocalAccessImageGenerationMode::Disabled {
        return CodexLocalAccessImageGenerationMode::Disabled;
    }
    // Responses Lite cannot host image_generation in chat/responses payloads.
    // Keep image endpoints available via ImagesOnly.
    if headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case(CODEX_RESPONSES_LITE_HEADER))
    {
        return CodexLocalAccessImageGenerationMode::ImagesOnly;
    }
    let value = headers
        .get(CODEX_LOCAL_ACCESS_DISABLE_HOSTED_IMAGE_GENERATION_HEADER)
        .map(String::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match value.as_str() {
        "chat" | "images_only" | "images-only" => CodexLocalAccessImageGenerationMode::ImagesOnly,
        "true" | "all" | "disabled" => CodexLocalAccessImageGenerationMode::Disabled,
        _ => collection_mode,
    }
}

fn build_images_responses_body(prompt: &str, images: &[String], tool: Value) -> Value {
    let mut content = vec![json!({
        "type": "input_text",
        "text": prompt,
    })];
    for image in images {
        let image_url = image.trim();
        if image_url.is_empty() {
            continue;
        }
        content.push(json!({
            "type": "input_image",
            "image_url": image_url,
        }));
    }

    json!({
        "instructions": "",
        "stream": true,
        "reasoning": {
            "effort": "medium",
            "summary": "auto",
        },
        "parallel_tool_calls": true,
        "include": ["reasoning.encrypted_content"],
        "model": DEFAULT_IMAGES_MAIN_MODEL,
        "store": false,
        "tool_choice": {
            "type": "image_generation",
        },
        "input": [{
            "type": "message",
            "role": "user",
            "content": content,
        }],
        "tools": [tool],
    })
}

fn build_images_generation_request(body: &Value) -> Result<(Value, bool, String), String> {
    let request_obj = body
        .as_object()
        .ok_or("images/generations 请求体必须是 JSON 对象".to_string())?;
    let prompt = json_string_field(request_obj, "prompt")
        .ok_or("images/generations 请求缺少 prompt".to_string())?;
    let response_format = normalize_image_response_format(request_obj.get("response_format"));
    let stream = request_obj
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let tool = build_image_generation_tool(request_obj, "generate", false)?;

    Ok((
        build_images_responses_body(prompt, &[], tool),
        stream,
        response_format,
    ))
}

fn extract_json_edit_images(request_obj: &Map<String, Value>) -> Vec<String> {
    let mut images = Vec::new();

    if let Some(image) = request_obj.get("image").and_then(Value::as_str) {
        let trimmed = image.trim();
        if !trimmed.is_empty() {
            images.push(trimmed.to_string());
        }
    }

    if let Some(image_array) = request_obj.get("images").and_then(Value::as_array) {
        for image in image_array {
            if let Some(url) = image
                .get("image_url")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                images.push(url.to_string());
            } else if let Some(url) = image
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                images.push(url.to_string());
            }
        }
    }

    images
}

fn build_images_edit_request_from_json(body: &Value) -> Result<(Value, bool, String), String> {
    let request_obj = body
        .as_object()
        .ok_or("images/edits 请求体必须是 JSON 对象".to_string())?;
    let prompt = json_string_field(request_obj, "prompt")
        .ok_or("images/edits 请求缺少 prompt".to_string())?;
    let images = extract_json_edit_images(request_obj);
    if images.is_empty() {
        return Err("images/edits 请求缺少 images[].image_url".to_string());
    }

    let response_format = normalize_image_response_format(request_obj.get("response_format"));
    let stream = request_obj
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut tool = build_image_generation_tool(request_obj, "edit", true)?;
    if let Some(mask_url) = request_obj
        .get("mask")
        .and_then(|mask| mask.get("image_url"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Some(tool_obj) = tool.as_object_mut() {
            tool_obj.insert(
                "input_image_mask".to_string(),
                json!({ "image_url": mask_url }),
            );
        }
    }

    Ok((
        build_images_responses_body(prompt, &images, tool),
        stream,
        response_format,
    ))
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn extract_multipart_boundary(content_type: &str) -> Option<String> {
    content_type.split(';').find_map(|part| {
        let trimmed = part.trim();
        let (name, value) = trimmed.split_once('=')?;
        if !name.trim().eq_ignore_ascii_case("boundary") {
            return None;
        }
        let boundary = value.trim().trim_matches('"').to_string();
        if boundary.is_empty() {
            None
        } else {
            Some(boundary)
        }
    })
}

fn parse_content_disposition_params(value: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    for part in value.split(';').skip(1) {
        let Some((name, raw_value)) = part.trim().split_once('=') else {
            continue;
        };
        let key = name.trim().to_ascii_lowercase();
        let value = raw_value.trim().trim_matches('"').to_string();
        if !key.is_empty() {
            params.insert(key, value);
        }
    }
    params
}

fn trim_part_trailing_newline(mut data: &[u8]) -> &[u8] {
    if data.ends_with(b"\r\n") {
        data = &data[..data.len().saturating_sub(2)];
    } else if data.ends_with(b"\n") {
        data = &data[..data.len().saturating_sub(1)];
    }
    data
}

fn parse_multipart_form_data(content_type: &str, body: &[u8]) -> Result<MultipartFormData, String> {
    let boundary = extract_multipart_boundary(content_type)
        .ok_or("multipart/form-data 缺少 boundary".to_string())?;
    let marker = format!("--{}", boundary).into_bytes();
    let mut form = MultipartFormData::default();
    let mut search_from = 0usize;

    loop {
        let Some(marker_index) = find_subslice(&body[search_from..], &marker) else {
            break;
        };
        let marker_start = search_from + marker_index;
        let mut part_start = marker_start + marker.len();

        if body
            .get(part_start..part_start + 2)
            .map(|bytes| bytes == b"--")
            .unwrap_or(false)
        {
            break;
        }
        if body
            .get(part_start..part_start + 2)
            .map(|bytes| bytes == b"\r\n")
            .unwrap_or(false)
        {
            part_start += 2;
        } else if body
            .get(part_start..part_start + 1)
            .map(|bytes| bytes == b"\n")
            .unwrap_or(false)
        {
            part_start += 1;
        }

        let Some(next_marker_offset) = find_subslice(&body[part_start..], &marker) else {
            break;
        };
        let next_marker_start = part_start + next_marker_offset;
        let part = trim_part_trailing_newline(&body[part_start..next_marker_start]);
        search_from = next_marker_start;

        let Some(header_end) = find_header_end(part) else {
            continue;
        };
        let header_text = String::from_utf8_lossy(&part[..header_end]);
        let part_body = &part[header_end..];
        let mut part_name = String::new();
        let mut part_filename = String::new();
        let mut part_content_type = String::new();

        for line in header_text.lines() {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            if name.trim().eq_ignore_ascii_case("content-disposition") {
                let params = parse_content_disposition_params(value);
                part_name = params.get("name").cloned().unwrap_or_default();
                part_filename = params.get("filename").cloned().unwrap_or_default();
            } else if name.trim().eq_ignore_ascii_case("content-type") {
                part_content_type = value.trim().to_string();
            }
        }

        if part_name.is_empty() {
            continue;
        }
        if part_filename.is_empty() {
            let text = String::from_utf8_lossy(part_body).trim().to_string();
            form.fields.insert(part_name, text);
        } else {
            form.files.push(MultipartFilePart {
                name: part_name,
                content_type: part_content_type,
                data: part_body.to_vec(),
            });
        }
    }

    Ok(form)
}

fn detect_image_mime_type(data: &[u8], fallback: &str) -> String {
    let fallback = fallback.trim();
    if !fallback.is_empty() && fallback != "application/octet-stream" {
        return fallback.to_string();
    }
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png".to_string()
    } else if data.starts_with(b"\xff\xd8\xff") {
        "image/jpeg".to_string()
    } else if data.starts_with(b"RIFF")
        && data
            .get(8..12)
            .map(|bytes| bytes == b"WEBP")
            .unwrap_or(false)
    {
        "image/webp".to_string()
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        "image/gif".to_string()
    } else {
        "application/octet-stream".to_string()
    }
}

fn multipart_file_to_data_url(file: &MultipartFilePart) -> String {
    let mime_type = detect_image_mime_type(&file.data, &file.content_type);
    format!(
        "data:{};base64,{}",
        mime_type,
        general_purpose::STANDARD.encode(&file.data)
    )
}

fn multipart_field_value<'a>(form: &'a MultipartFormData, key: &str) -> Option<&'a str> {
    form.fields
        .get(key)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn multipart_field_bool(form: &MultipartFormData, key: &str, fallback: bool) -> bool {
    match multipart_field_value(form, key)
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => fallback,
    }
}

fn multipart_field_number(form: &MultipartFormData, key: &str) -> Option<Value> {
    let raw = multipart_field_value(form, key)?;
    raw.parse::<i64>().ok().map(|value| json!(value))
}

fn build_images_edit_request_from_multipart(
    content_type: &str,
    body: &[u8],
) -> Result<(Value, bool, String), String> {
    let form = parse_multipart_form_data(content_type, body)?;
    let prompt =
        multipart_field_value(&form, "prompt").ok_or("images/edits 请求缺少 prompt".to_string())?;
    let image_files: Vec<&MultipartFilePart> = form
        .files
        .iter()
        .filter(|file| file.name == "image" || file.name == "image[]")
        .collect();
    if image_files.is_empty() {
        return Err("images/edits 请求缺少 image".to_string());
    }

    let mut request_obj = Map::new();
    request_obj.insert(
        "model".to_string(),
        Value::String(
            multipart_field_value(&form, "model")
                .unwrap_or(CODEX_IMAGE_MODEL_ID)
                .to_string(),
        ),
    );
    for key in [
        "size",
        "quality",
        "background",
        "output_format",
        "input_fidelity",
        "moderation",
    ] {
        if let Some(value) = multipart_field_value(&form, key) {
            request_obj.insert(key.to_string(), Value::String(value.to_string()));
        }
    }
    for key in ["output_compression", "partial_images"] {
        if let Some(value) = multipart_field_number(&form, key) {
            request_obj.insert(key.to_string(), value);
        }
    }

    let response_format = multipart_field_value(&form, "response_format")
        .unwrap_or("b64_json")
        .to_ascii_lowercase();
    let stream = multipart_field_bool(&form, "stream", false);
    let mut tool = build_image_generation_tool(&request_obj, "edit", true)?;
    if let Some(mask_file) = form.files.iter().find(|file| file.name == "mask") {
        if let Some(tool_obj) = tool.as_object_mut() {
            tool_obj.insert(
                "input_image_mask".to_string(),
                json!({ "image_url": multipart_file_to_data_url(mask_file) }),
            );
        }
    }

    let images: Vec<String> = image_files
        .into_iter()
        .map(multipart_file_to_data_url)
        .collect();

    Ok((
        build_images_responses_body(prompt, &images, tool),
        stream,
        response_format,
    ))
}

fn build_request_routing_hint(request: &ParsedRequest) -> RequestRoutingHint {
    let Some(body) = parse_request_body_json(&request.body) else {
        return RequestRoutingHint {
            session_affinity_key: extract_session_affinity_key(request),
            ..RequestRoutingHint::default()
        };
    };

    RequestRoutingHint {
        model_key: body
            .get("model")
            .and_then(Value::as_str)
            .map(resolve_supported_model_alias)
            .map(|model| normalize_model_key(&model))
            .unwrap_or_default(),
        previous_response_id: body
            .get("previous_response_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        session_affinity_key: extract_session_affinity_key(request),
    }
}

fn is_chat_completions_request(target: &str) -> bool {
    let path = target.split('?').next().unwrap_or(target).trim();
    path == CHAT_COMPLETIONS_PATH || path.ends_with("/chat/completions")
}

fn is_responses_completion_event(event_type: &str) -> bool {
    matches!(event_type, "response.completed" | "response.done")
}

fn response_text_type_for_role(role: &str) -> &'static str {
    if role.eq_ignore_ascii_case("assistant") {
        "output_text"
    } else {
        "input_text"
    }
}

fn truncate_to_byte_limit(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_string();
    }

    let mut end = 0usize;
    for (index, ch) in value.char_indices() {
        let next = index + ch.len_utf8();
        if next > limit {
            break;
        }
        end = next;
    }
    value[..end].to_string()
}

fn shorten_tool_name_if_needed(name: &str) -> String {
    const LIMIT: usize = 64;
    if name.len() <= LIMIT {
        return name.to_string();
    }
    if name.starts_with("mcp__") {
        if let Some(index) = name.rfind("__") {
            if index > 0 {
                let candidate = format!("mcp__{}", &name[index + 2..]);
                return truncate_to_byte_limit(&candidate, LIMIT);
            }
        }
    }
    truncate_to_byte_limit(name, LIMIT)
}

fn build_short_tool_name_map(body: &Value) -> HashMap<String, String> {
    const LIMIT: usize = 64;

    let mut names = Vec::new();
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        for tool in tools {
            if tool.get("type").and_then(Value::as_str) != Some("function") {
                continue;
            }
            if let Some(name) = tool
                .get("function")
                .and_then(Value::as_object)
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
            {
                names.push(name.to_string());
            }
        }
    }

    let mut used = HashSet::new();
    let mut short_name_map = HashMap::new();
    for name in names {
        let base_candidate = shorten_tool_name_if_needed(&name);
        let unique = if used.insert(base_candidate.clone()) {
            base_candidate
        } else {
            let mut suffix_index = 1usize;
            loop {
                let suffix = format!("_{}", suffix_index);
                let allowed = LIMIT.saturating_sub(suffix.len());
                let candidate = format!(
                    "{}{}",
                    truncate_to_byte_limit(&base_candidate, allowed),
                    suffix
                );
                if used.insert(candidate.clone()) {
                    break candidate;
                }
                suffix_index += 1;
            }
        };
        short_name_map.insert(name, unique);
    }

    short_name_map
}

fn build_reverse_tool_name_map_from_request(
    original_request_body: &[u8],
) -> HashMap<String, String> {
    let Some(body) = parse_request_body_json(original_request_body) else {
        return HashMap::new();
    };

    build_short_tool_name_map(&body)
        .into_iter()
        .map(|(original, shortened)| (shortened, original))
        .collect()
}

fn map_tool_name(name: &str, short_name_map: &HashMap<String, String>) -> String {
    short_name_map
        .get(name)
        .cloned()
        .unwrap_or_else(|| shorten_tool_name_if_needed(name))
}

fn normalize_chat_content_part(part: &Value, role: &str) -> Option<Value> {
    match part {
        Value::String(text) => Some(json!({
            "type": response_text_type_for_role(role),
            "text": text,
        })),
        Value::Object(obj) => {
            let part_type = obj.get("type").and_then(Value::as_str).unwrap_or("");
            match part_type {
                "" | "text" => {
                    let text = obj.get("text").and_then(Value::as_str).unwrap_or("");
                    Some(json!({
                        "type": response_text_type_for_role(role),
                        "text": text,
                    }))
                }
                "image_url" => {
                    if !role.eq_ignore_ascii_case("user") {
                        return None;
                    }
                    let image_url_value = obj.get("image_url")?;
                    match image_url_value {
                        Value::Object(image_url_obj) => {
                            let url = image_url_obj.get("url").and_then(Value::as_str)?;
                            Some(json!({
                                "type": "input_image",
                                "image_url": url,
                            }))
                        }
                        _ => None,
                    }
                }
                "file" => {
                    if !role.eq_ignore_ascii_case("user") {
                        return None;
                    }
                    let file_data = obj
                        .get("file")
                        .and_then(Value::as_object)
                        .and_then(|file| file.get("file_data"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if file_data.is_empty() {
                        return None;
                    }
                    let filename = obj
                        .get("file")
                        .and_then(Value::as_object)
                        .and_then(|file| file.get("filename"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let mut next = Map::new();
                    next.insert("type".to_string(), Value::String("input_file".to_string()));
                    next.insert(
                        "file_data".to_string(),
                        Value::String(file_data.to_string()),
                    );
                    if !filename.is_empty() {
                        next.insert("filename".to_string(), Value::String(filename.to_string()));
                    }
                    Some(Value::Object(next))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

fn normalize_chat_content_parts(content: &Value, role: &str) -> Vec<Value> {
    match content {
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| normalize_chat_content_part(part, role))
            .collect(),
        other => normalize_chat_content_part(other, role)
            .map(|part| vec![part])
            .unwrap_or_default(),
    }
}

fn normalize_chat_tool_call(
    tool_call: &Value,
    short_name_map: &HashMap<String, String>,
) -> Option<Value> {
    let tool_call_obj = tool_call.as_object()?;
    let tool_type = tool_call_obj
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("function");
    if tool_type != "function" {
        return None;
    }

    let function_obj = tool_call_obj.get("function").and_then(Value::as_object);
    let name = function_obj
        .and_then(|function| function.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let arguments = function_obj
        .and_then(|function| function.get("arguments"))
        .and_then(Value::as_str)
        .unwrap_or("{}");
    let call_id = tool_call_obj
        .get("id")
        .or_else(|| tool_call_obj.get("call_id"))
        .and_then(Value::as_str)
        .unwrap_or("");

    Some(json!({
        "type": "function_call",
        "call_id": call_id,
        "name": map_tool_name(name, short_name_map),
        "arguments": arguments,
    }))
}

fn normalize_chat_tool_calls(
    tool_calls: &Value,
    short_name_map: &HashMap<String, String>,
) -> Vec<Value> {
    tool_calls
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|tool_call| normalize_chat_tool_call(tool_call, short_name_map))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn normalize_chat_message_for_responses(
    message_obj: &Map<String, Value>,
    short_name_map: &HashMap<String, String>,
) -> Vec<Value> {
    let role = message_obj
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("user");

    if role.eq_ignore_ascii_case("tool") {
        let output = message_obj
            .get("content")
            .map(extract_message_content_text)
            .unwrap_or_default();
        let call_id = message_obj
            .get("tool_call_id")
            .and_then(Value::as_str)
            .unwrap_or("");
        return vec![json!({
            "type": "function_call_output",
            "call_id": call_id,
            "output": output,
        })];
    }

    let normalized_content = message_obj
        .get("content")
        .map(|content| normalize_chat_content_parts(content, role))
        .unwrap_or_default();
    let mut items = Vec::new();

    if !normalized_content.is_empty() {
        let mapped_role = if role.eq_ignore_ascii_case("system") {
            "developer"
        } else {
            role
        };
        let next = json!({
            "type": "message",
            "role": mapped_role,
            "content": normalized_content,
        });
        items.push(next);
    }

    if role.eq_ignore_ascii_case("assistant") {
        if let Some(tool_calls) = message_obj.get("tool_calls") {
            items.extend(normalize_chat_tool_calls(tool_calls, short_name_map));
        }
    }

    items
}

fn normalize_chat_messages_for_responses(
    messages: &Value,
    short_name_map: &HashMap<String, String>,
) -> Value {
    let Some(message_items) = messages.as_array() else {
        return messages.clone();
    };

    let mut normalized = Vec::new();
    for item in message_items {
        let Some(message_obj) = item.as_object() else {
            normalized.push(item.clone());
            continue;
        };
        normalized.extend(normalize_chat_message_for_responses(
            message_obj,
            short_name_map,
        ));
    }

    Value::Array(normalized)
}

fn normalize_chat_tool(tool: &Value, short_name_map: &HashMap<String, String>) -> Option<Value> {
    let tool_obj = tool.as_object()?;
    let tool_type = tool_obj
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("function");

    if tool_type != "function" {
        return Some(Value::Object(tool_obj.clone()));
    }

    let function_obj = tool_obj.get("function").and_then(Value::as_object);
    let name = function_obj
        .and_then(|function| function.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;

    let mut normalized = Map::new();
    normalized.insert("type".to_string(), Value::String("function".to_string()));
    normalized.insert(
        "name".to_string(),
        Value::String(map_tool_name(name, short_name_map)),
    );

    if let Some(description) = function_obj.and_then(|function| function.get("description")) {
        normalized.insert("description".to_string(), description.clone());
    }
    if let Some(parameters) = function_obj.and_then(|function| function.get("parameters")) {
        normalized.insert("parameters".to_string(), parameters.clone());
    }

    if let Some(strict) = function_obj
        .and_then(|function| function.get("strict"))
        .and_then(Value::as_bool)
    {
        normalized.insert("strict".to_string(), Value::Bool(strict));
    }

    Some(Value::Object(normalized))
}

fn normalize_chat_tools(tools: &Value, short_name_map: &HashMap<String, String>) -> Value {
    Value::Array(
        tools
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|tool| normalize_chat_tool(tool, short_name_map))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    )
}

fn normalize_chat_tool_choice(
    tool_choice: &Value,
    short_name_map: &HashMap<String, String>,
) -> Option<Value> {
    if let Some(mode) = tool_choice.as_str() {
        return Some(Value::String(mode.to_string()));
    }

    let Some(choice_obj) = tool_choice.as_object() else {
        return None;
    };
    let choice_type = choice_obj
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("function");
    if choice_type != "function" {
        return Some(Value::Object(choice_obj.clone()));
    }

    let name = choice_obj
        .get("function")
        .and_then(Value::as_object)
        .and_then(|function| function.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());

    name.map(|name| {
        json!({
            "type": "function",
            "name": map_tool_name(name, short_name_map),
        })
    })
}

fn extract_message_content_text(content: &Value) -> String {
    match content {
        Value::String(raw) => raw.to_string(),
        Value::Array(parts) => {
            let mut text = String::new();
            for part in parts {
                if let Some(part_text) = part.get("text").and_then(Value::as_str) {
                    append_non_empty_text(&mut text, part_text);
                    continue;
                }
                if let Some(part_text) = part.get("content").and_then(Value::as_str) {
                    append_non_empty_text(&mut text, part_text);
                }
            }
            text
        }
        _ => String::new(),
    }
}

fn build_responses_body_from_chat_completions(
    body: &Value,
) -> Result<(Value, bool, String), String> {
    let request_obj = body
        .as_object()
        .ok_or("chat/completions 请求体必须是 JSON 对象".to_string())?;
    let model = request_obj
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(resolve_supported_model_alias)
        .ok_or("chat/completions 请求缺少 model".to_string())?;
    if is_gpt_image_generation_model(&model) {
        return Err("This model is not supported on the Chat Completions endpoint".to_string());
    }
    let messages = request_obj
        .get("messages")
        .ok_or("chat/completions 请求缺少 messages".to_string())?;
    let short_name_map = build_short_tool_name_map(body);
    let input = normalize_chat_messages_for_responses(messages, &short_name_map);
    let stream = request_obj
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let mut responses_obj = Map::new();
    responses_obj.insert("instructions".to_string(), Value::String(String::new()));
    responses_obj.insert("stream".to_string(), Value::Bool(true));
    responses_obj.insert("store".to_string(), Value::Bool(false));
    responses_obj.insert("model".to_string(), Value::String(model.clone()));
    responses_obj.insert("input".to_string(), input);
    responses_obj.insert("parallel_tool_calls".to_string(), Value::Bool(true));
    responses_obj.insert(
        "reasoning".to_string(),
        json!({
            "effort": request_obj
                .get("reasoning_effort")
                .cloned()
                .unwrap_or_else(|| Value::String("medium".to_string())),
            "summary": "auto",
        }),
    );
    responses_obj.insert(
        "include".to_string(),
        Value::Array(vec![Value::String(
            "reasoning.encrypted_content".to_string(),
        )]),
    );

    if let Some(tools) = request_obj.get("tools") {
        responses_obj.insert(
            "tools".to_string(),
            normalize_chat_tools(tools, &short_name_map),
        );
    }

    if let Some(tool_choice) = request_obj.get("tool_choice") {
        if let Some(choice) = normalize_chat_tool_choice(tool_choice, &short_name_map) {
            responses_obj.insert("tool_choice".to_string(), choice);
        }
    }

    if let Some(service_tier) = request_obj.get("service_tier").and_then(Value::as_str) {
        if let Some(normalized) = normalize_proxy_service_tier(service_tier) {
            responses_obj.insert(
                "service_tier".to_string(),
                Value::String(normalized.to_string()),
            );
        }
    }

    let mut text_obj = Map::new();
    if let Some(response_format) = request_obj
        .get("response_format")
        .and_then(Value::as_object)
    {
        match response_format
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("")
        {
            "text" => {
                text_obj.insert("format".to_string(), json!({ "type": "text" }));
            }
            "json_schema" => {
                if let Some(json_schema) = response_format
                    .get("json_schema")
                    .and_then(Value::as_object)
                {
                    let mut format_obj = Map::new();
                    format_obj.insert("type".to_string(), Value::String("json_schema".to_string()));
                    if let Some(name) = json_schema.get("name") {
                        format_obj.insert("name".to_string(), name.clone());
                    }
                    if let Some(strict) = json_schema.get("strict") {
                        format_obj.insert("strict".to_string(), strict.clone());
                    }
                    if let Some(schema) = json_schema.get("schema") {
                        format_obj.insert("schema".to_string(), schema.clone());
                    }
                    text_obj.insert("format".to_string(), Value::Object(format_obj));
                }
            }
            _ => {}
        }
    }
    if let Some(text_value) = request_obj.get("text").and_then(Value::as_object) {
        if let Some(verbosity) = text_value.get("verbosity") {
            text_obj.insert("verbosity".to_string(), verbosity.clone());
        }
    }
    if !text_obj.is_empty() {
        responses_obj.insert("text".to_string(), Value::Object(text_obj));
    }

    Ok((Value::Object(responses_obj), stream, model))
}

fn normalize_proxy_service_tier(value: &str) -> Option<&'static str> {
    // priority/fast -> priority, flex -> flex, standard/default -> standard.
    match value.trim().to_ascii_lowercase().as_str() {
        "priority" | "fast" => Some("priority"),
        "flex" => Some("flex"),
        "standard" | "default" => Some("standard"),
        _ => None,
    }
}

/// Values safe to inject into upstream / sidecar payloads as an explicit default.
/// Omits standard/default so we do not force a service_tier field when the app
/// is running at normal speed (matches previous inject behavior).
fn normalize_upstream_inject_service_tier(value: &str) -> Option<&'static str> {
    match normalize_proxy_service_tier(value) {
        Some("priority") => Some("priority"),
        Some("flex") => Some("flex"),
        _ => None,
    }
}

fn apply_default_service_tier_if_missing(body_value: &mut Value, service_tier: Option<&str>) {
    let Some(service_tier) = service_tier.and_then(normalize_upstream_inject_service_tier) else {
        return;
    };
    let Some(body_obj) = body_value.as_object_mut() else {
        return;
    };
    if body_obj.contains_key("service_tier") {
        return;
    }
    body_obj.insert(
        "service_tier".to_string(),
        Value::String(service_tier.to_string()),
    );
}

fn request_body_has_service_tier(body_value: &Value) -> bool {
    body_value
        .as_object()
        .map(|body_obj| body_obj.contains_key("service_tier"))
        .unwrap_or(false)
}

fn service_tier_from_request_body(body: &[u8]) -> Option<String> {
    parse_request_body_json(body).and_then(|value| {
        value
            .get("service_tier")
            .and_then(Value::as_str)
            .and_then(normalize_proxy_service_tier)
            .map(str::to_string)
    })
}

fn normalize_proxy_reasoning_effort(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "none" | "minimal" | "min" => Some("minimal"),
        "low" => Some("low"),
        "medium" | "med" | "default" => Some("medium"),
        "high" => Some("high"),
        "xhigh" | "x-high" | "extra_high" | "extrahigh" => Some("xhigh"),
        _ => None,
    }
}

fn normalize_recorded_reasoning_effort(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "none" => Some("none"),
        "minimal" | "min" => Some("minimal"),
        "low" => Some("low"),
        "medium" | "med" | "default" => Some("medium"),
        "high" => Some("high"),
        "xhigh" | "x-high" | "extra_high" | "extrahigh" => Some("xhigh"),
        "max" => Some("max"),
        "auto" => Some("auto"),
        "ultra" => Some("ultra"),
        _ => None,
    }
}

fn reasoning_effort_from_request_body(body: &[u8]) -> Option<String> {
    let value = parse_request_body_json(body)?;
    if let Some(effort) = value
        .get("reasoning_effort")
        .and_then(Value::as_str)
        .and_then(normalize_proxy_reasoning_effort)
    {
        return Some(effort.to_string());
    }
    if let Some(effort) = value
        .pointer("/reasoning/effort")
        .and_then(Value::as_str)
        .and_then(normalize_proxy_reasoning_effort)
    {
        return Some(effort.to_string());
    }
    None
}

fn api_service_default_service_tier() -> Result<Option<&'static str>, String> {
    crate::modules::codex_speed::get_api_service_app_speed_config()
        .map(|config| codex_app_speed_service_tier(&config.speed))
}

fn sidecar_payload_default_service_tier(default_service_tier: Option<&str>) -> Option<Value> {
    let service_tier = default_service_tier.and_then(normalize_upstream_inject_service_tier)?;
    let models = SIDECAR_SERVICE_TIER_SUPPORTED_PAYLOAD_FORMATS
        .iter()
        .map(|payload_format| {
            json!({
                "name": SIDECAR_SERVICE_TIER_SUPPORTED_MODEL_PATTERN,
                "protocol": payload_format,
            })
        })
        .collect::<Vec<_>>();
    let mut params = Map::new();
    params.insert(
        "service_tier".to_string(),
        Value::String(service_tier.to_string()),
    );
    let mut rule = Map::new();
    rule.insert("models".to_string(), Value::Array(models));
    rule.insert("params".to_string(), Value::Object(params));
    let mut payload = Map::new();
    payload.insert(
        "default".to_string(),
        Value::Array(vec![Value::Object(rule)]),
    );
    Some(Value::Object(payload))
}

fn prepare_gateway_request_with_default_service_tier(
    mut request: ParsedRequest,
    default_service_tier: Option<&str>,
) -> Result<(ParsedRequest, GatewayResponseAdapter), String> {
    if is_images_generations_request(&request.target) {
        if !request.method.eq_ignore_ascii_case("POST") {
            return Err("images/generations 仅支持 POST".to_string());
        }
        let body_value = parse_request_body_json(&request.body)
            .ok_or("images/generations 请求体必须是合法 JSON".to_string())?;
        let (responses_body, stream, response_format) =
            build_images_generation_request(&body_value)?;
        request.target = RESPONSES_PATH.to_string();
        request.body = serde_json::to_vec(&responses_body)
            .map_err(|e| format!("序列化 images/generations 请求体失败: {}", e))?;
        request
            .headers
            .insert("accept".to_string(), "text/event-stream".to_string());
        request
            .headers
            .insert("content-type".to_string(), "application/json".to_string());
        return Ok((
            request,
            GatewayResponseAdapter::Images {
                stream,
                response_format,
                stream_prefix: "image_generation".to_string(),
            },
        ));
    }

    if is_images_edits_request(&request.target) {
        if !request.method.eq_ignore_ascii_case("POST") {
            return Err("images/edits 仅支持 POST".to_string());
        }
        let content_type = request
            .headers
            .get("content-type")
            .map(String::as_str)
            .unwrap_or("");
        let content_type_lower = content_type.to_ascii_lowercase();
        let (responses_body, stream, response_format) =
            if content_type_lower.starts_with("multipart/form-data") {
                build_images_edit_request_from_multipart(&content_type, &request.body)?
            } else {
                let body_value = parse_request_body_json(&request.body)
                    .ok_or("images/edits 请求体必须是合法 JSON".to_string())?;
                build_images_edit_request_from_json(&body_value)?
            };
        request.target = RESPONSES_PATH.to_string();
        request.body = serde_json::to_vec(&responses_body)
            .map_err(|e| format!("序列化 images/edits 请求体失败: {}", e))?;
        request
            .headers
            .insert("accept".to_string(), "text/event-stream".to_string());
        request
            .headers
            .insert("content-type".to_string(), "application/json".to_string());
        return Ok((
            request,
            GatewayResponseAdapter::Images {
                stream,
                response_format,
                stream_prefix: "image_edit".to_string(),
            },
        ));
    }

    if !is_chat_completions_request(&request.target) {
        if is_responses_request(&request.target) {
            if !request.method.eq_ignore_ascii_case("POST") {
                return Err("responses 仅支持 POST".to_string());
            }
            let mut body_value = parse_request_body_json(&request.body)
                .ok_or("responses 请求体必须是合法 JSON".to_string())?;
            let request_has_service_tier = request_body_has_service_tier(&body_value);
            rewrite_request_model_alias_value(&mut body_value);
            codex_protocol::normalize_responses_body_for_codex_with_lite(
                &mut body_value,
                request_uses_responses_lite(&request),
            );
            if !request_has_service_tier {
                apply_default_service_tier_if_missing(&mut body_value, default_service_tier);
            }
            request.body = serde_json::to_vec(&body_value)
                .map_err(|e| format!("序列化 responses 请求体失败: {}", e))?;
            request
                .headers
                .insert("accept".to_string(), "text/event-stream".to_string());
            request
                .headers
                .insert("content-type".to_string(), "application/json".to_string());
        } else if is_responses_compact_request(&request.target) {
            if !request.method.eq_ignore_ascii_case("POST") {
                return Err("responses/compact 仅支持 POST".to_string());
            }
            let mut body_value = parse_request_body_json(&request.body)
                .ok_or("responses/compact 请求体必须是合法 JSON".to_string())?;
            let request_has_service_tier = request_body_has_service_tier(&body_value);
            rewrite_request_model_alias_value(&mut body_value);
            codex_protocol::normalize_responses_body_for_codex_with_lite(
                &mut body_value,
                request_uses_responses_lite(&request),
            );
            if !request_has_service_tier {
                apply_default_service_tier_if_missing(&mut body_value, default_service_tier);
            }
            if let Some(body_obj) = body_value.as_object_mut() {
                body_obj.remove("stream");
            }
            request.body = serde_json::to_vec(&body_value)
                .map_err(|e| format!("序列化 responses/compact 请求体失败: {}", e))?;
            request
                .headers
                .insert("accept".to_string(), "application/json".to_string());
            request
                .headers
                .insert("content-type".to_string(), "application/json".to_string());
        } else if let Some(rewritten_body) = rewrite_request_model_alias(&request.body)? {
            request.body = rewritten_body;
        }
        let request_is_stream = is_stream_request(&request.headers, &request.body);
        return Ok((
            request,
            GatewayResponseAdapter::Passthrough { request_is_stream },
        ));
    }

    if !request.method.eq_ignore_ascii_case("POST") {
        return Err("chat/completions 仅支持 POST".to_string());
    }

    let body_value = parse_request_body_json(&request.body)
        .ok_or("chat/completions 请求体必须是合法 JSON".to_string())?;
    let request_has_service_tier = request_body_has_service_tier(&body_value);
    let original_request_body = request.body.clone();
    let (mut responses_body, stream, requested_model) =
        build_responses_body_from_chat_completions(&body_value)?;
    codex_protocol::normalize_responses_body_for_codex_with_lite(
        &mut responses_body,
        request_uses_responses_lite(&request),
    );
    if !request_has_service_tier {
        apply_default_service_tier_if_missing(&mut responses_body, default_service_tier);
    }
    request.target = RESPONSES_PATH.to_string();
    request.body = serde_json::to_vec(&responses_body)
        .map_err(|e| format!("序列化 responses 请求体失败: {}", e))?;
    request
        .headers
        .insert("accept".to_string(), "text/event-stream".to_string());
    request
        .headers
        .insert("content-type".to_string(), "application/json".to_string());

    Ok((
        request,
        GatewayResponseAdapter::ChatCompletions {
            stream,
            requested_model,
            original_request_body,
        },
    ))
}

#[cfg(test)]
fn prepare_gateway_request(
    request: ParsedRequest,
) -> Result<(ParsedRequest, GatewayResponseAdapter), String> {
    prepare_gateway_request_with_default_service_tier(request, None)
}

fn response_payload_root(value: &Value) -> &Value {
    value
        .get("response")
        .filter(|item| item.is_object())
        .unwrap_or(value)
}

fn append_non_empty_text(buffer: &mut String, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    buffer.push_str(text);
}

fn extract_output_text_from_response(response_body: &Value) -> String {
    let root = response_payload_root(response_body);
    let mut text = String::new();
    if let Some(output_items) = root.get("output").and_then(Value::as_array) {
        for item in output_items {
            if item.get("type").and_then(Value::as_str) != Some("message") {
                continue;
            }
            if let Some(content) = item.get("content").and_then(Value::as_array) {
                for part in content {
                    if part.get("type").and_then(Value::as_str) != Some("output_text") {
                        continue;
                    }
                    if let Some(part_text) = part.get("text").and_then(Value::as_str) {
                        append_non_empty_text(&mut text, part_text);
                    }
                }
            }
        }
    }
    text
}

fn extract_reasoning_text_from_response(response_body: &Value) -> String {
    let root = response_payload_root(response_body);
    let mut reasoning_text = String::new();
    if let Some(output_items) = root.get("output").and_then(Value::as_array) {
        for item in output_items {
            if item.get("type").and_then(Value::as_str) != Some("reasoning") {
                continue;
            }
            if let Some(summary_items) = item.get("summary").and_then(Value::as_array) {
                for summary_item in summary_items {
                    if summary_item.get("type").and_then(Value::as_str) != Some("summary_text") {
                        continue;
                    }
                    if let Some(text) = summary_item.get("text").and_then(Value::as_str) {
                        append_non_empty_text(&mut reasoning_text, text);
                    }
                }
            }
        }
    }
    reasoning_text
}

fn extract_response_tool_calls(
    response_body: &Value,
    reverse_tool_name_map: &HashMap<String, String>,
) -> Vec<Value> {
    let root = response_payload_root(response_body);
    root.get("output")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let item_obj = item.as_object()?;
                    if item_obj.get("type").and_then(Value::as_str) != Some("function_call") {
                        return None;
                    }
                    let name = item_obj
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())?;
                    let restored_name = reverse_tool_name_map
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| name.to_string());
                    let arguments = item_obj
                        .get("arguments")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let call_id = item_obj
                        .get("call_id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    Some(json!({
                        "id": call_id,
                        "type": "function",
                        "function": {
                            "name": restored_name,
                            "arguments": arguments,
                        },
                    }))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn build_chat_completion_message(
    response_body: &Value,
    reverse_tool_name_map: &HashMap<String, String>,
) -> Value {
    let content = extract_output_text_from_response(response_body);
    let reasoning_content = extract_reasoning_text_from_response(response_body);
    let tool_calls = extract_response_tool_calls(response_body, reverse_tool_name_map);
    let mut message = Map::new();
    message.insert("role".to_string(), Value::String("assistant".to_string()));
    message.insert("content".to_string(), Value::Null);
    message.insert("reasoning_content".to_string(), Value::Null);
    message.insert("tool_calls".to_string(), Value::Null);

    if !content.is_empty() {
        message.insert("content".to_string(), Value::String(content));
    }
    if !reasoning_content.is_empty() {
        message.insert(
            "reasoning_content".to_string(),
            Value::String(reasoning_content),
        );
    }
    if !tool_calls.is_empty() {
        message.insert("tool_calls".to_string(), Value::Array(tool_calls));
    }

    Value::Object(message)
}

fn resolve_chat_finish_reason(response_body: &Value, has_tool_calls: bool) -> String {
    let root = response_payload_root(response_body);
    if root.get("status").and_then(Value::as_str) == Some("completed") {
        if has_tool_calls {
            "tool_calls".to_string()
        } else {
            "stop".to_string()
        }
    } else {
        "stop".to_string()
    }
}

fn build_chat_completion_payload(
    response_body: &Value,
    requested_model: &str,
    original_request_body: &[u8],
) -> Value {
    let root = response_payload_root(response_body);
    let reverse_tool_name_map = build_reverse_tool_name_map_from_request(original_request_body);
    let id = root
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("chatcmpl-local-{}", now_ms()));
    let created = root
        .get("created_at")
        .or_else(|| root.get("created"))
        .and_then(Value::as_i64)
        .unwrap_or_else(|| chrono::Utc::now().timestamp());
    let model = root
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| requested_model.to_string());
    let message = build_chat_completion_message(response_body, &reverse_tool_name_map);
    let has_tool_calls = message
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|tool_calls| !tool_calls.is_empty())
        .unwrap_or(false);
    let finish_reason = resolve_chat_finish_reason(response_body, has_tool_calls);
    let usage = extract_usage_capture(response_body).unwrap_or_default();

    json!({
        "id": id,
        "object": "chat.completion",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish_reason,
            "native_finish_reason": finish_reason,
        }],
        "usage": {
            "prompt_tokens": usage.input_tokens,
            "completion_tokens": usage.output_tokens,
            "total_tokens": usage.total_tokens,
            "prompt_tokens_details": {
                "cached_tokens": usage.cached_tokens,
            },
            "completion_tokens_details": {
                "reasoning_tokens": usage.reasoning_tokens,
            },
        },
    })
}

#[derive(Debug, Default)]
struct ChatCompletionStreamState {
    response_id: String,
    created_at: i64,
    model: String,
    function_call_index: i64,
    has_received_arguments_delta: bool,
    has_tool_call_announced: bool,
}

fn push_sse_payload(stream_body: &mut String, payload: Value) {
    stream_body.push_str("data: ");
    stream_body.push_str(
        serde_json::to_string(&payload)
            .unwrap_or_else(|_| "{\"error\":\"failed to encode stream payload\"}".to_string())
            .as_str(),
    );
    stream_body.push_str("\n\n");
}

#[derive(Debug)]
struct ChatCompletionStreamTransformer {
    reverse_tool_name_map: HashMap<String, String>,
    requested_model: String,
    stream_buffer: Vec<u8>,
    state: ChatCompletionStreamState,
    response_capture: ResponseCapture,
}

impl ChatCompletionStreamTransformer {
    fn new(original_request_body: &[u8], requested_model: &str) -> Self {
        Self {
            reverse_tool_name_map: build_reverse_tool_name_map_from_request(original_request_body),
            requested_model: requested_model.to_string(),
            stream_buffer: Vec::new(),
            state: ChatCompletionStreamState {
                model: requested_model.to_string(),
                function_call_index: -1,
                ..Default::default()
            },
            response_capture: ResponseCapture::default(),
        }
    }

    fn feed(&mut self, chunk: &[u8]) -> Vec<u8> {
        if chunk.is_empty() {
            return Vec::new();
        }
        self.stream_buffer.extend_from_slice(chunk);
        self.process_buffer(false)
    }

    fn finish(mut self) -> (Vec<u8>, ResponseCapture) {
        let mut output = self.process_buffer(true);
        output.extend_from_slice(b"data: [DONE]\n\n");
        (output, self.response_capture)
    }

    fn process_buffer(&mut self, flush_tail: bool) -> Vec<u8> {
        let mut stream_body = String::new();

        loop {
            let Some((boundary_index, separator_len)) =
                find_sse_frame_boundary(&self.stream_buffer)
            else {
                break;
            };
            let frame = self.stream_buffer[..boundary_index].to_vec();
            self.stream_buffer.drain(..boundary_index + separator_len);
            self.process_frame(&frame, &mut stream_body);
        }

        if flush_tail && !self.stream_buffer.is_empty() {
            let frame = std::mem::take(&mut self.stream_buffer);
            self.process_frame(&frame, &mut stream_body);
        }

        stream_body.into_bytes()
    }

    fn process_frame(&mut self, frame: &[u8], stream_body: &mut String) {
        if frame.is_empty() {
            return;
        }

        let text = String::from_utf8_lossy(frame);
        let mut event_name: Option<String> = None;
        let mut data_lines = Vec::new();
        for raw_line in text.lines() {
            let line = raw_line.trim();
            if let Some(rest) = line.strip_prefix("event:") {
                let value = rest.trim();
                if !value.is_empty() {
                    event_name = Some(value.to_string());
                }
                continue;
            }
            if let Some(rest) = line.strip_prefix("data:") {
                let payload = rest.trim();
                if !payload.is_empty() {
                    data_lines.push(payload.to_string());
                }
            }
        }

        let payload = if data_lines.is_empty() {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return;
            }
            trimmed.to_string()
        } else {
            data_lines.join("\n")
        };

        if payload == "[DONE]" {
            return;
        }

        let Ok(event) = serde_json::from_str::<Value>(&payload) else {
            return;
        };

        if self.response_capture.terminal_error.is_none() {
            if let Some(signal) = upstream_response_failed_signal(event_name.as_deref(), &event) {
                self.response_capture.terminal_error =
                    Some(format_upstream_response_failed_error(&signal));
            }
        }
        if let Some(usage) = extract_usage_capture(&event) {
            self.response_capture.usage = Some(usage);
        }
        if self.response_capture.response_id.is_none() {
            self.response_capture.response_id = extract_response_id(&event);
        }

        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .or(event_name.as_deref())
            .unwrap_or("");

        if event_type == "response.created" {
            if let Some(response) = event.get("response").and_then(Value::as_object) {
                self.state.response_id = response
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                self.state.created_at = response
                    .get("created_at")
                    .and_then(Value::as_i64)
                    .unwrap_or_else(|| chrono::Utc::now().timestamp());
                self.state.model = response
                    .get("model")
                    .and_then(Value::as_str)
                    .unwrap_or(self.requested_model.as_str())
                    .to_string();
            }
            if self.response_capture.response_id.is_none() && !self.state.response_id.is_empty() {
                self.response_capture.response_id = Some(self.state.response_id.clone());
            }
            return;
        }

        let mut template = build_chat_chunk_template(&self.state, &self.requested_model, &event);

        match event_type {
            "response.reasoning_summary_text.delta" => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    template["choices"][0]["delta"]["role"] =
                        Value::String("assistant".to_string());
                    template["choices"][0]["delta"]["reasoning_content"] =
                        Value::String(delta.to_string());
                    push_sse_payload(stream_body, template);
                }
            }
            "response.reasoning_summary_text.done" => {
                template["choices"][0]["delta"]["role"] = Value::String("assistant".to_string());
                template["choices"][0]["delta"]["reasoning_content"] =
                    Value::String("\n\n".to_string());
                push_sse_payload(stream_body, template);
            }
            "response.output_text.delta" => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    template["choices"][0]["delta"]["role"] =
                        Value::String("assistant".to_string());
                    template["choices"][0]["delta"]["content"] = Value::String(delta.to_string());
                    push_sse_payload(stream_body, template);
                }
            }
            "response.output_item.added" => {
                let Some(item) = event.get("item").and_then(Value::as_object) else {
                    return;
                };
                if item.get("type").and_then(Value::as_str) != Some("function_call") {
                    return;
                }

                self.state.function_call_index += 1;
                self.state.has_received_arguments_delta = false;
                self.state.has_tool_call_announced = true;

                let name = item.get("name").and_then(Value::as_str).unwrap_or("");
                let restored_name = self
                    .reverse_tool_name_map
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| name.to_string());
                template["choices"][0]["delta"]["role"] = Value::String("assistant".to_string());
                template["choices"][0]["delta"]["tool_calls"] = json!([{
                    "index": self.state.function_call_index,
                    "id": item.get("call_id").cloned().unwrap_or(Value::String(String::new())),
                    "type": "function",
                    "function": {
                        "name": restored_name,
                        "arguments": "",
                    }
                }]);
                push_sse_payload(stream_body, template);
            }
            "response.function_call_arguments.delta" => {
                self.state.has_received_arguments_delta = true;
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    template["choices"][0]["delta"]["tool_calls"] = json!([{
                        "index": self.state.function_call_index,
                        "function": {
                            "arguments": delta,
                        }
                    }]);
                    push_sse_payload(stream_body, template);
                }
            }
            "response.function_call_arguments.done" => {
                if self.state.has_received_arguments_delta {
                    return;
                }
                if let Some(arguments) = event.get("arguments").and_then(Value::as_str) {
                    template["choices"][0]["delta"]["tool_calls"] = json!([{
                        "index": self.state.function_call_index,
                        "function": {
                            "arguments": arguments,
                        }
                    }]);
                    push_sse_payload(stream_body, template);
                }
            }
            "response.output_item.done" => {
                let Some(item) = event.get("item").and_then(Value::as_object) else {
                    return;
                };
                if item.get("type").and_then(Value::as_str) != Some("function_call") {
                    return;
                }

                if self.state.has_tool_call_announced {
                    self.state.has_tool_call_announced = false;
                    return;
                }

                self.state.function_call_index += 1;
                let name = item.get("name").and_then(Value::as_str).unwrap_or("");
                let restored_name = self
                    .reverse_tool_name_map
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| name.to_string());
                template["choices"][0]["delta"]["role"] = Value::String("assistant".to_string());
                template["choices"][0]["delta"]["tool_calls"] = json!([{
                    "index": self.state.function_call_index,
                    "id": item.get("call_id").cloned().unwrap_or(Value::String(String::new())),
                    "type": "function",
                    "function": {
                        "name": restored_name,
                        "arguments": item
                            .get("arguments")
                            .cloned()
                            .unwrap_or(Value::String(String::new())),
                    }
                }]);
                push_sse_payload(stream_body, template);
            }
            event_type if is_responses_completion_event(event_type) => {
                let finish_reason = if self.state.function_call_index >= 0 {
                    "tool_calls"
                } else {
                    "stop"
                };
                template["choices"][0]["finish_reason"] = Value::String(finish_reason.to_string());
                template["choices"][0]["native_finish_reason"] =
                    Value::String(finish_reason.to_string());
                push_sse_payload(stream_body, template);
            }
            _ => {}
        }
    }
}

fn build_chat_chunk_template(
    state: &ChatCompletionStreamState,
    requested_model: &str,
    event: &Value,
) -> Value {
    let model = event
        .get("model")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            if state.model.trim().is_empty() {
                None
            } else {
                Some(state.model.clone())
            }
        })
        .unwrap_or_else(|| requested_model.to_string());
    let id = if state.response_id.trim().is_empty() {
        format!("chatcmpl-local-{}", now_ms())
    } else {
        state.response_id.clone()
    };
    let created = if state.created_at > 0 {
        state.created_at
    } else {
        chrono::Utc::now().timestamp()
    };

    let usage = event
        .get("response")
        .and_then(|response| response.get("usage"))
        .cloned()
        .or_else(|| event.get("usage").cloned());

    let mut template = json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": Value::Null,
            "native_finish_reason": Value::Null,
        }],
    });
    if let Some(usage) = usage {
        let parsed_usage = extract_usage_capture(&json!({ "response": { "usage": usage } }))
            .or_else(|| extract_usage_capture(&json!({ "usage": usage })))
            .unwrap_or_default();
        template["usage"] = json!({
            "prompt_tokens": parsed_usage.input_tokens,
            "completion_tokens": parsed_usage.output_tokens,
            "total_tokens": parsed_usage.total_tokens,
            "prompt_tokens_details": {
                "cached_tokens": parsed_usage.cached_tokens,
            },
            "completion_tokens_details": {
                "reasoning_tokens": parsed_usage.reasoning_tokens,
            },
        });
    }
    template
}

fn build_chat_completion_stream_body(
    upstream_body: &[u8],
    original_request_body: &[u8],
    requested_model: &str,
) -> String {
    let mut transformer =
        ChatCompletionStreamTransformer::new(original_request_body, requested_model);
    let mut stream_body = transformer.feed(upstream_body);
    let (tail, _) = transformer.finish();
    stream_body.extend_from_slice(&tail);
    String::from_utf8(stream_body).unwrap_or_default()
}

