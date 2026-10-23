/// 供应商记录里的识图能力（`codex_model_providers.json`）。
///
/// v1.3.49 之前识图开关挂在供应商上，之后收敛为逐模型能力；升级后的账号记录
/// 可能还没有同步到逐模型表，网关配置因此把支持图片的模型当成 text-only 并丢图。
/// 这里以供应商记录作为兜底数据源，避免 Provider Gateway 静默删除 `input_image`。
#[derive(Debug, Clone, Default)]
struct CodexModelProviderVisionRecord {
    supports_vision: bool,
    /// key 为小写模型 ID。
    model_capabilities: HashMap<String, bool>,
}

struct CodexModelProviderVisionEntry {
    id: String,
    /// 规范化后的 Base URL（去掉末尾斜杠并转小写）。
    base_url: Option<String>,
    record: CodexModelProviderVisionRecord,
    credential_records: Vec<(String, CodexModelProviderVisionRecord)>,
}

const CODEX_MODEL_PROVIDERS_FILE: &str = "codex_model_providers.json";

fn normalize_provider_vision_base_url(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_end_matches('/').to_ascii_lowercase();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn load_codex_model_provider_vision_entries() -> Vec<CodexModelProviderVisionEntry> {
    let Ok(path) = account::get_data_dir().map(|dir| dir.join(CODEX_MODEL_PROVIDERS_FILE)) else {
        return Vec::new();
    };
    let Ok(content) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    parse_codex_model_provider_vision_entries(&content)
}

fn provider_vision_record_from_config(
    config: &Value,
    inherited: Option<&CodexModelProviderVisionRecord>,
) -> CodexModelProviderVisionRecord {
    let supports_vision = config
        .get("supportsVision")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| inherited.is_some_and(|record| record.supports_vision));
    let model_capabilities = if let Some(map) =
        config.get("modelCapabilities").and_then(Value::as_object)
    {
        map.iter()
            .filter_map(|(model, capability)| {
                let key = model.trim().to_ascii_lowercase();
                let flag = capability.get("supportsVision").and_then(Value::as_bool)?;
                (!key.is_empty()).then_some((key, flag))
            })
            .collect()
    } else {
        // Match resolveCodexModelProviderForApiKey: an owned catalog limits only
        // inherited metadata; an explicit capability map (including {}) replaces it.
        let model_ids = config
            .get("modelCatalog")
            .and_then(Value::as_array)
            .map(|models| {
                models
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|model| model.trim().to_ascii_lowercase())
                    .collect::<HashSet<_>>()
            });
        inherited
            .map(|record| {
                record
                    .model_capabilities
                    .iter()
                    .filter(|(model, _)| model_ids.as_ref().is_none_or(|ids| ids.contains(*model)))
                    .map(|(model, flag)| (model.clone(), *flag))
                    .collect()
            })
            .unwrap_or_default()
    };
    CodexModelProviderVisionRecord {
        supports_vision,
        model_capabilities,
    }
}

fn parse_codex_model_provider_vision_entries(content: &str) -> Vec<CodexModelProviderVisionEntry> {
    let Ok(value) = serde_json::from_str::<Value>(content) else {
        return Vec::new();
    };
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let id = item
                .get("id")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default()
                .to_string();
            let base_url = item
                .get("baseUrl")
                .and_then(Value::as_str)
                .and_then(normalize_provider_vision_base_url);
            if id.is_empty() && base_url.is_none() {
                return None;
            }
            let record = provider_vision_record_from_config(item, None);
            let credential_records = item
                .get("apiKeys")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|credential| {
                    let api_key = credential.get("apiKey").and_then(Value::as_str)?.trim();
                    if api_key.is_empty() {
                        return None;
                    }
                    Some((
                        api_key.to_string(),
                        provider_vision_record_from_config(credential, Some(&record)),
                    ))
                })
                .collect();
            Some(CodexModelProviderVisionEntry {
                id,
                base_url,
                record,
                credential_records,
            })
        })
        .collect()
}

fn codex_model_provider_vision_record_for_account<'a>(
    account: &CodexAccount,
    entries: &'a [CodexModelProviderVisionEntry],
) -> Option<&'a CodexModelProviderVisionRecord> {
    let provider_id = account
        .api_provider_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let account_base_url = account
        .api_base_url
        .as_deref()
        .and_then(normalize_provider_vision_base_url);
    entries
        .iter()
        .find(|entry| {
            let id_matches = provider_id.is_some_and(|value| value == entry.id);
            let base_url_matches = account_base_url
                .as_deref()
                .zip(entry.base_url.as_deref())
                .is_some_and(|(account_url, provider_url)| account_url == provider_url);
            id_matches || base_url_matches
        })
        .map(|entry| {
            account
                .openai_api_key
                .as_deref()
                .map(str::trim)
                .filter(|key| !key.is_empty())
                .and_then(|api_key| {
                    entry
                        .credential_records
                        .iter()
                        .find(|(key, _)| key == api_key)
                })
                .map(|(_, record)| record)
                .unwrap_or(&entry.record)
        })
}

/// 用供应商记录的识图能力补齐逐模型表。
///
/// 供应商记录先按账号凭据解析继承关系，再使用逐模型显式值和账号兜底。
/// 返回网关级 `supportsVision`（作为未被逐模型表覆盖时的兜底）。
fn merge_provider_vision_capabilities(
    model_capabilities: &mut HashMap<String, CodexLocalAccessProviderGatewayModelCapability>,
    account: &CodexAccount,
    provider: Option<&CodexModelProviderVisionRecord>,
    models: &[String],
) -> bool {
    let provider_flag = provider.is_some_and(|record| record.supports_vision);
    let gateway_supports_vision = provider_flag || account.api_supports_vision;
    for model in models {
        let key = model.trim().to_ascii_lowercase();
        if key.is_empty() {
            continue;
        }
        let provider_value =
            provider.and_then(|record| record.model_capabilities.get(&key).copied());
        let account_value = model_capabilities
            .get(&key)
            .map(|capability| capability.supports_vision);
        let effective = provider_value.or(account_value).unwrap_or_else(|| {
            gateway_supports_vision || codex_account::model_defaults_to_vision_input(model)
        });
        model_capabilities.insert(
            key,
            CodexLocalAccessProviderGatewayModelCapability {
                supports_vision: effective,
            },
        );
    }
    gateway_supports_vision
}
