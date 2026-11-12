#[cfg(test)]
mod profile_copy_integration_tests {
    use super::*;
    use rusqlite::Connection;
    use serde_json::json;

    struct TestEnvironment {
        root: PathBuf,
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }

    impl TestEnvironment {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("codex-instance-copy-{}", Uuid::new_v4()));
            fs::create_dir_all(root.join("source")).unwrap();
            let root = root.canonicalize().unwrap();
            let mut saved = Vec::new();
            for (key, value) in [
                ("CODEX_HOME", root.join("source")),
                ("COCKPIT_TOOLS_TEST_DATA_DIR", root.join("cockpit-data")),
            ] {
                saved.push((key, std::env::var_os(key)));
                std::env::set_var(key, value);
            }
            Self { root, saved }
        }
    }

    impl Drop for TestEnvironment {
        fn drop(&mut self) {
            for (key, value) in &self.saved {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn codex_profile_copy_create_instance_preserves_folderless_project() {
        let _lock = crate::modules::test_support::env_lock().lock().unwrap();
        let env = TestEnvironment::new();
        let source = env.root.join("source");
        let target = env.root.join("target");
        fs::create_dir_all(source.join("sessions")).unwrap();
        let id = "11111111-1111-4111-8111-111111111111";
        let relative = format!("sessions/rollout-2026-01-01T00-00-00-{id}.jsonl");
        let contents = format!(
            "{}\n",
            json!({"type":"session_meta","payload":{"id":id,"history_mode":"paginated"}})
        );
        fs::write(source.join(&relative), &contents).unwrap();
        let db = Connection::open(source.join("state_5.sqlite")).unwrap();
        db.execute_batch("CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, project_id TEXT); CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT); INSERT INTO projects VALUES ('project-current', 'Synthetic project');").unwrap();
        db.execute(
            "INSERT INTO threads VALUES (?1, ?2, NULL)",
            rusqlite::params![id, source.join(&relative).to_str().unwrap()],
        )
        .unwrap();
        drop(db);
        let state = json!({
            "local-projects": {"project-legacy": {"name":"Synthetic project","rootPaths":[]}},
            "thread-project-assignments": {id: {"projectId":"project-legacy","projectKind":"local"}},
            "app-server-project-id-by-legacy-project-id-by-host": {
                format!("local:{}",source.display()): {"project-legacy":"project-current"}
            }
        });
        fs::write(
            source.join(".codex-global-state.json"),
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();

        let profile = create_instance(CreateInstanceParams {
            name: "Synthetic clone".to_string(),
            user_data_dir: target.to_string_lossy().into_owned(),
            working_dir: None,
            extra_args: String::new(),
            bind_account_id: None,
            model_routing: None,
            copy_source_instance_id: Some("__default__".to_string()),
            init_mode: None, // The same default-copy path used when launching a new instance.
            launch_mode: None,
            app_speed: None,
        })
        .unwrap();
        let store = load_instance_store().unwrap();
        assert!(store.instances.iter().any(|item| item.id == profile.id));
        assert_eq!(profile.working_dir, None);
        assert_eq!(
            fs::read(target.join(&relative)).unwrap(),
            contents.as_bytes()
        );
        let copied = Connection::open(target.join("state_5.sqlite")).unwrap();
        let (path, project): (String, String) = copied
            .query_row("SELECT rollout_path, project_id FROM threads", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(Path::new(&path), target.join(relative));
        assert_eq!(project, "project-current");
    }

    fn params(name: &str, target: &Path, mode: &str) -> CreateInstanceParams {
        CreateInstanceParams {
            name: name.to_string(),
            user_data_dir: target.to_string_lossy().into_owned(),
            working_dir: None,
            extra_args: String::new(),
            bind_account_id: None,
            model_routing: None,
            copy_source_instance_id: Some("__default__".to_string()),
            init_mode: Some(mode.to_string()),
            launch_mode: None,
            app_speed: None,
        }
    }

    #[cfg(unix)]
    #[test]
    fn codex_profile_copy_preserves_dangling_vendor_skills_link() {
        let _lock = crate::modules::test_support::env_lock().lock().unwrap();
        let env = TestEnvironment::new();
        let source = env.root.join("source");
        let target = env.root.join("target");
        let vendor_skills = Path::new(CODEX_SHARED_VENDOR_IMPORTS_SKILLS_DIR);
        let global_vendor_skills = source.join(vendor_skills);
        let missing = env.root.join("vendor-skills");
        fs::create_dir_all(global_vendor_skills.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&missing, &global_vendor_skills).unwrap();
        fs::write(source.join("AGENTS.md"), b"shared instructions").unwrap();

        let mut create = params("CLI copy with dangling link", &target, "copy");
        create.launch_mode = Some(InstanceLaunchMode::Cli);
        let profile = create_instance(create).unwrap();
        assert_eq!(profile.launch_mode, InstanceLaunchMode::Cli);
        assert!(load_instance_store()
            .unwrap()
            .instances
            .iter()
            .any(|item| item.id == profile.id));
        assert_eq!(fs::read_link(&global_vendor_skills).unwrap(), missing);
        assert!(fs::symlink_metadata(target.join(vendor_skills)).is_err());
        assert!(target.join("skills").is_dir());
        assert!(target.join("rules").is_dir());
        assert_eq!(
            fs::read(target.join("AGENTS.md")).unwrap(),
            b"shared instructions"
        );

        fs::create_dir(&missing).unwrap();
        ensure_instance_shared_skills(&target).unwrap();
        assert_eq!(
            fs::canonicalize(target.join(vendor_skills)).unwrap(),
            missing
        );
    }

    #[test]
    fn codex_profile_copy_preserves_file_at_vendor_skills_path() {
        let _lock = crate::modules::test_support::env_lock().lock().unwrap();
        let env = TestEnvironment::new();
        let source = env.root.join("source");
        let target = env.root.join("target");
        let vendor_skills = Path::new(CODEX_SHARED_VENDOR_IMPORTS_SKILLS_DIR);
        let global_vendor_skills = source.join(vendor_skills);
        fs::create_dir_all(global_vendor_skills.parent().unwrap()).unwrap();
        fs::write(&global_vendor_skills, b"original contents").unwrap();

        let profile = create_instance(params("Desktop copy with file", &target, "copy")).unwrap();
        assert_eq!(profile.launch_mode, InstanceLaunchMode::App);
        assert_eq!(
            fs::read(global_vendor_skills).unwrap(),
            b"original contents"
        );
        assert_eq!(
            fs::read(target.join(vendor_skills)).unwrap(),
            b"original contents"
        );
    }

    #[test]
    fn codex_profile_copy_slow_copy_keeps_registry_available_and_concurrent_creates() {
        let _lock = crate::modules::test_support::env_lock().lock().unwrap();
        let env = TestEnvironment::new();
        let target = env.root.join("copied");
        let other_target = env.root.join("other");
        let cancelled = std::sync::atomic::AtomicBool::new(false);
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let result = std::thread::scope(|scope| {
            let copy_target = &target;
            let copy_cancelled = &cancelled;
            let copy = scope.spawn(move || {
                create_instance_with_profile_copy(
                    params("Paused copy", copy_target, "copy"),
                    copy_cancelled,
                    |source, target| {
                        started_tx.send(()).unwrap();
                        resume_rx
                            .recv_timeout(std::time::Duration::from_secs(10))
                            .map_err(|e| e.to_string())?;
                        modules::codex_profile_copy::copy_profile(source, target)
                    },
                )
            });
            started_rx
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap();
            let lock_available = CODEX_INSTANCE_STORE_LOCK.try_lock().is_ok();
            // Always release the copy before asserting, even if another create fails.
            let other = if lock_available {
                create_instance(params("Concurrent empty", &other_target, "empty"))
            } else {
                Err("registry locked".to_string())
            };
            resume_tx.send(()).unwrap();
            let copied = copy.join().unwrap();
            assert!(
                lock_available,
                "copying must not hold the global registry lock"
            );
            (copied.unwrap(), other.unwrap())
        });
        let store = load_instance_store().unwrap();
        for profile in [result.0, result.1] {
            assert!(store.instances.iter().any(|item| item.id == profile.id));
        }
    }

    #[test]
    fn codex_instance_slow_deletion_preserves_concurrent_registry_changes() {
        let _lock = crate::modules::test_support::env_lock().lock().unwrap();
        let env = TestEnvironment::new();
        let target = env.root.join("to-delete");
        let instance = create_instance(params("To delete", &target, "empty")).unwrap();
        let deleted_id = &instance.id;
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();

        let other = std::thread::scope(|scope| {
            let pending = scope.spawn(move || {
                delete_instance_with(deleted_id, |path| {
                    if path == target {
                        started_tx.send(()).unwrap();
                        resume_rx
                            .recv_timeout(std::time::Duration::from_secs(10))
                            .map_err(|error| error.to_string())?;
                    }
                    Ok(())
                })
            });
            started_rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap();
            let other = if CODEX_INSTANCE_STORE_LOCK.try_lock().is_ok() {
                create_instance(params("Keep", &env.root.join("keep"), "empty"))
            } else {
                Err("slow deletion held the registry lock".to_string())
            };
            resume_tx.send(()).unwrap();
            pending.join().unwrap().unwrap();
            other.unwrap()
        });
        let ids: Vec<_> = load_instance_store().unwrap().instances.into_iter().map(|item| item.id).collect();
        assert_eq!(ids, [other.id.clone()]);

        let error = delete_instance_with(&other.id, |_| Err("trash unavailable".into())).unwrap_err();
        assert_eq!(error, "trash unavailable");
        assert_eq!(load_instance_store().unwrap().instances[0].id, other.id);
    }

    #[test]
    fn codex_profile_copy_cancelled_worker_never_registers_and_releases_reservation() {
        let _lock = crate::modules::test_support::env_lock().lock().unwrap();
        let env = TestEnvironment::new();
        let target = env.root.join("cancelled");
        let cancelled = std::sync::atomic::AtomicBool::new(false);
        let error = create_instance_with_profile_copy(
            params("Cancelled copy", &target, "copy"),
            &cancelled,
            |_, _| {
                cancelled.store(true, std::sync::atomic::Ordering::Release);
                Ok(())
            },
        )
        .unwrap_err();
        assert!(error.contains("取消"));
        assert!(
            !target.exists(),
            "cancelled copy must not proceed to shared-skill writes"
        );
        assert!(load_instance_store().unwrap().instances.is_empty());
        // The failed transaction must not leave a reservation that prevents a retry.
        let retried = create_instance(params("Cancelled copy", &target, "empty")).unwrap();
        assert!(load_instance_store()
            .unwrap()
            .instances
            .iter()
            .any(|item| item.id == retried.id));
    }
}
