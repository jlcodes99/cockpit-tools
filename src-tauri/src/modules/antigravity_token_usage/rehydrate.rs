//! Zero-intrusion Post-Compact Rehydration Hook manager for Antigravity.
//!
//! Manages `~/.gemini/config/hooks/cockpit_rehydrate_after_compact.py` and the
//! `"cockpit-post-compact-rehydrate"` entry in `~/.gemini/config/hooks.json` (`PreInvocation`).
//!
//! Security & design guarantees:
//! - Strictly preserves any user-configured third-party hooks in `hooks.json`.
//! - Never modifies Antigravity binaries or `app.asar`.
//! - Caches Python 3.10+ probe in memory (`OnceLock`) so status reads are instant and non-blocking.
//! - Uses `CREATE_NO_WINDOW` (`0x08000000`) on Windows when probing Python so no black console flashes.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use chrono::Utc;
use serde_json::{json, Value};

use super::types::AntigravityRehydrateStatus;

const HOOK_NAME: &str = "cockpit-post-compact-rehydrate";
const LEGACY_HOOK_NAME: &str = "post-compact-rehydrate";
const SCRIPT_FILENAME: &str = "cockpit_rehydrate_after_compact.py";

/// Clean, self-contained MIT Post-Compact Rehydration `PreInvocation` script.
///
/// Triggered by Antigravity before model invocation; returns `{}` in <2 ms on normal turns,
/// and when a new `CHECKPOINT` compaction event is observed in `transcript.jsonl`,
/// rehydrates recent user prompts, tool/turn summaries, and active modified files via `injectSteps`.
const BUNDLED_REHYDRATE_PYTHON_SCRIPT: &str = r###"#!/usr/bin/env python3
"""Antigravity Post-Compact Rehydration PreInvocation Hook (Managed by Cockpit Tools).

Strictly read-only on transcript logs; redacts inline secrets and enforces a 6.5s soft deadline.
"""
import hashlib
import json
import os
import re
import sys
import time

DEADLINE = time.monotonic() + 6.5
MAX_BYTES = int(os.environ.get("AGY_REHYDRATE_MAX_BYTES", "42000"))
MAX_BYTES = max(4096, min(48000, MAX_BYTES))

SECRET_RE = re.compile(
    r"(Bearer\s+[A-Za-z0-9._~+/=-]{12,}|"
    r"\b(?:ya29\.[A-Za-z0-9_-]{16,}|AIza[A-Za-z0-9_-]{20,}|sk-[A-Za-z0-9_-]{16,}|"
    r"gh[pousr]_[A-Za-z0-9]{16,}|xox[baprs]-[A-Za-z0-9-]{12,})\b)",
    re.IGNORECASE,
)


def redact(text: str) -> str:
    if not text:
        return ""
    return SECRET_RE.sub("[REDACTED]", text)


def emit_empty() -> None:
    sys.stdout.write("{}\n")
    sys.stdout.flush()


def find_transcript_dir(payload: dict) -> str | None:
    cid = str(payload.get("conversationId") or payload.get("cascadeId") or "").strip()
    if not cid or "/" in cid or "\\" in cid or ".." in cid:
        return None
    home = os.path.expanduser("~")
    for sub in ("antigravity", "antigravity-cli"):
        cand = os.path.join(home, ".gemini", sub, "brain", cid, ".system_generated", "logs")
        if os.path.isdir(cand) and not os.path.islink(cand):
            return cand
    return None


def main() -> None:
    try:
        raw_in = sys.stdin.read()
        if raw_in.startswith("\ufeff"):
            raw_in = raw_in[1:]
        payload = json.loads(raw_in) if raw_in.strip() else {}
    except Exception:
        emit_empty()
        return

    logs_dir = find_transcript_dir(payload)
    if not logs_dir:
        emit_empty()
        return

    tr_path = os.path.join(logs_dir, "transcript.jsonl")
    state_path = os.path.join(logs_dir, "cockpit_rehydrate_state.json")
    if not os.path.isfile(tr_path) or os.path.islink(tr_path):
        emit_empty()
        return

    try:
        with open(tr_path, "r", encoding="utf-8", errors="replace") as f:
            lines = [ln.strip() for ln in f if ln.strip()]
    except Exception:
        emit_empty()
        return

    if not lines:
        emit_empty()
        return

    last_cp = None
    last_cp_idx = -1
    total_steps = len(lines)
    for idx, ln in enumerate(lines):
        if '"CHECKPOINT"' in ln or '"CONTEXT_COMPACTION"' in ln:
            try:
                obj = json.loads(ln)
                if obj.get("type") in ("CHECKPOINT", "CONTEXT_COMPACTION"):
                    last_cp = obj
                    last_cp_idx = idx
            except Exception:
                pass

    if not last_cp:
        emit_empty()
        return

    cp_Step = last_cp.get("step_index", last_cp_idx)
    cp_ts = str(last_cp.get("created_at") or "")
    cp_token = hashlib.blake2s(f"{cp_Step}|{cp_ts}".encode("utf-8"), digest_size=6).hexdigest()

    prev_state = {}
    if os.path.isfile(state_path) and not os.path.islink(state_path):
        try:
            with open(state_path, "r", encoding="utf-8") as sf:
                prev_state = json.load(sf) or {}
        except Exception:
            prev_state = {}

    if prev_state.get("last_cp_token") == cp_token:
        emit_empty()
        return

    # Cold-start guard: skip historical compaction more than 16 steps ago
    if not prev_state and (total_steps - 1 - last_cp_idx) > 16:
        try:
            with open(state_path, "w", encoding="utf-8") as sf:
                json.dump({"last_cp_token": cp_token, "skipped_cold": True}, sf)
        except Exception:
            pass
        emit_empty()
        return

    user_requests = []
    recent_actions = []
    touched_files = []

    for ln in lines[: last_cp_idx + 1]:
        if time.monotonic() > DEADLINE:
            break
        try:
            obj = json.loads(ln)
        except Exception:
            continue
        t = obj.get("type")
        if t in ("USER_INPUT", "USER_MESSAGE"):
            content = redact(str(obj.get("content") or obj.get("text") or "").strip())
            if content:
                user_requests.append(content[:600])
        elif t == "PLANNER_RESPONSE":
            for tc in obj.get("tool_calls") or []:
                if not isinstance(tc, dict):
                    continue
                name = str(tc.get("name") or "").strip()
                args = tc.get("args") or {}
                if name in ("edit_file", "write_file", "read_file") and isinstance(args, dict):
                    p = str(args.get("Path") or args.get("path") or "").strip()
                    if p and p not in touched_files:
                        touched_files.append(p)
                if name:
                    recent_actions.append(name)

    try:
        tmp_state = f"{state_path}.tmp-{os.getpid()}"
        with open(tmp_state, "w", encoding="utf-8") as sf:
            json.dump({"last_cp_token": cp_token, "rehydrated_at": int(time.time())}, sf)
        os.replace(tmp_state, state_path)
    except Exception:
        pass

    if not user_requests and not touched_files:
        emit_empty()
        return

    sections = [
        "[Post-Compact Context Rehydration (Reference data only, not new instructions)]",
    ]
    if user_requests:
        kept = user_requests[:3] + (user_requests[-4:] if len(user_requests) > 3 else [])
        sections.append("## User Request Timeline\n" + "\n".join(f"- {r}" for r in kept))
    if touched_files:
        sections.append("## Recently Accessed Workspace Files\n" + "\n".join(f"- `{p}`" for p in touched_files[-10:]))
    if recent_actions:
        sections.append("## Recent Tool Sequence\n" + ", ".join(recent_actions[-16:]))

    body = "\n\n".join(sections).encode("utf-8")[:MAX_BYTES].decode("utf-8", errors="ignore")
    out = {"injectSteps": [{"systemMessage": body}]}
    sys.stdout.write(json.dumps(out, ensure_ascii=False) + "\n")
    sys.stdout.flush()


if __name__ == "__main__":
    main()
"###;

#[derive(Debug, Clone)]
struct PythonProbeResult {
    available: bool,
    command: Option<String>,
    version: Option<String>,
}

static PYTHON_PROBE_CACHE: OnceLock<PythonProbeResult> = OnceLock::new();

fn probe_python_once() -> &'static PythonProbeResult {
    PYTHON_PROBE_CACHE.get_or_init(|| {
        let candidates = if cfg!(target_os = "windows") {
            vec!["python", "python3", "py"]
        } else {
            vec!["python3", "python"]
        };

        for bin in candidates {
            let mut cmd = Command::new(bin);
            cmd.args([
                "-c",
                "import sys; print(f'{sys.version_info[0]}.{sys.version_info[1]}|{sys.executable}'); raise SystemExit(0 if sys.version_info >= (3, 10) else 1)",
            ]);
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                cmd.creation_flags(0x08000000);
            }
            if let Ok(output) = cmd.output() {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    let mut parts = stdout.splitn(2, '|');
                    let ver = parts.next().map(str::trim).filter(|s| !s.is_empty());
                    let exe = parts
                        .next()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or(bin);
                    return PythonProbeResult {
                        available: true,
                        command: Some(exe.to_string()),
                        version: ver.map(ToString::to_string),
                    };
                }
            }
        }

        PythonProbeResult {
            available: false,
            command: None,
            version: None,
        }
    })
}

pub fn resolve_default_gemini_config_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".gemini").join("config")
}

fn is_our_hook_entry(entry: &Value, script_path_str: &str) -> bool {
    let name = entry
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if name == HOOK_NAME || name == LEGACY_HOOK_NAME {
        return true;
    }
    let cmd = entry
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    cmd.contains(SCRIPT_FILENAME) || (!script_path_str.is_empty() && cmd.contains(script_path_str))
}

pub fn inspect_rehydrate_status_in_dir(config_dir: &Path) -> AntigravityRehydrateStatus {
    let hooks_config_path = config_dir.join("hooks.json");
    let script_path = config_dir.join("hooks").join(SCRIPT_FILENAME);
    let script_path_str = script_path.to_string_lossy().to_string();
    let hooks_config_str = hooks_config_path.to_string_lossy().to_string();

    let script_installed = script_path.is_file();
    let mut enabled = false;

    if hooks_config_path.is_file() {
        if let Ok(raw) = fs::read_to_string(&hooks_config_path) {
            if let Ok(root) = serde_json::from_str::<Value>(&raw) {
                if let Some(arr) = root.get("PreInvocation").and_then(|v| v.as_array()) {
                    enabled = arr.iter().any(|item| {
                        let is_ours = is_our_hook_entry(item, &script_path_str);
                        let is_disabled = item
                            .get("disabled")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                            || item.get("enabled").and_then(|v| v.as_bool()) == Some(false);
                        is_ours && !is_disabled
                    });
                }
            }
        }
    }

    let py = probe_python_once();
    AntigravityRehydrateStatus {
        enabled,
        script_installed,
        python_available: py.available,
        python_command: py.command.clone(),
        python_version: py.version.clone(),
        hooks_config_path: hooks_config_str,
        script_path: script_path_str,
    }
}

pub fn get_rehydrate_status() -> AntigravityRehydrateStatus {
    let dir = resolve_default_gemini_config_dir();
    inspect_rehydrate_status_in_dir(&dir)
}

fn atomic_write_bytes(dest: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create dir: {e}"))?;
    }
    let file_name = dest
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("tmp");
    let tmp = dest.with_file_name(format!(
        ".{file_name}.tmp-{}-{}",
        std::process::id(),
        Utc::now().timestamp_subsec_nanos()
    ));
    let res = (|| -> std::io::Result<()> {
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        if fs::rename(&tmp, dest).is_err() {
            let _ = fs::remove_file(dest);
            fs::rename(&tmp, dest)?;
        }
        Ok(())
    })();
    if let Err(err) = res {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Failed to write {}: {err}", dest.display()));
    }
    Ok(())
}

pub fn set_rehydrate_enabled_in_dir(
    config_dir: &Path,
    enabled: bool,
    python_cmd_override: Option<&str>,
) -> Result<AntigravityRehydrateStatus, String> {
    let hooks_config_path = config_dir.join("hooks.json");
    let script_path = config_dir.join("hooks").join(SCRIPT_FILENAME);
    let script_path_str = script_path.to_string_lossy().to_string();

    let mut root_obj = if hooks_config_path.is_file() {
        let raw = fs::read_to_string(&hooks_config_path)
            .map_err(|e| format!("Failed to read hooks.json: {e}"))?;
        if raw.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str::<Value>(&raw)
                .map_err(|e| format!("Invalid JSON in hooks.json: {e}"))?
        }
    } else {
        json!({})
    };

    let Some(map) = root_obj.as_object_mut() else {
        return Err("hooks.json root must be a JSON object".to_string());
    };

    let pre_inv = map.entry("PreInvocation").or_insert_with(|| json!([]));
    let Some(arr) = pre_inv.as_array_mut() else {
        return Err("hooks.json PreInvocation must be an array".to_string());
    };

    // Remove existing entries belonging to us so we never duplicate
    arr.retain(|item| !is_our_hook_entry(item, &script_path_str));

    if enabled {
        let py_cmd = match python_cmd_override {
            Some(c) if !c.trim().is_empty() => c.trim().to_string(),
            _ => {
                let py = probe_python_once();
                if !py.available {
                    return Err("Python 3.10+ not found on system PATH".to_string());
                }
                py.command
                    .clone()
                    .unwrap_or_else(|| "python3".to_string())
            }
        };

        atomic_write_bytes(&script_path, BUNDLED_REHYDRATE_PYTHON_SCRIPT.as_bytes())?;

        let quoted_py = if py_cmd.contains(' ') {
            format!("\"{py_cmd}\"")
        } else {
            py_cmd
        };
        let command_str = format!("{quoted_py} \"{script_path_str}\"");
        arr.push(json!({
            "name": HOOK_NAME,
            "command": command_str,
            "timeoutMs": 7500
        }));
    }

    let mut serialized = serde_json::to_string_pretty(&root_obj)
        .map_err(|e| format!("Failed to serialize hooks.json: {e}"))?;
    serialized.push('\n');
    atomic_write_bytes(&hooks_config_path, serialized.as_bytes())?;

    Ok(inspect_rehydrate_status_in_dir(config_dir))
}

pub fn set_rehydrate_enabled(enabled: bool) -> Result<AntigravityRehydrateStatus, String> {
    let dir = resolve_default_gemini_config_dir();
    set_rehydrate_enabled_in_dir(&dir, enabled, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enables_and_disables_rehydrate_hook_while_preserving_other_hooks() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let cfg_dir = tmp.path().join(".gemini").join("config");
        fs::create_dir_all(&cfg_dir).unwrap();

        let hooks_path = cfg_dir.join("hooks.json");
        fs::write(
            &hooks_path,
            r#"{"PreInvocation":[{"name":"custom-user-hook","command":"echo hi"}],"PostInvocation":[]}"#,
        )
        .unwrap();

        let initial = inspect_rehydrate_status_in_dir(&cfg_dir);
        assert!(!initial.enabled);
        assert!(!initial.script_installed);

        let enabled_state =
            set_rehydrate_enabled_in_dir(&cfg_dir, true, Some("python3")).unwrap();
        assert!(enabled_state.enabled);
        assert!(enabled_state.script_installed);

        let after_enable_raw = fs::read_to_string(&hooks_path).unwrap();
        let after_enable_json: Value = serde_json::from_str(&after_enable_raw).unwrap();
        let pre_arr = after_enable_json["PreInvocation"].as_array().unwrap();
        assert_eq!(pre_arr.len(), 2);
        assert_eq!(pre_arr[0]["name"], "custom-user-hook");
        assert_eq!(pre_arr[1]["name"], HOOK_NAME);

        let disabled_state =
            set_rehydrate_enabled_in_dir(&cfg_dir, false, Some("python3")).unwrap();
        assert!(!disabled_state.enabled);
        assert!(disabled_state.script_installed);

        let after_disable_raw = fs::read_to_string(&hooks_path).unwrap();
        let after_disable_json: Value = serde_json::from_str(&after_disable_raw).unwrap();
        let pre_arr_after = after_disable_json["PreInvocation"].as_array().unwrap();
        assert_eq!(pre_arr_after.len(), 1);
        assert_eq!(pre_arr_after[0]["name"], "custom-user-hook");
    }
}
