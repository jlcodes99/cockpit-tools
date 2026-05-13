use std::{env, process::Command, time::Duration};

use serde::Deserialize;

use crate::modules::{config, logger};

#[derive(Debug, Clone)]
pub struct SwitchNotifyPayload {
    pub platform: String,
    pub account_id: String,
    pub account_label: String,
    pub trigger_type: String,
    pub trigger_source: String,
    pub note: Option<String>,
    pub recommended_account_id: Option<String>,
    pub recommended_account_label: Option<String>,
    pub recommended_account_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WecomWebhookResponse {
    errcode: Option<i64>,
    errmsg: Option<String>,
}

fn normalize_text(value: &str, fallback: &str) -> String {
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        fallback.to_string()
    } else {
        normalized
    }
}

pub(crate) fn mask_webhook_url(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "<empty>".to_string();
    }

    let Some(key_index) = trimmed.find("key=") else {
        return "***".to_string();
    };
    let key_start = key_index + 4;
    let key = &trimmed[key_start..];
    if key.len() <= 8 {
        return format!("{}***", &trimmed[..key_start]);
    }
    format!(
        "{}{}***{}",
        &trimmed[..key_start],
        &key[..4],
        &key[key.len() - 4..]
    )
}

fn command_stdout(command: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(command).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8(output.stdout).ok()?;
    let normalized = normalize_text(&value, "");
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn env_text(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        env::var(key)
            .ok()
            .map(|value| normalize_text(&value, ""))
            .filter(|value| !value.is_empty())
    })
}

fn local_device_name() -> String {
    #[cfg(target_os = "macos")]
    if let Some(name) = command_stdout("scutil", &["--get", "ComputerName"]) {
        return name;
    }

    if let Some(name) = env_text(&["COMPUTERNAME", "HOSTNAME"]) {
        return name;
    }

    if let Some(name) =
        command_stdout("hostname", &["-s"]).or_else(|| command_stdout("hostname", &[]))
    {
        return name;
    }

    "Unknown Device".to_string()
}

fn local_user_name() -> Option<String> {
    env_text(&["USER", "USERNAME", "LOGNAME"])
}

fn auto_local_actor_label() -> String {
    let device = local_device_name();
    match local_user_name() {
        Some(user) => format!("{} / {}", device, user),
        None => device,
    }
}

fn resolve_sender_label(sender_name: Option<&str>) -> String {
    let configured = sender_name.map(|value| normalize_text(value, ""));
    configured
        .filter(|value| !value.is_empty())
        .unwrap_or_else(auto_local_actor_label)
}

pub(crate) fn build_markdown_content(
    payload: &SwitchNotifyPayload,
    sender_name: Option<&str>,
) -> String {
    let platform = normalize_text(&payload.platform, "Unknown");
    let account = normalize_text(&payload.account_label, "-");
    let trigger_type = normalize_text(&payload.trigger_type, "manual");
    let trigger_source = normalize_text(&payload.trigger_source, "desktop");
    let sender_label = resolve_sender_label(sender_name);
    let switched_at = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let version = env!("CARGO_PKG_VERSION");

    let mut lines = vec![
        "### Cockpit Tools 账号切换通知".to_string(),
        format!("> 平台：<font color=\"info\">{}</font>", platform),
        format!("> 账号：{}", account),
        format!("> 通知来源：{}", sender_label),
        format!("> 触发类型：{}", trigger_type),
        format!("> 触发来源：{}", trigger_source),
        format!("> 切换时间：{}", switched_at),
        format!("> 应用版本：{}", version),
    ];

    if let Some(note) = payload
        .note
        .as_deref()
        .map(str::trim)
        .filter(|note| !note.is_empty())
    {
        lines.push(format!("> 备注：{}", note));
    }

    if let Some(recommended_account) = payload
        .recommended_account_label
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!(
            "> 推荐使用账号：<font color=\"warning\">{}</font>",
            normalize_text(recommended_account, "-")
        ));
        if let Some(reason) = payload
            .recommended_account_reason
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            lines.push(format!("> 推荐理由：{}", normalize_text(reason, "-")));
        }
    }

    lines.join("\n")
}

async fn send_wecom_markdown(webhook: String, content: String) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|err| format!("创建 HTTP 客户端失败: {}", err))?;
    let response = client
        .post(&webhook)
        .json(&serde_json::json!({
            "msgtype": "markdown",
            "markdown": {
                "content": content,
            },
        }))
        .send()
        .await
        .map_err(|err| format!("请求企微机器人失败: {}", err))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|err| format!("读取企微响应失败: {}", err))?;
    if !status.is_success() {
        return Err(format!(
            "企微机器人 HTTP 状态异常: status={}, body={}",
            status, body
        ));
    }

    if let Ok(parsed) = serde_json::from_str::<WecomWebhookResponse>(&body) {
        if parsed.errcode.unwrap_or(0) != 0 {
            return Err(format!(
                "企微机器人返回错误: errcode={:?}, errmsg={}",
                parsed.errcode,
                parsed.errmsg.unwrap_or_default()
            ));
        }
    }

    Ok(())
}

pub fn dispatch_switch_notification(payload: SwitchNotifyPayload) {
    let cfg = config::get_user_config();
    let webhook = cfg.wecom_switch_notify_webhook.trim().to_string();
    if webhook.is_empty() {
        logger::log_warn("[WeComSwitchNotify] 切号通知 webhook 为空");
        return;
    }

    let masked_webhook = mask_webhook_url(&webhook);
    let sender_name = cfg.wecom_switch_notify_sender_name.trim().to_string();
    let content = build_markdown_content(&payload, Some(sender_name.as_str()));
    let recommended_account_id = payload
        .recommended_account_id
        .as_deref()
        .map(|value| normalize_text(value, ""))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "-".to_string());
    let account_id = payload.account_id;
    let platform = payload.platform;

    tauri::async_runtime::spawn(async move {
        match send_wecom_markdown(webhook, content).await {
            Ok(()) => logger::log_info(&format!(
                "[WeComSwitchNotify] 切号通知发送成功: platform={}, account_id={}, recommended_account_id={}, webhook={}",
                platform, account_id, recommended_account_id, masked_webhook
            )),
            Err(err) => logger::log_warn(&format!(
                "[WeComSwitchNotify] 切号通知发送失败: platform={}, account_id={}, recommended_account_id={}, webhook={}, error={}",
                platform, account_id, recommended_account_id, masked_webhook, err
            )),
        }
    });
}

pub fn notify_switch(
    platform: &str,
    account_id: &str,
    account_label: &str,
    trigger_type: &str,
    trigger_source: &str,
    note: Option<&str>,
) {
    notify_switch_with_recommendation(
        platform,
        account_id,
        account_label,
        trigger_type,
        trigger_source,
        note,
        None,
        None,
        None,
    );
}

pub fn notify_switch_with_recommendation(
    platform: &str,
    account_id: &str,
    account_label: &str,
    trigger_type: &str,
    trigger_source: &str,
    note: Option<&str>,
    recommended_account_id: Option<&str>,
    recommended_account_label: Option<&str>,
    recommended_account_reason: Option<&str>,
) {
    dispatch_switch_notification(SwitchNotifyPayload {
        platform: platform.to_string(),
        account_id: account_id.to_string(),
        account_label: account_label.to_string(),
        trigger_type: trigger_type.to_string(),
        trigger_source: trigger_source.to_string(),
        note: note.map(str::to_string),
        recommended_account_id: recommended_account_id.map(str::to_string),
        recommended_account_label: recommended_account_label.map(str::to_string),
        recommended_account_reason: recommended_account_reason.map(str::to_string),
    });
}

#[cfg(test)]
mod tests {
    use super::{build_markdown_content, mask_webhook_url, normalize_text, SwitchNotifyPayload};

    #[test]
    fn mask_webhook_url_keeps_key_edges_only() {
        assert_eq!(
            mask_webhook_url("https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=12345678-1234"),
            "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=1234***1234"
        );
    }

    #[test]
    fn normalize_text_collapses_whitespace() {
        assert_eq!(normalize_text("  abc\n  def\tghi  ", "-"), "abc def ghi");
    }

    #[test]
    fn build_markdown_content_contains_plain_account_and_sender() {
        let content = build_markdown_content(
            &SwitchNotifyPayload {
                platform: "Codex".to_string(),
                account_id: "account-1".to_string(),
                account_label: "abcdef@example.com".to_string(),
                trigger_type: "manual".to_string(),
                trigger_source: "desktop".to_string(),
                note: None,
                recommended_account_id: None,
                recommended_account_label: None,
                recommended_account_reason: None,
            },
            Some("主力工作机"),
        );
        assert!(content.contains("Codex"));
        assert!(content.contains("abcdef@example.com"));
        assert!(content.contains("通知来源：主力工作机"));
    }

    #[test]
    fn build_markdown_content_contains_recommended_account() {
        let content = build_markdown_content(
            &SwitchNotifyPayload {
                platform: "Codex".to_string(),
                account_id: "account-1".to_string(),
                account_label: "current@example.com".to_string(),
                trigger_type: "manual".to_string(),
                trigger_source: "desktop".to_string(),
                note: None,
                recommended_account_id: Some("account-2".to_string()),
                recommended_account_label: Some("next@example.com".to_string()),
                recommended_account_reason: Some(
                    "Weekly 将在 2 小时后重置，剩余 Weekly 12%，5h 60%".to_string(),
                ),
            },
            Some("主力工作机"),
        );

        assert!(content.contains("推荐使用账号"));
        assert!(content.contains("next@example.com"));
        assert!(content.contains("推荐理由"));
        assert!(content.contains("Weekly 将在 2 小时后重置"));
    }
}
