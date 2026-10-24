//! Preserve Desktop policy while replacing only Cockpit's inference connection.
use crate::models::claude::{ClaudeAccount, ClaudeAuthMode};
use crate::modules::atomic_write;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static CONFIG_LOCK: Mutex<()> = Mutex::new(());
const OWNER_KEY: &str = "cockpitManagedGateway";

fn normalize_non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

pub(crate) fn build_provider_config(account: &ClaudeAccount) -> Result<Value, String> {
    if account.auth_mode != ClaudeAuthMode::DesktopGateway {
        return Err("账号不是 Claude Gateway 类型".to_string());
    }
    let connection_mode = crate::modules::claude_desktop_gateway::normalize_connection_mode(
        account.desktop_gateway_connection_mode.as_deref(),
    );
    let (base_url, api_key, auth_scheme) = if connection_mode == "local_mapping" {
        let endpoint = crate::modules::claude_desktop_gateway::ensure_gateway_for_account(account)?;
        (endpoint.base_url, endpoint.api_key, "bearer".to_string())
    } else {
        let api_key = account
            .api_key
            .as_deref()
            .and_then(|value| normalize_non_empty(Some(value)))
            .ok_or_else(|| "Claude Gateway 账号缺少 API Key".to_string())?;
        let base_url = account
            .api_base_url
            .as_deref()
            .and_then(|value| normalize_non_empty(Some(value)))
            .ok_or_else(|| "Claude Gateway 账号缺少 Base URL".to_string())?;
        let auth_scheme = crate::modules::claude_desktop_gateway::normalize_auth_scheme(
            account.desktop_gateway_auth_scheme.as_deref(),
        );
        (
            base_url,
            api_key.to_string(),
            if auth_scheme == "auto" {
                "bearer".to_string()
            } else {
                auth_scheme
            },
        )
    };
    let credential_kind = account
        .desktop_gateway_credential_kind
        .as_deref()
        .and_then(|value| normalize_non_empty(Some(value)))
        .unwrap_or_else(|| "static".to_string());
    if credential_kind != "static" {
        return Err("当前仅支持 static Gateway API Key".to_string());
    }
    let mut config = json!({
        "inferenceProvider": "gateway",
        "inferenceCredentialKind": "static",
        "inferenceGatewayBaseUrl": base_url,
        "inferenceGatewayApiKey": api_key,
        "inferenceGatewayAuthScheme": auth_scheme,
    });
    if let Some(models) = account
        .desktop_gateway_models
        .as_ref()
        .filter(|items| !items.is_empty())
    {
        let mapping_meta = crate::modules::claude_desktop_gateway::normalize_model_mappings(
            account.desktop_gateway_model_mappings.clone(),
        )
        .unwrap_or_default()
        .into_iter()
        .map(|mapping| (mapping.desktop_model.to_ascii_lowercase(), mapping))
        .collect::<BTreeMap<_, _>>();
        config["inferenceModels"] = Value::Array(
            models
                .iter()
                .filter_map(|model| {
                    let name = normalize_non_empty(Some(model))?;
                    let mut item = json!({ "name": name.clone() });
                    if let Some(mapping) = mapping_meta.get(&name.to_ascii_lowercase()) {
                        if let Some(label_override) = mapping
                            .label_override
                            .as_deref()
                            .and_then(|value| normalize_non_empty(Some(value)))
                        {
                            item["labelOverride"] = Value::String(label_override);
                        }
                        if let Some(supports_1m) = mapping.supports_1m {
                            item["supports1m"] = Value::Bool(supports_1m);
                        }
                    }
                    Some(item)
                })
                .collect(),
        );
    }
    Ok(config)
}

fn valid_id(id: &str) -> bool {
    let upper = id.to_ascii_uppercase();
    let reserved = matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.len() == 4
            && matches!(upper.as_bytes()[3], b'1'..=b'9'));
    !id.is_empty()
        && upper != "_META"
        && !reserved
        && id
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == b'-' || ch == b'_')
}

fn provider_path(dir: &Path, id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err("Claude configLibrary ID 非法，已取消写入".to_string());
    }
    Ok(dir.join(format!("{}.json", id)))
}

fn same_provider_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    // Windows can resolve differently cased IDs to the same existing file.
    // Compare resolved paths without changing the paths used for writes/deletes.
    #[cfg(windows)]
    {
        if let (Ok(left), Ok(right)) = (fs::canonicalize(left), fs::canonicalize(right)) {
            return left == right;
        }
    }
    false
}

fn is_gateway(config: &Value) -> bool {
    config
        .get("inferenceProvider")
        .and_then(Value::as_str)
        .is_some_and(|provider| provider.eq_ignore_ascii_case("gateway"))
}

fn is_connection_key(key: &str) -> bool {
    key == "inferenceProvider"
        || key == "inferenceCustomHeaders"
        || [
            "inferenceGateway",
            "inferenceCredential",
            "inferenceIdp",
            "inferenceAnthropic",
            "inferenceBedrock",
            "inferenceVertex",
            "inferenceFoundry",
        ]
        .iter()
        .any(|prefix| key.starts_with(prefix))
}

fn model_name(model: &Value) -> Option<&str> {
    model
        .as_str()
        .or_else(|| model.get("name").and_then(Value::as_str))
}

fn merge_provider_config(mut current: Value, generated: &Value) -> Result<Value, String> {
    let object = current
        .as_object_mut()
        .ok_or("Claude Gateway 配置必须是 JSON 对象")?;
    let old_models = object.remove("inferenceModels").unwrap_or(Value::Null);
    object.retain(|key, _| !is_connection_key(key));
    let generated = generated.as_object().ok_or("Claude Gateway 连接配置非法")?;
    object.extend(generated.clone());
    if let Some(models) = object
        .get_mut("inferenceModels")
        .and_then(Value::as_array_mut)
    {
        for model in models {
            let Some(name) = model_name(model) else {
                continue;
            };
            let previous = old_models.as_array().and_then(|items| {
                items.iter().find(|old| {
                    model_name(old).is_some_and(|old_name| old_name.eq_ignore_ascii_case(name))
                })
            });
            if let Some(mut previous) = previous.and_then(Value::as_object).cloned() {
                if let Some(overrides) = model.as_object() {
                    previous.extend(overrides.clone());
                    *model = Value::Object(previous);
                }
            }
        }
    }
    Ok(current)
}

#[derive(Clone)]
struct Change {
    path: PathBuf,
    before: Option<String>,
    after: Option<String>,
}

impl Change {
    fn read(path: PathBuf) -> Result<Self, String> {
        let before = match fs::read_to_string(&path) {
            Ok(content) => Some(content),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(format!(
                    "读取 Claude 配置失败: path={}, error={}",
                    path.display(),
                    error
                ))
            }
        };
        Ok(Self {
            path,
            after: before.clone(),
            before,
        })
    }

    fn json(&self) -> Result<Value, String> {
        let Some(content) = &self.before else {
            return Ok(json!({}));
        };
        // Do not include serde's input excerpt: configuration can contain credentials.
        let value: Value = serde_json::from_str(content)
            .map_err(|_| format!("Claude 配置 JSON 非法，已取消写入: {}", self.path.display()))?;
        if !value.is_object() {
            return Err(format!(
                "Claude 配置必须是 JSON 对象: {}",
                self.path.display()
            ));
        }
        Ok(value)
    }

    fn set_json(&mut self, value: &Value) -> Result<(), String> {
        self.after = Some(
            serde_json::to_string_pretty(value)
                .map_err(|_| "序列化 Claude 配置失败".to_string())?,
        );
        Ok(())
    }
}

fn commit(changes: &[Change]) -> Result<(), String> {
    // Include read-only sources in the preflight so edits made while planning
    // cannot be silently copied from an outdated snapshot and then deleted.
    for change in changes {
        if Change::read(change.path.clone())?.before != change.before {
            return Err(format!(
                "Claude 配置已被其他操作修改，已取消写入: {}",
                change.path.display()
            ));
        }
    }
    for (index, change) in changes.iter().enumerate() {
        if change.before == change.after {
            continue;
        }
        let result = atomic_write::replace_secret_string_if_matches(
            &change.path,
            change.before.as_deref(),
            change.after.as_deref(),
        )
        .and_then(|replaced| {
            if replaced {
                Ok(())
            } else {
                Err(format!(
                    "Claude 配置已被其他操作修改，已取消写入: {}",
                    change.path.display()
                ))
            }
        });
        if let Err(mut error) = result {
            for previous in changes[..index].iter().rev() {
                if previous.before == previous.after {
                    continue;
                }
                match atomic_write::replace_secret_string_if_matches(
                    &previous.path,
                    previous.after.as_deref(),
                    previous.before.as_deref(),
                ) {
                    Ok(true) => {}
                    _ => error.push_str(&format!(
                        "；无法回滚配置（保留较新的内容）: {}",
                        previous.path.display()
                    )),
                }
            }
            return Err(error);
        }
    }
    Ok(())
}

fn entries(meta: &Value) -> Result<Vec<Value>, String> {
    match meta.get("entries") {
        None => Ok(Vec::new()),
        Some(Value::Array(items))
            if items.iter().all(|item| {
                item.is_object() && item.get("id").and_then(Value::as_str).is_some()
            }) =>
        {
            Ok(items.clone())
        }
        _ => Err("Claude configLibrary entries 非法，已取消写入".to_string()),
    }
}

fn managed_ids(entries: &[Value], known_ids: &BTreeSet<String>) -> BTreeSet<String> {
    let mut ids = known_ids.clone();
    ids.extend(
        entries
            .iter()
            .filter(|entry| entry.get(OWNER_KEY) == Some(&Value::Bool(true)))
            .filter_map(|entry| entry.get("id").and_then(Value::as_str).map(str::to_owned)),
    );
    ids
}

fn retire(change: &mut Change, obsolete: &mut Vec<Change>) -> Result<(), String> {
    // Older writers created plaintext backups. Remove only backups of entries
    // whose ownership and provider type have already been checked.
    let mut backup = Change::read(change.path.with_extension("json.bak"))?;
    backup.after = None;
    obsolete.push(backup);
    change.after = None;
    Ok(())
}

fn deployment_changes(paths: &[PathBuf], mode: &str) -> Result<Vec<Change>, String> {
    paths
        .iter()
        .map(|path| {
            let mut change = Change::read(path.clone())?;
            let mut config = change.json()?;
            config["deploymentMode"] = json!(mode);
            change.set_json(&config)?;
            Ok(change)
        })
        .collect()
}

/// `known_ids` comes from persisted Gateway accounts, allowing safe migration of
/// pre-marker entries. Provider type alone is never proof of Cockpit ownership.
pub(crate) fn apply(
    dir: &Path,
    deployment_paths: &[PathBuf],
    config_id: &str,
    entry_name: &str,
    generated: &Value,
    known_ids: &BTreeSet<String>,
) -> Result<(), String> {
    let _guard = CONFIG_LOCK
        .lock()
        .map_err(|_| "Claude Gateway 配置锁已损坏")?;
    let mut meta_change = Change::read(dir.join("_meta.json"))?;
    let mut meta = meta_change.json()?;
    let mut entries = entries(&meta)?;
    let owned = managed_ids(&entries, known_ids);
    let mut target = Change::read(provider_path(dir, config_id)?)?;
    let target_config = target.json()?;
    if target.before.is_some() && (!owned.contains(config_id) || !is_gateway(&target_config)) {
        return Err("目标 Claude 配置不属于 Cockpit Gateway，已取消写入".to_string());
    }
    let source_change = match meta.get("appliedId") {
        Some(Value::String(id)) => {
            let source = Change::read(provider_path(dir, id)?)?;
            if source.before.is_none() {
                return Err("Claude 当前应用的配置不存在，已取消写入".to_string());
            }
            Some(source)
        }
        None => None,
        _ => return Err("Claude configLibrary appliedId 非法，已取消写入".to_string()),
    };
    let source = match &source_change {
        Some(source) => source.json()?,
        None => target_config,
    };
    target.set_json(&merge_provider_config(source, generated)?)?;

    let mut obsolete = Vec::new();
    let mut removed = BTreeSet::new();
    for id in &owned {
        let path = provider_path(dir, id)?;
        if same_provider_path(&path, &target.path) {
            continue;
        }
        if obsolete
            .iter()
            .any(|change: &Change| same_provider_path(&change.path, &path))
        {
            removed.insert(id.clone());
            continue;
        }
        let mut change = match &source_change {
            Some(source) if same_provider_path(&source.path, &path) => source.clone(),
            _ => Change::read(path)?,
        };
        if change.before.is_none() || is_gateway(&change.json()?) {
            retire(&mut change, &mut obsolete)?;
            obsolete.push(change);
            removed.insert(id.clone());
        }
    }
    entries.retain(|entry| {
        !entry
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| removed.contains(id))
    });
    if let Some(entry) = entries.iter_mut().find(|entry| {
        entry
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| provider_path(dir, id).ok())
            .is_some_and(|path| same_provider_path(&path, &target.path))
    }) {
        entry["id"] = json!(config_id);
        entry["name"] = json!(entry_name);
        entry[OWNER_KEY] = json!(true);
    } else {
        entries.push(json!({ "id": config_id, "name": entry_name, OWNER_KEY: true }));
    }
    meta["entries"] = json!(entries);
    meta["appliedId"] = json!(config_id);
    meta_change.set_json(&meta)?;
    // Prepare and validate every file before writing. Publish a complete provider
    // before selecting it; retire old credentials only after activating the new one.
    let modes = deployment_changes(deployment_paths, "3p")?;
    let mut changes = vec![target, meta_change];
    // A no-op provider update must still retire a pre-existing credential backup.
    // Changed providers already remove their .bak through the secret writer.
    if changes[0].before == changes[0].after {
        let mut backup = Change::read(changes[0].path.with_extension("json.bak"))?;
        backup.after = None;
        changes.push(backup);
    }
    changes.extend(modes);
    if let Some(source) = source_change {
        if !same_provider_path(&source.path, &changes[0].path)
            && !obsolete
                .iter()
                .any(|change| same_provider_path(&change.path, &source.path))
        {
            changes.push(source);
        }
    }
    changes.extend(obsolete);
    commit(&changes)
}

pub(crate) fn remove_managed(
    dir: &Path,
    deployment_paths: &[PathBuf],
    known_ids: &BTreeSet<String>,
) -> Result<(), String> {
    let _guard = CONFIG_LOCK
        .lock()
        .map_err(|_| "Claude Gateway 配置锁已损坏")?;
    let mut meta_change = Change::read(dir.join("_meta.json"))?;
    let mut meta = meta_change.json()?;
    let mut entries = entries(&meta)?;
    let owned = managed_ids(&entries, known_ids);
    let mut removed = BTreeSet::new();
    let mut obsolete = Vec::new();
    for id in &owned {
        let path = provider_path(dir, id)?;
        if obsolete
            .iter()
            .any(|change: &Change| same_provider_path(&change.path, &path))
        {
            removed.insert(id.clone());
            continue;
        }
        let mut change = Change::read(path)?;
        if change.before.is_none() || is_gateway(&change.json()?) {
            retire(&mut change, &mut obsolete)?;
            obsolete.push(change);
            removed.insert(id.clone());
        }
    }
    entries.retain(|entry| {
        !entry
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| removed.contains(id))
    });
    if meta
        .get("appliedId")
        .and_then(Value::as_str)
        .is_some_and(|id| removed.contains(id))
    {
        meta.as_object_mut().unwrap().remove("appliedId");
    }
    if !removed.is_empty() && meta_change.before.is_some() {
        meta["entries"] = json!(entries);
        meta_change.set_json(&meta)?;
    }
    let mut changes = deployment_changes(deployment_paths, "1p")?;
    changes.push(meta_change);
    changes.extend(obsolete);
    commit(&changes)
}

#[cfg(test)]
#[path = "claude_desktop_gateway_config_tests.rs"]
mod tests;
