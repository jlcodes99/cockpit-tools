//! Keep provider-native Chat options until the host has selected an account.
//! Responses routes omit Go-compatible ignored options; Chat routes can preserve
//! stop, top_k and multiple candidates without forcing a single Responses item.
use super::super::chat_upstream::{self, ResponsesStreamAdapter};
use super::{
    invalid, upstream, CompatContext, CompatFlavor, CompatFrame, CompatStreamAdapter, ServiceError,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub struct NativeChatRequest {
    pub body: Value,
    /// Full conversation for routing and opaque ownership admission, never a
    /// placeholder input that could bypass a cipher from the client history.
    pub responses: Value,
    pub context: CompatContext,
    pub stream: bool,
    pub responses_error: Option<ServiceError>,
}

pub fn prepare_native_chat(
    body: &Value,
    flavor: CompatFlavor,
    model: Option<&str>,
    stream: Option<bool>,
) -> Result<NativeChatRequest, ServiceError> {
    let strict = super::prepare_request(body.clone(), flavor, model, stream);
    let mut representable = body.clone();
    match flavor {
        CompatFlavor::Claude => {
            representable
                .as_object_mut()
                .ok_or_else(|| invalid("body", "expected an object"))?
                .remove("stop_sequences");
        }
        CompatFlavor::Gemini => {
            if let Some(config) = representable
                .get_mut("generationConfig")
                .and_then(Value::as_object_mut)
            {
                for key in [
                    "topK",
                    "stopSequences",
                    "candidateCount",
                    "responseModalities",
                ] {
                    config.remove(key);
                }
            }
        }
        CompatFlavor::Ollama => {}
    }
    let converted = super::prepare_request(representable, flavor, model, stream)?;
    let mut native = chat_upstream::responses_to_native_chat(&converted.responses)?;
    // The Go Codex translator supplies medium by default, while a native Chat
    // provider keeps its own default when the client expressed no thinking intent.
    let has_thinking = match flavor {
        CompatFlavor::Claude => {
            body.get("thinking").is_some() || body.pointer("/output_config/effort").is_some()
        }
        CompatFlavor::Gemini => [
            "/generationConfig/thinkingConfig",
            "/generationConfig/thinking_config",
            "/generationConfig/thinkingLevel",
            "/generationConfig/thinking_level",
        ]
        .iter()
        .any(|path| body.pointer(path).is_some()),
        CompatFlavor::Ollama => body.get("think").is_some(),
    };
    if !has_thinking {
        native.as_object_mut().unwrap().remove("reasoning_effort");
    }
    match flavor {
        CompatFlavor::Claude => {
            if body.pointer("/thinking/type").and_then(Value::as_str) == Some("enabled")
                && body.pointer("/thinking/budget_tokens").is_none()
            {
                native["reasoning_effort"] = json!("auto");
            }

            if let Some(stop) = body.get("stop_sequences") {
                validate_stop(stop)?;
                if !stop.as_array().unwrap().is_empty() {
                    native["stop"] = stop.clone();
                }
            }
            if let Some(max) = body.get("max_tokens") {
                native
                    .as_object_mut()
                    .unwrap()
                    .remove("max_completion_tokens");
                native["max_tokens"] = max.clone();
            }
        }
        CompatFlavor::Gemini => {
            let config = &body["generationConfig"];
            if let Some(stop) = config.get("stopSequences") {
                validate_stop(stop)?;
                if !stop.as_array().unwrap().is_empty() {
                    native["stop"] = stop.clone();
                }
            }
            if let Some(top_k) = config.get("topK") {
                native["top_k"] = top_k.clone();
            }
            if let Some(n) = config.get("candidateCount") {
                if n.as_u64().is_none_or(|n| n == 0) {
                    return Err(invalid("candidateCount", "expected a positive integer"));
                }
                native["n"] = n.clone();
            }
            if let Some(modalities) = config.get("responseModalities") {
                let converted = modalities
                    .as_array()
                    .ok_or_else(|| invalid("responseModalities", "expected an array"))?
                    .iter()
                    .map(|value| match value.as_str() {
                        Some("TEXT") => Ok(json!("text")),
                        Some("IMAGE") => Ok(json!("image")),
                        Some("AUDIO") => Ok(json!("audio")),
                        _ => Err(invalid("responseModalities", "unknown modality")),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                native["modalities"] = json!(converted);
            }
            if let Some(max) = config.get("maxOutputTokens") {
                native
                    .as_object_mut()
                    .unwrap()
                    .remove("max_completion_tokens");
                native["max_tokens"] = max.clone();
            }
        }
        CompatFlavor::Ollama => {
            if let Some(max) = body
                .pointer("/options/num_predict")
                .filter(|value| value.as_i64().is_some_and(|n| n > 0))
            {
                native
                    .as_object_mut()
                    .unwrap()
                    .remove("max_completion_tokens");
                native["max_tokens"] = max.clone();
            }
        }
    }
    Ok(NativeChatRequest {
        body: native,
        responses: converted.responses,
        context: converted.context,
        stream: converted.stream,
        responses_error: strict.err(),
    })
}

fn validate_stop(value: &Value) -> Result<(), ServiceError> {
    if !value
        .as_array()
        .is_some_and(|items| items.iter().all(Value::is_string))
    {
        return Err(invalid("stop sequences", "expected a string array"));
    }
    Ok(())
}

pub fn response_from_chat(value: &Value, context: &CompatContext) -> Result<Value, ServiceError> {
    let choices = value["choices"]
        .as_array()
        .filter(|choices| !choices.is_empty())
        .ok_or_else(|| upstream("Chat response has no choices"))?;
    if context.flavor != CompatFlavor::Gemini && choices.len() != 1 {
        return Err(upstream("This client protocol expects one candidate"));
    }
    let mut result = None;
    let mut candidates = Vec::new();
    for (position, choice) in choices.iter().enumerate() {
        let mut single = value.clone();
        single["choices"] = json!([choice]);
        let canonical = chat_upstream::chat_to_response(&single, &context.model)?;
        let converted = super::response_from_responses(&canonical, context)?;
        if context.flavor == CompatFlavor::Gemini {
            let mut candidate = converted["candidates"][0].clone();
            candidate["index"] = choice.get("index").cloned().unwrap_or(json!(position));
            candidates.push(candidate);
        }
        if result.is_none() {
            result = Some(converted);
        }
    }
    let mut result = result.unwrap();
    if context.flavor == CompatFlavor::Gemini {
        result["candidates"] = json!(candidates);
    }
    Ok(result)
}

struct Choice {
    upstream: ResponsesStreamAdapter,
    client: CompatStreamAdapter,
}
pub struct NativeChatStreamAdapter {
    context: CompatContext,
    choices: BTreeMap<u64, Choice>,
    expected: usize,
    finished: bool,
}
impl NativeChatStreamAdapter {
    pub fn new(context: CompatContext, expected: usize) -> Self {
        Self {
            context,
            choices: BTreeMap::new(),
            expected: expected.max(1),
            finished: false,
        }
    }
    pub fn push(&mut self, value: &Value) -> Result<Vec<CompatFrame>, ServiceError> {
        if self.finished {
            return Ok(Vec::new());
        }
        if let Some(error) = super::super::upstream_errors::chat_failure(value, 502) {
            return Err(error);
        }
        let choices = value["choices"]
            .as_array()
            .ok_or_else(|| upstream("Chat stream has no choices"))?;
        let mut frames = Vec::new();
        if choices.is_empty() {
            for choice in self.choices.values_mut() {
                choice.upstream.push(value)?;
            }
        }
        for choice in choices {
            let index = choice["index"].as_u64().unwrap_or(0);
            if self.context.flavor != CompatFlavor::Gemini && index != 0 {
                return Err(upstream("This client protocol expects one candidate"));
            }
            if !self.choices.contains_key(&index) && self.choices.len() >= self.expected {
                return Err(upstream("Upstream returned more candidates than requested"));
            }
            let entry = self.choices.entry(index).or_insert_with(|| Choice {
                upstream: ResponsesStreamAdapter::new(self.context.model.clone()),
                client: CompatStreamAdapter::new(self.context.clone()),
            });
            let mut single = value.clone();
            let mut choice = choice.clone();
            choice["index"] = json!(0);
            single["choices"] = json!([choice]);
            for event in entry.upstream.push(&single)? {
                for mut frame in entry.client.push(&event)? {
                    reindex(&mut frame, index);
                    frames.push(frame);
                }
            }
        }
        Ok(frames)
    }
    pub fn finish(&mut self) -> Result<Vec<CompatFrame>, ServiceError> {
        self.complete(false)
    }
    pub fn finish_done(&mut self) -> Result<Vec<CompatFrame>, ServiceError> {
        self.complete(true)
    }
    fn complete(&mut self, done: bool) -> Result<Vec<CompatFrame>, ServiceError> {
        if self.finished {
            return Ok(Vec::new());
        }
        if self.choices.len() != self.expected {
            return Err(upstream("Chat stream ended with missing candidates"));
        }
        let mut frames = Vec::new();
        for (index, choice) in &mut self.choices {
            let events = if done {
                choice.upstream.finish_done()?
            } else {
                choice.upstream.finish()?
            };
            for event in events {
                for mut frame in choice.client.push(&event)? {
                    reindex(&mut frame, *index);
                    frames.push(frame);
                }
            }
            choice.client.finish()?;
        }
        self.finished = true;
        Ok(frames)
    }
}
fn reindex(frame: &mut CompatFrame, index: u64) {
    if let Some(candidates) = frame
        .data
        .get_mut("candidates")
        .and_then(Value::as_array_mut)
    {
        for candidate in candidates {
            candidate["index"] = json!(index);
        }
    }
}

#[cfg(test)]
#[path = "compat_native_chat_tests.rs"]
mod tests;
