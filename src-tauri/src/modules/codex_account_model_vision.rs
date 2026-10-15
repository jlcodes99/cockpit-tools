// Codex 账号：模型识图默认规则。
//
// 未显式配置时的统一口径：`gpt-5.5` 及以上（含 5.5）默认支持图片输入。
// 允许 `vendor/gpt-5.6-sol` 这类带命名空间前缀、以及 `-luna` / `-sol` 这类后缀的写法；
// 其它模型（含 `gpt-5.4`、`gpt-5.3-codex`、`gpt-3.5`、`gpt-reserve`、`gpt-image-*` 与
// 非 GPT 家族模型）不受影响，仍由账号与供应商的显式配置决定。

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

/// `gpt-5.5` 及以上默认支持图片输入。
pub(crate) fn model_defaults_to_vision_input(model_id: &str) -> bool {
    parse_gpt_model_version(model_id).is_some_and(|version| version >= (5, 5))
}
