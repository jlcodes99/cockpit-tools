use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

const REASONING_ENCRYPTED_CONTENT_INCLUDE: &str = "reasoning.encrypted_content";
const CODEX_AUTO_REVIEW_MODEL_ID: &str = "codex-auto-review";
const CODEX_RESERVE_MODEL_ID: &str = "gpt-reserve";
const CODEX_RESERVE_TEMPLATE_MODEL_ID: &str = "gpt-5.6-luna";
/// 额度兜底模型对客户端展示的名称（跟随官方 5.6 命名）。
pub(crate) const CODEX_RESERVE_DISPLAY_NAME: &str = "GPT-5.6 Reserve";
const CODEX_MODEL_CATALOG_TEMPLATE_SLUG: &str = "gpt-5.5";
const CODEX_INPUT_ITEM_ID_LIMIT: usize = 64;
/// Codex 客户端模型目录统一的压缩格式哈希（comp_hash）。
///
/// 各模型带不同 comp_hash 时，客户端在切换模型后会触发 PreTurn 自动压缩
/// （`run_auto_compact{reason=CompHashChanged}`），与当前 token 用量无关；
/// 统一为最新官方值即可避免混合模型目录内切换模型被强制重建上下文。
pub(crate) const CODEX_CLIENT_COMP_HASH: &str = "3000";
const CODEX_CLIENT_MODEL_TEMPLATES_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../sidecars/cockpit-cliproxy/third_party/CLIProxyAPI/internal/registry/models/codex_client_models.json"
));
const DEFAULT_CONTEXT_WINDOW: i64 = 272_000;
/// 未知 / 自定义 Codex 模型的兜底上限。
///
/// 兜底不再声明 1M 上限：1M 级上下文属于具体模型的真实能力值，只能来自模型目录或
/// 用户显式覆盖；兜底一律与 `DEFAULT_CONTEXT_WINDOW` 保持一致，避免客户端按 1M
/// 上报所有第三方模型。
const DEFAULT_MAX_CONTEXT_WINDOW: i64 = DEFAULT_CONTEXT_WINDOW;
const LOCAL_PROXY_BYPASS_HOSTS: [&str; 5] =
    ["127.0.0.1", "127.0.0.0/8", "localhost", "::1", "::1/128"];

/// Codex 客户端「上下文 / 压缩」统一口径：压缩阈值固定取上下文窗口的 90%。
pub(crate) const AUTO_COMPACT_RATIO_PERCENT: i64 = 90;

/// 按统一口径派生自动压缩阈值：`context_window * 90 / 100`（整数除法，向下取整）。
///
/// 例：`516000 -> 464400`、`1000000 -> 900000`、`272000 -> 244800`、`128000 -> 115200`。
/// 非正输入返回 0，调用方据此判断「无法派生」。
pub(crate) fn derived_auto_compact_token_limit(context_window: i64) -> i64 {
    if context_window <= 0 {
        return 0;
    }
    context_window.saturating_mul(AUTO_COMPACT_RATIO_PERCENT) / 100
}

/// 为单个模型条目补齐压缩阈值：声明了 `context_window` 但缺少、非正或大于等于窗口的
/// 压缩阈值时，按统一口径派生写入；合法的显式阈值保持原样。
fn ensure_model_auto_compact_limit(object: &mut Map<String, Value>) -> bool {
    let Some(context_window) = object
        .get("context_window")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0)
    else {
        return false;
    };
    let existing = object
        .get("auto_compact_token_limit")
        .and_then(Value::as_i64)
        .filter(|value| *value > 0);
    // 已声明的压缩阈值必须小于上下文窗口，否则视为无效并重新派生。
    if existing.is_some_and(|value| value < context_window) {
        return false;
    }
    object.insert(
        "auto_compact_token_limit".to_string(),
        json!(derived_auto_compact_token_limit(context_window)),
    );
    true
}

/// 为整个 Codex 客户端模型目录补齐压缩阈值（统一口径的收口点）。
///
/// 只要条目声明了 `context_window > 0`，就必定带 `auto_compact_token_limit`；
/// 内置模板、`gpt-reserve`、模板回落条目与外部覆盖都不再留空。
pub(crate) fn ensure_client_model_auto_compact_limits(catalog: &mut Value) -> bool {
    let Some(models) = catalog.get_mut("models").and_then(Value::as_array_mut) else {
        return false;
    };
    let mut changed = false;
    for model in models {
        if let Some(object) = model.as_object_mut() {
            changed |= ensure_model_auto_compact_limit(object);
        }
    }
    changed
}

pub fn merge_local_no_proxy(raw: &str) -> String {
    let mut seen = HashSet::new();
    let mut items = Vec::new();

    for item in raw.split(',') {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            continue;
        }
        if seen.insert(trimmed.to_ascii_lowercase()) {
            items.push(trimmed.to_string());
        }
    }

    for host in LOCAL_PROXY_BYPASS_HOSTS {
        if seen.insert(host.to_ascii_lowercase()) {
            items.push(host.to_string());
        }
    }

    items.join(",")
}

pub fn is_codex_client_models_request(target: &str) -> bool {
    let Some(query) = target.split_once('?').map(|(_, query)| query) else {
        return false;
    };

    query.split('&').any(|pair| {
        pair.split_once('=')
            .map(|(key, _)| key)
            .unwrap_or(pair)
            .eq_ignore_ascii_case("client_version")
    })
}

pub fn build_codex_client_models_response(model_ids: &[String]) -> Value {
    let models = model_ids
        .iter()
        .enumerate()
        .map(|(index, model_id)| build_codex_client_model(model_id, index))
        .collect::<Vec<_>>();

    json!({ "models": models })
}

pub fn build_codex_client_models_response_with_model_definitions(
    definitions: &[(String, String)],
) -> Value {
    let definitions = definitions
        .iter()
        .map(|(model_id, display_name)| (model_id.clone(), display_name.clone(), None))
        .collect::<Vec<_>>();
    build_codex_client_models_response_with_model_definitions_and_reasoning(&definitions)
}

pub fn build_codex_client_models_response_with_model_definitions_and_reasoning(
    definitions: &[(String, String, Option<Vec<String>>)],
) -> Value {
    let models = definitions
        .iter()
        .enumerate()
        .map(|(index, (model_id, display_name, reasoning_efforts))| {
            let mut model = build_codex_client_model(model_id, index);
            if let Some(object) = model.as_object_mut() {
                object.insert(
                    "display_name".to_string(),
                    Value::String(display_name.clone()),
                );
                object.insert(
                    "description".to_string(),
                    Value::String(display_name.clone()),
                );
                if let Some(reasoning_efforts) = reasoning_efforts {
                    apply_reasoning_effort_override(object, reasoning_efforts);
                }
            }
            model
        })
        .collect::<Vec<_>>();
    json!({ "models": models })
}

pub fn apply_model_context_overrides(
    catalog: &mut Value,
    definitions: &[(String, Option<i64>, Option<i64>)],
) {
    let Some(models) = catalog.get_mut("models").and_then(Value::as_array_mut) else {
        return;
    };
    for model in models {
        let Some(slug) = model
            .get("slug")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let Some((_, context_window, auto_compact_token_limit)) = definitions
            .iter()
            .find(|(model_id, _, _)| model_id.trim().eq_ignore_ascii_case(slug))
        else {
            continue;
        };
        let Some(object) = model.as_object_mut() else {
            continue;
        };
        // 上下文窗口与压缩阈值必须成对出现：只给了压缩阈值属于半边配置，直接忽略，
        // 保持「跟随官方」语义（客户端继续使用官方目录值）。
        let Some(context_window) = context_window.filter(|value| *value > 0) else {
            continue;
        };
        object.insert("context_window".to_string(), json!(context_window));
        object.insert("max_context_window".to_string(), json!(context_window));
        let auto_compact_token_limit = auto_compact_token_limit
            .filter(|value| *value > 0 && *value < context_window)
            .unwrap_or_else(|| derived_auto_compact_token_limit(context_window));
        object.insert(
            "auto_compact_token_limit".to_string(),
            json!(auto_compact_token_limit),
        );
    }
}

/// Offer Reserve for explicit selection regardless of current account quota.
/// Luna is a capability fallback only; dispatch still checks account eligibility.
pub(crate) fn ensure_codex_reserve_fallback(catalog: &mut Value) {
    let Some(models) = catalog.get_mut("models").and_then(Value::as_array_mut) else {
        return;
    };
    if let Some(reserve) = models.iter_mut().find(|model| {
        model["slug"]
            .as_str()
            .is_some_and(|slug| slug.eq_ignore_ascii_case(CODEX_RESERVE_MODEL_ID))
    }) {
        reserve["visibility"] = json!("list");
        return;
    }
    let reserve = build_codex_client_model(CODEX_RESERVE_MODEL_ID, models.len());
    models.push(reserve);
}

fn apply_reasoning_effort_override(object: &mut Map<String, Value>, efforts: &[String]) {
    let levels = object
        .get("supported_reasoning_levels")
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| {
            codex_client_model_template("gpt-5.6-sol")
                .0
                .get("supported_reasoning_levels")
                .and_then(Value::as_array)
                .cloned()
        })
        .unwrap_or_default();
    let selected = efforts
        .iter()
        .filter_map(|effort| {
            levels
                .iter()
                .find(|level| level.get("effort").and_then(Value::as_str) == Some(effort))
                .cloned()
        })
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return;
    }
    object.insert(
        "supported_reasoning_levels".to_string(),
        Value::Array(selected.clone()),
    );
    let current_default = object
        .get("default_reasoning_level")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !selected.iter().any(|level| {
        level.get("effort").and_then(Value::as_str) == Some(current_default)
    }) {
        if let Some(first) = selected
            .first()
            .and_then(|level| level.get("effort"))
            .and_then(Value::as_str)
        {
            object.insert(
                "default_reasoning_level".to_string(),
                Value::String(first.to_string()),
            );
        }
    }
}

pub(crate) fn managed_codex_model_ids() -> Vec<String> {
    let catalog = codex_client_model_catalog();
    let overrides = catalog.get("model_overrides").and_then(Value::as_array);
    let models = overrides
        .filter(|models| !models.is_empty())
        .or_else(|| catalog.get("models").and_then(Value::as_array));

    let mut model_ids = models
        .into_iter()
        .flatten()
        .filter(|model| {
            overrides.is_some_and(|overrides| !overrides.is_empty())
                || (model
                    .get("use_responses_lite")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                    && !model
                        .get("visibility")
                        .and_then(Value::as_str)
                        .is_some_and(|visibility| visibility.eq_ignore_ascii_case("hide")))
        })
        .filter_map(|model| model.get("slug").and_then(Value::as_str))
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();

    // 官方推荐集把 GPT-6 家族排在最前，顺序固定为 Astra → Sol → Luna；
    // 只移动已存在的条目，不插入目录里没有的模型。
    for (offset, model_id) in ["gpt-6-astra", "gpt-6-sol", "gpt-6-luna"]
        .iter()
        .enumerate()
    {
        if let Some(index) = model_ids
            .iter()
            .position(|model| model.eq_ignore_ascii_case(model_id))
        {
            if index > offset {
                let model = model_ids.remove(index);
                model_ids.insert(offset, model);
            }
        }
    }

    model_ids
}

pub fn normalize_responses_body_for_codex(body: &mut Value) -> bool {
    normalize_responses_body_for_codex_with_lite(body, false)
}

pub fn normalize_responses_body_for_codex_with_lite(
    body: &mut Value,
    force_responses_lite: bool,
) -> bool {
    let responses_lite = force_responses_lite
        || body
            .get("model")
            .and_then(Value::as_str)
            .is_some_and(codex_model_uses_responses_lite);
    let Some(obj) = body.as_object_mut() else {
        return false;
    };

    let mut changed = false;
    changed |= ensure_string_field(obj, "instructions", "");
    changed |= ensure_bool_field(obj, "stream", true);
    changed |= ensure_bool_field(obj, "store", false);
    changed |= ensure_bool_field(obj, "parallel_tool_calls", !responses_lite);
    changed |= ensure_reasoning_include(obj);
    changed |= normalize_responses_input(obj);
    changed |= normalize_codex_builtin_tools(obj);
    if responses_lite {
        changed |= filter_responses_lite_tools_in_object(obj);
    }
    changed |= remove_unsupported_responses_fields(obj);

    changed
}

pub(crate) fn codex_model_uses_responses_lite(model_id: &str) -> bool {
    if model_id.trim().eq_ignore_ascii_case(CODEX_RESERVE_MODEL_ID) {
        return codex_client_model_template(CODEX_RESERVE_MODEL_ID).0["use_responses_lite"]
            .as_bool()
            .unwrap_or(false);
    }
    let catalog = codex_client_model_catalog();
    ["model_overrides", "models"]
        .into_iter()
        .filter_map(|key| catalog.get(key).and_then(Value::as_array))
        .any(|models| {
            models.iter().any(|model| {
                model
                    .get("slug")
                    .and_then(Value::as_str)
                    .is_some_and(|slug| slug.eq_ignore_ascii_case(model_id.trim()))
                    && model
                        .get("use_responses_lite")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
            })
        })
}

pub(crate) fn filter_responses_lite_tools(body: &mut Value) -> bool {
    body.as_object_mut()
        .is_some_and(filter_responses_lite_tools_in_object)
}

fn responses_lite_tool_allowed(tool: &Value) -> bool {
    match tool
        .get("type")
        .and_then(Value::as_str)
        .map(|tool_type| tool_type.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("function" | "custom") => true,
        Some("tool_search") => tool
            .get("execution")
            .and_then(Value::as_str)
            .is_some_and(|execution| execution.trim().eq_ignore_ascii_case("client")),
        _ => false,
    }
}

fn filter_responses_lite_tool_array(value: &mut Value) -> (bool, bool) {
    let Some(tools) = value.as_array_mut() else {
        return (false, false);
    };
    let before = tools.len();
    tools.retain(responses_lite_tool_allowed);
    (tools.len() != before, !tools.is_empty())
}

fn filter_responses_lite_tool_choice(choice: &mut Value) -> (bool, bool) {
    if let Some(choice_name) = choice.as_str() {
        let valid = matches!(
            choice_name.trim().to_ascii_lowercase().as_str(),
            "auto" | "none" | "required"
        );
        return (false, valid);
    }

    let Some(choice_object) = choice.as_object_mut() else {
        return (false, false);
    };
    let choice_type = choice_object
        .get("type")
        .and_then(Value::as_str)
        .map(|value| value.trim().to_ascii_lowercase());

    match choice_type.as_deref() {
        Some("function" | "custom") => return (false, true),
        Some("tool_search") => {
            let client_executed = choice_object
                .get("execution")
                .and_then(Value::as_str)
                .is_some_and(|execution| execution.trim().eq_ignore_ascii_case("client"));
            return (false, client_executed);
        }
        _ => {}
    }

    if choice_type.as_deref() != Some("allowed_tools") {
        return (false, false);
    }

    let mut changed = false;
    let mut has_allowed_tools = false;
    for key in ["tools", "allowed_tools"] {
        if let Some(value) = choice_object.get_mut(key) {
            let (value_changed, value_has_allowed_tools) = filter_responses_lite_tool_array(value);
            changed |= value_changed;
            has_allowed_tools |= value_has_allowed_tools;
        }
    }
    (changed, has_allowed_tools)
}

fn filter_responses_lite_tools_in_object(object: &mut Map<String, Value>) -> bool {
    let mut changed = false;

    if let Some(tools) = object.get_mut("tools") {
        changed |= filter_responses_lite_tool_array(tools).0;
    }

    let remove_tool_choice = object
        .get_mut("tool_choice")
        .map(|choice| {
            let (choice_changed, choice_valid) = filter_responses_lite_tool_choice(choice);
            changed |= choice_changed;
            !choice_valid
        })
        .unwrap_or(false);
    if remove_tool_choice {
        object.remove("tool_choice");
        changed = true;
    }

    if let Some(Value::Array(input)) = object.get_mut("input") {
        let before = input.len();
        input.retain_mut(|item| {
            let Some(item_object) = item.as_object_mut() else {
                return true;
            };
            if !item_object
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|item_type| item_type.eq_ignore_ascii_case("additional_tools"))
            {
                return true;
            }
            changed |= filter_responses_lite_tools_in_object(item_object);
            item_object
                .get("tools")
                .and_then(Value::as_array)
                .is_some_and(|tools| !tools.is_empty())
        });
        changed |= input.len() != before;
    }

    if let Some(Value::Object(response)) = object.get_mut("response") {
        changed |= filter_responses_lite_tools_in_object(response);
    }

    changed
}

fn build_codex_client_model(model_id: &str, index: usize) -> Value {
    let visibility = if matches!(
        model_id,
        CODEX_AUTO_REVIEW_MODEL_ID
            | "gpt-image-2"
            | "gpt-image-2.5"
            | "grok-imagine-image"
            | "grok-imagine-video"
            | "grok-imagine-image-quality"
    ) {
        "hide"
    } else {
        "list"
    };

    let (mut model, is_catalog_model) = codex_client_model_template(model_id);
    let object = model
        .as_object_mut()
        .expect("Codex client model template should be a JSON object");
    object.insert("slug".to_string(), Value::String(model_id.to_string()));
    if model_id.trim().eq_ignore_ascii_case(CODEX_RESERVE_MODEL_ID) {
        object.insert(
            "display_name".to_string(),
            json!(CODEX_RESERVE_DISPLAY_NAME),
        );
        object.insert("visibility".to_string(), json!("list"));
    }
    if !is_catalog_model {
        let display_name = display_name_for_model(model_id);
        object.insert(
            "display_name".to_string(),
            Value::String(display_name.clone()),
        );
        object.insert("description".to_string(), Value::String(display_name));
        object.insert("context_window".to_string(), json!(DEFAULT_CONTEXT_WINDOW));
        object.insert(
            "max_context_window".to_string(),
            json!(DEFAULT_MAX_CONTEXT_WINDOW),
        );
        object.insert(
            "auto_compact_token_limit".to_string(),
            json!(derived_auto_compact_token_limit(DEFAULT_CONTEXT_WINDOW)),
        );
        object.insert("priority".to_string(), json!(1000 + index));
        object.insert(
            "additional_speed_tiers".to_string(),
            Value::Array(Vec::new()),
        );
        object.insert("service_tiers".to_string(), Value::Array(Vec::new()));
        inherit_routed_gpt_capabilities(object, model_id);
        // Grok 平台模型由第三方 executor 承接，官方模型目录没有对应条目。
        // 缺少 multi_agent_version 时 Codex 客户端不会为这些模型注入子智能体
        // 协作工具（spawn/send/wait 等），会话里就表现为「找不到协作工具」。
        if is_grok_codex_model(model_id) {
            object.insert(
                "multi_agent_version".to_string(),
                json!(GROK_MULTI_AGENT_VERSION),
            );
            object.insert(
                "minimal_client_version".to_string(),
                json!(GROK_MINIMAL_CLIENT_VERSION),
            );
        }
    }
    if visibility == "hide" || !object.contains_key("visibility") {
        object.insert(
            "visibility".to_string(),
            Value::String(visibility.to_string()),
        );
    }
    object.insert("supported_in_api".to_string(), Value::Bool(true));
    object.insert("availability_nux".to_string(), Value::Null);
    object.insert("upgrade".to_string(), Value::Null);
    object.insert(
        "comp_hash".to_string(),
        Value::String(CODEX_CLIENT_COMP_HASH.to_string()),
    );
    apply_deepseek_multi_agent_capability(&mut model);
    // 统一口径收口：任何声明了上下文窗口的条目（内置模板、模板回落、外部覆盖）都必须
    // 带自动压缩阈值，避免目录里出现「有窗口无阈值」的空档。
    if let Some(object) = model.as_object_mut() {
        ensure_model_auto_compact_limit(object);
    }
    model
}

/// 只为已适配的 DeepSeek 模型补协作声明；必须在模板覆盖之后调用。
pub(crate) fn apply_deepseek_multi_agent_capability(model: &mut Value) {
    let slug = model.get("slug").and_then(Value::as_str).unwrap_or_default();
    let id = slug.trim().rsplit('/').next().unwrap_or_default().to_ascii_lowercase();
    if !matches!(id.as_str(), "deepseek-flash" | "deepseek-v4-flash" | "deepseek-v4-pro") {
        return;
    }
    model["multi_agent_version"] = json!("v2");
}

fn codex_client_model_catalog() -> &'static Value {
    static CATALOG: OnceLock<Value> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(CODEX_CLIENT_MODEL_TEMPLATES_JSON)
            .expect("Codex client model templates JSON should be valid")
    })
}

fn inherit_routed_gpt_capabilities(object: &mut Map<String, Value>, model_id: &str) {
    let Some((namespace, upstream)) = model_id.split_once('/') else {
        return;
    };
    if namespace.is_empty() || !upstream.starts_with("gpt-") || upstream.contains('/') {
        return;
    }
    let (template, known) = codex_client_model_template(upstream);
    if !known {
        return;
    }
    for field in [
        "supported_reasoning_levels",
        "default_reasoning_level",
        "service_tiers",
        "additional_speed_tiers",
        "context_window",
        "max_context_window",
        "auto_compact_token_limit",
    ] {
        if let Some(value) = template.get(field) {
            object.insert(field.to_string(), value.clone());
        }
    }
}

/// Grok 平台模型沿用的官方多智能体协议版本；与官方 DeepSeek 条目保持一致。
const GROK_MULTI_AGENT_VERSION: &str = "v2";
/// 声明 v2 协作能力所需的最低客户端版本，取官方同代模型的值。
const GROK_MINIMAL_CLIENT_VERSION: &str = "0.144.0";

fn is_grok_codex_model(model_id: &str) -> bool {
    let model_id = model_id.trim();
    crate::modules::codex_account::GROK_CODEX_MODELS
        .iter()
        .any(|id| id.eq_ignore_ascii_case(model_id))
}

fn codex_client_model_template(model_id: &str) -> (Value, bool) {
    let payload = codex_client_model_catalog();
    let models = payload
        .get("models")
        .and_then(Value::as_array)
        .expect("Codex client model templates should include models");
    if let Some(model) = models.iter().find(|model| {
        model
            .get("slug")
            .and_then(Value::as_str)
            .is_some_and(|slug| slug.eq_ignore_ascii_case(model_id))
    }) {
        return (model.clone(), true);
    }

    if model_id.eq_ignore_ascii_case(CODEX_RESERVE_MODEL_ID) {
        if let Some(luna) = models
            .iter()
            .find(|model| model["slug"].as_str() == Some(CODEX_RESERVE_TEMPLATE_MODEL_ID))
        {
            return (luna.clone(), true);
        }
    }

    let default_model = models
        .iter()
        .find(|model| {
            model.get("slug").and_then(Value::as_str) == Some(CODEX_MODEL_CATALOG_TEMPLATE_SLUG)
        })
        .cloned()
        .expect("Codex client model templates should include gpt-5.5");
    let Some(model_override) = payload
        .get("model_overrides")
        .and_then(Value::as_array)
        .and_then(|overrides| {
            overrides.iter().find(|model| {
                model
                    .get("slug")
                    .and_then(Value::as_str)
                    .is_some_and(|slug| slug.eq_ignore_ascii_case(model_id))
            })
        })
    else {
        return (default_model, false);
    };

    let mut model = default_model;
    let target = model
        .as_object_mut()
        .expect("Codex client model template should be a JSON object");
    for (key, value) in model_override
        .as_object()
        .expect("Codex client model override should be a JSON object")
    {
        target.insert(key.clone(), value.clone());
    }
    (model, true)
}

fn display_name_for_model(model_id: &str) -> String {
    match model_id {
        "gpt-5-codex" => "GPT-5 Codex".to_string(),
        "gpt-5-codex-mini" => "GPT-5 Codex Mini".to_string(),
        "gpt-5.4" => "GPT-5.4".to_string(),
        "gpt-5.4-mini" => "GPT-5.4 Mini".to_string(),
        "gpt-5.3-codex" => "GPT-5.3 Codex".to_string(),
        "gpt-5.3-codex-spark" => "GPT-5.3 Codex Spark".to_string(),
        "gpt-6-astra" => "GPT-6 Astra".to_string(),
        "gpt-6-sol" => "GPT-6 Sol".to_string(),
        "gpt-6-luna" => "GPT-6 Luna".to_string(),
        "gpt-5.2" => "GPT-5.2".to_string(),
        "gpt-5.2-codex" => "GPT-5.2 Codex".to_string(),
        "gpt-5.1-codex-max" => "GPT-5.1 Codex Max".to_string(),
        "gpt-5.1-codex-mini" => "GPT-5.1 Codex Mini".to_string(),
        "gpt-image-2" => "GPT Image 2".to_string(),
        "gpt-image-2.5" => "GPT Image 2.5".to_string(),
        CODEX_AUTO_REVIEW_MODEL_ID => "Codex Auto Review".to_string(),
        other => other.to_string(),
    }
}

fn ensure_string_field(obj: &mut Map<String, Value>, key: &str, value: &str) -> bool {
    if obj.get(key).and_then(Value::as_str) == Some(value) {
        return false;
    }
    if obj.get(key).is_some_and(Value::is_string) {
        return false;
    }
    obj.insert(key.to_string(), Value::String(value.to_string()));
    true
}

fn ensure_bool_field(obj: &mut Map<String, Value>, key: &str, value: bool) -> bool {
    if obj.get(key).and_then(Value::as_bool) == Some(value) {
        return false;
    }
    obj.insert(key.to_string(), Value::Bool(value));
    true
}

fn ensure_reasoning_include(obj: &mut Map<String, Value>) -> bool {
    match obj.get_mut("include") {
        Some(Value::Array(items)) => {
            if items
                .iter()
                .any(|item| item.as_str() == Some(REASONING_ENCRYPTED_CONTENT_INCLUDE))
            {
                false
            } else {
                items.push(Value::String(
                    REASONING_ENCRYPTED_CONTENT_INCLUDE.to_string(),
                ));
                true
            }
        }
        _ => {
            obj.insert(
                "include".to_string(),
                Value::Array(vec![Value::String(
                    REASONING_ENCRYPTED_CONTENT_INCLUDE.to_string(),
                )]),
            );
            true
        }
    }
}

fn normalize_responses_input(obj: &mut Map<String, Value>) -> bool {
    let Some(input) = obj.get_mut("input") else {
        return false;
    };

    match input {
        Value::String(text) => {
            let text = text.clone();
            *input = Value::Array(vec![message_item("user", &text)]);
            true
        }
        Value::Array(items) => {
            let mut changed = false;
            changed |= normalize_responses_item_ids(items);
            for item in items.iter_mut() {
                changed |= normalize_responses_input_item(item);
            }
            changed |= ensure_responses_call_ids(items);
            changed
        }
        Value::Object(_) => {
            let item = input.clone();
            let mut items = vec![item];
            normalize_responses_item_ids(&mut items);
            normalize_responses_input_item(&mut items[0]);
            *input = Value::Array(items);
            true
        }
        _ => false,
    }
}

/// Repair item IDs written by providers that used the `function_call` prefix
/// for a `custom_tool_call`. Official Responses validates the prefix against
/// the item type when replaying a conversation, so this must happen before
/// both direct official requests and gateway requests.
fn normalize_responses_item_ids(items: &mut [Value]) -> bool {
    let mut replacements = HashMap::new();
    let mut used = HashSet::new();
    for item in items.iter() {
        let Some(obj) = item.as_object() else {
            continue;
        };
        let Some(id) = obj.get("id").and_then(Value::as_str) else {
            continue;
        };
        if id.is_empty() {
            continue;
        }
        let Some(item_type) = obj.get("type").and_then(Value::as_str) else {
            continue;
        };
        let prefix = match item_type {
            "function_call" => "fc",
            "custom_tool_call" => "ctc",
            "custom_tool_call_output" => "ctco",
            "message" => "msg",
            "reasoning" => "rs",
            _ => continue,
        };
        let mut candidate = if id.starts_with(&format!("{prefix}_")) {
            id.to_string()
        } else {
            format!("{prefix}_{id}")
        };
        if candidate.chars().count() > CODEX_INPUT_ITEM_ID_LIMIT {
            candidate = shorten_codex_item_id(&candidate, 0);
        }
        if used.contains(&candidate) {
            for attempt in 1.. {
                let shortened = shorten_codex_item_id(&candidate, attempt);
                if used.insert(shortened.clone()) {
                    candidate = shortened;
                    break;
                }
            }
        } else {
            used.insert(candidate.clone());
        }
        if candidate != id {
            replacements.insert(id.to_string(), candidate);
        }
    }

    if replacements.is_empty() {
        return false;
    }
    let mut changed = false;
    for item in items.iter_mut() {
        let Some(obj) = item.as_object_mut() else {
            continue;
        };
        for key in ["id", "item_id"] {
            let Some(value) = obj
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_owned)
            else {
                continue;
            };
            if let Some(replacement) = replacements.get(&value) {
                obj.insert(key.to_string(), Value::String(replacement.clone()));
                changed = true;
            }
        }
    }
    changed
}

fn shorten_codex_item_id(id: &str, attempt: usize) -> String {
    let mut input = id.to_string();
    if attempt > 0 {
        input.push('\0');
        input.push_str(&attempt.to_string());
    }
    let digest = Sha256::digest(input.as_bytes());
    let suffix = format!(
        "_{}",
        digest[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    let prefix_len = CODEX_INPUT_ITEM_ID_LIMIT.saturating_sub(suffix.len());
    id.chars().take(prefix_len).collect::<String>() + &suffix
}

fn normalize_responses_input_item(item: &mut Value) -> bool {
    let Some(obj) = item.as_object_mut() else {
        return false;
    };

    // Leave namespace semantics to the upstream-compatible protocol layer.
    // The host must not discard replay metadata before provider selection.
    let mut changed = false;
    let role = obj
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("user")
        .to_ascii_lowercase();

    if role == "system" {
        obj.insert("role".to_string(), Value::String("developer".to_string()));
        changed = true;
    }

    if !obj.contains_key("type") && (obj.contains_key("role") || obj.contains_key("content")) {
        obj.insert("type".to_string(), Value::String("message".to_string()));
        changed = true;
    }

    let normalized_role = obj
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("user")
        .to_ascii_lowercase();
    if let Some(content) = obj.get_mut("content") {
        changed |= normalize_message_content(content, &normalized_role);
    }

    changed
}

fn responses_call_item_requires_id(item_type: &str) -> bool {
    matches!(
        item_type,
        "function_call" | "custom_tool_call" | "tool_call" | "mcp_tool_call"
    )
}

fn responses_call_output_requires_id(item_type: &str) -> bool {
    matches!(
        item_type,
        "function_call_output"
            | "custom_tool_call_output"
            | "tool_call_output"
            | "mcp_tool_call_output"
    )
}

fn next_generated_call_id(prefix: &str, index: usize, used: &mut HashSet<String>) -> String {
    let base = format!("{}_{}", prefix, index);
    if used.insert(base.clone()) {
        return base;
    }
    for suffix in 1.. {
        let candidate = format!("{}_{}", base, suffix);
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("call id suffix space is unbounded")
}

fn response_item_non_empty_string<'a>(obj: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn responses_call_output_can_stand_alone(item_type: &str, obj: &Map<String, Value>) -> bool {
    item_type == "function_call_output" && response_item_non_empty_string(obj, "name").is_some()
}

fn ensure_responses_call_ids(items: &mut Vec<Value>) -> bool {
    let mut used = HashSet::new();
    for item in items.iter() {
        if let Some(call_id) = item
            .as_object()
            .and_then(|obj| response_item_non_empty_string(obj, "call_id"))
        {
            used.insert(call_id.to_string());
        }
    }

    let mut pending: Vec<(String, Option<String>)> = Vec::new();
    let mut drop_indices = Vec::new();
    let mut changed = false;
    for (index, item) in items.iter_mut().enumerate() {
        let Some(obj) = item.as_object_mut() else {
            continue;
        };
        let item_type = obj
            .get("type")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_ascii_lowercase();
        let is_call = responses_call_item_requires_id(&item_type);
        let is_output = responses_call_output_requires_id(&item_type);
        if !is_call && !is_output {
            continue;
        }
        let name = response_item_non_empty_string(obj, "name").map(ToOwned::to_owned);
        let call_id = match response_item_non_empty_string(obj, "call_id") {
            Some(call_id) => call_id.to_string(),
            None => {
                let generated = if is_call {
                    next_generated_call_id("call_missing", index, &mut used)
                } else {
                    let matched = name
                        .as_deref()
                        .and_then(|name| {
                            pending
                                .iter()
                                .position(|(_, pending_name)| pending_name.as_deref() == Some(name))
                        })
                        .or_else(|| (!pending.is_empty()).then_some(0));
                    match matched {
                        Some(position) => pending.remove(position).0,
                        None if responses_call_output_can_stand_alone(&item_type, obj) => {
                            continue;
                        }
                        None => {
                            drop_indices.push(index);
                            continue;
                        }
                    }
                };
                obj.insert("call_id".to_string(), Value::String(generated.clone()));
                changed = true;
                generated
            }
        };

        if is_call {
            pending.push((call_id, name));
        } else if let Some(position) = pending.iter().position(|(id, _)| id == &call_id) {
            pending.remove(position);
        }
    }
    for index in drop_indices.into_iter().rev() {
        items.remove(index);
        changed = true;
    }
    changed
}

fn normalize_message_content(content: &mut Value, role: &str) -> bool {
    match content {
        Value::String(text) => {
            let text = text.clone();
            *content = Value::Array(vec![text_part(role, &text)]);
            true
        }
        Value::Array(parts) => {
            let mut changed = false;
            for part in parts {
                changed |= normalize_content_part(part, role);
            }
            changed
        }
        _ => false,
    }
}

fn normalize_content_part(part: &mut Value, role: &str) -> bool {
    let Some(obj) = part.as_object_mut() else {
        return false;
    };

    let mut changed = false;
    if !obj.contains_key("text") {
        if let Some(text) = obj
            .get("content")
            .and_then(Value::as_str)
            .map(str::to_string)
        {
            obj.insert("text".to_string(), Value::String(text));
            changed = true;
        }
    }

    let desired_type = response_text_type_for_role(role);
    match obj.get("type").and_then(Value::as_str) {
        Some("text") | None => {
            if obj.contains_key("text") {
                obj.insert("type".to_string(), Value::String(desired_type.to_string()));
                changed = true;
            }
        }
        Some("input_text") if role == "assistant" => {
            obj.insert("type".to_string(), Value::String("output_text".to_string()));
            changed = true;
        }
        Some("output_text") if role != "assistant" => {
            obj.insert("type".to_string(), Value::String("input_text".to_string()));
            changed = true;
        }
        _ => {}
    }

    changed
}

fn message_item(role: &str, text: &str) -> Value {
    json!({
        "type": "message",
        "role": role,
        "content": [text_part(role, text)],
    })
}

fn text_part(role: &str, text: &str) -> Value {
    json!({
        "type": response_text_type_for_role(role),
        "text": text,
    })
}

fn response_text_type_for_role(role: &str) -> &'static str {
    if role.eq_ignore_ascii_case("assistant") {
        "output_text"
    } else {
        "input_text"
    }
}

fn normalize_codex_builtin_tools(obj: &mut Map<String, Value>) -> bool {
    let mut changed = false;

    if let Some(Value::Array(tools)) = obj.get_mut("tools") {
        for tool in tools {
            changed |= normalize_builtin_tool_value(tool);
        }
    }

    if let Some(tool_choice) = obj.get_mut("tool_choice") {
        changed |= normalize_builtin_tool_value(tool_choice);
        if let Some(Value::Array(tools)) = tool_choice.get_mut("tools") {
            for tool in tools {
                changed |= normalize_builtin_tool_value(tool);
            }
        }
    }

    changed
}

fn normalize_builtin_tool_value(value: &mut Value) -> bool {
    let Some(obj) = value.as_object_mut() else {
        return false;
    };
    let Some(tool_type) = obj.get("type").and_then(Value::as_str) else {
        return false;
    };
    let normalized = match tool_type {
        "web_search_preview" | "web_search_preview_2025_03_11" => "web_search",
        _ => return false,
    };

    obj.insert("type".to_string(), Value::String(normalized.to_string()));
    true
}

fn remove_unsupported_responses_fields(obj: &mut Map<String, Value>) -> bool {
    let mut changed = false;
    for key in [
        "max_output_tokens",
        "max_completion_tokens",
        "temperature",
        "top_p",
        "truncation",
        "context_management",
        "user",
        "prompt_cache_retention",
        "safety_identifier",
        "stream_options",
    ] {
        changed |= obj.remove(key).is_some();
    }

    if obj.get("service_tier").is_some()
        && obj.get("service_tier").and_then(Value::as_str) != Some("priority")
    {
        obj.remove("service_tier");
        changed = true;
    }

    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deepseek_multi_agent_catalog_is_scoped_and_preserves_identity() {
        for slug in ["deepseek-flash", "deepseek-v4-flash", "deepseek-v4-pro", "relay/deepseek-v4-pro"] {
            let model = build_codex_client_model(slug, 0);
            assert_eq!(model["slug"], json!(slug));
            assert_eq!(model["multi_agent_version"], json!("v2"));
        }
        for slug in ["gpt-5.5", "gpt-5.6-luna", "gpt-reserve", "grok-4.6", "glm-4.6", "deepseek-custom"] {
            let mut model = build_codex_client_model(slug, 0);
            let before = model.clone();
            apply_deepseek_multi_agent_capability(&mut model);
            assert_eq!(model, before, "{slug}");
        }
    }

    #[test]
    fn grok_models_declare_multi_agent_capability() {
        let catalog = build_codex_client_models_response(&[
            "grok-4.6".into(),
            "grok-4.5".into(),
            "grok-4.3".into(),
            "unknown-third-party".into(),
        ]);
        let models = catalog["models"].as_array().expect("models");
        for slug in ["grok-4.6", "grok-4.5", "grok-4.3"] {
            let model = models
                .iter()
                .find(|model| model["slug"] == json!(slug))
                .unwrap_or_else(|| panic!("missing {slug}"));
            assert_eq!(model["multi_agent_version"], json!("v2"), "{slug}");
            assert_eq!(model["minimal_client_version"], json!("0.144.0"), "{slug}");
        }
        // 其它第三方模型仍保持模板默认值，避免误开协作能力。
        let unknown = models
            .iter()
            .find(|model| model["slug"] == json!("unknown-third-party"))
            .expect("unknown model");
        assert_eq!(unknown["multi_agent_version"], Value::Null);
        assert_eq!(unknown["minimal_client_version"], json!("0.124.0"));
    }

    #[test]
    fn routed_gpt_models_preserve_capabilities_and_dispatch_identity() {
        for upstream in ["gpt-6-astra", "gpt-5.6-sol", "gpt-5.6-luna"] {
            let routed = format!("cpa/{upstream}");
            let catalog = build_codex_client_models_response(&[upstream.into(), routed.clone()]);
            for field in ["service_tiers", "additional_speed_tiers", "supported_reasoning_levels", "context_window"] {
                assert_eq!(catalog["models"][0][field], catalog["models"][1][field], "{upstream}: {field}");
            }
            assert_eq!(catalog["models"][1]["slug"], routed);
            assert_eq!(catalog["models"][1]["priority"], json!(1001));
            assert_ne!(catalog["models"][0]["priority"], catalog["models"][1]["priority"]);
        }
        let catalog = build_codex_client_models_response(&["cpa/unknown-model".into()]);
        assert_eq!(catalog["models"][0]["service_tiers"], json!([]));
        assert_eq!(catalog["models"][0]["priority"], json!(1000));
    }

    #[test]
    fn routed_gpt_models_keep_catalog_order_instead_of_official_priority() {
        let catalog = build_codex_client_models_response(&[
            "cpa/gpt-5.6-sol".into(),
            "cpa/gpt-5.6-terra".into(),
            "1024/gpt-5.6-sol".into(),
            "weilong/gpt-6-astra".into(),
        ]);
        let priorities = catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|model| model["priority"].as_i64())
            .collect::<Vec<_>>();
        assert_eq!(
            priorities,
            vec![Some(1000), Some(1001), Some(1002), Some(1003)]
        );
    }

    #[test]
    fn routed_gpt_models_keep_explicit_reasoning_selection() {
        let catalog = build_codex_client_models_response_with_model_definitions_and_reasoning(&[
            ("cpa/gpt-6-astra".into(), "CPA Astra".into(), Some(vec!["ultra".into()])),
        ]);
        assert_eq!(catalog["models"][0]["supported_reasoning_levels"][0]["effort"], "ultra");
        assert_eq!(catalog["models"][0]["display_name"], "CPA Astra");
    }

    #[test]
    fn reserve_is_selectable_without_changing_other_models_or_request_id() {
        let mut catalog = build_codex_client_models_response(&[
            "gpt-6-astra".to_string(),
            CODEX_RESERVE_TEMPLATE_MODEL_ID.to_string(),
        ]);
        let before = catalog["models"].as_array().unwrap().clone();
        ensure_codex_reserve_fallback(&mut catalog);
        ensure_codex_reserve_fallback(&mut catalog);
        let models = catalog["models"].as_array().unwrap();
        assert_eq!(&models[..before.len()], before.as_slice());
        assert_eq!(models.len(), before.len() + 1);
        let mut expected = before[1].clone();
        expected["slug"] = json!(CODEX_RESERVE_MODEL_ID);
        expected["visibility"] = json!("list");
        expected["display_name"] = json!(CODEX_RESERVE_DISPLAY_NAME);
        assert_eq!(models.last(), Some(&expected));
        // 统一口径：Reserve 沿用 Luna 模板的上下文窗口，压缩阈值必须同时存在且为 90%。
        let reserve_context = expected["context_window"].as_i64().expect("context window");
        assert_eq!(
            expected["auto_compact_token_limit"].as_i64(),
            Some(derived_auto_compact_token_limit(reserve_context))
        );

        let explicit = build_codex_client_models_response(&[CODEX_RESERVE_MODEL_ID.to_string()]);
        assert_eq!(explicit["models"][0], expected);
        assert!(codex_model_uses_responses_lite(CODEX_RESERVE_MODEL_ID));
    }

    #[test]
    fn reserve_fallback_preserves_explicit_overrides_and_custom_only_catalogs() {
        let mut custom = json!({"models": [{"slug": "custom-model"}]});
        let before = custom.clone();
        ensure_codex_reserve_fallback(&mut custom);
        assert_eq!(custom["models"][0], before["models"][0]);
        assert_eq!(custom["models"][1]["slug"], "gpt-reserve");
        assert_eq!(custom["models"][1]["visibility"], "list");

        let mut catalog = build_codex_client_models_response(&[CODEX_RESERVE_MODEL_ID.to_string()]);
        apply_model_context_overrides(&mut catalog, &[
            (CODEX_RESERVE_MODEL_ID.to_string(), Some(516_000), Some(460_000))
        ]);
        ensure_codex_reserve_fallback(&mut catalog);
        assert_eq!(catalog["models"][0]["context_window"], 516_000);
        assert_eq!(catalog["models"][0]["auto_compact_token_limit"], 460_000);
        assert_eq!(catalog["models"][0]["visibility"], "list");
    }

    #[test]
    fn merges_local_no_proxy_hosts() {
        assert_eq!(
            merge_local_no_proxy("example.com, localhost"),
            "example.com,localhost,127.0.0.1,127.0.0.0/8,::1,::1/128"
        );
        assert_eq!(
            merge_local_no_proxy(""),
            "127.0.0.1,127.0.0.0/8,localhost,::1,::1/128"
        );
    }

    #[test]
    fn normalizes_string_input_for_codex_responses() {
        let mut body = json!({
            "model": "gpt-5.4",
            "input": "pong",
            "stream": false,
            "store": true,
            "temperature": 0.1,
        });

        assert!(normalize_responses_body_for_codex(&mut body));
        assert_eq!(body.get("stream").and_then(Value::as_bool), Some(true));
        assert_eq!(body.get("store").and_then(Value::as_bool), Some(false));
        assert_eq!(body.get("instructions").and_then(Value::as_str), Some(""));
        assert!(body.get("temperature").is_none());
        assert_eq!(
            body.pointer("/input/0/content/0/type")
                .and_then(Value::as_str),
            Some("input_text")
        );
        assert_eq!(
            body.pointer("/input/0/content/0/text")
                .and_then(Value::as_str),
            Some("pong")
        );
    }

    #[test]
    fn disables_parallel_tool_calls_for_responses_lite_models() {
        let mut body = json!({
            "model": "gpt-5.6-luna",
            "input": "pong",
            "parallel_tool_calls": true,
        });

        normalize_responses_body_for_codex(&mut body);
        assert_eq!(
            body.get("parallel_tool_calls").and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn responses_lite_header_forces_parallel_tool_calls_off() {
        let mut body = json!({
            "model": "custom-model",
            "input": "pong",
            "parallel_tool_calls": true,
        });

        normalize_responses_body_for_codex_with_lite(&mut body, true);
        assert_eq!(
            body.get("parallel_tool_calls").and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn responses_lite_keeps_only_supported_tools_in_all_declaration_locations() {
        let mut body = json!({
            "model": "gpt-5.6-sol",
            "input": [
                {
                    "type": "additional_tools",
                    "tools": [
                        {"type": "function", "name": "additional_function"},
                        {"type": "custom", "name": "additional_custom"},
                        {"type": "tool_search", "execution": "client"},
                        {"type": "image_generation"},
                        {"type": "web_search"},
                        {"type": "namespace", "name": "mcp__additional"}
                    ]
                },
                {
                    "type": "additional_tools",
                    "tools": [{"type": "image_generation"}]
                },
                {"role": "user", "content": "hello"}
            ],
            "tools": [
                {"type": "function", "name": "lookup"},
                {"type": "custom", "name": "apply_patch"},
                {"type": "tool_search", "execution": "client"},
                {"type": "tool_search"},
                {"type": "tool_search", "execution": "server"},
                {"type": "image_generation"},
                {"type": "web_search"},
                {"type": "namespace", "name": "mcp__root"}
            ],
            "tool_choice": {
                "type": "allowed_tools",
                "mode": "auto",
                "tools": [
                    {"type": "function", "name": "lookup"},
                    {"type": "custom", "name": "apply_patch"},
                    {"type": "tool_search", "execution": "client"},
                    {"type": "image_generation"},
                    {"type": "web_search"},
                    {"type": "namespace", "name": "mcp__root"}
                ]
            },
            "response": {
                "tools": [
                    {"type": "function", "name": "nested_function"},
                    {"type": "image_generation"},
                    {"type": "web_search"}
                ],
                "tool_choice": {"type": "image_generation"}
            }
        });

        assert!(normalize_responses_body_for_codex(&mut body));
        for pointer in ["/tools", "/tool_choice/tools", "/input/0/tools"] {
            assert_eq!(
                body.pointer(pointer)
                    .and_then(Value::as_array)
                    .map(|tools| {
                        tools
                            .iter()
                            .filter_map(|tool| tool.get("type").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                    }),
                Some(vec!["function", "custom", "tool_search"]),
                "unexpected tools at {pointer}"
            );
        }
        assert_eq!(
            body.pointer("/response/tools")
                .and_then(Value::as_array)
                .map(|tools| {
                    tools
                        .iter()
                        .filter_map(|tool| tool.get("type").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                }),
            Some(vec!["function"])
        );
        assert!(body.pointer("/response/tool_choice").is_none());
        assert_eq!(
            body.get("input").and_then(Value::as_array).map(Vec::len),
            Some(2)
        );
    }

    #[test]
    fn non_lite_responses_keep_official_hosted_tools() {
        let mut body = json!({
            "model": "gpt-5.4",
            "input": "hello",
            "tools": [
                {"type": "function", "name": "lookup"},
                {"type": "image_generation"},
                {"type": "web_search"},
                {"type": "namespace", "name": "mcp__root"}
            ],
            "tool_choice": {"type": "image_generation"}
        });

        normalize_responses_body_for_codex(&mut body);
        assert_eq!(
            body.get("tools").and_then(Value::as_array).map(Vec::len),
            Some(4)
        );
        assert_eq!(
            body.pointer("/tool_choice/type").and_then(Value::as_str),
            Some("image_generation")
        );
    }

    #[test]
    fn normalizes_system_role_and_builtin_tool_aliases() {
        let mut body = json!({
            "model": "gpt-5.4",
            "input": [{
                "type": "message",
                "role": "system",
                "content": "be concise"
            }],
            "tools": [{"type": "web_search_preview"}],
        });

        normalize_responses_body_for_codex(&mut body);
        assert_eq!(
            body.pointer("/input/0/role").and_then(Value::as_str),
            Some("developer")
        );
        assert_eq!(
            body.pointer("/input/0/content/0/type")
                .and_then(Value::as_str),
            Some("input_text")
        );
        assert_eq!(
            body.pointer("/tools/0/type").and_then(Value::as_str),
            Some("web_search")
        );
    }

    #[test]
    fn preserves_namespace_on_replayed_function_calls() {
        let mut body = json!({
            "model": "gpt-5.4",
            "input": [
                {
                    "type": "function_call",
                    "call_id": "call_1",
                    "name": "lookup",
                    "namespace": "mcp__example",
                    "arguments": "{}"
                },
                {
                    "type": "function_call_output",
                    "call_id": "call_1",
                    "output": "result"
                },
                {
                    "type": "function_call",
                    "call_id": "call_2",
                    "name": "plain_lookup",
                    "arguments": "{}"
                }
            ],
            "tools": [{
                "type": "namespace",
                "name": "mcp__example"
            }]
        });

        assert!(normalize_responses_body_for_codex(&mut body));
        assert_eq!(
            body.pointer("/input/0/namespace").and_then(Value::as_str),
            Some("mcp__example")
        );
        assert_eq!(
            body.pointer("/input/0/name").and_then(Value::as_str),
            Some("lookup")
        );
        assert_eq!(
            body.pointer("/input/0/call_id").and_then(Value::as_str),
            Some("call_1")
        );
        assert_eq!(
            body.pointer("/input/1/call_id").and_then(Value::as_str),
            Some("call_1")
        );
        assert_eq!(
            body.pointer("/input/1/output").and_then(Value::as_str),
            Some("result")
        );
        assert!(body.pointer("/input/2/namespace").is_none());
        assert_eq!(
            body.pointer("/input/2/name").and_then(Value::as_str),
            Some("plain_lookup")
        );
        assert_eq!(
            body.pointer("/input/2/call_id").and_then(Value::as_str),
            Some("call_2")
        );
        assert_eq!(
            body.pointer("/tools/0/type").and_then(Value::as_str),
            Some("namespace")
        );
    }

    #[test]
    fn repairs_mismatched_custom_tool_call_item_ids() {
        let mut body = json!({
            "model": "gpt-6-astra",
            "input": [
                {
                    "type": "custom_tool_call",
                    "id": "fc_legacy_0",
                    "call_id": "call-legacy",
                    "name": "apply_patch",
                    "input": "patch"
                },
                {
                    "type": "custom_tool_call_output",
                    "id": "fc_legacy_output_0",
                    "item_id": "fc_legacy_0",
                    "call_id": "call-legacy",
                    "output": "ok"
                }
            ]
        });

        assert!(normalize_responses_body_for_codex(&mut body));
        assert_eq!(body["input"][0]["id"], "ctc_fc_legacy_0");
        assert_eq!(body["input"][1]["id"], "ctco_fc_legacy_output_0");
        assert_eq!(body["input"][1]["item_id"], "ctc_fc_legacy_0");
    }

    #[test]
    fn shortens_long_and_colliding_response_item_ids() {
        let long_id = format!("fc_{}", "x".repeat(80));
        let mut body = json!({
            "model": "gpt-6-astra",
            "input": [
                {"type": "function_call", "id": long_id, "call_id": "call-1"},
                {"type": "function_call", "id": "fc_same", "call_id": "call-2"},
                {"type": "function_call", "id": "same", "call_id": "call-3"}
            ]
        });

        assert!(normalize_responses_body_for_codex(&mut body));
        let items = body["input"].as_array().expect("input array");
        let ids = items
            .iter()
            .map(|item| item["id"].as_str().expect("item id"))
            .collect::<Vec<_>>();
        assert!(ids.iter().all(|id| id.starts_with("fc_") && id.len() <= 64));
        assert_eq!(ids.len(), ids.iter().collect::<HashSet<_>>().len());
    }

    #[test]
    fn synthesizes_missing_call_ids_for_replayed_tool_items() {
        let mut body = json!({
            "model": "deepseek-v4-flash",
            "input": [
                {
                    "type": "function_call",
                    "name": "exec_command",
                    "arguments": "{\"cmd\":\"pwd\"}"
                },
                {
                    "type": "function_call_output",
                    "output": "/workspace"
                },
                {
                    "type": "custom_tool_call",
                    "name": "apply_patch",
                    "input": "*** Begin Patch"
                },
                {
                    "type": "custom_tool_call_output",
                    "output": "Done!"
                },
                {
                    "type": "function_call",
                    "call_id": "call_existing",
                    "name": "lookup",
                    "arguments": "{}"
                },
                {
                    "type": "function_call_output",
                    "call_id": "call_existing",
                    "output": "ok"
                }
            ]
        });

        assert!(normalize_responses_body_for_codex(&mut body));
        let input = body.get("input").and_then(Value::as_array).unwrap();
        let first_call_id = input[0]
            .get("call_id")
            .and_then(Value::as_str)
            .expect("function call id");
        assert!(first_call_id.starts_with("call_missing"));
        assert_eq!(
            input[1].get("call_id").and_then(Value::as_str),
            Some(first_call_id)
        );
        let custom_call_id = input[2]
            .get("call_id")
            .and_then(Value::as_str)
            .expect("custom tool call id");
        assert!(custom_call_id.starts_with("call_missing"));
        assert_eq!(
            input[3].get("call_id").and_then(Value::as_str),
            Some(custom_call_id)
        );
        assert_eq!(
            input[4].get("call_id").and_then(Value::as_str),
            Some("call_existing")
        );
        assert_eq!(
            input[5].get("call_id").and_then(Value::as_str),
            Some("call_existing")
        );
    }

    #[test]
    fn drops_anonymous_orphan_outputs_while_preserving_paired_history() {
        let mut body = json!({
            "model": "deepseek-v4-flash",
            "input": [
                {"type": "message", "role": "user", "content": "continue"},
                {"type": "function_call_output", "output": "orphan result"},
                {"type": "function_call", "name": "exec_command", "arguments": "{}"},
                {"type": "function_call_output", "output": "paired result"},
                {"type": "function_call_output", "name": "heartbeat", "output": "keep standalone"}
            ]
        });

        assert!(normalize_responses_body_for_codex(&mut body));
        let input = body.get("input").and_then(Value::as_array).unwrap();
        assert_eq!(input.len(), 4);
        assert_eq!(
            input[1].get("type").and_then(Value::as_str),
            Some("function_call")
        );
        let call_id = input[1]
            .get("call_id")
            .and_then(Value::as_str)
            .expect("synthesized call id");
        assert_eq!(
            input[2].get("call_id").and_then(Value::as_str),
            Some(call_id)
        );
        assert_eq!(
            input[3].get("name").and_then(Value::as_str),
            Some("heartbeat")
        );
        assert!(input[3].get("call_id").is_none());
    }

    #[test]
    fn preserves_existing_replay_input_items() {
        let mut body = json!({
            "model": "gpt-5.6-sol",
            "input": [
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "continue"}]},
                {"type": "function_call", "call_id": "old_unanswered", "name": "exec_command", "arguments": "{}"},
                {"type": "function_call", "call_id": "old_answered", "name": "lookup", "arguments": "{}"},
                {"type": "function_call_output", "call_id": "old_answered", "output": "ok"}
            ]
        });
        let expected_input = body["input"].clone();

        normalize_responses_body_for_codex(&mut body);
        assert_eq!(body["input"], expected_input);
    }

    #[test]
    fn preserving_supported_call_namespaces_does_not_report_change() {
        for item_type in [
            "function_call",
            "custom_tool_call",
            "tool_call",
            "mcp_tool_call",
        ] {
            let mut item = json!({
                "type": item_type,
                "namespace": "mcp__example"
            });
            let expected = item.clone();

            assert!(
                !normalize_responses_input_item(&mut item),
                "{item_type} should not report a change"
            );
            assert_eq!(item, expected);
        }
    }

    #[test]
    fn preserves_namespace_from_non_call_replayed_input_items() {
        let mut body = json!({
            "model": "gpt-5.4",
            "input": [
                {
                    "type": "message",
                    "role": "user",
                    "namespace": "mcp__example",
                    "content": "hello"
                }
            ]
        });

        assert!(normalize_responses_body_for_codex(&mut body));
        assert_eq!(
            body.pointer("/input/0/namespace").and_then(Value::as_str),
            Some("mcp__example")
        );
    }

    #[test]
    fn codex_client_models_use_models_field_only() {
        let response = build_codex_client_models_response(&["gpt-5.4".to_string()]);
        assert!(response.get("models").and_then(Value::as_array).is_some());
        assert!(response.get("object").is_none());
        assert!(response.get("data").is_none());

        assert_eq!(
            response.pointer("/models/0/slug").and_then(Value::as_str),
            Some("gpt-5.4")
        );
        assert_eq!(
            response
                .pointer("/models/0/prefer_websockets")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            response
                .pointer("/models/0/shell_type")
                .and_then(Value::as_str),
            Some("shell_command")
        );
        assert_eq!(
            response
                .pointer("/models/0/supported_in_api")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert!(response
            .pointer("/models/0/input_modalities")
            .and_then(Value::as_array)
            .is_some());
    }

    #[test]
    fn codex_client_models_share_unified_compaction_hash() {
        let response = build_codex_client_models_response(&[
            "gpt-5.5".to_string(),
            "gpt-5.6-sol".to_string(),
            "gpt-6-astra".to_string(),
            "deepseek-flash".to_string(),
            "custom-third-party".to_string(),
        ]);
        let models = response
            .get("models")
            .and_then(Value::as_array)
            .expect("models should be an array");
        assert_eq!(models.len(), 5);
        for model in models {
            assert_eq!(
                model.get("comp_hash").and_then(Value::as_str),
                Some(CODEX_CLIENT_COMP_HASH),
                "model {:?} should share the unified compaction hash",
                model.get("slug").and_then(Value::as_str)
            );
        }
    }

    #[test]
    fn codex_spark_compatibility_model_is_visible_with_a_safe_catalog_fallback() {
        let response = build_codex_client_models_response(&[
            "gpt-5.3-codex".to_string(),
            "gpt-5.3-codex-spark".to_string(),
        ]);
        let models = response
            .get("models")
            .and_then(Value::as_array)
            .expect("models should be an array");
        let spark = models
            .iter()
            .find(|model| model.get("slug").and_then(Value::as_str) == Some("gpt-5.3-codex-spark"))
            .expect("Spark should be visible to Codex clients");

        assert_eq!(
            spark.get("display_name").and_then(Value::as_str),
            Some("GPT-5.3-Codex-Spark")
        );
        assert_eq!(
            spark.get("visibility").and_then(Value::as_str),
            Some("list")
        );
        assert_eq!(
            spark.get("supported_in_api").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn codex_5_6_models_preserve_official_reasoning_and_speed_capabilities() {
        assert_eq!(
            managed_codex_model_ids(),
            vec![
                "gpt-6-astra",
                "gpt-6-sol",
                "gpt-6-luna",
                "gpt-5.6-sol",
                "gpt-5.6-terra",
                "gpt-5.6-luna"
            ]
        );
        let response = build_codex_client_models_response(&managed_codex_model_ids());
        let models = response
            .get("models")
            .and_then(Value::as_array)
            .expect("models should be an array");

        for (slug, default_effort, supports_ultra) in [
            ("gpt-5.6-sol", "low", true),
            ("gpt-5.6-terra", "medium", true),
            ("gpt-5.6-luna", "medium", false),
        ] {
            let model = models
                .iter()
                .find(|model| model.get("slug").and_then(Value::as_str) == Some(slug))
                .expect("5.6 model should exist");
            let efforts = model
                .get("supported_reasoning_levels")
                .and_then(Value::as_array)
                .expect("reasoning levels should exist")
                .iter()
                .filter_map(|level| level.get("effort").and_then(Value::as_str))
                .collect::<Vec<_>>();
            assert_eq!(
                model.get("default_reasoning_level").and_then(Value::as_str),
                Some(default_effort)
            );
            assert!(efforts.contains(&"max"));
            assert_eq!(efforts.contains(&"ultra"), supports_ultra);
            assert_eq!(
                model
                    .pointer("/additional_speed_tiers/0")
                    .and_then(Value::as_str),
                Some("fast")
            );
            assert_eq!(
                model.pointer("/service_tiers/0/id").and_then(Value::as_str),
                Some("priority")
            );
            assert_eq!(
                model.get("context_window").and_then(Value::as_i64),
                Some(272_000)
            );
            assert_eq!(
                model.get("max_context_window").and_then(Value::as_i64),
                Some(921_000)
            );
            assert_eq!(
                model.get("tool_mode").and_then(Value::as_str),
                Some("code_mode_only")
            );
            assert_eq!(
                model.get("use_responses_lite").and_then(Value::as_bool),
                Some(true)
            );
            assert_eq!(
                model.get("shell_type").and_then(Value::as_str),
                Some("shell_command")
            );
            assert_eq!(
                model.get("apply_patch_tool_type").and_then(Value::as_str),
                Some("freeform")
            );
        }
    }

    #[test]
    fn gpt_6_astra_preserves_official_catalog_limits_and_reasoning_levels() {
        let response = build_codex_client_models_response(&["gpt-6-astra".to_string()]);
        let model = response
            .pointer("/models/0")
            .expect("Astra model should be present");
        assert_eq!(
            model.get("display_name").and_then(Value::as_str),
            Some("GPT-6 Astra")
        );
        assert_eq!(
            model.get("context_window").and_then(Value::as_i64),
            Some(256_000)
        );
        assert_eq!(
            model.get("max_context_window").and_then(Value::as_i64),
            Some(256_000)
        );
        // 统一口径：上下文窗口必须带 90% 的压缩阈值。
        assert_eq!(
            model.get("auto_compact_token_limit").and_then(Value::as_i64),
            Some(230_400)
        );
        assert_eq!(
            model.get("max_completion_tokens").and_then(Value::as_i64),
            Some(128_000)
        );
        let efforts = model
            .get("supported_reasoning_levels")
            .and_then(Value::as_array)
            .expect("Astra reasoning levels should exist")
            .iter()
            .filter_map(|level| level.get("effort").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(
            efforts,
            vec!["low", "medium", "high", "xhigh", "max", "ultra"]
        );
        assert_eq!(
            model
                .pointer("/service_tiers/0/description")
                .and_then(Value::as_str),
            Some("2x speed, increased usage")
        );
        assert_eq!(
            model
                .pointer("/additional_speed_tiers/0")
                .and_then(Value::as_str),
            Some("fast")
        );
        assert_eq!(
            model.get("use_responses_lite").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn gpt_6_sol_and_luna_preserve_official_catalog_limits_and_reasoning_levels() {
        for (slug, official_name, fallback_name, priority, supports_ultra) in [
            ("gpt-6-sol", "GPT-6 Sol", "GPT-6 Sol", 2, true),
            ("gpt-6-luna", "GPT-6 Luna", "GPT-6 Luna", 3, false),
        ] {
            let response = build_codex_client_models_response(&[slug.to_string()]);
            let model = response
                .pointer("/models/0")
                .expect("GPT-6 model should be present");
            assert_eq!(
                model.get("display_name").and_then(Value::as_str),
                Some(official_name)
            );
            assert_eq!(display_name_for_model(slug), fallback_name);
            assert_eq!(model.get("priority").and_then(Value::as_i64), Some(priority));
            assert_eq!(
                model.get("context_window").and_then(Value::as_i64),
                Some(256_000)
            );
            assert_eq!(
                model.get("max_context_window").and_then(Value::as_i64),
                Some(256_000)
            );
            // 统一口径：上下文窗口必须带 90% 的压缩阈值。
            assert_eq!(
                model.get("auto_compact_token_limit").and_then(Value::as_i64),
                Some(230_400),
                "{slug}"
            );
            let efforts = model
                .get("supported_reasoning_levels")
                .and_then(Value::as_array)
                .expect("GPT-6 reasoning levels should exist")
                .iter()
                .filter_map(|level| level.get("effort").and_then(Value::as_str))
                .collect::<Vec<_>>();
            assert!(efforts.contains(&"max"), "{slug}: {efforts:?}");
            assert_eq!(efforts.contains(&"ultra"), supports_ultra, "{slug}: {efforts:?}");
        }
    }

    #[test]
    fn gpt_6_astra_filters_reasoning_efforts_to_official_six_levels() {
        let response = build_codex_client_models_response_with_model_definitions_and_reasoning(&[
            (
                "gpt-6-astra".to_string(),
                "6 Astra".to_string(),
                Some(vec!["low".to_string(), "ultra".to_string(), "max".to_string()]),
            ),
        ]);
        let model = response
            .pointer("/models/0")
            .expect("Astra model should be present");
        let efforts = model
            .get("supported_reasoning_levels")
            .and_then(Value::as_array)
            .expect("Astra reasoning levels should exist")
            .iter()
            .filter_map(|level| level.get("effort").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(efforts, vec!["low", "ultra", "max"]);
        assert_eq!(
            model.get("default_reasoning_level").and_then(Value::as_str),
            Some("low")
        );
    }

    #[test]
    fn model_catalog_uses_configured_display_names() {
        let response = build_codex_client_models_response_with_model_definitions(&[
            ("gpt-5.6-sol".to_string(), "Sol Display".to_string()),
            ("custom-model".to_string(), "Custom Display".to_string()),
        ]);
        assert_eq!(
            response
                .pointer("/models/0/display_name")
                .and_then(Value::as_str),
            Some("Sol Display")
        );
        assert_eq!(
            response
                .pointer("/models/1/display_name")
                .and_then(Value::as_str),
            Some("Custom Display")
        );
    }

    #[test]
    fn custom_model_uses_general_capabilities_and_overrides_identity() {
        let response = build_codex_client_models_response_with_model_definitions(&[
            ("gpt-5.5".to_string(), "GPT-5.5".to_string()),
            ("custom-model".to_string(), "Custom Model".to_string()),
        ]);
        let models = response["models"].as_array().expect("models array");
        let base = &models[0];
        let custom = &models[1];
        assert_eq!(custom["slug"], "custom-model");
        assert_eq!(custom["display_name"], "Custom Model");
        assert_eq!(custom["description"], "Custom Model");
        assert_eq!(custom["context_window"], DEFAULT_CONTEXT_WINDOW);
        assert_eq!(custom["max_context_window"], DEFAULT_MAX_CONTEXT_WINDOW);
        // 兜底不再声明 1M 上限；压缩阈值按统一口径派生为窗口的 90%。
        assert_eq!(custom["max_context_window"], 272_000);
        assert_eq!(custom["auto_compact_token_limit"], 244_800);
        assert_eq!(
            custom["supported_reasoning_levels"],
            base["supported_reasoning_levels"]
        );
        assert_eq!(custom["input_modalities"], base["input_modalities"]);
        assert_eq!(custom["visibility"], "list");
    }

    #[test]
    fn codex_default_model_priorities_follow_official_order() {
        let model_ids = [
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.6-luna",
            "gpt-5.5",
            "gpt-5.4",
            "gpt-5.4-mini",
        ]
        .map(str::to_string);
        let response = build_codex_client_models_response(&model_ids);
        let priorities = response
            .get("models")
            .and_then(Value::as_array)
            .expect("models should be an array")
            .iter()
            .map(|model| model.get("priority").and_then(Value::as_i64))
            .collect::<Vec<_>>();

        assert_eq!(
            priorities,
            vec![Some(4), Some(7), Some(8), Some(12), Some(16), Some(23)]
        );
    }

    #[test]
    fn derived_auto_compact_limits_follow_ninety_percent_rule() {
        for (window, expected) in [
            (516_000, 464_400),
            (1_000_000, 900_000),
            (272_000, 244_800),
            (128_000, 115_200),
            (256_000, 230_400),
            (1_048_576, 943_718),
        ] {
            assert_eq!(
                derived_auto_compact_token_limit(window),
                expected,
                "window={window}"
            );
        }
        assert_eq!(derived_auto_compact_token_limit(0), 0);
        assert_eq!(derived_auto_compact_token_limit(-1), 0);
        assert_eq!(AUTO_COMPACT_RATIO_PERCENT, 90);
    }

    #[test]
    fn catalog_models_without_compact_limit_derive_ninety_percent() {
        let mut catalog = json!({"models": [
            {"slug": "windowless-model"},
            {"slug": "official-model", "context_window": 272_000},
            {"slug": "explicit-model", "context_window": 516_000, "auto_compact_token_limit": 460_000},
            {"slug": "stale-model", "context_window": 100_000, "auto_compact_token_limit": 100_000},
        ]});
        assert!(ensure_client_model_auto_compact_limits(&mut catalog));
        assert!(catalog["models"][0].get("auto_compact_token_limit").is_none());
        assert_eq!(catalog["models"][1]["auto_compact_token_limit"], 244_800);
        // 显式给出的（小于窗口的）阈值保持原样。
        assert_eq!(catalog["models"][2]["auto_compact_token_limit"], 460_000);
        // 大于等于窗口的无效阈值会被 90% 派生值替换。
        assert_eq!(catalog["models"][3]["auto_compact_token_limit"], 90_000);
    }

    #[test]
    fn unknown_codex_models_keep_conservative_catalog_fallback() {
        let response = build_codex_client_models_response(&["custom-model".to_string()]);
        assert_eq!(
            response
                .pointer("/models/0/additional_speed_tiers")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
        assert_eq!(
            response
                .pointer("/models/0/service_tiers")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
    }
}
