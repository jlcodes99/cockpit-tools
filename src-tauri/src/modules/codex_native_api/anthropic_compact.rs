//! CPA 9717e325 Claude compaction: summarize with the selected model, then retain
//! its opaque capsule. Replay admission still owns caller/account authorization.
use super::{anthropic, contract::ServiceError};
use aes_gcm::{
    aead::{rand_core::RngCore, Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const PREFIX: &str = "cpa-ag-compact-v1:";
const SUMMARY_PROMPT: &str = "Please provide a concise and comprehensive summary of the preceding conversation and task progress so far, including user goals, key findings, actions taken, and current status, so that work can continue smoothly.";

fn invalid(message: &str) -> ServiceError {
    ServiceError::new(400, "invalid_anthropic_compaction", message)
}

pub(super) fn triggered(body: &Value) -> bool {
    body["input"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| item["type"] == "compaction_trigger")
    })
}

fn cipher() -> Aes256Gcm {
    Aes256Gcm::new(&Sha256::digest(b"CLIProxyAPI"))
}

pub(super) fn seal(summary: &str, model: &str) -> Result<String, ServiceError> {
    let plaintext =
        json!({"summary":summary,"model":model,"created_at":chrono::Utc::now().timestamp()})
            .to_string();
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let encrypted = cipher()
        .encrypt(Nonce::from_slice(&nonce), plaintext.as_bytes())
        .map_err(|_| invalid("Unable to seal Claude compaction"))?;
    let mut data = nonce.to_vec();
    data.extend(encrypted);
    Ok(format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(data)))
}

pub(super) fn unseal(raw: &str) -> Result<String, ServiceError> {
    let raw = raw.trim();
    let data = URL_SAFE_NO_PAD
        .decode(
            raw.strip_prefix(PREFIX)
                .ok_or_else(|| invalid("Unrecognized Claude compaction capsule"))?,
        )
        .map_err(|_| invalid("Invalid Claude compaction capsule encoding"))?;
    if data.len() < 28 {
        return Err(invalid("Truncated Claude compaction capsule"));
    }
    let plaintext = cipher()
        .decrypt(Nonce::from_slice(&data[..12]), &data[12..])
        .map_err(|_| invalid("Invalid Claude compaction capsule"))?;
    let value: Value = serde_json::from_slice(&plaintext)
        .map_err(|_| invalid("Invalid Claude compaction content"))?;
    value["summary"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| invalid("Claude compaction contains no summary"))
}

/// Capsule crypto uses the shared bounded blocking pool, never a Tokio worker.
pub(super) async fn expand_wire(state: &super::server::AppState, mut body: Value) -> Result<Value, ServiceError> {
    let present = body["input"].as_array().is_some_and(|items| items.iter().any(|item| {
        matches!(item["type"].as_str(), Some("compaction" | "compaction_summary")) &&
            (item["encrypted_content"].as_str().is_some_and(|raw| raw.trim().starts_with(PREFIX)) || item["id"].as_str().is_some_and(|id| id.starts_with("cmp_ag_compact_")))
    }));
    if !present { return Ok(body); }
    super::blocking::run_prepared(state, move || { expand_recognized(&mut body)?; Ok(body) }).await
}

/// Only CPA-minted state is decrypted; official/foreign opaque state is untouched.
pub(super) fn expand_recognized(body: &mut Value) -> Result<(), ServiceError> {
    if let Some(input) = body.get_mut("input").and_then(Value::as_array_mut) {
        for item in input {
            if item["type"] != "compaction" && item["type"] != "compaction_summary" { continue; }
            let raw = item["encrypted_content"].as_str().unwrap_or("").trim();
            let known_id = item["id"].as_str().is_some_and(|id| id.trim().starts_with("cmp_ag_compact_"));
            if !raw.starts_with(PREFIX) && !known_id { continue; }
            let summary = unseal(raw)?;
            *item = json!({"type":"message","role":"developer","content":[{"type":"input_text","text":format!("Context summary from previous turns:\n{summary}")}]});
        }
    }
    Ok(())
}

pub(super) fn expand(body: &mut Value) -> Result<(), ServiceError> {
    if let Some(input) = body["input"].as_array_mut() {
        for item in input {
            if matches!(
                item["type"].as_str(),
                Some("compaction" | "compaction_summary")
            ) {
                let summary = unseal(item["encrypted_content"].as_str().unwrap_or(""))?;
                *item = json!({"type":"message","role":"developer","content":[{"type":"input_text","text":format!("Context summary from previous turns:\n{summary}")}]});
            }
        }
    }
    Ok(())
}

pub(super) fn prepare(body: &mut Value) -> Result<(), ServiceError> {
    expand(body)?;
    let mut input = if let Some(text) = body["input"].as_str() {
        vec![json!({"role":"user","content":text})]
    } else {
        body["input"].as_array().cloned().unwrap_or_default()
    };
    input.retain(|item| item["type"] != "compaction_trigger");
    input.push(json!({"type":"message","role":"user","content":[{"type":"input_text","text":SUMMARY_PROMPT}]}));
    body["input"] = json!(input);
    let object = body
        .as_object_mut()
        .ok_or_else(|| invalid("Compaction requires an object request"))?;
    for field in [
        "previous_response_id",
        "parallel_tool_calls",
        "truncation",
        "metadata",
        "text",
    ] {
        object.remove(field);
    }
    object.insert("stream".into(), json!(false));
    object.insert("tool_choice".into(), json!("none"));
    Ok(())
}

pub(super) fn finish(summary: Value, model: &str) -> Result<Value, ServiceError> {
    if summary["status"] != "completed"
        || !summary["error"].is_null()
        || !summary["incomplete_details"].is_null()
    {
        return Err(ServiceError::new(
            502,
            "anthropic_compaction_incomplete",
            "Claude did not complete the context summary; existing history was preserved",
        ));
    }
    let texts: Vec<_> = summary["output"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item["type"] == "message")
        .flat_map(|item| item["content"].as_array().into_iter().flatten())
        .filter_map(|part| part["text"].as_str())
        .filter(|s| !s.is_empty())
        .collect();
    if texts.is_empty() {
        return Err(ServiceError::new(
            502,
            "anthropic_compaction_empty",
            "Claude returned no context summary; existing history was preserved",
        ));
    }
    let capsule = seal(&texts.join("\n"), model)?;
    Ok(
        json!({"id":format!("resp_ag_compact_{}",uuid::Uuid::now_v7()),"object":"response.compaction","model":model,"status":"completed","created_at":chrono::Utc::now().timestamp(),"output":[{"id":format!("cmp_ag_compact_{}",uuid::Uuid::now_v7()),"type":"compaction","status":"completed","encrypted_content":capsule}],"usage":summary["usage"]}),
    )
}

// Same framing as CPA's inline compact operation; the summary itself is never
// emitted as an ordinary assistant answer or executed as a tool call.
pub(super) fn events(response: Value, model: &str) -> Result<Vec<Value>, ServiceError> {
    super::grok_wire::compaction::events(response, model)
}

pub(super) fn messages(body: &Value) -> Result<Value, ServiceError> {
    let mut body = body.clone();
    prepare(&mut body)?;
    let mut messages = anthropic::responses_to_messages(&body)?;
    // A compact request can contain tool history without current definitions.
    // Match CPA's summary-only flattening in that case.
    if messages["tools"].as_array().is_none_or(Vec::is_empty) {
        messages.as_object_mut().unwrap().remove("tool_choice");
        for message in messages["messages"].as_array_mut().unwrap() {
            for block in message["content"].as_array_mut().unwrap() {
                if block["type"] == "tool_use" {
                    *block = json!({"type":"text","text":format!("Tool call {} ({}): {}",block["name"].as_str().unwrap_or(""),block["id"].as_str().unwrap_or(""),block["input"])});
                } else if block["type"] == "tool_result" {
                    *block = json!({"type":"text","text":format!("Tool result {}: {}",block["tool_use_id"].as_str().unwrap_or(""),block["content"])});
                }
            }
        }
    }
    Ok(messages)
}
