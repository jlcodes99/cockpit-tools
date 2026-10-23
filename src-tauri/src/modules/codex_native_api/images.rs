//! Images protocol adapters. Account selection and network transport remain host-owned.
//! Wire references: OpenAI Images API and Responses image_generation tool documentation.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Map, Value};

use super::contract::ServiceError;

#[path = "images_result.rs"]
mod result;
pub(super) use result::{
    merge_responses, native_from_events, native_to_events, ImageAccumulator, ImageResponseFormat,
};

#[path = "images_http.rs"]
mod transport;
pub(super) use transport::{edits, generations};
#[path = "images_assets.rs"]
mod assets;
#[path = "images_bridge.rs"]
mod bridge;
pub(super) use bridge::{is_forced_generation, responses};

const MAX_INPUT_IMAGES: usize = 16;
const MAX_MULTIPART_PARTS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ImageOperation {
    Generation,
    Edit,
}

impl ImageOperation {
    pub(super) fn path(self) -> &'static str {
        match self {
            Self::Generation => "images/generations",
            Self::Edit => "images/edits",
        }
    }
    pub(super) fn event_prefix(self) -> &'static str {
        match self {
            Self::Generation => "image_generation",
            Self::Edit => "image_edit",
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct ImagePart {
    pub name: String,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(super) struct ImageRequest {
    pub operation: ImageOperation,
    pub model: Option<String>,
    pub stream: bool,
    pub n: u32,
    pub response_format: ImageResponseFormat,
    pub output_format: String,
    prompt: String,
    original: Value,
    parts: Option<Vec<ImagePart>>,
    input_images: Vec<Value>,
    mask: Option<Value>,
}

fn bad(message: impl Into<String>) -> ServiceError {
    ServiceError::new(400, "invalid_images_request", message)
}

impl ImageRequest {
    pub(super) fn from_json(operation: ImageOperation, body: Value) -> Result<Self, ServiceError> {
        let object = body
            .as_object()
            .ok_or_else(|| bad("Images request must be an object"))?;
        let prompt = object
            .get("prompt")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| bad("A non-empty image prompt is required"))?
            .to_owned();
        let model = optional_string(object, "model")?;
        if model.as_ref().is_some_and(|value| {
            value.is_empty() || value.len() > 200 || value.chars().any(char::is_whitespace)
        }) {
            return Err(bad(
                "Image model must be a non-empty ID without whitespace, up to 200 bytes",
            ));
        }
        let stream = match object.get("stream") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(value)) => *value,
            _ => return Err(bad("stream must be a boolean")),
        };
        let n = integer(object, "n", 1, 1, 10)? as u32;
        let response_format = match optional_string(object, "response_format")?.as_deref() {
            None | Some("b64_json") => ImageResponseFormat::Base64,
            Some("url") => ImageResponseFormat::Url,
            _ => return Err(bad("response_format must be b64_json or url")),
        };
        let output_format =
            optional_string(object, "output_format")?.unwrap_or_else(|| "png".to_owned());
        if !matches!(output_format.as_str(), "png" | "jpeg" | "webp") {
            return Err(bad("output_format must be png, jpeg or webp"));
        }
        for name in [
            "size",
            "quality",
            "background",
            "moderation",
            "input_fidelity",
        ] {
            optional_string(object, name)?;
        }
        integer(object, "output_compression", 100, 0, 100)?;
        integer(object, "partial_images", 0, 0, 3)?;
        let mut input_images = Vec::new();
        for key in ["image", "images"] {
            if let Some(value) = object.get(key).filter(|value| !value.is_null()) {
                match value {
                    Value::Array(images) => {
                        for image in images {
                            input_images.push(image_reference(image)?);
                        }
                    }
                    image => input_images.push(image_reference(image)?),
                }
            }
        }
        if input_images.len() > MAX_INPUT_IMAGES {
            return Err(bad("At most 16 input images are supported"));
        }
        if operation == ImageOperation::Edit && input_images.is_empty() {
            return Err(bad("Image edits require at least one input image"));
        }
        if operation == ImageOperation::Generation && !input_images.is_empty() {
            return Err(bad("Use images/edits when providing input images"));
        }
        let mask = object
            .get("mask")
            .filter(|value| !value.is_null())
            .map(image_reference)
            .transpose()?;
        if mask.is_some() && operation != ImageOperation::Edit {
            return Err(bad("An image mask is only valid for image edits"));
        }
        Ok(Self {
            operation,
            model,
            stream,
            n,
            response_format,
            output_format,
            prompt,
            original: body,
            parts: None,
            input_images,
            mask,
        })
    }

    /// Multipart bytes are retained unchanged for native Images upstreams.
    /// OAuth converts file parts into inline image inputs without disk or URL fetches.
    pub(super) fn from_multipart(
        operation: ImageOperation,
        parts: Vec<ImagePart>,
    ) -> Result<Self, ServiceError> {
        if parts.len() > MAX_MULTIPART_PARTS {
            return Err(bad("Too many multipart image fields"));
        }
        let mut body = Map::new();
        let mut images = Vec::new();
        for part in &parts {
            match part.name.as_str() {
                "image" | "image[]" | "images" | "images[]" => images.push(part_reference(part)?),
                "mask" => {
                    if body.contains_key("mask") {
                        return Err(bad("Only one image mask is allowed"));
                    }
                    body.insert("mask".into(), part_reference(part)?);
                }
                key => {
                    if part.filename.is_some() {
                        return Err(bad("Unexpected file part in image request"));
                    }
                    if body.contains_key(key) {
                        return Err(bad(format!("Duplicate multipart field: {key}")));
                    }
                    let text = std::str::from_utf8(&part.bytes)
                        .map_err(|_| bad("Image form fields must be UTF-8"))?;
                    let value = match key {
                        "n" | "partial_images" | "output_compression" => Value::Number(
                            text.parse::<u64>()
                                .map_err(|_| bad(format!("{key} must be an integer")))?
                                .into(),
                        ),
                        "stream" | "async" => Value::Bool(
                            text.parse::<bool>()
                                .map_err(|_| bad(format!("{key} must be true or false")))?,
                        ),
                        _ => Value::String(text.to_owned()),
                    };
                    body.insert(key.to_owned(), value);
                }
            }
        }
        if !images.is_empty() {
            body.insert("images".into(), Value::Array(images));
        }
        let mut request = Self::from_json(operation, Value::Object(body))?;
        request.parts = Some(parts);
        Ok(request)
    }

    pub(super) fn is_multipart(&self) -> bool {
        self.parts.is_some()
    }

    /// The host supplies the exact selected Images model, including vendor suffixes.
    pub(super) fn native_json(&self, actual_model: &str) -> Value {
        let mut body = self.original.clone();
        body["model"] = json!(actual_model);
        body
    }

    pub(super) fn native_parts(&self, actual_model: &str) -> Option<Vec<ImagePart>> {
        let mut parts = self.parts.clone()?;
        if let Some(model) = parts.iter_mut().find(|part| part.name == "model") {
            model.bytes = actual_model.as_bytes().to_vec();
        } else {
            parts.push(ImagePart {
                name: "model".into(),
                filename: None,
                content_type: None,
                bytes: actual_model.as_bytes().to_vec(),
            });
        }
        Some(parts)
    }

    /// A single image operation. The caller executes `n` bounded operations and
    /// combines results; `n` is not an invented Responses tool parameter or prompt.
    pub(super) fn to_responses(
        &self,
        main_model: &str,
        image_model: &str,
    ) -> Result<Value, ServiceError> {
        let mut tool = json!({"type":"image_generation", "model":image_model,
            "action": if self.operation == ImageOperation::Edit { "edit" } else { "generate" }});
        for key in [
            "size",
            "quality",
            "background",
            "output_format",
            "output_compression",
            "moderation",
            "input_fidelity",
            "partial_images",
        ] {
            if let Some(value) = self.original.get(key).filter(|value| !value.is_null()) {
                tool[key] = value.clone();
            }
        }
        if let Some(mask) = &self.mask {
            tool["input_image_mask"] = mask.clone();
        }
        let mut content = vec![json!({"type":"input_text","text":self.prompt})];
        for image in &self.input_images {
            let mut image = image.clone();
            image["type"] = json!("input_image");
            content.push(image);
        }
        let mut request = json!({"model":main_model, "stream":true,"store":false,
            "input":[{"role":"user","content":content}], "tools":[tool],
            "tool_choice":{"type":"image_generation"}, "parallel_tool_calls":false});
        for name in ["client_metadata", "prompt_cache_key", "service_tier"] {
            if let Some(value) = self.original.get(name) {
                request[name] = value.clone();
            }
        }
        // Never silently reinterpret vendor-specific image controls as model instructions.
        for key in ["style", "aspect_ratio", "resolution"] {
            if self.original.get(key).is_some() {
                return Err(bad(format!("{key} requires a native Images API source")));
            }
        }
        Ok(request)
    }
}

fn optional_string(
    object: &Map<String, Value>,
    name: &str,
) -> Result<Option<String>, ServiceError> {
    match object.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        _ => Err(bad(format!("{name} must be a string"))),
    }
}

fn integer(
    object: &Map<String, Value>,
    name: &str,
    default: u64,
    min: u64,
    max: u64,
) -> Result<u64, ServiceError> {
    let value = match object.get(name) {
        None | Some(Value::Null) => default,
        Some(value) => value
            .as_u64()
            .ok_or_else(|| bad(format!("{name} must be an integer")))?,
    };
    if !(min..=max).contains(&value) {
        return Err(bad(format!("{name} must be between {min} and {max}")));
    }
    Ok(value)
}

fn image_reference(value: &Value) -> Result<Value, ServiceError> {
    if let Some(url) = value.as_str() {
        return validated_url_reference(url);
    }
    let object = value
        .as_object()
        .ok_or_else(|| bad("Image references must be URLs or file reference objects"))?;
    if let Some(file) = optional_string(object, "file_id")? {
        if file.is_empty() {
            return Err(bad("Image file_id must not be empty"));
        }
        if object.contains_key("image_url") || object.contains_key("url") {
            return Err(bad("An image reference must specify either file_id or URL"));
        }
        return Ok(json!({"file_id":file}));
    }
    let url = object.get("image_url").or_else(|| object.get("url"));
    let url = url
        .and_then(|value| {
            value
                .as_str()
                .or_else(|| value.get("url").and_then(Value::as_str))
        })
        .ok_or_else(|| bad("Image references require image_url or file_id"))?;
    validated_url_reference(url)
}

fn validated_url_reference(value: &str) -> Result<Value, ServiceError> {
    if value.starts_with("data:image/") {
        let encoded = value
            .split_once(";base64,")
            .map(|(_, data)| data)
            .filter(|encoded| !encoded.is_empty())
            .ok_or_else(|| bad("Inline images must use base64 data URLs"))?;
        STANDARD
            .decode(encoded)
            .map_err(|_| bad("Inline image contains invalid base64"))?;
    } else {
        assets::checked_url(value)?;
    }
    Ok(json!({"image_url":value}))
}

fn part_reference(part: &ImagePart) -> Result<Value, ServiceError> {
    let file = part.filename.is_some()
        || part
            .content_type
            .as_deref()
            .is_some_and(|value| value.starts_with("image/"));
    if !file {
        return validated_url_reference(
            std::str::from_utf8(&part.bytes).map_err(|_| bad("Invalid image form reference"))?,
        );
    }
    if part.bytes.is_empty() {
        return Err(bad("Image files must not be empty"));
    }
    let declared = part
        .content_type
        .as_deref()
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    let mime = match declared {
        Some("image/png" | "image/jpeg" | "image/webp" | "image/gif") => declared.unwrap(),
        Some("application/octet-stream") | None => {
            if part.bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                "image/png"
            } else if part.bytes.starts_with(b"\xff\xd8\xff") {
                "image/jpeg"
            } else if part.bytes.starts_with(b"RIFF") && part.bytes.get(8..12) == Some(b"WEBP") {
                "image/webp"
            } else if part.bytes.starts_with(b"GIF8") {
                "image/gif"
            } else {
                return Err(bad("Image files need a supported image content type"));
            }
        }
        _ => return Err(bad("Unsupported image content type")),
    };
    Ok(json!({"image_url":format!("data:{mime};base64,{}", STANDARD.encode(&part.bytes))}))
}

#[cfg(test)]
#[path = "images_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "images_http_tests.rs"]
mod http_tests;
