//! Shared managed-catalog speed capabilities and retirement of the old speed-only catalog.
use serde_json::Value;
use std::path::Path;

pub(crate) const FILE_NAME: &str = "cockpit-native-speed-models.json";
const MANAGED_FILE: &str = "cockpit-model-catalog.json";

pub(crate) fn is_reference(base_dir: &Path, reference: &str) -> bool {
    reference.trim() == FILE_NAME || Path::new(reference.trim()) == base_dir.join(FILE_NAME)
}

fn read_doc(base_dir: &Path) -> Result<toml_edit::Document, String> {
    let text = std::fs::read_to_string(base_dir.join("config.toml")).map_err(|e| e.to_string())?;
    crate::modules::codex_config_format::read_codex_config_doc_from_str(&text)
        .map_err(|e| e.to_string())
}

pub(crate) fn decorate(catalog_json: &str) -> Result<String, String> {
    let mut catalog: Value = serde_json::from_str(catalog_json).map_err(|e| e.to_string())?;
    if supplement(&mut catalog)? {
        serde_json::to_string_pretty(&catalog).map_err(|e| e.to_string())
    } else {
        Ok(catalog_json.to_owned())
    }
}

pub(crate) fn remove_legacy_references(base_dir: &Path, doc: &mut toml_edit::Document) -> bool {
    let mut changed = false;
    if doc
        .get("model_catalog_json")
        .and_then(|v| v.as_str())
        .is_some_and(|v| is_reference(base_dir, v))
    {
        doc.remove("model_catalog_json");
        changed = true;
    }
    if let Some(profiles) = doc.get_mut("profiles").and_then(|v| v.as_table_like_mut()) {
        for (_, item) in profiles.iter_mut() {
            if let Some(profile) = item.as_table_like_mut() {
                if profile
                    .get("model_catalog_json")
                    .and_then(|v| v.as_str())
                    .is_some_and(|v| is_reference(base_dir, v))
                {
                    profile.remove("model_catalog_json");
                    changed = true;
                }
            }
        }
    }
    changed
}

/// Only clear references we own; user-supplied catalogs and model choices are preserved.
/// Commit config before deleting the old file so an interrupted cleanup is retryable.
pub(crate) fn clear_reference(base_dir: &Path) -> Result<(), String> {
    if base_dir.join("config.toml").is_file() {
        let mut doc = read_doc(base_dir)?;
        let changed = remove_legacy_references(base_dir, &mut doc);
        if changed {
            let text = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
            crate::modules::codex_config_format::write_codex_config_toml_atomic(
                &base_dir.join("config.toml"),
                &text,
            )
            .map_err(|e| e.to_string())?;
        }
    }
    crate::modules::atomic_write::remove_file_locked(&base_dir.join(FILE_NAME))?;
    Ok(())
}

/// Startup maintenance, independent of authentication and Fast adaptation.
/// A saved managed policy wins over the obsolete speed snapshot; otherwise return to
/// the existing provider/native model source without creating a new managed catalog.
pub(crate) fn prepare_managed_catalog(base_dir: &Path) -> Result<(), String> {
    if base_dir.join("config.toml").is_file() {
        let doc = read_doc(base_dir)?;
        let legacy = doc
            .get("model_catalog_json")
            .and_then(|v| v.as_str())
            .is_some_and(|v| is_reference(base_dir, v))
            || doc
                .get("profiles")
                .and_then(|v| v.as_table_like())
                .is_some_and(|profiles| {
                    profiles.iter().any(|(_, p)| {
                        p.get("model_catalog_json")
                            .and_then(|v| v.as_str())
                            .is_some_and(|v| is_reference(base_dir, v))
                    })
                });
        if legacy {
            crate::modules::codex_account::reapply_experimental_model_policy_if_enabled(base_dir)?;
        }
    }
    clear_reference(base_dir)?;
    let path = base_dir.join(MANAGED_FILE);
    if path.is_file() {
        let mut changed = false;
        crate::modules::atomic_write::update_string_atomic(&path, |current| {
            let current = current.ok_or("受管模型目录已被移除")?;
            let decorated = decorate(current)?;
            changed = decorated != current;
            Ok(decorated)
        })?;
        if changed {
            crate::modules::codex_local_access::invalidate_codex_model_cache(base_dir)?;
        }
    }
    Ok(())
}

/// Supplement supported managed models without changing their set, order or other capabilities.
pub(crate) fn supplement(catalog: &mut Value) -> Result<bool, String> {
    let templates = crate::modules::codex_protocol::build_codex_client_models_response(
        &crate::modules::codex_protocol::managed_codex_model_ids(),
    );
    let models = catalog
        .get_mut("models")
        .and_then(Value::as_array_mut)
        .ok_or("受管模型目录缺少 models")?;
    let mut changed = false;
    for model in models {
        let Some(slug) = model.get("slug").and_then(Value::as_str) else {
            continue;
        };
        let template = templates["models"].as_array().and_then(|models| {
            models
                .iter()
                .find(|model| model["slug"].as_str() == Some(slug))
        });
        let Some(tier) = template
            .and_then(|model| model["service_tiers"].as_array())
            .and_then(|tiers| tiers.iter().find(|tier| tier["id"] == "ultrafast"))
        else {
            continue;
        };
        let Some(object) = model.as_object_mut() else {
            continue;
        };
        let tiers = object
            .entry("additional_speed_tiers")
            .or_insert_with(|| serde_json::json!([]))
            .as_array_mut()
            .ok_or("受管 additional_speed_tiers 格式错误")?;
        if !tiers.iter().any(|v| v == "ultrafast") {
            tiers.push(Value::String("ultrafast".into()));
            changed = true;
        }
        let tiers = object
            .entry("service_tiers")
            .or_insert_with(|| serde_json::json!([]))
            .as_array_mut()
            .ok_or("受管 service_tiers 格式错误")?;
        if !tiers.iter().any(|v| v["id"] == "ultrafast") {
            tiers.push(tier.clone());
            changed = true;
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    fn fixture() -> Value {
        json!({"models":[
            {"slug":"gpt-6.1-sol","instructions":"native","hidden":false,
             "context_window":800000,"auto_compact_token_limit":700000,
             "additional_speed_tiers":["fast"],"service_tiers":[{"id":"priority","name":"Native Fast"}]},
            {"slug":"custom-model","hidden":true,"service_tiers":[{"id":"priority"}]},
            {"slug":"gpt-6-astra","additional_speed_tiers":["fast","ultrafast"],
             "service_tiers":[{"id":"ultrafast","name":"Upstream Ultrafast"}]}
        ]})
    }

    #[test]
    fn capabilities_preserve_models_order_limits_and_existing_tiers() {
        let mut catalog = fixture();
        let original = catalog.clone();
        assert!(supplement(&mut catalog).unwrap());
        assert_eq!(catalog["models"].as_array().unwrap().len(), 3);
        assert_eq!(catalog["models"][1], original["models"][1]);
        assert_eq!(catalog["models"][2], original["models"][2]);
        let mut first = catalog["models"][0].clone();
        assert_eq!(first["service_tiers"][1]["id"], "ultrafast");
        assert_eq!(
            first["additional_speed_tiers"],
            json!(["fast", "ultrafast"])
        );
        first["service_tiers"] = original["models"][0]["service_tiers"].clone();
        first["additional_speed_tiers"] = original["models"][0]["additional_speed_tiers"].clone();
        assert_eq!(first, original["models"][0]);
        assert!(!supplement(&mut catalog).unwrap());
    }

    #[test]
    fn managed_catalog_upgrade_does_not_change_config_or_enable_management() {
        let dir = tempfile::tempdir().unwrap();
        let config = "model = 'custom-model'\nmodel_catalog_json = 'cockpit-model-catalog.json'\nservice_tier = 'default'\n";
        fs::write(dir.path().join("config.toml"), config).unwrap();
        fs::write(dir.path().join(MANAGED_FILE), fixture().to_string()).unwrap();
        prepare_managed_catalog(dir.path()).unwrap();
        let upgraded = fs::read(dir.path().join(MANAGED_FILE)).unwrap();
        let modified = fs::metadata(dir.path().join(MANAGED_FILE))
            .unwrap()
            .modified()
            .unwrap();
        prepare_managed_catalog(dir.path()).unwrap();
        assert_eq!(fs::read(dir.path().join(MANAGED_FILE)).unwrap(), upgraded);
        assert_eq!(
            fs::metadata(dir.path().join(MANAGED_FILE))
                .unwrap()
                .modified()
                .unwrap(),
            modified
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("config.toml")).unwrap(),
            config
        );
        assert!(!dir
            .path()
            .join(".cockpit-experimental-model-catalog-enabled")
            .exists());
        assert!(!dir.path().join(FILE_NAME).exists());
    }

    #[test]
    fn retired_speed_references_are_removed_without_creating_managed_catalog() {
        for absolute in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let reference = if absolute {
                dir.path().join(FILE_NAME).to_string_lossy().into_owned()
            } else {
                FILE_NAME.into()
            };
            fs::write(dir.path().join(FILE_NAME), fixture().to_string()).unwrap();
            fs::write(dir.path().join("config.toml"), format!(
                "model = 'gpt-6.1-sol'\nservice_tier = 'priority'\nmodel_catalog_json = '{reference}'\nprofile = 'work'\n[profiles.work]\nmodel_catalog_json = '{reference}'\n[profiles.custom]\nmodel_catalog_json = 'user.json'\n"
            )).unwrap();
            prepare_managed_catalog(dir.path()).unwrap();
            let doc = read_doc(dir.path()).unwrap();
            assert!(doc.get("model_catalog_json").is_none());
            assert!(doc["profiles"]["work"].get("model_catalog_json").is_none());
            assert_eq!(
                doc["profiles"]["custom"]["model_catalog_json"].as_str(),
                Some("user.json")
            );
            assert_eq!(doc["model"].as_str(), Some("gpt-6.1-sol"));
            assert_eq!(doc["service_tier"].as_str(), Some("priority"));
            assert!(!dir.path().join(FILE_NAME).exists());
            assert!(!dir.path().join(MANAGED_FILE).exists());
            let config = fs::read(dir.path().join("config.toml")).unwrap();
            prepare_managed_catalog(dir.path()).unwrap();
            assert_eq!(fs::read(dir.path().join("config.toml")).unwrap(), config);
        }
    }

    #[test]
    fn invalid_config_keeps_legacy_file_for_retry_and_instances_are_isolated() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        for dir in [first.path(), second.path()] {
            fs::write(dir.join(FILE_NAME), fixture().to_string()).unwrap();
            fs::write(dir.join("config.toml"), "invalid [").unwrap();
        }
        assert!(prepare_managed_catalog(first.path()).is_err());
        assert!(first.path().join(FILE_NAME).exists());
        fs::write(
            first.path().join("config.toml"),
            "model_catalog_json = 'user.json'\n",
        )
        .unwrap();
        fs::write(first.path().join("user.json"), "user-owned").unwrap();
        prepare_managed_catalog(first.path()).unwrap();
        assert!(!first.path().join(FILE_NAME).exists());
        assert_eq!(
            fs::read_to_string(first.path().join("user.json")).unwrap(),
            "user-owned"
        );
        assert_eq!(
            read_doc(first.path()).unwrap()["model_catalog_json"].as_str(),
            Some("user.json")
        );
        assert!(second.path().join(FILE_NAME).exists());
    }
}

#[cfg(test)]
#[path = "codex_model_catalog_capabilities_wire_tests.rs"]
mod wire_tests;
