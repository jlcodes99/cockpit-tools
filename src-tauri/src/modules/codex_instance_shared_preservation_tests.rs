#[test]
fn distinct_instance_skills_rules_and_agents_survive_repeated_sync() {
    use super::*;
    let root = std::env::temp_dir().join(format!(
        "cockpit-instance-preserve-{}",
        uuid::Uuid::new_v4()
    ));
    let global = root.join("global");
    let instance = root.join("instance");
    for relative in ["skills", "rules", "vendor_imports/skills"] {
        fs::create_dir_all(global.join(relative)).unwrap();
        fs::create_dir_all(instance.join(relative)).unwrap();
        fs::write(global.join(relative).join("definition.md"), "shared").unwrap();
        fs::write(
            instance.join(relative).join("definition.md"),
            "instance-specific",
        )
        .unwrap();
        for _ in 0..2 {
            if relative == "rules" {
                sync_shared_directory(&instance, &global, Path::new(relative)).unwrap();
            } else {
                ensure_instance_skill_directory(&instance, &global, Path::new(relative)).unwrap();
            }
            assert_eq!(
                fs::read_to_string(instance.join(relative).join("definition.md")).unwrap(),
                "instance-specific"
            );
            assert_eq!(
                fs::read_to_string(global.join(relative).join("definition.md")).unwrap(),
                "shared"
            );
            assert!(!is_shared_directory_link(
                &fs::symlink_metadata(instance.join(relative)).unwrap()
            ));
        }
    }
    fs::write(global.join("AGENTS.md"), "shared instructions").unwrap();
    fs::write(instance.join("AGENTS.md"), "local instructions").unwrap();
    for _ in 0..2 {
        sync_shared_file(&instance, &global, Path::new("AGENTS.md")).unwrap();
        assert_eq!(
            fs::read_to_string(instance.join("AGENTS.md")).unwrap(),
            "local instructions"
        );
        assert_eq!(
            fs::read_to_string(global.join("AGENTS.md")).unwrap(),
            "shared instructions"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn instance_skill_directories_keep_explicit_local_paths_across_starts() {
    use super::*;
    let root = tempfile::tempdir().unwrap();
    let global = root.path().join("global");
    let instance = root.path().join("instance");
    for relative in [CODEX_SHARED_SKILLS_DIR_NAME, CODEX_SHARED_VENDOR_IMPORTS_SKILLS_DIR] {
        let relative = Path::new(relative);
        let shared = global.join(relative);
        let local = instance.join(relative);
        fs::create_dir_all(&shared).unwrap();
        fs::write(shared.join("SKILL.md"), "shared").unwrap();

        ensure_instance_skill_directory(&instance, &global, relative).unwrap();
        assert_eq!(fs::read_to_string(local.join("SKILL.md")).unwrap(), "shared");
        if is_shared_directory_link(&fs::symlink_metadata(&local).unwrap()) {
            remove_symlink(&local).unwrap();
        } else {
            fs::remove_dir_all(&local).unwrap();
        }
        fs::create_dir(&local).unwrap();
        ensure_instance_skill_directory(&instance, &global, relative).unwrap();
        assert!(is_directory_empty(&local).unwrap());

        fs::write(local.join("SKILL.md"), "shared").unwrap();
        ensure_instance_skill_directory(&instance, &global, relative).unwrap();
        assert!(!is_shared_directory_link(&fs::symlink_metadata(&local).unwrap()));
        fs::remove_file(shared.join("SKILL.md")).unwrap();
        ensure_instance_skill_directory(&instance, &global, relative).unwrap();
        assert!(is_directory_empty(&shared).unwrap());
        assert_eq!(fs::read_to_string(local.join("SKILL.md")).unwrap(), "shared");
    }
}
