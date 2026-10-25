#[cfg(target_os = "macos")]
use super::write_antigravity_keychain_payload;
use super::{build_antigravity_credential_payload, write_antigravity_credential_payload};
#[cfg(target_os = "macos")]
use std::os::unix::fs::PermissionsExt;
#[cfg(target_os = "macos")]
use std::path::PathBuf;
#[cfg(target_os = "macos")]
use std::time::Duration;

#[cfg(target_os = "macos")]
struct Fixture(PathBuf);

#[cfg(target_os = "macos")]
impl Fixture {
    fn new(script: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "antigravity-keychain-write-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("credential"), "old-credential").unwrap();
        std::fs::write(dir.join("security"), script).unwrap();
        std::fs::set_permissions(dir.join("security"), std::fs::Permissions::from_mode(0o700))
            .unwrap();
        Self(dir)
    }

    fn old_credential(&self) -> String {
        std::fs::read_to_string(self.0.join("credential")).unwrap()
    }
}

#[cfg(target_os = "macos")]
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
#[cfg(target_os = "macos")]
fn failed_keychain_upsert_does_not_delete_existing_credential() {
    let fixture = Fixture::new(
        r#"#!/bin/sh
fixture_dir="${0%/*}"
case "$1" in
  delete-generic-password) /bin/rm -f "$fixture_dir/credential"; exit 0 ;;
  add-generic-password) echo 'injected write failure' >&2; exit 1 ;;
  *) exit 2 ;;
esac
"#,
    );
    // Inject only the fixture executable; never invoke the real Keychain or change PATH.
    let error = write_antigravity_keychain_payload(
        "fixture-credential",
        fixture.0.join("security").as_os_str(),
        Duration::from_secs(2),
    )
    .unwrap_err();
    assert!(error.contains("injected write failure"));
    assert_eq!(fixture.old_credential(), "old-credential");
}

#[test]
#[cfg(target_os = "macos")]
fn repeated_keychain_updates_preserve_authorized_access_without_resetting_acl() {
    use base64::{engine::general_purpose::STANDARD, Engine as _};

    let fixture = Fixture::new(
        r#"#!/bin/sh
fixture_dir="${0%/*}"
[ "$1" = 'add-generic-password' ] || exit 2
shift
while [ "$#" -gt 0 ]; do
  case "$1" in
    -A|-T) echo 'access-policy reset would prompt for authorization' >&2; exit 3 ;;
    -U) shift ;;
    -s|-a) shift; shift ;;
    -w) shift; printf '%s' "$1" > "$fixture_dir/credential"; shift ;;
    *) exit 4 ;;
  esac
done
"#,
    );
    for payload in ["account-one", "account-two", "account-one"] {
        write_antigravity_keychain_payload(
            payload,
            fixture.0.join("security").as_os_str(),
            Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(
            fixture.old_credential(),
            format!("go-keyring-base64:{}", STANDARD.encode(payload))
        );
    }
}

#[test]
#[cfg(target_os = "macos")]
fn keychain_timeout_kills_writer_before_late_credential_overwrite() {
    let fixture = Fixture::new(
        r#"#!/bin/sh
/bin/sleep 0.5
printf 'late-credential' > "${0%/*}/credential"
"#,
    );
    let start = std::time::Instant::now();
    let error = write_antigravity_keychain_payload(
        "fixture-credential",
        fixture.0.join("security").as_os_str(),
        Duration::from_millis(100),
    )
    .unwrap_err();
    assert!(error.contains("timed out"));
    assert!(start.elapsed() < Duration::from_secs(2));
    // Wait beyond the fixture write delay to catch a dropped-but-still-running writer.
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(fixture.old_credential(), "old-credential");
}

fn account_with_id_token(id_token: Option<&str>) -> crate::models::Account {
    let mut token = crate::models::TokenData::new(
        "fixture-access".into(),
        "fixture-refresh".into(),
        3600,
        Some("fixture@example.com".into()),
        None,
        None,
    );
    token.expiry_timestamp = 1_800_000_000;
    token.id_token = id_token.map(str::to_owned);
    crate::models::Account::new(
        "fixture-account".into(),
        "fixture@example.com".into(),
        token,
    )
}

#[test]
fn standalone_payload_preserves_top_level_id_token_and_native_token_shape() {
    let payload =
        build_antigravity_credential_payload(&account_with_id_token(Some("fixture-id"))).unwrap();
    let value: serde_json::Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(value["auth_method"], "consumer");
    assert_eq!(value["id_token"], "fixture-id");
    assert_eq!(value["token"]["access_token"], "fixture-access");
    assert_eq!(value["token"]["refresh_token"], "fixture-refresh");
    assert_eq!(value["token"]["token_type"], "Bearer");
    let expiry =
        chrono::DateTime::parse_from_rfc3339(value["token"]["expiry"].as_str().unwrap()).unwrap();
    assert_eq!(expiry.timestamp(), 1_800_000_000);

    let without_id = build_antigravity_credential_payload(&account_with_id_token(None)).unwrap();
    let without_id: serde_json::Value = serde_json::from_str(&without_id).unwrap();
    assert!(without_id.get("id_token").is_none());
}

#[test]
fn standalone_fallback_replaces_old_account_with_the_same_system_payload() {
    let dir = tempfile::tempdir().unwrap();
    let fallback = dir.path().join(".gemini/jetski-standalone-oauth-token");
    std::fs::create_dir_all(fallback.parent().unwrap()).unwrap();
    std::fs::write(&fallback, "old-account").unwrap();
    let unrelated_cli = dir.path().join(".gemini/oauth_creds.json");
    std::fs::write(&unrelated_cli, "unrelated-cli").unwrap();
    let payload =
        build_antigravity_credential_payload(&account_with_id_token(Some("fixture-id"))).unwrap();
    let mut system_payload = String::new();
    write_antigravity_credential_payload(&payload, &fallback, |value| {
        system_payload = value.to_string();
        Ok(())
    })
    .unwrap();
    assert_eq!(std::fs::read_to_string(&fallback).unwrap(), system_payload);
    assert_eq!(system_payload, payload);
    assert_eq!(
        std::fs::read_to_string(&unrelated_cli).unwrap(),
        "unrelated-cli"
    );
    assert!(!fallback.with_extension("bak").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&fallback).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn system_write_failure_keeps_the_previous_standalone_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let fallback = dir.path().join("jetski-standalone-oauth-token");
    std::fs::write(&fallback, "old-account").unwrap();
    let error = write_antigravity_credential_payload("new-account", &fallback, |_| {
        Err("system write failed".into())
    })
    .unwrap_err();
    assert_eq!(error, "system write failed");
    assert_eq!(std::fs::read_to_string(&fallback).unwrap(), "old-account");
}

#[test]
fn fallback_write_failure_reports_partial_switch_instead_of_success() {
    let dir = tempfile::tempdir().unwrap();
    let fallback = dir.path().join("jetski-standalone-oauth-token");
    std::fs::create_dir(&fallback).unwrap();
    let mut system_written = false;
    let error = write_antigravity_credential_payload("fixture-secret", &fallback, |_| {
        system_written = true;
        Ok(())
    })
    .unwrap_err();
    assert!(system_written);
    assert!(error.contains("系统凭据已写入"));
    assert!(error.contains("请重试切号"));
    assert!(!error.contains("fixture-secret"));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
