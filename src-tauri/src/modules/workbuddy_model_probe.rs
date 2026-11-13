use super::workbuddy_oauth::billing_headers;
use serde_json::{json, Value};
#[path = "workbuddy_probe_acp.rs"]
mod acp;
use acp::{looks_rate_limited, parse_acp_post_error, read_acp_reply};

const WORKBUDDY_WEB_ENDPOINT: &str = "https://www.workbuddy.cn";
/// CodeBuddy CN 与 WorkBuddy 同构（额度共享），仅应用面域名不同。
const CODEBUDDY_CN_WEB_ENDPOINT: &str = "https://www.codebuddy.cn";

/// 探测所需的账号字段（WorkBuddy / CodeBuddy CN 结构一致，抽出来复用同一套流程）。
pub(crate) struct ProbeAccount<'a> {
    pub id: &'a str,
    pub access_token: &'a str,
    pub uid: Option<&'a str>,
    pub enterprise_id: Option<&'a str>,
    pub domain: Option<&'a str>,
}

impl<'a> ProbeAccount<'a> {
    pub(crate) fn from_workbuddy(account: &'a crate::models::workbuddy::WorkbuddyAccount) -> Self {
        Self {
            id: &account.id,
            access_token: &account.access_token,
            uid: account.uid.as_deref(),
            enterprise_id: account.enterprise_id.as_deref(),
            domain: account.domain.as_deref(),
        }
    }

    pub(crate) fn from_codebuddy(account: &'a crate::models::codebuddy::CodebuddyAccount) -> Self {
        Self {
            id: &account.id,
            access_token: &account.access_token,
            uid: account.uid.as_deref(),
            enterprise_id: account.enterprise_id.as_deref(),
            domain: account.domain.as_deref(),
        }
    }
}

fn build_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) WorkBuddy/5.9.3 Chrome/138.0.7204.251 Electron/37.10.3 Safari/537.36")
        .timeout(std::time::Duration::from_secs(45))
        .build()
        .map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbuddyModelProbeResult {
    pub status: String,
    pub http_status: Option<u16>,
    pub conversation_id: Option<String>,
    /// A protocol code or upstream diagnostic, localized by the caller.
    pub message: String,
    pub reply: String,
    pub keep_conversation: bool,
    pub conversation_deleted: bool,
    pub cleanup_error: Option<String>,
}

static ACTIVE_PROBES: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashSet<(String, String)>>,
> = std::sync::LazyLock::new(Default::default);

struct ProbeLease((String, String));

impl Drop for ProbeLease {
    fn drop(&mut self) {
        ACTIVE_PROBES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0);
    }
}

pub async fn probe_model(
    account: &crate::models::workbuddy::WorkbuddyAccount,
    model: &str,
    keep_conversation: bool,
) -> Result<WorkbuddyModelProbeResult, String> {
    probe_model_for(
        &ProbeAccount::from_workbuddy(account),
        WORKBUDDY_WEB_ENDPOINT,
        model,
        keep_conversation,
    )
    .await
}

/// CodeBuddy CN 平台的模型探测（与 WorkBuddy 共用同一套后端契约）。
pub async fn probe_cn_model(
    account: &crate::models::codebuddy::CodebuddyAccount,
    model: &str,
    keep_conversation: bool,
) -> Result<WorkbuddyModelProbeResult, String> {
    probe_model_for(
        &ProbeAccount::from_codebuddy(account),
        CODEBUDDY_CN_WEB_ENDPOINT,
        model,
        keep_conversation,
    )
    .await
}

async fn probe_model_for(
    account: &ProbeAccount<'_>,
    base: &str,
    model: &str,
    keep_conversation: bool,
) -> Result<WorkbuddyModelProbeResult, String> {
    let model = model.trim();
    if model.is_empty() {
        return Err("WORKBUDDY_PROBE_MODEL_REQUIRED".into());
    }
    let key = (account.id.to_string(), model.to_string());
    if !ACTIVE_PROBES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key.clone())
    {
        return Err("WORKBUDDY_PROBE_BUSY".into());
    }
    let _lease = ProbeLease(key);
    probe_with_client(&build_client()?, base, account, model, keep_conversation).await
}

async fn probe_with_client(
    client: &reqwest::Client,
    base: &str,
    account: &ProbeAccount<'_>,
    model: &str,
    keep_conversation: bool,
) -> Result<WorkbuddyModelProbeResult, String> {
    let headers = billing_headers(
        account.access_token,
        account.uid,
        account.enterprise_id,
        account.domain,
        "en",
    )?;
    let response = client
        .post(format!("{base}/console/as/conversations/"))
        .headers(headers.clone())
        .timeout(std::time::Duration::from_secs(15))
        .json(&json!({"prompt":"Reply with OK.","model":model,"plugins":[]}))
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?;
    let http_status = response.status().as_u16();
    let text = response
        .text()
        .await
        .map_err(|e| e.without_url().to_string())?;
    let body: Value = match serde_json::from_str(&text) {
        Ok(body) => body,
        Err(_) if !(200..300).contains(&http_status) => Value::Null,
        Err(_) => return Err("WORKBUDDY_PROBE_INVALID_RESPONSE".into()),
    };
    let conversation_id = body
        .pointer("/data/id")
        .or_else(|| body.get("id"))
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .map(str::to_string);
    let mut result = WorkbuddyModelProbeResult {
        status: "unknown".into(),
        http_status: Some(http_status),
        conversation_id,
        message: String::new(),
        reply: String::new(),
        keep_conversation,
        conversation_deleted: false,
        cleanup_error: None,
    };
    let created = if http_status == 429 {
        Err("WORKBUDDY_PROBE_RATE_LIMITED".to_string())
    } else if !(200..300).contains(&http_status) {
        Err(parse_acp_post_error(&text)
            .or_else(|| {
                body.get("message")
                    .or_else(|| body.get("msg"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| format!("HTTP {http_status}")))
    } else if let Some(error) = parse_acp_post_error(&text) {
        Err(error)
    } else if result.conversation_id.is_none() {
        Err("WORKBUDDY_PROBE_INVALID_RESPONSE".to_string())
    } else {
        Ok(())
    };
    let outcome = match created {
        Err(error) => Err(error),
        Ok(()) => {
            let cid = result.conversation_id.as_deref().unwrap();
            match fetch_conversation_session(client, base, cid, account).await {
                Err(error) => Err(error),
                Ok((api_url, token, session)) => {
                    let reply = read_acp_reply(
                        client,
                        &api_url,
                        &token,
                        &session,
                        model,
                        account.enterprise_id,
                        45,
                    )
                    .await;
                    match reply.error {
                        Some(error) => Err(error),
                        // 服务端可能不推送 agent_message_chunk 文本块
                        //（usage.outputTokens 已证明模型执行、stopReason=end_turn）。
                        // prompt 正常结束即视为可用，回复文本有则带上。
                        None if reply.prompt_done || !reply.text.trim().is_empty() => Ok(reply.text),
                        None => Err("WORKBUDDY_PROBE_NO_REPLY".into()),
                    }
                }
            }
        }
    };
    match outcome {
        Ok(reply) => {
            result.status = "available".into();
            result.reply = reply;
        }
        Err(error) => {
            result.status = if looks_rate_limited(&error) {
                "rate_limited"
            } else if error.starts_with("WORKBUDDY_PROBE_") {
                "unknown"
            } else {
                "error"
            }
            .into();
            result.message = if account.access_token.is_empty() {
                error
            } else {
                error.replace(account.access_token, "<redacted>")
            };
        }
    }
    // Cleanup is separate from probe success. A failed DELETE must stay visible.
    if !keep_conversation {
        if let Some(id) = &result.conversation_id {
            let deleted = client
                .delete(format!(
                    "{base}/console/as/conversations/{}",
                    urlencoding::encode(id)
                ))
                .headers(headers)
                .timeout(std::time::Duration::from_secs(10))
                .send()
                .await;
            match deleted {
                Ok(response) if response.status().is_success() => match response.text().await {
                    Ok(text) if text.trim().is_empty() => result.conversation_deleted = true,
                    Ok(text) => match parse_acp_post_error(&text) {
                        Some(error) => result.cleanup_error = Some(error),
                        None => match serde_json::from_str::<Value>(&text) {
                            Ok(body)
                                if body
                                    .get("code")
                                    .is_some_and(|code| code == 0 || code == "0")
                                    || body
                                        .get("success")
                                        .is_some_and(|success| success == true) =>
                            {
                                result.conversation_deleted = true;
                            }
                            Ok(_) => {
                                result.cleanup_error =
                                    Some("WORKBUDDY_PROBE_INVALID_RESPONSE".into())
                            }
                            Err(_) => {
                                result.cleanup_error =
                                    Some("WORKBUDDY_PROBE_INVALID_RESPONSE".into())
                            }
                        },
                    },
                    Err(error) => result.cleanup_error = Some(error.without_url().to_string()),
                },
                Ok(response) => {
                    result.cleanup_error = Some(format!("HTTP {}", response.status().as_u16()))
                }
                Err(error) => result.cleanup_error = Some(error.without_url().to_string()),
            }
        }
    }
    if !account.access_token.is_empty() {
        if let Some(error) = &mut result.cleanup_error {
            *error = error.replace(account.access_token, "<redacted>");
        }
    }
    Ok(result)
}

fn truncate_chars(value: &str, max: usize) -> String {
    let mut out: String = value.chars().take(max).collect();
    if value.chars().count() > max {
        out.push('…');
    }
    out
}

/// 取会话的沙箱地址与会话 token（对齐客户端 getSandboxInfo）。
async fn fetch_conversation_session(
    client: &reqwest::Client,
    base: &str,
    conversation_id: &str,
    account: &ProbeAccount<'_>,
) -> Result<(String, String, String), String> {
    let headers = billing_headers(
        account.access_token,
        account.uid,
        account.enterprise_id,
        account.domain,
        "zh",
    )?;

    let response = client
        .get(format!(
            "{base}/console/as/conversations/{}/session",
            urlencoding::encode(conversation_id)
        ))
        .headers(headers)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("HTTP {}", status.as_u16()));
    }
    let body: Value = response
        .json()
        .await
        .map_err(|e| e.without_url().to_string())?;
    let data = body
        .get("data")
        .filter(|data| data.is_object())
        .unwrap_or(&body);
    let api_url = data
        .get("link")
        .or_else(|| data.get("endpoint"))
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .ok_or("WORKBUDDY_PROBE_INVALID_SESSION")?;
    let token = data
        .get("token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .ok_or("WORKBUDDY_PROBE_INVALID_SESSION")?;
    let session = data
        .get("sessionId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .unwrap_or(conversation_id);
    let mut endpoint = url::Url::parse(api_url).map_err(|_| "WORKBUDDY_PROBE_INVALID_SESSION")?;
    if base.starts_with("https://") && endpoint.scheme() == "http" {
        endpoint
            .set_scheme("https")
            .map_err(|_| "WORKBUDDY_PROBE_INVALID_SESSION")?;
    }
    Ok((endpoint.to_string(), token.into(), session.into()))
}

/// 对话选择器模型条目（来自客户端 ModelsService.getTeamsModels 同源接口）。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbuddyCloudModel {
    pub id: String,
    pub name: Option<String>,
    /// 计费倍率（如 "x0.15"），对应客户端 credits 字段
    pub credits: Option<String>,
    pub description: Option<String>,
    pub supports_images: bool,
    pub is_default: bool,
}

/// 拉取对话选择器的实时模型列表。
///
/// 对齐客户端 ModelsService.getTeamsModels：
/// GET /console/enterprises/{enterpriseId|personal}/models，
/// 企业账号用真实 enterpriseId，个人账号用字面量 `personal`。
/// 响应 body（或 body.data）含 models: [{id, name, credits, descriptionZh, ...}]。
pub async fn fetch_cloud_models(
    account: &crate::models::workbuddy::WorkbuddyAccount,
) -> Result<Vec<WorkbuddyCloudModel>, String> {
    fetch_cloud_models_for(
        WORKBUDDY_WEB_ENDPOINT,
        &ProbeAccount::from_workbuddy(account),
    )
    .await
}

/// CodeBuddy CN 平台的实时模型目录（与 WorkBuddy 同一套接口，域名不同）。
pub async fn fetch_cn_cloud_models(
    account: &crate::models::codebuddy::CodebuddyAccount,
) -> Result<Vec<WorkbuddyCloudModel>, String> {
    fetch_cloud_models_for(
        CODEBUDDY_CN_WEB_ENDPOINT,
        &ProbeAccount::from_codebuddy(account),
    )
    .await
}

async fn fetch_cloud_models_for(
    base: &str,
    account: &ProbeAccount<'_>,
) -> Result<Vec<WorkbuddyCloudModel>, String> {
    let scope = account
        .enterprise_id
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("personal")
        .to_string();
    let client = build_client()?;
    let headers = billing_headers(
        account.access_token,
        account.uid,
        account.enterprise_id,
        account.domain,
        "zh",
    )?;

    let response = client
        .get(format!(
            "{base}/console/enterprises/{}/models",
            urlencoding::encode(&scope)
        ))
        .headers(headers)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|error| error.without_url().to_string())?
        .error_for_status()
        .map_err(|error| error.without_url().to_string())?;
    let parsed: Value = response
        .json()
        .await
        .map_err(|error| error.without_url().to_string())?;
    if let Some(error) = parse_acp_post_error(&parsed.to_string()) {
        return Err(error);
    }
    let payload = parsed
        .get("data")
        .filter(|data| data.is_object())
        .unwrap_or(&parsed);
    let models = payload
        .get("models")
        .and_then(Value::as_array)
        .ok_or("WORKBUDDY_PROBE_INVALID_RESPONSE")?;
    let mut out: Vec<WorkbuddyCloudModel> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for entry in models {
        let id = entry
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if id.is_empty() || !seen.insert(id.clone()) {
            continue;
        }
        out.push(WorkbuddyCloudModel {
            id,
            name: entry
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            credits: entry
                .get("credits")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            description: entry
                .get("descriptionZh")
                .or_else(|| entry.get("description"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            supports_images: entry
                .get("supportsImages")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            is_default: entry
                .get("isDefault")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        });
    }
    Ok(out)
}

#[cfg(test)]
#[path = "workbuddy_model_probe_tests.rs"]
mod tests;
