// System commands 测试：配置 patch、备份与路径行为。
// 测试作为原 tests 模块内部实现被 include。
    use super::{
        antigravity_metadata_root_matches_target_with_product_metadata,
        antigravity_product_json_target, apply_codex_quota_alert_thresholds,
        apply_general_config_updates, lock_general_config_transaction,
        normalize_antigravity_metadata_root, read_antigravity_product_json_metadata, UserConfig,
    };
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    struct MetadataTestDir(PathBuf);

    impl MetadataTestDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "cockpit-tools-antigravity-metadata-{}-{}",
                std::process::id(),
                name
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("create metadata test directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for MetadataTestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn assert_same_fs_path(left: &Path, right: &Path) {
        let left = std::fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
        let right = std::fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
        assert_eq!(left, right);
    }

    fn write_antigravity_product_json(root: &Path) {
        let product_dir = root.join("resources").join("app");
        std::fs::create_dir_all(&product_dir).expect("create product metadata directory");
        std::fs::write(
            product_dir.join("product.json"),
            r#"{"nameShort":"Antigravity IDE","ideVersion":"1.2.3"}"#,
        )
        .expect("write product metadata");
    }

    #[test]
    fn antigravity_metadata_uses_install_root_for_root_executable() {
        let install = MetadataTestDir::new("root-executable");
        write_antigravity_product_json(install.path());
        let executable = install.path().join("antigravity-ide");
        std::fs::write(&executable, b"launcher").expect("write root executable");

        let root = normalize_antigravity_metadata_root(&executable)
            .expect("resolve metadata root from executable");
        let metadata = read_antigravity_product_json_metadata(&root)
            .expect("read product.json from install root");

        assert_same_fs_path(&root, install.path());
        assert_eq!(metadata.version, "1.2.3");
    }

    #[test]
    fn antigravity_metadata_uses_install_root_for_bin_launcher() {
        let install = MetadataTestDir::new("bin-launcher");
        write_antigravity_product_json(install.path());
        let executable = install.path().join("bin").join("antigravity-ide");
        std::fs::create_dir_all(executable.parent().expect("launcher parent"))
            .expect("create launcher directory");
        std::fs::write(&executable, b"launcher").expect("write bin launcher");

        let root = normalize_antigravity_metadata_root(&executable)
            .expect("resolve metadata root from bin launcher");
        let metadata = read_antigravity_product_json_metadata(&root)
            .expect("read product.json from install root");

        assert_same_fs_path(&root, install.path());
        assert_eq!(metadata.version, "1.2.3");
    }

    #[test]
    fn antigravity_metadata_skips_invalid_primary_product_json() {
        let install = MetadataTestDir::new("invalid-primary-product-json");
        let primary = install.path().join("resources").join("app");
        let fallback = install.path().join("app");
        std::fs::create_dir_all(&primary).expect("create primary metadata directory");
        std::fs::create_dir_all(&fallback).expect("create fallback metadata directory");
        std::fs::write(primary.join("product.json"), b"not-json")
            .expect("write invalid primary product metadata");
        std::fs::write(
            fallback.join("product.json"),
            r#"{"nameShort":"Antigravity IDE","ideVersion":"9.8.7"}"#,
        )
        .expect("write fallback product metadata");

        let metadata = read_antigravity_product_json_metadata(install.path())
            .expect("read fallback product metadata");

        assert_eq!(metadata.version, "9.8.7");
    }

    #[test]
    fn metadata_target_skips_unrelated_primary_product_json() {
        let install = MetadataTestDir::new("unrelated-primary-product-json");
        let primary = install.path().join("resources").join("app");
        let fallback = install.path().join("app");
        std::fs::create_dir_all(&primary).expect("create primary metadata directory");
        std::fs::create_dir_all(&fallback).expect("create fallback metadata directory");
        std::fs::write(
            primary.join("product.json"),
            r#"{"nameShort":"Other IDE","version":"1.0.0"}"#,
        )
        .expect("write unrelated primary product metadata");
        std::fs::write(
            fallback.join("product.json"),
            r#"{"nameShort":"Antigravity IDE","ideVersion":"9.8.7"}"#,
        )
        .expect("write fallback product metadata");

        assert_eq!(
            antigravity_product_json_target(install.path()),
            Some("antigravity_ide")
        );
        let metadata = read_antigravity_product_json_metadata(install.path())
            .expect("read fallback Antigravity metadata");
        assert_eq!(metadata.product_name, "Antigravity IDE");
        assert_eq!(metadata.version, "9.8.7");
    }

    #[cfg(unix)]
    #[test]
    fn antigravity_metadata_follows_bin_launcher_symlink_to_install_root() {
        let install = MetadataTestDir::new("symlink-space-含");
        write_antigravity_product_json(install.path());
        let launcher = install.path().join("bin").join("antigravity-ide");
        let link = install.path().join("launcher-link");
        std::fs::create_dir_all(launcher.parent().expect("launcher parent"))
            .expect("create launcher directory");
        std::fs::write(&launcher, b"launcher").expect("write bin launcher");
        std::os::unix::fs::symlink("bin/antigravity-ide", &link).expect("create launcher symlink");

        let root = normalize_antigravity_metadata_root(&link)
            .expect("resolve metadata root from symlink launcher");
        let metadata = read_antigravity_product_json_metadata(&root)
            .expect("read product metadata through symlink root");

        assert_same_fs_path(&root, install.path());
        assert_eq!(metadata.version, "1.2.3");
    }

    #[test]
    fn configured_neutral_executable_uses_product_metadata_for_ide_target() {
        let install = MetadataTestDir::new("neutral-configured-executable");
        write_antigravity_product_json(install.path());
        let executable = install.path().join("MyIDE.AppImage");
        std::fs::write(&executable, b"launcher").expect("write neutral executable");

        let root = normalize_antigravity_metadata_root(&executable)
            .expect("resolve metadata root from neutral executable");
        assert!(
            antigravity_metadata_root_matches_target_with_product_metadata(
                &root,
                Some("antigravity_ide"),
                true,
            )
        );
        assert_eq!(
            read_antigravity_product_json_metadata(&root)
                .expect("read configured product metadata")
                .version,
            "1.2.3"
        );
    }

    #[test]
    fn ide_version_field_classifies_metadata_without_a_product_name() {
        let install = MetadataTestDir::new("unnamed-product");
        let product_dir = install.path().join("resources").join("app");
        std::fs::create_dir_all(&product_dir).expect("create unnamed product metadata directory");
        std::fs::write(
            product_dir.join("product.json"),
            r#"{"ideVersion":"2.4.6"}"#,
        )
        .expect("write unnamed product metadata");

        assert!(
            antigravity_metadata_root_matches_target_with_product_metadata(
                install.path(),
                Some("antigravity_ide"),
                true,
            )
        );
        assert!(
            !antigravity_metadata_root_matches_target_with_product_metadata(
                install.path(),
                Some("antigravity"),
                true,
            )
        );
    }

    #[test]
    fn product_metadata_takes_precedence_over_install_directory_spelling() {
        let parent = MetadataTestDir::new("metadata-target-precedence");
        let install = parent.path().join("antigravity-ide");
        let product_dir = install.join("resources").join("app");
        std::fs::create_dir_all(&product_dir).expect("create legacy product metadata directory");
        std::fs::write(
            product_dir.join("product.json"),
            r#"{"nameShort":"Antigravity","version":"3.2.1"}"#,
        )
        .expect("write legacy product metadata");

        assert!(
            antigravity_metadata_root_matches_target_with_product_metadata(
                &install,
                Some("antigravity"),
                true,
            )
        );
        assert!(
            !antigravity_metadata_root_matches_target_with_product_metadata(
                &install,
                Some("antigravity_ide"),
                true,
            )
        );
    }

    #[test]
    fn general_config_transaction_lock_serializes_side_effecting_writes() {
        let first_guard = lock_general_config_transaction().expect("acquire first transaction");
        let (attempt_tx, attempt_rx) = mpsc::channel();
        let (acquired_tx, acquired_rx) = mpsc::channel();

        let worker = thread::spawn(move || {
            attempt_tx.send(()).expect("signal lock attempt");
            let _guard = lock_general_config_transaction().expect("acquire second transaction");
            acquired_tx.send(()).expect("signal lock acquisition");
        });

        attempt_rx.recv().expect("wait for lock attempt");
        assert!(acquired_rx.recv_timeout(Duration::from_millis(50)).is_err());
        drop(first_guard);
        acquired_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("second transaction should continue after unlock");
        worker.join().expect("join transaction worker");
    }

    #[test]
    fn general_config_patch_only_changes_submitted_fields() {
        let mut config = UserConfig {
            theme: "dark".to_string(),
            auto_refresh_minutes: 10,
            ..UserConfig::default()
        };
        let updates = serde_json::json!({ "theme": "light" })
            .as_object()
            .expect("patch should be an object")
            .clone();

        apply_general_config_updates(&mut config, &updates).expect("patch should succeed");

        assert_eq!(config.theme, "light");
        assert_eq!(config.auto_refresh_minutes, 10);
    }

    #[test]
    fn general_config_patch_persists_session_sharing_switches() {
        let mut config = UserConfig::default();
        let updates = serde_json::json!({
            "codebuddy_share_sessions_on_switch": true,
            "codebuddy_cn_share_sessions_on_switch": true,
            "trae_share_sessions_on_switch": true,
            "trae_solo_share_sessions_on_switch": true,
            "trae_cn_share_sessions_on_switch": true,
            "trae_solo_cn_share_sessions_on_switch": true,
            "workbuddy_share_sessions_on_switch": true,
        })
        .as_object()
        .expect("patch should be an object")
        .clone();

        apply_general_config_updates(&mut config, &updates)
            .expect("session sharing patch should succeed");

        assert!(config.codebuddy_share_sessions_on_switch);
        assert!(config.codebuddy_cn_share_sessions_on_switch);
        assert!(config.trae_share_sessions_on_switch);
        assert!(config.trae_solo_share_sessions_on_switch);
        assert!(config.trae_cn_share_sessions_on_switch);
        assert!(config.trae_solo_cn_share_sessions_on_switch);
        assert!(config.workbuddy_share_sessions_on_switch);
    }

    #[test]
    fn general_config_patch_rejects_non_general_fields() {
        let mut config = UserConfig::default();
        let updates = serde_json::json!({ "webdav_sync_password": "secret" })
            .as_object()
            .expect("patch should be an object")
            .clone();

        let error = apply_general_config_updates(&mut config, &updates)
            .expect_err("unsupported field should fail");

        assert!(error.contains("webdav_sync_password"));
    }

    #[test]
    fn unrelated_general_save_preserves_distinct_codex_quota_thresholds() {
        let mut config = UserConfig {
            codex_quota_alert_threshold: 20,
            codex_quota_alert_primary_threshold: 10,
            codex_quota_alert_secondary_threshold: 30,
            ..UserConfig::default()
        };

        apply_codex_quota_alert_thresholds(&mut config, Some(20), None, None);

        assert_eq!(config.codex_quota_alert_primary_threshold, 10);
        assert_eq!(config.codex_quota_alert_secondary_threshold, 30);
    }

    #[test]
    fn changed_legacy_codex_quota_threshold_updates_both_windows() {
        let mut config = UserConfig {
            codex_quota_alert_threshold: 20,
            codex_quota_alert_primary_threshold: 10,
            codex_quota_alert_secondary_threshold: 30,
            ..UserConfig::default()
        };

        apply_codex_quota_alert_thresholds(&mut config, Some(40), None, None);

        assert_eq!(config.codex_quota_alert_threshold, 40);
        assert_eq!(config.codex_quota_alert_primary_threshold, 40);
        assert_eq!(config.codex_quota_alert_secondary_threshold, 40);
    }

    #[test]
    fn explicit_codex_quota_window_thresholds_take_precedence() {
        let mut config = UserConfig::default();

        apply_codex_quota_alert_thresholds(&mut config, Some(40), Some(15), Some(25));

        assert_eq!(config.codex_quota_alert_threshold, 40);
        assert_eq!(config.codex_quota_alert_primary_threshold, 15);
        assert_eq!(config.codex_quota_alert_secondary_threshold, 25);
    }
