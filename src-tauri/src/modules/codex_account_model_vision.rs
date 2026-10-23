// Codex 账号：模型识图默认规则。
//
// 未显式配置时，按随程序附带的 models.json 查找完整模型 ID，输入类型包含 image 时默认支持图片。
// 允许 `vendor/gpt-5.6-sol` 这类命名空间前缀，未找到模型或缺少图片输入声明时默认 false；
// 账号与供应商的显式配置仍优先。
// 与前端 codexModelProviderVision.ts 读取同一份目录，避免默认能力规则分歧。

/// 解析 `gpt-<major>.<minor>` 版本号；无法识别时返回 None。
pub(crate) fn parse_gpt_model_version(model_id: &str) -> Option<(u32, u32)> {
    let normalized = model_id.trim().to_ascii_lowercase();
    // `openai/gpt-5.6-sol` 取最后一段，避免命名空间前缀影响解析。
    let name = normalized.rsplit('/').next().unwrap_or(normalized.as_str());
    let rest = name.strip_prefix("gpt-")?;
    let head = rest.split(|c: char| !c.is_ascii_digit() && c != '.').next()?;
    let mut parts = head.split('.');
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts
        .next()
        .and_then(|item| item.parse::<u32>().ok())
        .unwrap_or(0);
    Some((major, minor))
}

/// 按 models.json 的完整模型 ID 和图片输入声明判断默认图片能力。
pub(crate) fn model_defaults_to_vision_input(model_id: &str) -> bool {
    let normalized = model_id.trim().to_ascii_lowercase();
    let name = normalized.rsplit('/').next().unwrap_or(normalized.as_str());
    static CATALOG: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
    let catalog = CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../sidecars/cockpit-cliproxy/third_party/CLIProxyAPI/internal/registry/models/models.json"
        )))
        .expect("Embedded models.json should be valid")
    });
    catalog
        .as_object()
        .expect("Embedded models.json should contain provider model lists")
        .values()
        .filter_map(serde_json::Value::as_array)
        .flatten()
        .any(|model| {
            model.get("id").and_then(serde_json::Value::as_str)
                .is_some_and(|id| id.trim().eq_ignore_ascii_case(name))
                && model.get("supportedInputModalities").and_then(serde_json::Value::as_array)
                    .is_some_and(|modalities| modalities.iter().any(|modality| modality.as_str() == Some("image")))
        })
}
