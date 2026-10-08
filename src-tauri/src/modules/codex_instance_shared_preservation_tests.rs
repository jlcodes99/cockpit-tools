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
            sync_shared_directory(&instance, &global, Path::new(relative)).unwrap();
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
