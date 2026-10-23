//! Go-compatible ignored generation hints, applied only to the Responses conversion copy.
//! The original request remains available to native Chat upstreams without losing options.
use serde_json::Value;

use super::compat::CompatFlavor;

pub(super) fn chat(mut body: Value, completions: bool) -> Value {
    remove(
        &mut body,
        &[
            "n",
            "stop",
            "seed",
            "logit_bias",
            "prediction",
            "audio",
            "suffix",
            "presence_penalty",
            "frequency_penalty",
            "modalities",
        ],
    );
    if completions {
        remove(&mut body, &["echo", "best_of", "logprobs"]);
    }
    if body.get("functions").is_some() || body.get("function_call").is_some() {
        // The Rust legacy-functions extension is retained, but its one-call envelope
        // cannot represent the ignored parallel generation hint.
        remove(&mut body, &["parallel_tool_calls"]);
    }
    if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
        for message in messages {
            if matches!(
                message["role"].as_str(),
                Some("system" | "developer" | "user" | "assistant")
            ) {
                // Go does not interpret named participants or referenced audio output.
                remove(message, &["name", "audio"]);
            }
        }
    }
    body
}

pub(super) fn compatibility(mut body: Value, flavor: CompatFlavor) -> Value {
    match flavor {
        CompatFlavor::Claude => {
            remove(
                &mut body,
                &[
                    "stop_sequences",
                    "top_k",
                    "container",
                    "context_management",
                    "mcp_servers",
                ],
            );
            // Citation metadata has no effect on the Go document-to-input_file bridge.
            if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
                for message in messages {
                    if let Some(blocks) = message.get_mut("content").and_then(Value::as_array_mut) {
                        for block in blocks {
                            if block["type"] == "document" {
                                remove(block, &["citations"]);
                            }
                        }
                    }
                }
            }
        }
        CompatFlavor::Gemini => {
            remove(&mut body, &["safetySettings", "cachedContent"]);
            if let Some(config) = body.get_mut("generationConfig") {
                remove(
                    config,
                    &[
                        "stopSequences",
                        "topK",
                        "responseLogprobs",
                        "logprobs",
                        "audioTimestamp",
                        "mediaResolution",
                        "speechConfig",
                        "candidateCount",
                        "responseModalities",
                    ],
                );
                if config
                    .get("responseMimeType")
                    .is_some_and(|mime| mime != "application/json" && mime != "text/plain")
                {
                    remove(config, &["responseMimeType"]);
                }
            }
            if let Some(choice) = body.pointer_mut("/toolConfig/functionCallingConfig") {
                if choice
                    .get("mode")
                    .and_then(Value::as_str)
                    .is_some_and(|mode| !matches!(mode, "AUTO" | "NONE" | "ANY"))
                {
                    remove(choice, &["mode"]);
                }
            }
        }
        CompatFlavor::Ollama => {
            if let Some(options) = body.get_mut("options").and_then(Value::as_object_mut) {
                options.retain(|key, value| match key.as_str() {
                    "temperature" | "top_p" => value.is_number(),
                    "num_predict" => value.as_i64().is_some_and(|value| value > 0),
                    _ => false,
                });
            }
        }
    }
    body
}

fn remove(body: &mut Value, names: &[&str]) {
    if let Some(object) = body.as_object_mut() {
        for name in names {
            object.remove(*name);
        }
    }
}

#[cfg(test)]
#[path = "compat_options_tests.rs"]
mod tests;
