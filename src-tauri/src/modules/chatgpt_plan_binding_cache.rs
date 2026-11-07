//! The account-list path reads metadata only. Source decryption is deduplicated
//! and batched behind it; a changed or unvalidated snapshot is never merged.
use super::{association_outcome, source_id, storage, ChatgptPlanAccountSummary, Registration};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{LazyLock, Mutex},
};
use tauri::Emitter;

#[derive(Clone)]
pub(super) struct Job {
    key: PathBuf,
    signature: String,
    record: Registration,
}

struct Entry {
    signature: String,
    outcome: Result<(), String>,
}

#[derive(Default)]
struct State {
    entries: HashMap<PathBuf, Entry>,
    pending: HashMap<PathBuf, Job>,
    running: bool,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| Mutex::new(State::default()));

pub(super) fn job(record: &Registration) -> Result<Job, String> {
    let root = crate::modules::account::get_data_dir().map_err(|_| "CHATGPT_PLAN_STORAGE")?;
    let source = record
        .summary
        .source_account_id
        .as_deref()
        .and_then(|id| source_id(id).ok());
    let path = source
        .as_ref()
        .map(|id| root.join("codex_accounts").join(format!("{id}.json")));
    let metadata = path
        .as_ref()
        .and_then(|path| std::fs::symlink_metadata(path).ok())
        .filter(|metadata| metadata.is_file() && !metadata.file_type().is_symlink());
    let stamp = metadata.as_ref().map(|metadata| {
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|time| time.as_nanos());
        let created = metadata
            .created()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|time| time.as_nanos());
        #[cfg(unix)]
        let inode = {
            use std::os::unix::fs::MetadataExt;
            (metadata.dev(), metadata.ino())
        };
        #[cfg(not(unix))]
        let inode = (0u64, 0u64);
        (metadata.len(), modified, created, inode)
    });
    // A deletion tombstone revokes an account even if its detail file has not
    // yet been removed. This reads only the small status record, never tokens.
    let status =
        source.as_ref().map(
            |id| match crate::modules::codex_account::account_storage_status(id) {
                Ok(crate::modules::codex_account::CodexAccountStorageStatus::Present) => "present",
                Ok(crate::modules::codex_account::CodexAccountStorageStatus::Missing) => "missing",
                Ok(crate::modules::codex_account::CodexAccountStorageStatus::Deleted) => "deleted",
                Err(_) => "unknown",
            },
        );
    let binding = record
        .source_binding
        .as_ref()
        .map(|binding| (&binding.source_account_id, &binding.identity_fingerprint));
    let signature = serde_json::json!([
        record.summary.source_account_id,
        binding,
        record.summary.subject,
        record.summary.client_id,
        record.summary.email,
        stamp,
        status,
    ])
    .to_string();
    Ok(Job {
        key: root.join("chatgpt-plan").join(&record.summary.id),
        signature,
        record: record.clone(),
    })
}

fn apply(summary: &mut ChatgptPlanAccountSummary, outcome: &Result<(), String>) {
    summary.source_binding_valid = outcome.is_ok();
    summary.source_binding_error = outcome.as_ref().err().cloned();
}

pub(super) fn cached_summary(
    record: &Registration,
) -> Result<(ChatgptPlanAccountSummary, Option<Job>), String> {
    let mut summary = record.summary.clone();
    summary.source_binding_valid = false;
    summary.source_binding_error = None;
    if record.summary.source_account_id.is_none() && record.source_binding.is_none() {
        return Ok((summary, None));
    }
    let job = job(record)?;
    let state = STATE.lock().map_err(|_| "CHATGPT_PLAN_STORAGE")?;
    if let Some(entry) = state
        .entries
        .get(&job.key)
        .filter(|entry| entry.signature == job.signature)
    {
        apply(&mut summary, &entry.outcome);
        return Ok((summary, None));
    }
    Ok((summary, Some(job)))
}

/// Only publish the validation for the exact association and source-file
/// snapshot inspected before decryption. A concurrent refresh is pending again.
pub(super) fn finish(
    job: Job,
    outcome: Result<(), String>,
) -> Result<(ChatgptPlanAccountSummary, bool), String> {
    let current = storage::load(&job.record.summary.id)?;
    let current_job = self::job(&current)?;
    let mut summary = current.summary;
    if current_job.key != job.key || current_job.signature != job.signature {
        schedule(vec![current_job]);
        return Ok((summary, false));
    }
    apply(&mut summary, &outcome);
    let mut state = STATE.lock().map_err(|_| "CHATGPT_PLAN_STORAGE")?;
    let changed = state
        .entries
        .get(&job.key)
        .is_none_or(|entry| entry.signature != job.signature || entry.outcome != outcome);
    state.entries.insert(
        job.key,
        Entry {
            signature: job.signature,
            outcome,
        },
    );
    Ok((summary, changed))
}

pub(super) fn schedule(jobs: Vec<Job>) {
    if jobs.is_empty() {
        return;
    }
    let start = {
        let Ok(mut state) = STATE.lock() else {
            return;
        };
        for job in jobs {
            state.pending.insert(job.key.clone(), job);
        }
        if state.running {
            false
        } else {
            state.running = true;
            true
        }
    };
    if start {
        tauri::async_runtime::spawn(run());
    }
}

struct WorkerGuard {
    completed: bool,
}
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        if !self.completed {
            if let Ok(mut state) = STATE.lock() {
                state.running = false;
            }
        }
    }
}

async fn run() {
    let mut guard = WorkerGuard { completed: false };
    loop {
        let batch = {
            let Ok(mut state) = STATE.lock() else {
                return;
            };
            if state.pending.is_empty() {
                state.running = false;
                guard.completed = true;
                return;
            }
            let keys = state.pending.keys().take(8).cloned().collect::<Vec<_>>();
            keys.into_iter()
                .filter_map(|key| state.pending.remove(&key))
                .collect::<Vec<_>>()
        };
        let changed = storage::blocking(move || {
            let mut changed = false;
            for job in batch {
                // Association validity is independent of disconnected/expired credentials.
                let outcome = association_outcome(&job.record);
                if let Ok((_, updated)) = finish(job, outcome) {
                    changed |= updated;
                }
            }
            Ok(changed)
        })
        .await
        .unwrap_or(false);
        if changed {
            if let Some(app) = crate::get_app_handle() {
                let _ = app.emit("codex-local-access-state-updated", ());
            }
        }
        tokio::task::yield_now().await;
    }
}

#[cfg(test)]
pub(super) async fn wait_for_idle() {
    for _ in 0..300 {
        if STATE
            .lock()
            .is_ok_and(|state| !state.running && state.pending.is_empty())
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("binding metadata validation did not finish");
}
