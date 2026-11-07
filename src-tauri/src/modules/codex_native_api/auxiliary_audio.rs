//! Audio uploads are validated and kept out of diagnostic body snapshots.
use super::super::contract::ServiceError;
use axum::body::Bytes;
use futures_util::stream;
use serde_json::{json, Value};

#[derive(Clone)]
pub(super) struct Part {
    pub name: String,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub bytes: Bytes,
}

fn invalid(message: &str) -> ServiceError {
    ServiceError::new(400, "audio_invalid_request", message)
}

pub(super) async fn multipart(content_type: &str, bytes: Bytes) -> Result<Vec<Part>, ServiceError> {
    let boundary = multer::parse_boundary(content_type)
        .map_err(|_| invalid("A multipart boundary is required"))?;
    let input = stream::once(async move { Ok::<_, std::io::Error>(bytes) });
    let mut parser = multer::Multipart::new(input, boundary);
    let mut parts = Vec::new();
    while let Some(field) = parser
        .next_field()
        .await
        .map_err(|_| invalid("Invalid multipart body"))?
    {
        if parts.len() >= 64 {
            return Err(invalid("Too many multipart fields"));
        }
        let name = field.name().unwrap_or("").to_string();
        if name.is_empty() {
            return Err(invalid("Multipart fields must have names"));
        }
        let filename = field.file_name().map(str::to_owned);
        let content_type = field.content_type().map(ToString::to_string);
        let bytes = field
            .bytes()
            .await
            .map_err(|_| invalid("Invalid multipart field"))?;
        if filename.is_none() && name != "file" && bytes.len() > 64 * 1024 {
            return Err(invalid("Multipart field is too large"));
        }
        parts.push(Part {
            name,
            filename,
            content_type,
            bytes,
        });
    }
    Ok(parts)
}

pub(super) struct AudioRequest {
    pub operation: String,
    pub model: String,
    pub format: String,
    pub native: bool,
    parts: Vec<Part>,
    json: Option<Value>,
}

impl AudioRequest {
    pub async fn parse(path: &str, content_type: &str, bytes: Bytes) -> Result<Self, ServiceError> {
        let native = path.ends_with("/transcribe");
        let operation = if native {
            "transcriptions"
        } else {
            path.rsplit('/').next().unwrap_or("")
        }
        .to_string();
        if !matches!(
            operation.as_str(),
            "transcriptions" | "translations" | "speech"
        ) {
            return Err(invalid("Unknown audio operation"));
        }
        if operation == "speech" {
            let value: Value =
                serde_json::from_slice(&bytes).map_err(|_| invalid("Speech requires JSON"))?;
            let model = value
                .get("model")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| invalid("A speech model is required"))?
                .to_string();
            return Ok(Self {
                operation,
                model,
                format: "binary".into(),
                native,
                parts: vec![],
                json: Some(value),
            });
        }
        let parts = multipart(content_type, bytes).await?;
        if parts
            .iter()
            .filter(|p| p.name == "file" && !p.bytes.is_empty())
            .count()
            != 1
        {
            return Err(invalid("Exactly one nonempty audio file is required"));
        }
        for key in ["file", "model", "response_format", "stream", "language"] {
            if parts.iter().filter(|p| p.name == key).count() > 1 {
                return Err(invalid("Duplicate audio field"));
            }
        }
        let field = |name: &str| {
            parts
                .iter()
                .find(|p| p.name == name)
                .map(|p| String::from_utf8_lossy(&p.bytes).trim().to_string())
        };
        let model = field("model").unwrap_or_default();
        let format = field("response_format").unwrap_or_else(|| "json".into());
        Ok(Self {
            operation,
            model,
            format,
            native,
            parts,
            json: None,
        })
    }

    pub fn apply(
        &self,
        builder: reqwest::RequestBuilder,
        oauth: bool,
        model: &str,
    ) -> Result<reqwest::RequestBuilder, ServiceError> {
        if oauth && self.operation != "transcriptions" {
            return Err(ServiceError::new(
                400,
                "audio_not_supported",
                "This audio operation requires an API key account",
            ));
        }
        if let Some(mut body) = self.json.clone() {
            body["model"] = json!(model);
            return Ok(builder.json(&body));
        }
        if oauth && !self.native && !matches!(self.format.as_str(), "json" | "text") {
            return Err(invalid("This response format requires an API key account"));
        }
        let mut form = reqwest::multipart::Form::new();
        for part in &self.parts {
            if part.name == "model" {
                continue;
            }
            if oauth && !self.native {
                match part.name.as_str() {
                    "response_format" => continue,
                    "stream"
                        if std::str::from_utf8(&part.bytes)
                            .is_ok_and(|value| value.trim() == "false") =>
                    {
                        continue
                    }
                    "file" | "language" => {}
                    _ => {
                        return Err(invalid(
                            "Audio field is unsupported by the OAuth transcription service",
                        ))
                    }
                }
            }
            let mut value = reqwest::multipart::Part::bytes(part.bytes.to_vec());
            if let Some(filename) = &part.filename {
                value = value.file_name(filename.clone());
            }
            if let Some(content_type) = &part.content_type {
                value = value
                    .mime_str(content_type)
                    .map_err(|_| invalid("Invalid audio content type"))?;
            }
            form = form.part(part.name.clone(), value);
        }
        if !oauth {
            let model = if model.is_empty() && self.native {
                "gpt-4o-mini-transcribe"
            } else {
                model
            };
            if model.is_empty() {
                return Err(invalid("An audio model is required"));
            }
            form = form.text("model", model.to_string());
        }
        Ok(builder.multipart(form))
    }
}

pub(super) async fn call_body(
    content_type: &str,
    bytes: Bytes,
    session: Option<Value>,
) -> Result<Value, ServiceError> {
    let mut value = if content_type.starts_with("multipart/form-data") {
        let parts = multipart(content_type, bytes).await?;
        if ["sdp", "session"]
            .iter()
            .any(|name| parts.iter().filter(|part| &part.name == name).count() > 1)
        {
            return Err(invalid("Duplicate Realtime call field"));
        }
        let sdp = parts
            .iter()
            .find(|p| p.name == "sdp")
            .ok_or_else(|| invalid("Call requires an SDP field"))?;
        let mut value = json!({"sdp":std::str::from_utf8(&sdp.bytes).map_err(|_|invalid("SDP must be UTF-8"))?});
        if let Some(part) = parts.iter().find(|p| p.name == "session") {
            value["session"] =
                serde_json::from_slice(&part.bytes).map_err(|_| invalid("Invalid call session"))?;
        }
        value
    } else if content_type.starts_with("application/sdp") || content_type.starts_with("text/plain")
    {
        json!({"sdp":std::str::from_utf8(&bytes).map_err(|_|invalid("SDP must be UTF-8"))?})
    } else {
        serde_json::from_slice(&bytes)
            .map_err(|_| invalid("Call requires JSON, SDP or multipart data"))?
    };
    if !value.is_object() {
        return Err(invalid("Call body must be an object"));
    }
    if let Some(session) = session {
        value["session"] = session;
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn native_sdp_keeps_session_and_rejects_nonobjects() {
        let body = call_body(
            "application/sdp",
            Bytes::from_static(b"v=0\r\n"),
            Some(json!({"model":"gpt-realtime"})),
        )
        .await
        .unwrap();
        assert_eq!(body["sdp"], "v=0\r\n");
        assert_eq!(body["session"]["model"], "gpt-realtime");
        assert!(
            call_body("application/json", Bytes::from_static(b"[]"), None)
                .await
                .is_err()
        );
    }
}
