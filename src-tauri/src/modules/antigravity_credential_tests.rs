use super::write_antigravity_keychain_payload;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::Duration;

struct Fixture(PathBuf);

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

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
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
