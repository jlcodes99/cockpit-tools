// Codex 账号模块：官方实验性上下文管理开关。
// 仅修改 config.toml 中的 features.context_management.experimental_mode，
// 不参与模型目录、API Service 网关或上下文窗口/压缩阈值的配置。
use toml_edit::{Item, Value};

const CODEX_FEATURES_KEY: &str = "features";
const CODEX_CONTEXT_MANAGEMENT_KEY: &str = "context_management";
const CODEX_CONTEXT_MANAGEMENT_EXPERIMENTAL_MODE_KEY: &str = "experimental_mode";

pub(crate) fn read_context_management_experimental_mode_from_doc(doc: &Document) -> bool {
    doc.get(CODEX_FEATURES_KEY)
        .and_then(Item::as_table_like)
        .and_then(|features| features.get(CODEX_CONTEXT_MANAGEMENT_KEY))
        .and_then(Item::as_table_like)
        .and_then(|context_management| {
            context_management.get(CODEX_CONTEXT_MANAGEMENT_EXPERIMENTAL_MODE_KEY)
        })
        .and_then(Item::as_bool)
        .unwrap_or(false)
}

fn read_context_management_config(path: &Path) -> Result<(Document, bool), String> {
    let existed = path.exists();
    let doc = crate::modules::codex_config_format::load_codex_config_doc(path)?;
    Ok((doc, existed))
}

fn new_context_management_item(features_is_inline: bool) -> Item {
    if features_is_inline {
        Item::Value(Value::InlineTable(toml_edit::InlineTable::new()))
    } else {
        toml_edit::table()
    }
}

fn set_context_management_experimental_mode(
    doc: &mut Document,
    enabled: bool,
) -> Result<bool, String> {
    let features_is_inline = doc
        .get(CODEX_FEATURES_KEY)
        .and_then(Item::as_inline_table)
        .is_some();
    if doc.get(CODEX_FEATURES_KEY).is_none() {
        doc[CODEX_FEATURES_KEY] = toml_edit::table();
    }

    let features = doc
        .get_mut(CODEX_FEATURES_KEY)
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| "Codex config.toml 中 features 不是合法表结构".to_string())?;
    if features.get(CODEX_CONTEXT_MANAGEMENT_KEY).is_none() {
        features.insert(
            CODEX_CONTEXT_MANAGEMENT_KEY,
            new_context_management_item(features_is_inline),
        );
    }

    let context_management = features
        .get_mut(CODEX_CONTEXT_MANAGEMENT_KEY)
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| {
            "Codex config.toml 中 features.context_management 不是合法表结构".to_string()
        })?;
    let previous = context_management
        .get(CODEX_CONTEXT_MANAGEMENT_EXPERIMENTAL_MODE_KEY)
        .and_then(Item::as_bool);
    if previous == Some(enabled) {
        return Ok(false);
    }
    context_management.insert(
        CODEX_CONTEXT_MANAGEMENT_EXPERIMENTAL_MODE_KEY,
        value(enabled),
    );
    Ok(true)
}

fn remove_context_management_experimental_mode(doc: &mut Document) -> Result<bool, String> {
    let Some(features) = doc
        .get_mut(CODEX_FEATURES_KEY)
        .and_then(Item::as_table_like_mut)
    else {
        return Ok(false);
    };
    let Some(context_management) = features
        .get_mut(CODEX_CONTEXT_MANAGEMENT_KEY)
        .and_then(Item::as_table_like_mut)
    else {
        if features.get(CODEX_CONTEXT_MANAGEMENT_KEY).is_some() {
            return Err(
                "Codex config.toml 中 features.context_management 不是合法表结构".to_string(),
            );
        }
        return Ok(false);
    };
    Ok(context_management
        .remove(CODEX_CONTEXT_MANAGEMENT_EXPERIMENTAL_MODE_KEY)
        .is_some())
}

/// 保存官方 Codex 实验性上下文管理开关。
///
/// 缺失配置且目标为 false 时不创建 config.toml，保证未开启时完全回退官方默认行为。
pub fn save_context_management_for_base_dir(
    base_dir: &Path,
    enabled: bool,
) -> Result<CodexQuickConfig, String> {
    let _guard = CODEX_ACCOUNT_MUTATION_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let _lease = try_acquire_profile_mutation_lease(base_dir, "context-management-toggle")?;
    let config_path = get_config_toml_path(base_dir);
    let (mut doc, config_exists) = read_context_management_config(&config_path)?;
    let changed = if enabled {
        set_context_management_experimental_mode(&mut doc, true)?
    } else {
        remove_context_management_experimental_mode(&mut doc)?
    };

    if changed {
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("创建 Codex 配置目录失败: {}", error))?;
        }
        let content = crate::modules::codex_config_format::codex_config_doc_to_string(&mut doc);
        crate::modules::codex_config_format::write_codex_config_toml_atomic(&config_path, &content)
            .map_err(|error| format!("写入 Codex config.toml 失败: {}", error))?;
    } else if !config_exists && !enabled {
        return read_quick_config_from_config_toml(base_dir);
    }

    read_quick_config_from_config_toml(base_dir)
}
