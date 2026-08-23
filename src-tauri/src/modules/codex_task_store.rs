use std::path::PathBuf;

pub use ::codex_task_supervisor::CodexTaskStore;

const SUPERVISOR_DB_FILE: &str = "codex_task_supervisor.sqlite3";

pub fn default_codex_task_store() -> Result<CodexTaskStore, String> {
    Ok(CodexTaskStore::new(
        crate::modules::account::get_data_dir()?.join(SUPERVISOR_DB_FILE),
    ))
}

pub fn managed_tasks_root() -> Result<PathBuf, String> {
    Ok(crate::modules::account::get_data_dir()?.join("codex-managed-tasks"))
}
