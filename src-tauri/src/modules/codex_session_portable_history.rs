//! Create a self-contained rollout using the official client's legacy JSONL reader.
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{json, Value};

use super::{list_rollout_files, rollout_reader, SESSION_DIRS};
use crate::modules::codex_history_projection_repair::rollout_file_ids;
use crate::modules::codex_session_rollout_records::Records;

#[derive(Clone, Deserialize)]
struct Cutoff {
    thread_id: uuid::Uuid,
    end_ordinal_exclusive: u64,
    end_byte_offset: u64,
}

#[derive(Deserialize)]
struct RecordHeader {
    ordinal: Option<u64>,
    #[serde(rename = "type")]
    kind: String,
    payload: Option<RecordPayload>,
}

#[derive(Deserialize)]
struct RecordPayload {
    #[serde(rename = "type")]
    kind: Option<String>,
    item: Option<ItemHeader>,
}

#[derive(Deserialize)]
struct ItemHeader {
    #[serde(rename = "type")]
    kind: Option<String>,
}

impl RecordHeader {
    fn needs_legacy_event(&self) -> bool {
        self.kind == "event_msg"
            && self.payload.as_ref().is_some_and(|payload| {
                payload.kind.as_deref() == Some("item_completed")
                    && matches!(
                        payload.item.as_ref().and_then(|item| item.kind.as_deref()),
                        Some(
                            "UserMessage"
                                | "AgentMessage"
                                | "Reasoning"
                                | "WebSearch"
                                | "ImageView"
                                | "ImageGeneration"
                        )
                    )
            })
    }
}

type RolloutPaths = HashMap<String, Vec<PathBuf>>;

#[derive(Default)]
pub(super) struct Catalog {
    profiles: HashMap<PathBuf, RolloutPaths>,
}

impl Catalog {
    fn resolve(&mut self, root: &Path, id: uuid::Uuid) -> Result<&Path, String> {
        if !self.profiles.contains_key(root) {
            let mut paths = RolloutPaths::new();
            for folder in SESSION_DIRS {
                let directory = root.join(folder);
                if !directory.is_dir() {
                    continue;
                }
                for path in list_rollout_files(&directory)? {
                    if let Some((_, physical)) = rollout_file_ids(&path) {
                        paths.entry(physical).or_default().push(path);
                    }
                }
            }
            self.profiles.insert(root.to_owned(), paths);
        }
        let paths = self.profiles[root]
            .get(&id.to_string())
            .ok_or_else(|| format!("missing history rollout {id}"))?;
        let first = &paths[0];
        if paths
            .iter()
            .any(|path| !rollout_reader::same_storage_path(first, path))
        {
            return Err(format!("ambiguous history rollout {id}"));
        }
        Ok(paths
            .iter()
            .find(|path| !rollout_reader::is_compressed(path))
            .unwrap_or(first))
    }
}

pub(super) fn is_paginated(metadata: &Value) -> bool {
    metadata["payload"]["history_mode"] == "paginated"
        || !metadata["payload"]["history_base"].is_null()
}

fn history_base(metadata: &mut Value) -> Result<Option<Cutoff>, String> {
    let payload = metadata["payload"]
        .as_object_mut()
        .ok_or("invalid session metadata")?;
    payload
        .remove("history_base")
        .filter(|value| !value.is_null())
        .map(|value| serde_json::from_value::<Cutoff>(value).map_err(|e| e.to_string()))
        .transpose()
}

pub(super) fn write_portable_rollout(
    root: &Path,
    head: &Path,
    mut metadata: Value,
    target: &Path,
    catalog: &mut Catalog,
) -> Result<(), String> {
    let mut next = history_base(&mut metadata)?;
    metadata["payload"]["history_mode"] = Value::String("legacy".into());
    metadata
        .as_object_mut()
        .ok_or("invalid session record")?
        .remove("ordinal");
    let mut chain = Vec::new();
    let mut seen = HashSet::from([head.to_owned()]);
    while let Some(cutoff) = next {
        let source = catalog.resolve(root, cutoff.thread_id)?.to_owned();
        if !seen.insert(source.clone()) {
            return Err(format!("cyclic history rollout {}", cutoff.thread_id));
        }
        let mut source_meta = rollout_reader::metadata(&source)
            .map_err(|e| e.to_string())?
            .ok_or("missing ancestor metadata")?;
        let (logical, _) = rollout_file_ids(&source).ok_or("invalid ancestor filename")?;
        if source_meta["payload"]["id"] != logical
            || source_meta["payload"]["history_mode"] != "paginated"
        {
            return Err(format!(
                "invalid ancestor metadata for {}",
                cutoff.thread_id
            ));
        }
        next = history_base(&mut source_meta)?;
        chain.push((source, cutoff));
    }

    let mut output = BufWriter::new(File::create(target).map_err(|e| e.to_string())?);
    serde_json::to_writer(&mut output, &metadata).map_err(|e| e.to_string())?;
    output.write_all(b"\n").map_err(|e| e.to_string())?;
    for (source, cutoff) in chain.iter().rev() {
        append_records(source, Some(cutoff), &mut output)
            .map_err(|error| format!("{}: {error}", cutoff.thread_id))?;
    }
    append_records(head, None, &mut output).map_err(|e| e.to_string())?;
    output.flush().map_err(|e| e.to_string())
}

fn append_records(path: &Path, cutoff: Option<&Cutoff>, output: &mut impl Write) -> io::Result<()> {
    let before = fs::metadata(path)?;
    // The parser skips large unneeded fields without allocating whole records.
    let mut records = Records::new(rollout_reader::open(path)?, |_: &[u8]| {
        std::thread::yield_now();
        Ok(())
    });
    let header = loop {
        let record = records
            .next::<RecordHeader>()?
            .ok_or_else(|| io::Error::other("missing rollout metadata"))?;
        if !record.blank {
            break record.value?;
        }
    };
    let start = records.position;
    let mut end = start;
    let mut source = rollout_reader::open(path)?;
    copy_bytes(&mut source, &mut io::sink(), start)?;
    let mut copied = start;
    let mut next_ordinal = header.ordinal.map_or(0, |value| value.saturating_add(1));
    let mut trailing_newline = true;
    loop {
        let line_start = records.position;
        let Some(record) = records.next::<RecordHeader>()? else {
            break;
        };
        if record.blank {
            continue;
        }
        let parsed = record.value?;
        if let Some(cutoff) = cutoff {
            let Some(ordinal) = parsed.ordinal else {
                continue;
            };
            if ordinal >= cutoff.end_ordinal_exclusive {
                next_ordinal = ordinal;
                break;
            }
            next_ordinal = ordinal.saturating_add(1);
        }
        end = records.position;
        trailing_newline = record.complete;
        if parsed.needs_legacy_event() {
            copy_bytes(&mut source, output, line_start - copied)?;
            let length = end - line_start;
            let mut item_source = source.by_ref().take(length);
            let value: Value = serde_json::from_reader(&mut item_source)?;
            if item_source.limit() != 0 {
                return Err(io::Error::other("history changed during export"));
            }
            write_json_line(output, &value)?;
            for payload in legacy_events(&value["payload"]["item"]) {
                write_json_line(
                    output,
                    &json!({
                        "timestamp": value["timestamp"], "type": "event_msg", "payload": payload,
                    }),
                )?;
            }
            copied = end;
            trailing_newline = true;
        }
    }
    if cutoff.is_some_and(|limit| {
        limit.end_ordinal_exclusive == 0 || next_ordinal < limit.end_ordinal_exclusive
    }) {
        return Err(io::Error::other("history ordinal cutoff is unavailable"));
    }

    copy_bytes(&mut source, output, end - copied)?;
    // Older files may have a byte cutoff different from the line boundary.
    // Use ordinals to choose records and check that the recorded byte span exists.
    if let Some(limit) = cutoff {
        let remaining = limit.end_byte_offset.saturating_sub(end);
        if io::copy(&mut source.take(remaining), &mut io::sink())? != remaining {
            return Err(io::Error::other("history byte cutoff is unavailable"));
        }
    }
    if !trailing_newline {
        output.write_all(b"\n")?;
    }
    let after = fs::metadata(path)?;
    if after.len() != before.len() || after.modified()? != before.modified()? {
        return Err(io::Error::other("history changed during export"));
    }
    Ok(())
}

fn copy_bytes(input: &mut impl Read, output: &mut impl Write, length: u64) -> io::Result<()> {
    if io::copy(&mut input.take(length), output)? == length {
        Ok(())
    } else {
        Err(io::Error::other("history changed during export"))
    }
}

fn write_json_line(output: &mut impl Write, value: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *output, value)?;
    output.write_all(b"\n")
}

fn legacy_events(item: &Value) -> Vec<Value> {
    let array = |key: &str| item[key].as_array().into_iter().flatten();
    match item["type"].as_str() {
        Some("UserMessage") => vec![legacy_user_message(item)],
        Some("AgentMessage") => array("content")
            .filter_map(|entry| {
                Some(json!({
                    "type": "agent_message", "message": entry["text"].as_str()?,
                    "phase": item["phase"], "memory_citation": item["memory_citation"],
                    "delivery": item["delivery"], "questions": item["questions"],
                }))
            })
            .collect(),
        Some("Reasoning") => array("summary_text")
            .map(|text| {
                json!({
                    "type": "agent_reasoning", "text": text,
                })
            })
            .collect(),
        Some("WebSearch") => vec![json!({
            "type": "web_search_end", "call_id": item["id"], "query": item["query"],
            "action": item["action"], "results": item["results"],
        })],
        Some("ImageView") => vec![json!({
            "type": "view_image_tool_call", "call_id": item["id"], "path": item["path"],
        })],
        Some("ImageGeneration") => vec![json!({
            "type": "image_generation_end", "call_id": item["id"], "status": item["status"],
            "revised_prompt": item["revised_prompt"], "result": item["result"],
            "saved_path": item["saved_path"],
        })],
        _ => Vec::new(),
    }
}

fn legacy_user_message(item: &Value) -> Value {
    let mut message = String::new();
    let mut elements = Vec::new();
    let mut images = Vec::new();
    let mut image_details = Vec::new();
    let mut files = Vec::new();
    let mut file_details = Vec::new();
    let mut image_order = Vec::new();
    let mut local_images = Vec::new();
    let mut local_details = Vec::new();
    let mut audio = Vec::new();
    let mut local_audio = Vec::new();
    for input in item["content"].as_array().into_iter().flatten() {
        match input["type"].as_str() {
            Some("text") => {
                for element in input["text_elements"].as_array().into_iter().flatten() {
                    if let (Some(start), Some(end)) = (
                        element["byte_range"]["start"].as_u64(),
                        element["byte_range"]["end"].as_u64(),
                    ) {
                        let mut element = element.clone();
                        let offset = message.len() as u64;
                        if let (Some(start), Some(end)) =
                            (offset.checked_add(start), offset.checked_add(end))
                        {
                            element["byte_range"] = json!({ "start": start, "end": end });
                            elements.push(element);
                        }
                    }
                }
                message.push_str(input["text"].as_str().unwrap_or_default());
            }
            Some("image") if input["image_url"].is_string() => {
                images.push(&input["image_url"]);
                image_details.push(&input["detail"]);
                image_order.push("inline");
            }
            Some("image") if input["file_id"].is_string() => {
                files.push(&input["file_id"]);
                file_details.push(&input["detail"]);
                image_order.push("file");
            }
            Some("local_image") => {
                local_images.push(&input["path"]);
                local_details.push(&input["detail"]);
            }
            Some("audio") => audio.push(&input["audio_url"]),
            Some("local_audio") => local_audio.push(&input["path"]),
            _ => {}
        }
    }
    json!({
        "type": "user_message", "client_id": item["client_id"], "message": message,
        "text_elements": elements, "images": images, "image_details": image_details,
        "file_ids": files, "file_id_details": file_details, "image_order": image_order,
        "local_images": local_images, "local_image_details": local_details,
        "audio": audio, "local_audio": local_audio,
    })
}
