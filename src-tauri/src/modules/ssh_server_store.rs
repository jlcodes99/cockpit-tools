//! All SSH selection and status writers share the atomic file path lock.
use crate::models::ssh_server::{SshServer, SshServerList, SshServerStore};
use crate::modules::atomic_write;
use std::collections::HashSet;
use std::path::Path;

fn normalize(store: &mut SshServerStore) {
    store.version = "2".into();
    let mut seen = HashSet::new();
    store
        .selected_server_ids
        .retain(|id| store.servers.iter().any(|s| &s.id == id) && seen.insert(id.clone()));
}

pub(super) fn load(path: &Path) -> Result<SshServerStore, String> {
    if !path.exists() {
        return Ok(SshServerStore::default());
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read SSH servers store: {e}"))?;
    let mut store: SshServerStore = atomic_write::parse_json_with_auto_restore(path, &content)
        .map_err(|e| format!("Failed to parse SSH servers store: {e}"))?;
    normalize(&mut store);
    Ok(store)
}

pub(super) fn update(
    path: &Path,
    mutate: impl FnOnce(&mut SshServerStore) -> Result<(), String>,
) -> Result<SshServerList, String> {
    // Recover a damaged JSON before entering the non-reentrant writer lock.
    load(path)?;
    let mut result = None;
    atomic_write::update_string_atomic(path, |raw| {
        let mut store: SshServerStore = raw
            .map(serde_json::from_str)
            .transpose()
            .map_err(|e| format!("Failed to parse SSH servers store: {e}"))?
            .unwrap_or_default();
        mutate(&mut store)?;
        normalize(&mut store);
        let content = serde_json::to_string_pretty(&store).map_err(|e| e.to_string())?;
        result = Some(store.into());
        Ok(content)
    })?;
    result.ok_or_else(|| "SSH store update did not run".into())
}

fn validate_selection(store: &SshServerStore, ids: &[String]) -> Result<(), String> {
    for id in ids {
        if !store.servers.iter().any(|s| &s.id == id) {
            return Err(format!("SSH server not found: {id}"));
        }
    }
    Ok(())
}

pub(super) fn select(path: &Path, ids: Vec<String>) -> Result<SshServerList, String> {
    update(path, |store| {
        validate_selection(store, &ids)?;
        store.selected_server_ids = ids;
        Ok(())
    })
}

pub(super) fn set_selected(path: &Path, id: &str, selected: bool) -> Result<SshServerList, String> {
    update(path, |store| {
        validate_selection(store, &[id.to_string()])?;
        store.selected_server_ids.retain(|item| item != id);
        if selected {
            store.selected_server_ids.push(id.to_string());
        }
        Ok(())
    })
}

pub(super) fn switch_targets(store: &SshServerStore) -> Vec<SshServer> {
    store
        .servers
        .iter()
        .filter(|server| {
            server.sync_on_codex_switch && store.selected_server_ids.contains(&server.id)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
#[path = "ssh_server_store_tests.rs"]
mod tests;
