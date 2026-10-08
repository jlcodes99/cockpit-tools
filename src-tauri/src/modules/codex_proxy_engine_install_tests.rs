use super::*;

#[test]
fn installer_active_phases_block_preflight_and_terminal_phases_allow_rechecking() {
    for phase in ["downloading", "importing", "verifying", "extracting", "checking", "installing"] {
        assert_eq!(check_preflight_phase(Some(phase)).unwrap_err(), "ENGINE_INSTALL_BUSY");
    }
    for phase in [None, Some("idle"), Some("completed"), Some("failed"), Some("cancelled")] {
        assert!(check_preflight_phase(phase).is_ok());
    }
}

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("cockpit-engine-install-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(unix)]
fn version_fixture(temp: &Temp, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = temp.0.join("version-fixture");
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path
}

#[cfg(unix)]
#[tokio::test]
async fn cold_start_budget_and_failure_codes_are_distinct_from_archive_verification() {
    let temp = Temp::new();
    let control = InstallControl::for_test();
    let binary = version_fixture(&temp, "sleep 0.15\nprintf 'Mihomo Meta v1.19.31\\n'");
    assert_eq!(
        check_first_launch(&binary, &control, Duration::from_millis(20))
            .await
            .unwrap_err(),
        "ENGINE_INSTALL_START_TIMEOUT"
    );
    check_first_launch(&binary, &control, Duration::from_secs(2))
        .await
        .unwrap();
    let binary = version_fixture(&temp, "printf '0.0.0\\n'");
    assert_eq!(
        check_first_launch(&binary, &control, Duration::from_secs(2))
            .await
            .unwrap_err(),
        "ENGINE_INSTALL_VERSION"
    );
    let binary = version_fixture(&temp, "exit 7");
    assert_eq!(
        check_first_launch(&binary, &control, Duration::from_secs(2))
            .await
            .unwrap_err(),
        "ENGINE_INSTALL_START_FAILED"
    );
    assert_eq!(
        check_first_launch(&temp.0.join("missing"), &control, Duration::from_secs(2))
            .await
            .unwrap_err(),
        "ENGINE_INSTALL_START_FAILED"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn cancelling_cold_start_kills_child_before_activation() {
    let temp = Temp::new();
    let pid_file = temp.0.join("pid");
    let binary = version_fixture(
        &temp,
        &format!("echo $$ > '{}'\nexec /bin/sleep 10", pid_file.display()),
    );
    let control = InstallControl::for_test();
    let worker_control = control.clone();
    let worker = tokio::spawn(async move {
        check_first_launch(&binary, &worker_control, FIRST_LAUNCH_TIMEOUT).await
    });
    let pid: i32 = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            // Shell redirection creates the file before echo writes the PID.
            if let Some(pid) = fs::read_to_string(&pid_file)
                .ok()
                .and_then(|text| text.trim().parse::<i32>().ok())
                .filter(|pid| *pid > 0)
            {
                break pid;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    control.state.store(1, Ordering::Release);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), worker)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err(),
        "ENGINE_INSTALL_CANCELLED"
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while unsafe { libc::kill(pid, 0) } == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(!temp.0.join("active.json").exists());
}

#[test]
fn pinned_assets_cover_supported_platforms_and_current_version() {
    let manifest: Manifest = serde_json::from_str(MANIFEST).unwrap();
    assert_eq!(manifest.version, codex_proxy_engine::ENGINE_VERSION);
    assert_eq!(manifest.assets.len(), 6);
    for asset in manifest.assets.values() {
        assert!(asset.file.starts_with("mihomo-") && asset.file.contains("v1.19.31"));
        assert!(!asset.file.contains('/'));
        assert_eq!(asset.sha256.len(), 64);
        assert!(asset.size < MAX_ARCHIVE);
    }
}
#[test]
fn download_redirects_cannot_change_to_untrusted_http_or_credentialed_urls() {
    for url in [
        "https://github.com/MetaCubeX/mihomo",
        "https://release-assets.githubusercontent.com/path?sig=opaque",
    ] {
        assert!(approved_redirect(&url::Url::parse(url).unwrap()));
    }
    for url in [
        "http://github.com/a",
        "https://github.com.evil.example/a",
        "https://evil.example/a",
        "https://u:p@github.com/a",
        "https://github.com:444/a",
        "http://127.0.0.1/a",
    ] {
        assert!(!approved_redirect(&url::Url::parse(url).unwrap()));
    }
}
#[test]
fn cross_process_install_lock_prevents_overlapping_staging() {
    let temp = Temp::new();
    let mut first = create_staging(temp.0.clone(), &uuid::Uuid::new_v4().to_string()).unwrap();
    assert!(
        matches!(create_staging(temp.0.clone(), &uuid::Uuid::new_v4().to_string()), Err(e) if e == "ENGINE_INSTALL_BUSY")
    );
    first.cleanup();
    let mut next = create_staging(temp.0.clone(), &uuid::Uuid::new_v4().to_string()).unwrap();
    next.cleanup();
}
#[test]
fn cancellation_and_commit_have_one_winner() {
    for _ in 0..64 {
        let control = InstallControl::for_test();
        let cancel_state = control.state.clone();
        let cancel = std::thread::spawn(move || {
            cancel_state
                .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        });
        let commit = control
            .state
            .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
            .is_ok();
        assert_ne!(commit, cancel.join().unwrap());
    }
}
#[tokio::test]
async fn corrupt_import_preserves_active_and_can_be_retried() {
    let temp = Temp::new();
    let active = temp.0.join("active.json");
    fs::write(&active, "previous pointer untouched").unwrap();
    let input = temp.0.join("invalid.zip");
    fs::write(&input, b"not an official archive").unwrap();
    let result = install_at(
        temp.0.clone(),
        asset().unwrap(),
        Some(input),
        InstallControl::for_test(),
    )
    .await;
    assert_eq!(result.unwrap_err(), "ENGINE_INSTALL_CHECKSUM");
    assert_eq!(
        fs::read_to_string(active).unwrap(),
        "previous pointer untouched"
    );
}
#[tokio::test]
async fn pre_cancelled_import_does_not_create_installation() {
    let temp = Temp::new();
    let root = temp.0.join("not-created");
    let result = install_at(
        root.clone(),
        asset().unwrap(),
        None,
        InstallControl::for_test_cancelled(),
    )
    .await;
    assert_eq!(result.unwrap_err(), "ENGINE_INSTALL_CANCELLED");
    assert!(!root.exists());
}
#[tokio::test]
async fn expired_import_does_not_publish() {
    let temp = Temp::new();
    let root = temp.0.join("not-created");
    let result = install_at(
        root.clone(),
        asset().unwrap(),
        None,
        InstallControl::for_test_timed_out(),
    )
    .await;
    assert_eq!(result.unwrap_err(), "ENGINE_INSTALL_TIMEOUT");
    assert!(!root.exists());
}
#[test]
fn next_install_recovers_abandoned_staging_but_keeps_other_files() {
    let temp = Temp::new();
    let orphan = temp.0.join(format!("staging-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&orphan).unwrap();
    fs::write(orphan.join("partial.archive"), b"partial").unwrap();
    fs::write(temp.0.join("keep.txt"), b"unrelated").unwrap();
    let mut stage = create_staging(temp.0.clone(), &uuid::Uuid::new_v4().to_string()).unwrap();
    assert!(!orphan.exists());
    assert!(temp.0.join("keep.txt").is_file());
    stage.cleanup();
}
fn release(root: &Path) -> Installed {
    let directory = format!("release-{}", uuid::Uuid::new_v4());
    let path = root.join(&directory);
    fs::create_dir(&path).unwrap();
    let mut files = BTreeMap::new();
    for name in if cfg!(windows) {
        vec!["mihomo.exe"]
    } else {
        vec!["mihomo"]
    } {
        fs::write(path.join(name), b"fixture-binary").unwrap();
        files.insert(
            name.into(),
            archive::file_sha256(&path.join(name), &InstallControl::for_test()).unwrap(),
        );
    }
    File::create(path.join(".lease")).unwrap();
    let installed = Installed {
        directory,
        version: codex_proxy_engine::ENGINE_VERSION.into(),
        target: platform_target().unwrap().into(),
        archive_sha256: asset().unwrap().sha256,
        files,
    };
    fs::write(
        path.join("installed.json"),
        serde_json::to_vec(&installed).unwrap(),
    )
    .unwrap();
    installed
}

#[tokio::test]
async fn readiness_hash_check_distinguishes_missing_intact_and_damaged_without_repairing_data() {
    let temp = Temp::new();
    assert!(load_installed(&temp.0).unwrap().is_none());
    let installed = release(&temp.0);
    let pointer = temp.0.join("active.json");
    let record = serde_json::to_vec(&installed).unwrap();
    fs::write(&pointer, &record).unwrap();
    let binary = temp.0.join(&installed.directory).join(binary_name());
    verify_managed_in(temp.0.clone(), &binary).await.unwrap();
    fs::write(&binary, b"modified-binary").unwrap();
    // Status is intentionally a metadata read; an explicit operation verifies content.
    assert!(load_installed(&temp.0).unwrap().is_some());
    assert_eq!(verify_managed_in(temp.0.clone(), &binary).await.unwrap_err(), "ENGINE_INSTALL_VERIFY");
    assert_eq!(fs::read(&pointer).unwrap(), record);
    assert_eq!(fs::read(&binary).unwrap(), b"modified-binary");
    fs::remove_file(&binary).unwrap();
    assert_eq!(load_installed(&temp.0).err().unwrap(), "ENGINE_INSTALL_VERIFY");
    assert_eq!(fs::read(&pointer).unwrap(), record);
}
#[test]
fn immutable_release_record_survives_active_switch_and_running_lease_prevents_cleanup() {
    let temp = Temp::new();
    let old = release(&temp.0);
    let old_path = temp.0.join(&old.directory);
    let lease = File::open(old_path.join(".lease")).unwrap();
    FileExt::try_lock_shared(&lease).unwrap();
    let _previous = release(&temp.0);
    let current = release(&temp.0);
    fs::write(
        temp.0.join("active.json"),
        serde_json::to_vec(&current).unwrap(),
    )
    .unwrap();
    assert_eq!(
        load_record(&temp.0, &old_path.join("installed.json"))
            .unwrap()
            .unwrap()
            .directory,
        old.directory
    );
    assert_eq!(
        load_installed(&temp.0).unwrap().unwrap().directory,
        current.directory
    );
    prune_releases(&temp.0, &current.directory);
    assert!(old_path.join(binary_name()).is_file());
    drop(lease);
}
#[test]
fn active_pointer_rejects_path_escape_and_missing_companion() {
    let temp = Temp::new();
    let mut installed = release(&temp.0);
    let active = temp.0.join("active.json");
    installed.directory = "../outside".into();
    fs::write(&active, serde_json::to_vec(&installed).unwrap()).unwrap();
    assert!(load_installed(&temp.0).is_err());
    let installed = release(&temp.0);
    fs::remove_file(temp.0.join(&installed.directory).join(binary_name())).unwrap();
    fs::write(&active, serde_json::to_vec(&installed).unwrap()).unwrap();
    assert!(load_installed(&temp.0).is_err());
}
#[tokio::test]
#[ignore = "requires explicitly supplied official engine archive; installs only in temporary test directory"]
async fn real_official_archive_installs_and_repairs_without_touching_user_accounts() {
    let input = PathBuf::from(
        std::env::var_os("COCKPIT_TEST_ENGINE_ARCHIVE").expect("set official archive path"),
    );
    let temp = Temp::new();
    install_at(
        temp.0.clone(),
        asset().unwrap(),
        Some(input.clone()),
        InstallControl::for_test(),
    )
    .await
    .unwrap();
    let first = load_installed(&temp.0).unwrap().unwrap();
    let binary = temp.0.join(&first.directory).join(binary_name());
    codex_proxy_engine::verify_version(&binary).await.unwrap();
    for (name, hash) in &first.files {
        assert_eq!(
            &archive::file_sha256(
                &temp.0.join(&first.directory).join(name),
                &InstallControl::for_test()
            )
            .unwrap(),
            hash
        );
    }
    install_at(
        temp.0.clone(),
        asset().unwrap(),
        Some(input),
        InstallControl::for_test(),
    )
    .await
    .unwrap();
    let second = load_installed(&temp.0).unwrap().unwrap();
    assert_ne!(first.directory, second.directory);
    assert!(binary.exists());
    assert!(temp.0.join(&second.directory).join(binary_name()).exists());
}

#[tokio::test]
async fn supervisor_times_out_without_waiting_for_slow_worker_and_prevents_late_publish() {
    let control = InstallControl::for_test_timed_out();
    let worker_control = control.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let worker = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(60)).await;
        let result = worker_control.check();
        tx.send(result.is_err()).unwrap();
        result
    });
    assert_eq!(
        supervise(&control, worker).await.unwrap_err(),
        "ENGINE_INSTALL_TIMEOUT"
    );
    assert!(rx.await.unwrap());
}
#[tokio::test]
#[ignore = "downloads a pinned official archive; installs only in a temporary directory"]
async fn real_official_download_verifies_and_installs_in_isolation() {
    assert_eq!(
        std::env::var("COCKPIT_TEST_ENGINE_DOWNLOAD").as_deref(),
        Ok("1")
    );
    let temp = Temp::new();
    install_at(
        temp.0.clone(),
        asset().unwrap(),
        None,
        InstallControl::for_test(),
    )
    .await
    .unwrap();
    let installed = load_installed(&temp.0).unwrap().unwrap();
    codex_proxy_engine::verify_version(&temp.0.join(installed.directory).join(binary_name()))
        .await
        .unwrap();
}

#[test]
fn legacy_sing_box_install_is_not_reused() {
    let temp = Temp::new();
    let mut installed = release(&temp.0);
    installed.version = "1.14.1".into();
    installed.files.clear();
    installed.files.insert("sing-box".into(), "a".repeat(64));
    fs::write(
        temp.0.join("active.json"),
        serde_json::to_vec(&installed).unwrap(),
    )
    .unwrap();
    assert!(load_installed(&temp.0).unwrap().is_none());
}

/// Windows denies renaming the release directory while security software still holds the
/// freshly executed binary open; the same rename succeeds a moment later.
#[test]
fn publish_rename_retries_transient_handle_races() {
    let attempts = std::sync::atomic::AtomicUsize::new(0);
    let published = retry_transient_publish(|| {
        let attempt = attempts.fetch_add(1, Ordering::SeqCst);
        if attempt < 3 {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "the staged binary is still held open",
            ))
        } else {
            Ok("release-published")
        }
    })
    .expect("a transient handle race must not fail the install");
    assert_eq!(published, "release-published");
    assert_eq!(attempts.load(Ordering::SeqCst), 4);
}

#[test]
fn publish_rename_does_not_retry_permanent_errors() {
    let attempts = std::sync::atomic::AtomicUsize::new(0);
    let error = retry_transient_publish::<()>(|| {
        attempts.fetch_add(1, Ordering::SeqCst);
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "staging directory disappeared",
        ))
    })
    .expect_err("a permanent error must surface immediately");
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

#[test]
fn publish_rename_reports_the_last_error_after_the_budget_is_spent() {
    let attempts = std::sync::atomic::AtomicUsize::new(0);
    let error = retry_transient_publish::<()>(|| {
        attempts.fetch_add(1, Ordering::SeqCst);
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "still held open",
        ))
    })
    .expect_err("an exhausted retry budget must report failure");
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        PUBLISH_RENAME_ATTEMPTS as usize
    );
}
