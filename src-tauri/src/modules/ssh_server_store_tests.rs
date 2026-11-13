use super::*;
use crate::models::ssh_server::SshAuthConfig;
use serde_json::json;

fn server(id: &str) -> SshServer {
    SshServer {
        id: id.into(),
        name: id.into(),
        host: "fixture.invalid".into(),
        port: 22,
        username: "fixture".into(),
        codex_home: "~/.codex".into(),
        auth: SshAuthConfig::Agent,
        sync_on_codex_switch: true,
        created_at: 1,
        updated_at: 1,
        last_sync: None,
    }
}

fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ssh.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&json!({"version":"1", "selected_server_id":"a",
        "servers":[server("a"),server("b"),server("c")]}))
        .unwrap(),
    )
    .unwrap();
    (temp, path)
}

#[test]
fn selecting_a_second_host_retains_the_first_and_deselecting_removes_only_one() {
    let (_temp, path) = fixture();
    let two = set_selected(&path, "b", true).unwrap();
    assert_eq!(two.selected_server_ids, ["a", "b"]);
    assert_eq!(
        switch_targets(&load(&path).unwrap())
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    set_selected(&path, "a", false).unwrap();
    assert_eq!(load(&path).unwrap().selected_server_ids, ["b"]);
    let bytes = std::fs::read(&path).unwrap();
    assert!(set_selected(&path, "unknown", true).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    select(&path, Vec::new()).unwrap();
    assert!(load(&path).unwrap().selected_server_ids.is_empty());
}

#[test]
fn deletion_disabled_sync_and_duplicate_ids_do_not_expand_the_target_set() {
    let (_temp, path) = fixture();
    select(&path, vec!["a".into(), "b".into(), "b".into(), "c".into()]).unwrap();
    update(&path, |store| {
        store.servers.retain(|s| s.id != "a");
        store
            .servers
            .iter_mut()
            .find(|s| s.id == "c")
            .unwrap()
            .sync_on_codex_switch = false;
        Ok(())
    })
    .unwrap();
    let store = load(&path).unwrap();
    assert_eq!(store.selected_server_ids, ["b", "c"]);
    assert_eq!(
        switch_targets(&store)
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        ["b"]
    );
}

#[test]
fn concurrent_selections_and_status_updates_preserve_each_other() {
    let (_temp, path) = fixture();
    select(&path, Vec::new()).unwrap();
    std::thread::scope(|scope| {
        for id in ["a", "b", "c"] {
            let path = &path;
            scope.spawn(move || {
                set_selected(path, id, true).unwrap();
            });
        }
        scope.spawn(|| {
            update(&path, |store| {
                store.servers[0].updated_at = 42;
                Ok(())
            })
            .unwrap();
        });
    });
    let store = load(&path).unwrap();
    assert_eq!(store.selected_server_ids.len(), 3);
    assert_eq!(store.servers[0].updated_at, 42);
}
