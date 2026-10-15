use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::models::codex::{CodexExperimentalModelDefinition, CodexQuickConfig};
use crate::modules;

pub(super) const PENDING_MODEL_CATALOG_FILE: &str = ".cockpit-pending-model-catalog.json";

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct PendingModelCatalog {
    pub(super) enabled: bool,
    models: Vec<CodexExperimentalModelDefinition>,
    default_model_id: Option<String>,
}

impl PendingModelCatalog {
    pub(super) fn apply_to_view(&self, config: &mut CodexQuickConfig) {
        config.experimental_model_catalog_enabled = self.enabled;
        config.experimental_model_catalog_models = self.models.clone();
        config.experimental_model_catalog_default_model_id = self.default_model_id.clone();
    }
}

pub(super) fn read_pending_model_catalog(
    profile: &Path,
) -> Result<Option<PendingModelCatalog>, String> {
    match std::fs::read(profile.join(PENDING_MODEL_CATALOG_FILE)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| format!("读取待生效模型配置失败: {}", error)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("读取待生效模型配置失败: {}", error)),
    }
}

pub(super) fn save_pending_model_catalog(
    profile: &Path,
    enabled: bool,
    models: Vec<CodexExperimentalModelDefinition>,
    default_model_id: Option<String>,
) -> Result<CodexQuickConfig, String> {
    let mut config = modules::codex_account::read_quick_config_from_config_toml(profile)?;
    if !config.experimental_model_catalog_available {
        return Err("当前实例不支持受管模型目录".to_string());
    }
    let models = if enabled {
        modules::codex_account::normalize_experimental_model_definitions(models)?
    } else {
        models
    };
    let default_model_id = default_model_id.filter(|id| {
        enabled
            && models
                .iter()
                .any(|model| model.model_id.eq_ignore_ascii_case(id))
    });
    let draft = PendingModelCatalog {
        enabled,
        models,
        default_model_id,
    };
    let content = serde_json::to_string_pretty(&draft).map_err(|error| error.to_string())?;
    modules::atomic_write::write_string_atomic(
        &profile.join(PENDING_MODEL_CATALOG_FILE),
        &content,
    )?;
    draft.apply_to_view(&mut config);
    Ok(config)
}

pub(super) fn restore_pending_model_catalog(
    profile: &Path,
    previous: Option<&PendingModelCatalog>,
) -> Result<(), String> {
    let path = profile.join(PENDING_MODEL_CATALOG_FILE);
    match previous {
        Some(previous) => modules::atomic_write::write_string_atomic(
            &path,
            &serde_json::to_string_pretty(previous).map_err(|error| error.to_string())?,
        ),
        None => modules::atomic_write::remove_file_locked(&path).map(|_| ()),
    }
}

pub(super) fn apply_pending_model_catalog(profile: &Path) -> Result<(), String> {
    let Some(draft) = read_pending_model_catalog(profile)? else {
        return Ok(());
    };
    modules::codex_account::save_model_catalog_for_base_dir_preserving_context(
        profile,
        draft.enabled,
        draft.models,
        draft.default_model_id,
    )?;
    modules::atomic_write::remove_file_locked(&profile.join(PENDING_MODEL_CATALOG_FILE))?;
    Ok(())
}
