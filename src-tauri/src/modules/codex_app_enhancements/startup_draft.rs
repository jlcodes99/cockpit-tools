//! Read-only proof that the native home composer already has a durable draft.
use serde_json::Value;
use std::path::Path;
use std::time::Duration;
use tokio::io::AsyncReadExt;

const LIMIT: u64 = 8 * 1024 * 1024;

fn matches(state: &Value, snapshot: &Value) -> bool {
    if snapshot.get("placement").and_then(Value::as_str) != Some("home") {
        return false;
    }
    let Some(document) = snapshot
        .get("document")
        .filter(|v| v.get("type").and_then(Value::as_str) == Some("doc"))
    else {
        return false;
    };
    let Some(atoms) = state.get("electron-persisted-atom-state") else {
        return false;
    };
    // The home identity is not the last or an arbitrary matching thread. The
    // native client restores this exact identity on its home route.
    let Some(identity) = atoms
        .pointer("/draft-thread-identities-v1/new-conversation")
        .and_then(Value::as_str)
        .filter(|key| key.starts_with("client-new-thread:") && key.len() <= 128)
    else {
        return false;
    };
    let Some(draft) = atoms
        .get("composer-prompt-drafts-v2")
        .and_then(|v| v.get(identity))
    else {
        return false;
    };
    draft.pointer("/prompt/document") == Some(document)
        && draft
            .pointer("/prompt/plainTextMode")
            .and_then(Value::as_bool)
            .is_some()
}

/// No credentials or drafts enter diagnostics. A timeout is distinct from an
/// unsaved draft so callers stop polling rather than pile up slow disk work.
pub(super) async fn persisted(path: &Path, snapshot: &Value) -> Result<bool, ()> {
    if snapshot.to_string().len() > 65536 {
        return Ok(false);
    }
    tokio::time::timeout(Duration::from_secs(1), async {
        let Ok(file) = tokio::fs::File::open(path).await else {
            return Ok(false);
        };
        let mut bytes = Vec::new();
        if file.take(LIMIT + 1).read_to_end(&mut bytes).await.is_err() || bytes.len() as u64 > LIMIT
        {
            return Ok(false);
        }
        let snapshot = snapshot.clone();
        tokio::task::spawn_blocking(move || {
            serde_json::from_slice::<Value>(&bytes)
                .ok()
                .is_some_and(|state| matches(&state, &snapshot))
        })
        .await
        .map_err(|_| ())
    })
    .await
    .map_err(|_| ())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> (Value, Value) {
        let snapshot = json!({"placement":"home","document":{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"unsent draft"}]}]}});
        let state = json!({"electron-persisted-atom-state":{
            "draft-thread-identities-v1":{"new-conversation":"client-new-thread:fixture"},
            "composer-prompt-drafts-v2":{"client-new-thread:fixture":{"prompt":{"document":snapshot["document"],"plainTextMode":false}}}
        }});
        (state, snapshot)
    }
    #[test]
    fn only_exact_native_home_identity_and_document_are_accepted() {
        let (mut state, mut snapshot) = fixture();
        assert!(matches(&state, &snapshot));
        snapshot["document"]["content"][0]["content"][0]["text"] = json!("new unsaved input");
        assert!(!matches(&state, &snapshot));
        let (_, snapshot) = fixture();
        state["electron-persisted-atom-state"]["draft-thread-identities-v1"]["new-conversation"] =
            json!("client-new-thread:other");
        assert!(!matches(&state, &snapshot));
        assert!(!matches(&json!({}), &snapshot));
        let (state, mut snapshot) = fixture();
        snapshot["placement"] = json!("thread");
        assert!(!matches(&state, &snapshot));
    }
    #[tokio::test]
    async fn disk_proof_rejects_corrupt_missing_and_stale_state_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".codex-global-state.json");
        let (state, snapshot) = fixture();
        assert!(!persisted(&path, &snapshot).await.unwrap());
        tokio::fs::write(&path, b"partial").await.unwrap();
        assert!(!persisted(&path, &snapshot).await.unwrap());
        let bytes = serde_json::to_vec(&state).unwrap();
        tokio::fs::write(&path, &bytes).await.unwrap();
        assert!(persisted(&path, &snapshot).await.unwrap());
        assert_eq!(tokio::fs::read(&path).await.unwrap(), bytes);
        tokio::fs::write(&path, vec![b' '; LIMIT as usize + 1])
            .await
            .unwrap();
        assert!(!persisted(&path, &snapshot).await.unwrap());
    }
}
