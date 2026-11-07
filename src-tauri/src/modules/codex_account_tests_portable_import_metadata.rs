// Codex 账号测试：Cockpit Tools 导出格式附带的标签/备注名/分组元数据。
// 测试与生产实现共享 super 作用域，验证元数据读取、落盘与分组归类行为。

#[tokio::test(flavor = "current_thread")]
async fn cockpit_tools_import_restores_exported_tags_and_group() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let _env = TestEnvGuard::new("codex-portable-metadata-import-test");

    let tokens = make_codex_tokens(
        "portable@example.com",
        "acc-portable",
        "org-portable",
        "portable",
        "rt-portable",
    );
    let content = serde_json::json!([{
        "id_token": tokens.id_token,
        "access_token": tokens.access_token,
        "refresh_token": tokens.refresh_token,
        "account_id": "acc-portable",
        "last_refresh": "2025-01-01T00:00:00Z",
        "email": "portable@example.com",
        "type": "codex",
        "expired": "2090-01-01T00:00:00Z",
        "tags": ["sync", "team"],
        "account_name": "Portable Name",
        "account_structure": "workspace",
        "group": "同步分组",
    }]);

    let accounts =
        super::import_from_json(&serde_json::to_string(&content).expect("serialize export"))
            .await
            .expect("import Cockpit Tools export");

    assert_eq!(accounts.len(), 1);
    let expected_tags = Some(vec!["sync".to_string(), "team".to_string()]);
    assert_eq!(accounts[0].tags, expected_tags);
    assert_eq!(accounts[0].account_name.as_deref(), Some("Portable Name"));
    assert_eq!(accounts[0].account_structure.as_deref(), Some("workspace"));

    // 元数据必须真正落盘，而不只是内存里的导入结果。
    let reloaded = super::load_account(&accounts[0].id).expect("reload imported account");
    assert_eq!(reloaded.tags, expected_tags);
    assert_eq!(reloaded.account_name.as_deref(), Some("Portable Name"));

    let groups_path = crate::modules::account::get_data_dir()
        .expect("data dir")
        .join("codex_account_groups.json");
    let groups: Vec<serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(&groups_path).expect("read groups"))
            .expect("parse groups");
    assert_eq!(groups.len(), 1);
    assert_eq!(
        groups[0].get("name").and_then(|value| value.as_str()),
        Some("同步分组")
    );
    assert_eq!(
        groups[0]
            .get("accountIds")
            .and_then(|value| value.as_array())
            .and_then(|ids| ids.first())
            .and_then(|value| value.as_str()),
        Some(reloaded.id.as_str())
    );
}

#[test]
fn portable_export_metadata_reads_tags_name_and_group() {
    let value = serde_json::json!({
        "access_token": "token-value",
        "email": "demo@example.com",
        "tags": [" Plus ", "", "finance"],
        "account_name": "Team A",
        "account_structure": "workspace",
        "group": " 财务分组 ",
    });

    let metadata = super::CodexPortableAccountMetadata::from_value(&value);

    assert_eq!(
        metadata.tags.as_deref(),
        Some(["Plus".to_string(), "finance".to_string()].as_slice())
    );
    assert_eq!(metadata.account_name.as_deref(), Some("Team A"));
    assert_eq!(metadata.account_structure.as_deref(), Some("workspace"));
    assert_eq!(metadata.group_name.as_deref(), Some("财务分组"));

    let empty = super::CodexPortableAccountMetadata::from_value(&serde_json::json!({
        "access_token": "token-value",
    }));
    assert!(empty.tags.is_none());
    assert!(empty.account_name.is_none());
    assert!(empty.account_structure.is_none());
    assert!(empty.group_name.is_none());
}

#[test]
fn imported_group_name_creates_then_reuses_and_moves_accounts() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let _env = TestEnvGuard::new("codex-import-group-name-test");

    super::assign_imported_account_to_group_by_name("acc-1", "财务分组")
        .expect("create group by name");
    super::assign_imported_account_to_group_by_name("acc-2", "财务分组")
        .expect("reuse group by name");
    super::assign_imported_account_to_group_by_name("acc-2", "其他分组")
        .expect("move account into another group");

    let path = crate::modules::account::get_data_dir()
        .expect("data dir")
        .join("codex_account_groups.json");
    let groups: Vec<serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read groups"))
            .expect("parse groups");

    assert_eq!(groups.len(), 2);
    let default_group = groups
        .iter()
        .find(|group| group.get("name").and_then(|value| value.as_str()) == Some("财务分组"))
        .expect("default group exists");
    assert_eq!(
        default_group
            .get("accountIds")
            .and_then(|value| value.as_array())
            .map(|ids| ids.len()),
        Some(1)
    );
    let moved_group = groups
        .iter()
        .find(|group| group.get("name").and_then(|value| value.as_str()) == Some("其他分组"))
        .expect("moved group exists");
    assert_eq!(
        moved_group
            .get("accountIds")
            .and_then(|value| value.as_array())
            .and_then(|ids| ids.first())
            .and_then(|value| value.as_str()),
        Some("acc-2")
    );
}
