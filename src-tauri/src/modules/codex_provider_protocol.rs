// Shared protocol resolution for provider connectivity and Pelican tests.
pub(crate) fn normalize_model_provider_wire_api(value: Option<&str>, base_url: &str) -> String {
    match value.map(str::trim) {
        Some("chat_completions") | Some("chat") => return "chat_completions".to_string(),
        Some("responses") => return "responses".to_string(),
        _ => {}
    }
    // DeepSeek defaults to official Responses when the caller did not choose a protocol.
    if reqwest::Url::parse(base_url.trim())
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .is_some_and(|host| host.eq_ignore_ascii_case("api.deepseek.com"))
    {
        return "responses".to_string();
    }
    let lower = base_url.trim().to_ascii_lowercase();
    if lower.contains("/chat/completions")
        || lower.contains("api.moonshot.cn")
        || lower.contains("api.siliconflow.cn")
        || lower.contains("api.siliconflow.com")
        || lower.contains("open.bigmodel.cn")
        || lower.contains("api.z.ai")
        || lower.contains("volces.com")
        || lower.contains("bytepluses.com")
        || lower.contains("qianfan.baidubce.com")
        || lower.contains("dashscope.aliyuncs.com")
        || lower.contains("api.stepfun.com")
        || lower.contains("api.stepfun.ai")
        || lower.contains("modelscope.cn")
        || lower.contains("api.longcat.chat")
        || lower.contains("api.minimax.io")
        || lower.contains("api.mini-max.chat")
        || lower.contains("api.minimaxi.com")
        || lower.contains("api.mimo.dev")
        || lower.contains("token-plan-cn.xiaomimimo.com")
        || lower.contains("api.novita.ai")
        || lower.contains("integrate.api.nvidia.com")
        || lower.contains("runapi.co")
        || lower.contains("relaxycode.com")
        || lower.contains("compshare.cn")
        || lower.contains("api.lemondata.cc")
        || lower.contains("e-flowcode.cc")
        || lower.contains("cc-api.pipellm.ai")
        || lower.contains("openrouter.ai")
        || lower.contains("api.therouter.ai")
    {
        "chat_completions".to_string()
    } else {
        "responses".to_string()
    }
}
