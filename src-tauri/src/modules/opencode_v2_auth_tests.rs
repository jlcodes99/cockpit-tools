use super::*;

fn legacy(refresh: &str) -> Value {
    json!({"type":"oauth","refresh":refresh,"access":"test-access","expires":4102444800000_i64,
        "accountId":"test-workspace"})
}

#[test]
fn import_uses_the_official_legacy_method_and_a_stable_id_for_each_grant() {
    let (id, body) = import_entry("account", legacy("refresh-first")).unwrap();
    let data: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(data[0]["id"], id);
    assert_eq!(data[0]["integrationID"], "openai");
    assert_eq!(data[0]["active"], false);
    assert_eq!(
        data[0]["value"],
        json!({
            "type":"oauth","methodID":"chatgpt-browser","refresh":"refresh-first","access":"test-access",
            "expires":4102444800000_i64,"metadata":{"accountID":"test-workspace"}
        })
    );
    assert_eq!(
        import_entry("account", legacy("refresh-first")).unwrap().0,
        id
    );
    assert_ne!(
        import_entry("account", legacy("refresh-next")).unwrap().0,
        id
    );
    assert_ne!(
        import_entry("another-account", legacy("refresh-first"))
            .unwrap()
            .0,
        id
    );
    assert!(managed_id(&id));
    assert!(!managed_id("cred_manual"));
    assert_eq!(major_version("2.0.20\n"), Some(2));
    assert_eq!(major_version("opencode v2.0.20"), Some(2));
    assert_eq!(major_version("OpenCode is starting"), None);
}

#[cfg(unix)]
#[test]
fn v2_switch_uses_stdin_keeps_manual_credentials_and_retries_without_duplicates() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let cli = temp.path().join("opencode");
    fs::write(&cli, r#"#!/usr/bin/env python3
import json, pathlib, sys
action = sys.argv[2]
path = pathlib.Path('credential-fixture.json')
state = json.loads(path.read_text()) if path.exists() else {
    'active': 'cred_manual',
    'items': {'cred_manual': {'id': 'cred_manual'}}
}
if action == 'import':
    for entry in json.load(sys.stdin):
        assert entry['integrationID'] == 'openai' and entry['value']['methodID'] == 'chatgpt-browser'
        state['items'].setdefault(entry['id'], entry)
elif action == 'switch':
    assert sys.argv[3] == 'openai'
    if pathlib.Path('fail-switch').exists():
        print('fixture secret that must not appear in the error', file=sys.stderr)
        sys.exit(9)
    assert sys.argv[4] in state['items']
    state['active'] = sys.argv[4]
elif action == 'list':
    assert sys.argv[3:] == ['--format', 'json']
    ids = sorted(state['items'], key=lambda value: value != state['active'])
    print(json.dumps([{'id':'openai','connections':[{'type':'credential','id':v} for v in ids]}]))
elif action == 'logout':
    assert sys.argv[3] == 'openai'
    assert sys.argv[4] != state['active'] and sys.argv[4] != 'cred_manual'
    del state['items'][sys.argv[4]]
else:
    sys.exit(2)
path.write_text(json.dumps(state))
"#).unwrap();
    fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).unwrap();
    let state = || -> Value {
        serde_json::from_slice(&fs::read(temp.path().join("credential-fixture.json")).unwrap())
            .unwrap()
    };
    let selected = |refresh: &str| import_entry("account", legacy(refresh)).unwrap().0;
    assert!(sync(
        &cli,
        temp.path(),
        "account",
        legacy("refresh-first"),
        || true
    )
    .unwrap());
    assert_eq!(state()["active"], selected("refresh-first"));
    assert_eq!(state()["items"].as_object().unwrap().len(), 2);

    fs::write(temp.path().join("fail-switch"), "").unwrap();
    let error = sync(&cli, temp.path(), "account", legacy("refresh-next"), || {
        true
    })
    .unwrap_err();
    assert!(error.contains("auth switch"));
    assert!(!error.contains("fixture secret"));
    assert_eq!(state()["active"], selected("refresh-first"));

    fs::remove_file(temp.path().join("fail-switch")).unwrap();
    let checks = std::cell::Cell::new(0);
    assert!(
        !sync(&cli, temp.path(), "account", legacy("superseded"), || {
            checks.set(checks.get() + 1);
            checks.get() == 1
        })
        .unwrap()
    );
    assert_eq!(state()["active"], selected("refresh-first"));
    for _ in 0..2 {
        assert!(
            sync(&cli, temp.path(), "account", legacy("refresh-next"), || {
                true
            })
            .unwrap()
        );
    }
    let result = state();
    assert_eq!(result["active"], selected("refresh-next"));
    assert_eq!(result["items"].as_object().unwrap().len(), 2);
    assert_eq!(result["items"]["cred_manual"]["id"], "cred_manual");
}
