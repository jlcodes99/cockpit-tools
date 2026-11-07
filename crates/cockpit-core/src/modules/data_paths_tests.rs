use super::*;
use std::sync::{Arc, Barrier};

struct TestRoot(PathBuf);

impl TestRoot {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("cockpit-data-path-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn legacy(&self) -> PathBuf {
        self.0.join(LEGACY_DATA_DIR)
    }
    fn current(&self) -> PathBuf {
        self.0.join(DATA_DIR)
    }

    fn seed_legacy(&self) -> PathBuf {
        let legacy = self.legacy();
        fs::create_dir_all(legacy.join("instances/codex/profile")).unwrap();
        fs::write(legacy.join("accounts.json"), "original accounts").unwrap();
        legacy
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn fresh_install_selects_new_root_without_creating_or_scanning() {
    let root = TestRoot::new();
    let selected = select_root(&root.0, false).unwrap();
    assert_eq!(selected.path, root.current());
    assert!(selected.compatibility.is_none());
    assert!(!root.current().exists());
    assert!(!root.legacy().exists());
}

#[test]
fn upgrade_keeps_existing_accounts_and_requests_background_alias() {
    let root = TestRoot::new();
    let legacy = root.seed_legacy();
    let selected = select_root(&root.0, false).unwrap();
    assert_eq!(selected.path, legacy);
    assert_eq!(
        selected.compatibility,
        Some((root.current(), legacy.clone()))
    );
    assert_eq!(
        fs::read_to_string(legacy.join("accounts.json")).unwrap(),
        "original accounts"
    );
    assert!(!root.current().exists());
}

#[test]
fn compatibility_is_idempotent_and_both_paths_write_the_same_accounts() {
    let root = TestRoot::new();
    let legacy = root.seed_legacy();
    let before = fs::canonicalize(legacy.join("instances/codex/profile")).unwrap();
    ensure_compatibility_link(&root.current(), &legacy).unwrap();
    ensure_compatibility_link(&root.current(), &legacy).unwrap();
    let selected = select_root(&root.0, false).unwrap();
    assert_eq!(selected.path, root.current());
    assert!(selected.compatibility.is_none());
    assert_eq!(
        fs::read_to_string(root.current().join("accounts.json")).unwrap(),
        "original accounts"
    );
    fs::write(
        root.current().join("accounts.json"),
        "updated from new path",
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(legacy.join("accounts.json")).unwrap(),
        "updated from new path"
    );
    fs::write(legacy.join("accounts.json"), "updated from old path").unwrap();
    assert_eq!(
        fs::read_to_string(root.current().join("accounts.json")).unwrap(),
        "updated from old path"
    );
    let after = fs::canonicalize(root.current().join("instances/codex/profile")).unwrap();
    assert_eq!(
        before, after,
        "Codex app-data canonical identity must remain stable"
    );
    assert_eq!(
        md5::compute(before.to_string_lossy().as_bytes()),
        md5::compute(after.to_string_lossy().as_bytes())
    );
}

#[test]
fn two_real_directories_never_merge_or_overwrite() {
    let root = TestRoot::new();
    let legacy = root.seed_legacy();
    fs::create_dir(root.current()).unwrap();
    fs::write(
        root.current().join("accounts.json"),
        "independent new accounts",
    )
    .unwrap();
    assert_eq!(select_root(&root.0, false).unwrap().path, legacy);
    assert!(ensure_compatibility_link(&root.current(), &legacy).is_err());
    assert_eq!(
        fs::read_to_string(root.current().join("accounts.json")).unwrap(),
        "independent new accounts"
    );
    assert_eq!(
        fs::read_to_string(legacy.join("accounts.json")).unwrap(),
        "original accounts"
    );
}

#[test]
fn alias_creation_failure_keeps_old_data_and_can_retry() {
    let root = TestRoot::new();
    let legacy = root.seed_legacy();
    let result = ensure_compatibility_link_with(&root.current(), &legacy, |_, _| {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "injected link failure",
        ))
    });
    assert!(result.is_err());
    assert_eq!(select_root(&root.0, false).unwrap().path, legacy);
    assert_eq!(
        fs::read_to_string(legacy.join("accounts.json")).unwrap(),
        "original accounts"
    );
    assert!(!root.current().exists());
    ensure_compatibility_link(&root.current(), &legacy).unwrap();
    assert_eq!(select_root(&root.0, false).unwrap().path, root.current());
}

#[test]
fn competing_process_creating_the_same_alias_is_success() {
    let root = TestRoot::new();
    let legacy = root.seed_legacy();
    ensure_compatibility_link_with(&root.current(), &legacy, |target, alias| {
        create_directory_link(target, alias).unwrap();
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "another process won",
        ))
    })
    .unwrap();
    assert!(same_directory(&root.current(), &legacy));
}

#[test]
fn competing_process_with_independent_data_is_never_overwritten() {
    let root = TestRoot::new();
    let legacy = root.seed_legacy();
    assert!(
        ensure_compatibility_link_with(&root.current(), &legacy, |_, alias| {
            fs::create_dir(alias).unwrap();
            fs::write(alias.join("accounts.json"), "concurrent independent data").unwrap();
            Err(io::Error::new(io::ErrorKind::AlreadyExists, "occupied"))
        })
        .is_err()
    );
    assert_eq!(
        fs::read_to_string(root.current().join("accounts.json")).unwrap(),
        "concurrent independent data"
    );
    assert_eq!(
        fs::read_to_string(legacy.join("accounts.json")).unwrap(),
        "original accounts"
    );
}

#[test]
fn development_and_production_roots_remain_separate() {
    let root = TestRoot::new();
    root.seed_legacy();
    assert_eq!(
        select_root(&root.0, true).unwrap().path,
        root.0.join(DEV_DATA_DIR)
    );
    let dev_legacy = root.0.join(LEGACY_DEV_DATA_DIR);
    fs::create_dir(&dev_legacy).unwrap();
    ensure_compatibility_link(&root.0.join(DEV_DATA_DIR), &dev_legacy).unwrap();
    assert_eq!(
        select_root(&root.0, true).unwrap().path,
        root.0.join(DEV_DATA_DIR)
    );
    assert_eq!(select_root(&root.0, false).unwrap().path, root.legacy());
    assert!(!same_directory(&root.legacy(), &root.0.join(DEV_DATA_DIR)));
}

#[test]
fn home_and_windows_roaming_compatibility_are_independent() {
    let root = TestRoot::new();
    let roaming = root.0.join("Roaming");
    fs::create_dir_all(roaming.join(LEGACY_DATA_DIR).join("instances/cursor")).unwrap();
    root.seed_legacy();
    ensure_compatibility_link(&root.current(), &root.legacy()).unwrap();
    assert_eq!(
        select_root(&roaming, false).unwrap().path,
        roaming.join(LEGACY_DATA_DIR)
    );
    ensure_compatibility_link(&roaming.join(DATA_DIR), &roaming.join(LEGACY_DATA_DIR)).unwrap();
    assert!(roaming.join(DATA_DIR).join("instances/cursor").is_dir());
    assert!(!root.current().join("instances/cursor").exists());
}

#[test]
fn concurrent_background_requests_are_single_flight() {
    let root = TestRoot::new();
    let alias = Arc::new(root.current());
    let barrier = Arc::new(Barrier::new(12));
    let threads: Vec<_> = (0..12)
        .map(|_| {
            let alias = alias.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                claim_compatibility_task(&alias)
            })
        })
        .collect();
    let count = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .filter(|claimed| *claimed)
        .count();
    assert_eq!(count, 1);
    assert!(!claim_compatibility_task(&alias));
}

#[test]
fn cache_keeps_current_session_root_after_background_completion() {
    let root = TestRoot::new();
    let legacy = root.seed_legacy();
    let cache = OnceLock::new();
    assert_eq!(resolve_cached_root(&cache, &root.0, false).unwrap(), legacy);
    // Model another process/task finishing while this session has open paths.
    ensure_compatibility_link(&root.current(), &legacy).unwrap();
    assert_eq!(resolve_cached_root(&cache, &root.0, false).unwrap(), legacy);
    assert_eq!(select_root(&root.0, false).unwrap().path, root.current());
}

#[test]
fn initial_filesystem_failure_can_recover_without_restarting() {
    let root = TestRoot::new();
    let cache = OnceLock::new();
    fs::write(root.current(), "temporarily unavailable directory").unwrap();
    assert!(resolve_cached_root(&cache, &root.0, false).is_err());
    assert!(cache.get().is_none());
    fs::remove_file(root.current()).unwrap();
    assert_eq!(
        resolve_cached_root(&cache, &root.0, false).unwrap(),
        root.current()
    );
}

#[cfg(unix)]
#[test]
fn dangling_new_alias_is_not_treated_as_a_fresh_install() {
    let root = TestRoot::new();
    create_directory_link(&root.0.join("missing"), &root.current()).unwrap();
    assert!(select_root(&root.0, false).is_err());
    let legacy = root.seed_legacy();
    assert_eq!(select_root(&root.0, false).unwrap().path, legacy);
    assert!(ensure_compatibility_link(&root.current(), &legacy).is_err());
    assert_eq!(
        fs::read_link(root.current()).unwrap(),
        root.0.join("missing")
    );
}

#[test]
fn alias_to_a_different_store_is_preserved() {
    let root = TestRoot::new();
    let other = root.0.join("other");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("accounts.json"), "other accounts").unwrap();
    create_directory_link(&other, &root.current()).unwrap();
    let legacy = root.seed_legacy();
    assert_eq!(select_root(&root.0, false).unwrap().path, legacy);
    assert!(ensure_compatibility_link(&root.current(), &legacy).is_err());
    assert_eq!(
        fs::read_to_string(root.current().join("accounts.json")).unwrap(),
        "other accounts"
    );
}

#[test]
fn inaccessible_or_invalid_roots_return_errors_instead_of_empty_accounts() {
    let root = TestRoot::new();
    fs::write(root.current(), "occupied file").unwrap();
    assert!(select_root(&root.0, false).is_err());
    assert!(select_root(&root.current(), false).is_err());
    fs::write(root.legacy(), "invalid legacy file").unwrap();
    assert!(select_root(&root.0, false).is_err());
}

#[test]
fn custom_override_is_respected_without_touching_home_or_roaming() {
    let root = TestRoot::new();
    for profile in ["dev", "production"] {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let output = command
            .args([
                "--exact",
                "modules::data_paths::tests::environment_override_child",
            ])
            .env("COCKPIT_DATA_PATH_TEST_CHILD", &root.0)
            .env(
                "COCKPIT_TOOLS_DATA_DIR",
                format!("  {}  ", root.0.join("custom").display()),
            )
            .env("COCKPIT_TOOLS_PROFILE", profile)
            .env("APPDATA", root.0.join("Roaming"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    }
    assert!(!root.0.join("custom").exists());
    assert!(!root.0.join("Roaming").exists());
}

#[test]
fn environment_override_child() {
    let Some(root) = std::env::var_os("COCKPIT_DATA_PATH_TEST_CHILD") else {
        return;
    };
    let root = PathBuf::from(root);
    let expected = root.join("custom");
    assert_eq!(resolve_data_dir().unwrap(), expected);
    assert_eq!(fallback_data_dir(), expected);
    assert_eq!(
        managed_instances_root_dir("cursor").unwrap(),
        expected.join("instances/cursor")
    );
    assert!(!expected.exists());
    assert!(!root.join("Roaming").exists());
}
