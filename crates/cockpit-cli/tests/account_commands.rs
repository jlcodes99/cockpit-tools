use cockpit_core::models::codex::{CodexAccount, CodexApiProviderMode};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cockpit-cli"))
        .args(args)
        .env("HOME", root)
        .env("APPDATA", root)
        .env("XDG_DATA_HOME", root.join("local-data"))
        .env("CODEX_HOME", root.join("codex"))
        .env("COCKPIT_TOOLS_DATA_DIR", root.join("data"))
        .env_remove("COCKPIT_TOOLS_TEST_DATA_DIR")
        .env_remove("COCKPIT_TEST_DATA_DIR")
        .output()
        .unwrap()
}

fn write_json(path: &Path, value: Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}

fn success_json(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn codex_lists_and_switches_without_returning_credentials_in_json() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let email = "shared@example.test";
    let data = root.join("data");
    let mut summaries = Vec::new();
    for (id, secret) in [("one", "sk-fixture-one"), ("two", "sk-fixture-two")] {
        let account = CodexAccount::new_api_key(
            id.into(),
            email.into(),
            secret.into(),
            CodexApiProviderMode::OpenaiBuiltin,
            None,
            None,
            None,
        );
        write_json(
            &data.join("codex_accounts").join(format!("{id}.json")),
            serde_json::to_value(&account).unwrap(),
        );
        summaries.push(
            json!({"id":id,"email":email,"plan_type":"API_KEY","created_at":1,"last_used":1}),
        );
    }
    write_json(
        &data.join("codex_accounts.json"),
        json!({"version":"1.0","accounts":summaries}),
    );

    let listed = success_json(run(root, &["list", "codex", "--json"]));
    assert_eq!(listed.as_array().unwrap().len(), 2);
    assert!(!listed.to_string().contains("sk-fixture"));
    let ambiguous = run(root, &["switch", "codex", email, "--json"]);
    assert!(!ambiguous.status.success());
    assert!(ambiguous.stdout.is_empty());
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("More than one account"));
    assert!(!root.join("codex/auth.json").exists());
    assert_eq!(
        success_json(run(root, &["--json", "switch", "codex", "one"])),
        json!({"platform":"codex","accountId":"one"})
    );
    let saved: Value =
        serde_json::from_slice(&fs::read(root.join("codex/auth.json")).unwrap()).unwrap();
    assert_eq!(saved["OPENAI_API_KEY"], "sk-fixture-one");
}

fn cursor_db(root: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    let profile = root.join("Library/Application Support/Cursor");
    #[cfg(target_os = "linux")]
    let profile = root.join(".config/Cursor");
    #[cfg(target_os = "windows")]
    let profile = root.join("Cursor");
    profile.join("User/globalStorage/state.vscdb")
}

#[test]
fn cursor_switch_uses_unique_email_and_rolls_back_failed_writes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let data = root.join("data");
    let account = json!({
        "id":"cursor-fixture", "email":"cursor@example.test", "created_at":1, "last_used":1,
        "access_token":"fixture-cursor-token"
    });
    write_json(
        &data.join("cursor_accounts/cursor-fixture.json"),
        account.clone(),
    );
    write_json(
        &data.join("cursor_accounts.json"),
        json!({"version":"1.0","accounts":[account]}),
    );
    let state_db = cursor_db(root);
    fs::create_dir_all(state_db.parent().unwrap()).unwrap();
    let database = Connection::open(state_db).unwrap();
    database
        .execute_batch(
            "CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         INSERT INTO ItemTable VALUES ('cursorAuth/accessToken', 'old-token');
         INSERT INTO ItemTable VALUES ('cursorAuth/refreshToken', 'old-refresh');
         CREATE TRIGGER reject_email BEFORE INSERT ON ItemTable
         WHEN NEW.key = 'cursor.email' BEGIN SELECT RAISE(ABORT, 'fixture rejection'); END;",
        )
        .unwrap();

    let args = ["switch", "cursor", "CURSOR@example.test", "--json"];
    let failure = run(root, &args);
    assert!(!failure.status.success());
    assert!(failure.stdout.is_empty());
    assert!(String::from_utf8_lossy(&failure.stderr).contains("fixture rejection"));
    let token = |key: &str| {
        database.query_row("SELECT value FROM ItemTable WHERE key=?1", [key], |row| {
            row.get::<_, String>(0)
        })
    };
    assert_eq!(token("cursorAuth/accessToken").unwrap(), "old-token");
    assert_eq!(token("cursorAuth/refreshToken").unwrap(), "old-refresh");

    database.execute("DROP TRIGGER reject_email", []).unwrap();
    assert_eq!(
        success_json(run(root, &args)),
        json!({"platform":"cursor","accountId":"cursor-fixture"})
    );
    assert_eq!(
        token("cursorAuth/accessToken").unwrap(),
        "fixture-cursor-token"
    );
    assert!(matches!(
        token("cursorAuth/refreshToken"),
        Err(rusqlite::Error::QueryReturnedNoRows)
    ));
}

#[test]
fn invalid_platform_and_unimplemented_operations_fail_on_stderr() {
    let directory = tempfile::tempdir().unwrap();
    for args in [
        ["list", "unknown", "--json"].as_slice(),
        ["switch", "github_copilot", "fixture", "--json"].as_slice(),
        ["quota", "cursor", "--json"].as_slice(),
    ] {
        let output = run(directory.path(), args);
        assert!(!output.status.success(), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(!output.stderr.is_empty(), "{args:?}");
    }
}
