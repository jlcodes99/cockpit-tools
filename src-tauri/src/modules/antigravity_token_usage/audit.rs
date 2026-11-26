//! CLI session merger and single-conversation context & execution audit engine.
//!
//! - Merges `~/.gemini/antigravity-cli` telemetry into the primary `ScanResult`
//!   without double-counting shared session IDs.
//! - Parses `transcript.jsonl` and resolved turns on demand when a conversation detail
//!   modal is opened to surface context window pressure, compaction (`CHECKPOINT`)
//!   events, tool call breakdown, command executions, file reads/edits, and subagent spawns.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::models::resolve_model_context_limit;
use super::scanner::{decode_workspace_uris, UNASSIGNED_WORKSPACE_KEY};
use super::types::{
    ChildConversationRef, ClientSourceKind, ConversationContextAudit, ConversationKind, ScanResult,
    ToolCallSummary, TurnUsageRecord,
};

/// Resolves the companion `~/.gemini/antigravity-cli` directory when `data_dir` points
/// to `~/.gemini/antigravity` (or has a sibling `antigravity-cli` folder).
pub fn resolve_companion_cli_data_dir(primary_data_dir: &Path) -> Option<PathBuf> {
    let dir_name = primary_data_dir.file_name()?.to_str()?;
    if !dir_name.eq_ignore_ascii_case("antigravity") {
        return None;
    }
    let candidate = primary_data_dir.parent()?.join("antigravity-cli");
    candidate.is_dir().then_some(candidate)
}

/// Attempts to infer a workspace path from a conversation's `transcript.jsonl`
/// when `conversation_summaries.db` is absent (common in `antigravity-cli` sessions).
pub fn infer_workspace_from_transcript(transcript_path: &Path) -> Option<(String, String, String)> {
    let file = std::fs::File::open(transcript_path).ok()?;
    use std::io::{BufRead, BufReader};
    let reader = BufReader::new(file);

    for line_res in reader.lines().take(48) {
        let Ok(line) = line_res else {
            continue;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Check for workspace path / file paths in early lines
        if let Ok(obj) = serde_json::from_str::<serde_json::Value>(trimmed) {
            for key in ["workspacePath", "workspace_path", "cwd", "workingDirectory"] {
                if let Some(p) = obj.get(key).and_then(|v| v.as_str()).map(str::trim) {
                    if !p.is_empty() {
                        let ws = decode_workspace_uris(p);
                        if ws.workspace_key != UNASSIGNED_WORKSPACE_KEY {
                            return Some((ws.workspace_uri, ws.workspace_path, ws.workspace_name));
                        }
                    }
                }
            }
            if let Some(arr) = obj.get("workspacePaths").and_then(|v| v.as_array()) {
                for item in arr {
                    if let Some(p) = item.as_str().map(str::trim) {
                        if !p.is_empty() {
                            let ws = decode_workspace_uris(p);
                            if ws.workspace_key != UNASSIGNED_WORKSPACE_KEY {
                                return Some((
                                    ws.workspace_uri,
                                    ws.workspace_path,
                                    ws.workspace_name,
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Merges a scanned `antigravity-cli` result into the primary desktop `ScanResult`
/// with zero double-counting of conversation IDs.
pub fn merge_cli_scan_into_primary(primary: &mut ScanResult, cli_scan: ScanResult) {
    primary.cli_data_dir = Some(cli_scan.data_dir.clone());
    for (k, v) in cli_scan.dynamic_enum_map {
        primary.dynamic_enum_map.entry(k).or_insert(v);
    }

    let mut added_cli = 0usize;
    for mut conv in cli_scan.conversations {
        if primary
            .index_by_id
            .contains_key(&conv.summary.conversation_id)
        {
            continue;
        }
        conv.summary.client_source = ClientSourceKind::Cli;
        if conv.summary.workspace_path.is_empty() {
            let tr_path = cli_scan
                .data_dir
                .join("brain")
                .join(&conv.summary.conversation_id)
                .join(".system_generated")
                .join("logs")
                .join("transcript.jsonl");
            if tr_path.is_file() {
                if let Some((uri, path, name)) = infer_workspace_from_transcript(&tr_path) {
                    conv.summary.workspace_uri = uri;
                    conv.summary.workspace_path = path;
                    conv.summary.workspace_name = name;
                }
            }
        }
        added_cli += 1;
        primary.conversations.push(conv);
    }

    if added_cli > 0 {
        primary.scan.cli_sessions_found = added_cli;
        primary.scan.total_db_files = primary
            .scan
            .total_db_files
            .saturating_add(cli_scan.scan.total_db_files);
        primary.scan.brain_dirs_found = primary
            .scan
            .brain_dirs_found
            .saturating_add(cli_scan.scan.brain_dirs_found);

        // Rebuild parent-child references across merged conversations
        let mut children_map: HashMap<String, Vec<ChildConversationRef>> = HashMap::new();
        for c in &primary.conversations {
            if !c.summary.parent_conversation_id.is_empty() {
                children_map
                    .entry(c.summary.parent_conversation_id.clone())
                    .or_default()
                    .push(ChildConversationRef {
                        conversation_id: c.summary.conversation_id.clone(),
                        title: c.summary.display_title.clone(),
                        agent_name: c.summary.agent_name.clone(),
                        kind: c.summary.kind,
                        exact_total_tokens: c.summary.exact.total_tokens,
                        estimated_total_tokens: c.summary.estimated.total_tokens,
                    });
            }
        }
        for kids in children_map.values_mut() {
            kids.sort_by(|a, b| {
                b.exact_total_tokens
                    .cmp(&a.exact_total_tokens)
                    .then_with(|| b.estimated_total_tokens.cmp(&a.estimated_total_tokens))
                    .then_with(|| a.conversation_id.cmp(&b.conversation_id))
            });
        }
        for c in &mut primary.conversations {
            if let Some(kids) = children_map.remove(&c.summary.conversation_id) {
                if kids.iter().any(|k| k.kind == ConversationKind::Swarm) {
                    c.summary.has_swarm_children = true;
                }
                c.children = kids;
            }
        }

        primary.conversations.sort_by(|a, b| {
            b.summary
                .last_modified_time
                .cmp(&a.summary.last_modified_time)
                .then_with(|| a.summary.conversation_id.cmp(&b.summary.conversation_id))
        });
        primary.index_by_id.clear();
        for (idx, c) in primary.conversations.iter().enumerate() {
            primary
                .index_by_id
                .insert(c.summary.conversation_id.clone(), idx);
        }
    }
}

/// Builds a deep context & execution audit for a single conversation from its resolved turns
/// and `transcript.jsonl` (if available on disk).
pub fn build_conversation_context_audit(
    root_dir: &Path,
    conversation_id: &str,
    primary_model_id: &str,
    turns: &[TurnUsageRecord],
    children_count: usize,
) -> ConversationContextAudit {
    let context_limit_tokens = resolve_model_context_limit(primary_model_id);

    // Peak and latest context window size from per-turn prompt + cache tokens:
    // Each turn's context footprint reaching the model is `input_tokens + cache_read_tokens + cache_write_tokens`.
    let mut peak_context_tokens = 0u64;
    let mut latest_context_tokens = 0u64;

    for turn in turns {
        let ctx = if turn.is_exact {
            turn.exact
                .input_tokens
                .saturating_add(turn.exact.cache_read_tokens)
                .saturating_add(turn.exact.cache_write_tokens)
        } else {
            turn.estimated.input_tokens
        };
        if ctx > 0 {
            latest_context_tokens = ctx;
            if ctx > peak_context_tokens {
                peak_context_tokens = ctx;
            }
        }
    }

    let tr_path = root_dir
        .join("brain")
        .join(conversation_id)
        .join(".system_generated")
        .join("logs")
        .join("transcript.jsonl");

    let mut user_prompt_count = 0usize;
    let mut assistant_turn_count = turns.len();
    let mut tool_call_count = 0usize;
    let mut command_execution_count = 0usize;
    let mut file_read_count = 0usize;
    let mut file_edit_count = 0usize;
    let mut subagent_spawn_count = children_count;
    let mut compaction_steps: Vec<usize> = Vec::new();
    let mut tool_counts: HashMap<String, usize> = HashMap::new();
    let mut active_files: Vec<String> = Vec::new();
    let mut seen_files: HashSet<String> = HashSet::new();

    if tr_path.is_file() {
        if let Ok(content) = std::fs::read_to_string(&tr_path) {
            let mut planner_count = 0usize;
            let mut subagent_calls_in_tr = 0usize;

            for (line_idx, raw_line) in content.lines().enumerate() {
                let line = raw_line.trim();
                if line.is_empty() {
                    continue;
                }
                let Ok(obj) = serde_json::from_str::<serde_json::Value>(line) else {
                    continue;
                };
                let step_index = obj
                    .get("step_index")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize)
                    .unwrap_or(line_idx);
                let step_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");

                match step_type {
                    "USER_INPUT" | "USER_MESSAGE" => {
                        user_prompt_count += 1;
                    }
                    "CHECKPOINT" | "CONTEXT_COMPACTION" => {
                        compaction_steps.push(step_index);
                    }
                    "PLANNER_RESPONSE" => {
                        planner_count += 1;
                        if let Some(tcs) = obj.get("tool_calls").and_then(|v| v.as_array()) {
                            for tc in tcs {
                                let name = tc
                                    .get("name")
                                    .or_else(|| tc.get("tool_name"))
                                    .and_then(|v| v.as_str())
                                    .map(str::trim)
                                    .unwrap_or("");
                                if name.is_empty() {
                                    continue;
                                }
                                tool_call_count += 1;
                                *tool_counts.entry(name.to_string()).or_insert(0) += 1;

                                match name {
                                    "run_command" => command_execution_count += 1,
                                    "read_file" | "view_file" => file_read_count += 1,
                                    "edit_file" | "write_file" => file_edit_count += 1,
                                    "invoke_subagent" | "swarm" => subagent_calls_in_tr += 1,
                                    _ => {}
                                }

                                if let Some(args) = tc.get("args").or_else(|| tc.get("arguments")) {
                                    for path_key in ["Path", "path", "TargetFile", "file_path"] {
                                        if let Some(p) =
                                            args.get(path_key).and_then(|v| v.as_str())
                                        {
                                            let clean = p.trim();
                                            if !clean.is_empty()
                                                && active_files.len() < 12
                                                && seen_files.insert(clean.to_string())
                                            {
                                                active_files.push(clean.to_string());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }

            if planner_count > assistant_turn_count {
                assistant_turn_count = planner_count;
            }
            if subagent_calls_in_tr > subagent_spawn_count {
                subagent_spawn_count = subagent_calls_in_tr;
            }
        }
    }

    let mut top_tools: Vec<ToolCallSummary> = tool_counts
        .into_iter()
        .map(|(tool_name, count)| ToolCallSummary { tool_name, count })
        .collect();
    top_tools.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.tool_name.cmp(&b.tool_name))
    });
    top_tools.truncate(8);

    let peak_context_percent = if context_limit_tokens > 0 && peak_context_tokens > 0 {
        (((peak_context_tokens as f64 / context_limit_tokens as f64) * 1000.0).round() / 10.0)
            .min(100.0)
    } else {
        0.0
    };

    ConversationContextAudit {
        peak_context_tokens,
        latest_context_tokens,
        context_limit_tokens,
        peak_context_percent,
        user_prompt_count,
        assistant_turn_count,
        tool_call_count,
        command_execution_count,
        file_read_count,
        file_edit_count,
        subagent_spawn_count,
        compaction_count: compaction_steps.len(),
        compaction_steps,
        top_tools,
        active_files,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::antigravity_token_usage::types::{
        DataSourceKind, ExactTokenTotals, TurnEstimatedTotals,
    };

    #[test]
    fn audits_transcript_compaction_tools_and_peak_context() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let conv_id = "conv-audit-1";
        let logs_dir = tmp
            .path()
            .join("brain")
            .join(conv_id)
            .join(".system_generated")
            .join("logs");
        std::fs::create_dir_all(&logs_dir).unwrap();
        let tr_path = logs_dir.join("transcript.jsonl");
        std::fs::write(
            &tr_path,
            concat!(
                r#"{"step_index":0,"type":"USER_INPUT","content":"hello"}"#,
                "\n",
                r#"{"step_index":1,"type":"PLANNER_RESPONSE","tool_calls":[{"name":"read_file","args":{"Path":"/repo/src/main.rs"}},{"name":"run_command","args":{"CommandLine":"cargo test"}}]}"#,
                "\n",
                r#"{"step_index":2,"type":"CHECKPOINT","content":"compacted"}"#,
                "\n",
                r#"{"step_index":3,"type":"PLANNER_RESPONSE","tool_calls":[{"name":"edit_file","args":{"Path":"/repo/src/main.rs"}}]}"#,
                "\n"
            ),
        )
        .unwrap();

        let turns = vec![TurnUsageRecord {
            step_index: 1,
            timestamp: None,
            model_id: "gemini-4-hydrogen-xhigh".to_string(),
            model_display_name: "Gemini 4 Hydrogen (XHigh)".to_string(),
            raw_model_enum: None,
            response_model: None,
            api_provider: "Google Gemini".to_string(),
            is_exact: true,
            source: DataSourceKind::TranscriptJsonl,
            status: 0,
            exact: ExactTokenTotals {
                input_tokens: 28_000,
                output_tokens: 1_000,
                thinking_tokens: 500,
                cache_read_tokens: 100_000,
                cache_write_tokens: 0,
                total_tokens: 129_500,
            },
            estimated: TurnEstimatedTotals::default(),
            estimated_cost_usd: 0.0235,
            cache_savings_usd: 0.045,
            message_id: None,
        }];

        let audit = build_conversation_context_audit(
            tmp.path(),
            conv_id,
            "gemini-4-hydrogen-xhigh",
            &turns,
            0,
        );
        assert_eq!(audit.peak_context_tokens, 128_000);
        assert_eq!(audit.context_limit_tokens, 256_000);
        assert_eq!(audit.peak_context_percent, 50.0);
        assert_eq!(audit.user_prompt_count, 1);
        assert_eq!(audit.compaction_count, 1);
        assert_eq!(audit.compaction_steps, vec![2]);
        assert_eq!(audit.tool_call_count, 3);
        assert_eq!(audit.command_execution_count, 1);
        assert_eq!(audit.file_read_count, 1);
        assert_eq!(audit.file_edit_count, 1);
        assert_eq!(audit.active_files, vec!["/repo/src/main.rs".to_string()]);
    }
}
