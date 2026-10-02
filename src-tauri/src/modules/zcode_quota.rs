//! ZCode 额度查询。
//!
//! 请求构造与响应解析移植自 zcode-switch（quota.rs），直连官方接口：
//! - bigmodel 监控通道：`open.bigmodel.cn/api/monitor/usage/quota/limit`（+ 订阅列表）
//! - z.ai 余额通道：`zcode.z.ai/api/v1/zcode-plan/billing/balance`（模拟 ZCode 客户端请求头）

use chrono::{Local, TimeZone};
use reqwest::Client;
use serde_json::Value;
use std::time::Duration;

use crate::models::zcode::ZcodeQuotaItem;
use crate::modules::zcode_credentials::{
    default_secret, device_mid, new_gen_provider_config, node_platform_for, safe_decrypt,
};

pub const QUOTA_LIMIT_URL: &str = "https://open.bigmodel.cn/api/monitor/usage/quota/limit";
pub const SUBSCRIPTION_URL: &str = "https://open.bigmodel.cn/api/biz/subscription/list";
pub const BILLING_BALANCE_URL: &str = "https://zcode.z.ai/api/v1/zcode-plan/billing/balance";
const CLIENT_APP_VERSION: &str = "3.11.2";
const ZCODE_ORIGIN: &str = "https://zcode.z.ai";
const ZCODE_LANG: &str = "zh-CN";
const ZCODE_CHANNEL: &str = "stable";

#[cfg(windows)]
fn no_window(prog: &str) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut c = std::process::Command::new(prog);
    c.creation_flags(0x0800_0000);
    c
}

#[cfg(not(windows))]
fn no_window(prog: &str) -> std::process::Command {
    std::process::Command::new(prog)
}

pub(crate) fn client_platform() -> String {
    let os = node_platform_for(std::env::consts::OS);
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    };
    format!("{os}-{arch}")
}

fn os_version() -> Option<String> {
    static CACHE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            #[cfg(windows)]
            {
                let out = no_window("reg")
                    .args([
                        "query",
                        r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion",
                        "/v",
                        "CurrentBuildNumber",
                    ])
                    .output()
                    .ok()?;
                let txt = String::from_utf8_lossy(&out.stdout);
                let build = txt
                    .lines()
                    .find(|l| l.contains("CurrentBuildNumber"))?
                    .rsplit(' ')
                    .find(|t| !t.is_empty())?
                    .to_string();
                Some(format!("10.0.{build}"))
            }
            #[cfg(not(windows))]
            {
                let out = no_window("uname").arg("-r").output().ok()?;
                let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
                (!v.is_empty()).then_some(v)
            }
        })
        .clone()
}

// ponytail: 非 Windows 的时区仅支持 TZ 环境变量与 /etc/localtime 符链两种来源，
// 极端配置下回退 "unknown"；主流 macOS/Linux 结果与 zcode-switch 的 iana-time-zone 一致，
// 升级路径是引入 iana-time-zone crate。
fn client_timezone() -> String {
    static CACHE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            #[cfg(windows)]
            {
                let out = no_window("tzutil").arg("/g").output().ok();
                let name = out
                    .and_then(|o| String::from_utf8(o.stdout).ok())
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();
                return match name.as_str() {
                    "China Standard Time" | "China Daylight Time" => "Asia/Shanghai",
                    "Singapore Standard Time" => "Asia/Singapore",
                    "Tokyo Standard Time" => "Asia/Tokyo",
                    "UTC" => "UTC",
                    _ => "unknown",
                }
                .to_string();
            }
            #[cfg(not(windows))]
            {
                if let Ok(tz) = std::env::var("TZ") {
                    let tz = tz.trim();
                    if !tz.is_empty() {
                        return tz.to_string();
                    }
                }
                if let Ok(link) = std::fs::read_link("/etc/localtime") {
                    let path = link.to_string_lossy();
                    if let Some(idx) = path.rfind("zoneinfo/") {
                        return path[idx + "zoneinfo/".len()..].to_string();
                    }
                }
                "unknown".to_string()
            }
        })
        .clone()
}

pub(crate) fn zcode_app_version() -> String {
    static CACHE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CACHE
        .get_or_init(|| {
            #[cfg(windows)]
            {
                for hive in [
                    r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
                    r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
                    r"HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
                ] {
                    let Ok(out) = no_window("reg").args(["query", hive, "/s"]).output() else {
                        continue;
                    };
                    let txt = String::from_utf8_lossy(&out.stdout);
                    let (mut name, mut ver) = (String::new(), String::new());
                    for line in txt.lines() {
                        let l = line.trim();
                        if l.starts_with("HKEY_") {
                            if is_zcode_display_name(&name) && !ver.is_empty() {
                                return normalize_version(&ver);
                            }
                            name.clear();
                            ver.clear();
                            continue;
                        }
                        if let Some(rest) = l.strip_prefix("DisplayName") {
                            name = rest
                                .trim_start()
                                .trim_start_matches("REG_SZ")
                                .trim()
                                .to_string();
                        } else if let Some(rest) = l.strip_prefix("DisplayVersion") {
                            ver = rest
                                .trim_start()
                                .trim_start_matches("REG_SZ")
                                .trim()
                                .to_string();
                        }
                    }
                    if is_zcode_display_name(&name) && !ver.is_empty() {
                        return normalize_version(&ver);
                    }
                }
            }
            CLIENT_APP_VERSION.to_string()
        })
        .clone()
}

#[cfg(windows)]
fn is_zcode_display_name(name: &str) -> bool {
    let l = name.to_lowercase();
    l.contains("zcode") && !l.contains("switch")
}

#[cfg(windows)]
fn normalize_version(v: &str) -> String {
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() >= 3 {
        format!("{}.{}.{}", parts[0], parts[1], parts[2])
    } else {
        v.to_string()
    }
}

fn zai_billing_headers(token: &str, mid: Option<String>) -> Vec<(String, String)> {
    let ver = zcode_app_version();
    let mut h: Vec<(String, String)> = vec![
        ("User-Agent".into(), format!("ZCode/{ver}")),
        ("HTTP-Referer".into(), ZCODE_ORIGIN.into()),
        ("X-Title".into(), "Z Code@electron".into()),
        ("X-ZCode-App-Version".into(), ver),
        ("X-Platform".into(), client_platform()),
        ("X-Release-Channel".into(), ZCODE_CHANNEL.into()),
        ("X-Client-Language".into(), ZCODE_LANG.into()),
        ("X-Client-Timezone".into(), client_timezone()),
        ("X-Os-Category".into(), std::env::consts::OS.into()),
    ];
    if let Some(v) = os_version() {
        h.push(("X-Os-Version".into(), v));
    }
    if let Some(mid) = mid {
        h.push(("X-Device-Mid".into(), mid));
    }
    h.push(("Authorization".into(), format!("Bearer {token}")));
    h.push(("x-request-id".into(), uuid::Uuid::new_v4().to_string()));
    h
}

fn bigmodel_headers(token: &str) -> Vec<(String, String)> {
    vec![
        ("Authorization".into(), format!("Bearer {token}")),
        (
            "User-Agent".into(),
            format!("ZCode/{}", zcode_app_version()),
        ),
        ("x-request-id".into(), uuid::Uuid::new_v4().to_string()),
    ]
}

// ---------------------------------------------------------------------------
// 查询结果结构
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct QuotaOverview {
    pub total: Option<f64>,
    pub used: Option<f64>,
    pub remaining: Option<f64>,
    pub percent_used: Option<f64>,
    pub plan_tier: Option<String>,
    pub plan_expire: Option<String>,
    pub is_empty: bool,
    pub items: Vec<ZcodeQuotaItem>,
    pub source: String,
    plans: Vec<PlanSlot>,
}

#[derive(Debug, Clone, Default)]
struct PlanSlot {
    pid: String,
    tier: Option<String>,
    name: Option<String>,
    expire: Option<String>,
    total: Option<f64>,
    used: Option<f64>,
    remaining: Option<f64>,
    percent_used: Option<f64>,
    items: Vec<ZcodeQuotaItem>,
}

// ---------------------------------------------------------------------------
// token 候选与通道选择
// ---------------------------------------------------------------------------

fn looks_like_token(v: &str) -> bool {
    v.trim().len() > 20
}

fn coding_plan_api_keys(config: Option<&Value>) -> Vec<String> {
    let mut keys = vec![];
    let Some(providers) = config
        .and_then(|c| c.get("provider"))
        .and_then(|p| p.as_object())
    else {
        return keys;
    };
    let mut ordered: Vec<(&String, &Value)> = providers.iter().collect();
    ordered.sort_by_key(|(_, p)| {
        if p.get("enabled").and_then(|e| e.as_bool()).unwrap_or(false) {
            0
        } else {
            1
        }
    });
    for (id, p) in ordered {
        if !id.contains("coding-plan") {
            continue;
        }
        if let Some(k) = p
            .get("options")
            .and_then(|o| o.get("apiKey"))
            .and_then(|k| k.as_str())
        {
            if !k.starts_with("enc:") && looks_like_token(k) && !keys.contains(&k.to_string()) {
                keys.push(k.to_string());
            }
        }
    }
    keys
}

fn coding_plan_keys_from_creds(creds: &Value, secret: &str) -> Vec<String> {
    let Some(map) = creds.as_object() else {
        return vec![];
    };
    let mut out: Vec<String> = vec![];
    for (k, v) in map {
        if !k.starts_with("account-provider:") || !k.ends_with(":api-key") {
            continue;
        }
        if !k.contains("coding-plan") {
            continue;
        }
        let Some(p) = v.as_str().and_then(|v| safe_decrypt(Some(v), secret)) else {
            continue;
        };
        if looks_like_token(&p) && !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

pub fn candidate_tokens(
    creds: &Value,
    config: Option<&Value>,
    secret: &str,
    new_gen: bool,
) -> Vec<String> {
    let mut tokens: Vec<String> = vec![];
    let add = |plain: Option<String>, tokens: &mut Vec<String>| {
        if let Some(p) = plain {
            if looks_like_token(&p) && !tokens.contains(&p) {
                tokens.push(p);
            }
        }
    };
    let creds_keys = coding_plan_keys_from_creds(creds, secret);
    if new_gen {
        for k in &creds_keys {
            add(Some(k.clone()), &mut tokens);
        }
    }
    for k in coding_plan_api_keys(config) {
        tokens.push(k);
    }
    let active = safe_decrypt(
        creds.get("oauth:active_provider").and_then(|v| v.as_str()),
        secret,
    )
    .unwrap_or_else(|| "zai".into());
    let map = creds.as_object();
    add(
        map.and_then(|m| m.get("zcodejwttoken"))
            .and_then(|v| v.as_str())
            .and_then(|v| safe_decrypt(Some(v), secret)),
        &mut tokens,
    );
    for key in [
        format!("oauth:{active}:access_token"),
        "oauth:bigmodel:access_token".to_string(),
        "oauth:zai:access_token".to_string(),
    ] {
        add(
            map.and_then(|m| m.get(&key))
                .and_then(|v| v.as_str())
                .and_then(|v| safe_decrypt(Some(v), secret)),
            &mut tokens,
        );
    }
    if !new_gen {
        for k in creds_keys {
            add(Some(k), &mut tokens);
        }
    }
    tokens
}

fn zai_billing_token(creds: &Value, config: Option<&Value>, secret: &str) -> Option<String> {
    let jwt = safe_decrypt(creds.get("zcodejwttoken").and_then(|v| v.as_str()), secret);
    let active = safe_decrypt(
        creds.get("oauth:active_provider").and_then(|v| v.as_str()),
        secret,
    );
    let use_jwt = if active.as_deref() != Some("bigmodel") {
        jwt.is_some()
    } else {
        false
    };
    if use_jwt {
        return jwt;
    }
    let providers = config?.get("provider")?.as_object()?;
    let mut ordered: Vec<(&String, &Value)> = providers.iter().collect();
    ordered.sort_by_key(|(_, p)| {
        if p.get("enabled").and_then(|e| e.as_bool()).unwrap_or(false) {
            0
        } else {
            1
        }
    });
    for (id, p) in ordered {
        if id.contains("start-plan") {
            if let Some(k) = p
                .get("options")
                .and_then(|o| o.get("apiKey"))
                .and_then(|k| k.as_str())
            {
                if !k.starts_with("enc:") && looks_like_token(k) {
                    return Some(k.to_string());
                }
            }
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq)]
enum Channel {
    Monitor(String),
    ZaiBilling(String),
}

fn pick_channels(
    creds: &Value,
    config: Option<&Value>,
    secret: &str,
    new_gen: bool,
) -> Vec<Channel> {
    let mut chans: Vec<Channel> = vec![];
    if new_gen {
        for k in coding_plan_keys_from_creds(creds, secret) {
            if !chans.contains(&Channel::Monitor(k.clone())) {
                chans.push(Channel::Monitor(k));
            }
        }
        if let Some(t) = zai_billing_token(creds, config, secret) {
            if !chans.contains(&Channel::ZaiBilling(t.clone())) {
                chans.push(Channel::ZaiBilling(t));
            }
        }
    }
    let Some(providers) = config
        .and_then(|c| c.get("provider"))
        .and_then(|p| p.as_object())
    else {
        return chans;
    };
    let mut ordered: Vec<(&String, &Value)> = providers.iter().collect();
    ordered.sort_by_key(|(_, p)| {
        if p.get("enabled").and_then(|e| e.as_bool()).unwrap_or(false) {
            0
        } else {
            1
        }
    });
    let provider_key = |p: &Value| -> Option<String> {
        let k = p
            .get("options")
            .and_then(|o| o.get("apiKey"))
            .and_then(|k| k.as_str())?;
        (!k.starts_with("enc:") && looks_like_token(k)).then(|| k.to_string())
    };
    for (id, p) in ordered {
        let key = provider_key(p);
        if id.contains("start-plan") {
            let jwt = safe_decrypt(creds.get("zcodejwttoken").and_then(|v| v.as_str()), secret);
            let active = safe_decrypt(
                creds.get("oauth:active_provider").and_then(|v| v.as_str()),
                secret,
            );
            let use_jwt = if id.starts_with("builtin:zai") {
                jwt.is_some()
            } else {
                jwt.is_some() && active.as_deref() == Some("bigmodel")
            };
            let tok = if use_jwt { jwt } else { None }.or(key);
            if let Some(t) = tok {
                if !chans.contains(&Channel::ZaiBilling(t.clone())) {
                    chans.push(Channel::ZaiBilling(t));
                }
            }
        } else if id.contains("coding-plan") {
            if let Some(k) = key {
                if !chans.contains(&Channel::Monitor(k.clone())) {
                    chans.push(Channel::Monitor(k));
                }
            }
        }
    }
    if !new_gen {
        for k in coding_plan_keys_from_creds(creds, secret) {
            if !chans.contains(&Channel::Monitor(k.clone())) {
                chans.push(Channel::Monitor(k));
            }
        }
    }
    chans
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

async fn http_get_json(
    client: &Client,
    url: &str,
    token: &str,
    retry_429: bool,
) -> Result<Value, String> {
    let retry_delays = [500u64, 1500, 4000];
    let mut headers: Vec<(String, String)> = if url.contains("zcode.z.ai") {
        zai_billing_headers(token, device_mid())
    } else {
        bigmodel_headers(token)
    };
    // reqwest 对非法头值会在发送时整体失败，这里兜底剔除（与 ureq 的宽容行为对齐）
    headers.retain(|(_, v)| !v.contains('\n') && !v.contains('\r'));

    let mut backoff = if retry_429 {
        retry_delays.iter()
    } else {
        [].iter()
    };
    let mut last_err: Option<String> = None;
    loop {
        let mut req = client.get(url).timeout(Duration::from_secs(20));
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        match req.send().await {
            Ok(resp) => {
                let code = resp.status().as_u16();
                let body = resp.text().await.unwrap_or_default();
                if let Ok(v) = serde_json::from_str::<Value>(&body) {
                    if v.get("code").and_then(|c| c.as_i64()) == Some(401) {
                        return Err("登录凭证已失效（业务 401）".to_string());
                    }
                }
                if code == 429 {
                    match backoff.next() {
                        Some(d) => {
                            last_err = Some("请求过于频繁，已触发限流".to_string());
                            tokio::time::sleep(Duration::from_millis(*d)).await;
                            continue;
                        }
                        None => {
                            return Err(
                                last_err.unwrap_or_else(|| "请求被限流（HTTP 429）".to_string())
                            )
                        }
                    }
                }
                if !(200..300).contains(&code) {
                    if code == 401 || code == 403 {
                        return Err(format!("凭证无效或已过期（HTTP {code}）"));
                    }
                    let msg = serde_json::from_str::<Value>(&body)
                        .ok()
                        .and_then(|v| {
                            ["message", "msg", "error"]
                                .iter()
                                .find_map(|k| v.get(k).and_then(|x| x.as_str()).map(String::from))
                        })
                        .unwrap_or_default();
                    return Err(format!("额度接口请求失败（HTTP {code}）: {msg}"));
                }
                return Ok(if body.is_empty() {
                    Value::Null
                } else {
                    serde_json::from_str(&body).unwrap_or(Value::String(body.clone()))
                });
            }
            Err(e) => return Err(format!("网络请求失败: {e}")),
        }
    }
}

fn business_ok(v: &Value) -> bool {
    let code = v.get("code").and_then(|c| c.as_i64());
    let success = v.get("success").and_then(|s| s.as_bool());
    (code.is_none() || code == Some(200) || code == Some(0)) && success != Some(false)
}

fn biz_err_message(resp: &Value, default: &str) -> String {
    let msg = ["msg", "message", "error"]
        .iter()
        .find_map(|k| resp.get(k).and_then(|x| x.as_str()).map(String::from))
        .unwrap_or_default();
    if msg.is_empty() {
        return default.to_string();
    }
    let code = resp.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
    format!("额度接口返回业务错误（code {code}）: {msg}")
}

fn is_no_plan_message(msg: &str) -> bool {
    msg.contains("不存在coding plan") || msg.contains("没有资格")
}

async fn query_with_token(client: &Client, token: &str) -> Result<QuotaOverview, String> {
    let mut best_err: Option<String>;

    match http_get_json(client, QUOTA_LIMIT_URL, token, true).await {
        Ok(limit_resp) => {
            if business_ok(&limit_resp) {
                let sub = http_get_json(client, SUBSCRIPTION_URL, token, true)
                    .await
                    .ok();
                let mut ov = normalize_quota_limit(&limit_resp, sub.as_ref());
                ov.source = "bigmodel.cn/api/monitor".into();
                return Ok(ov);
            }
            best_err = Some(match limit_resp.get("code").and_then(|c| c.as_i64()) {
                Some(401) => "登录凭证已失效（业务 401）".to_string(),
                Some(_) => biz_err_message(&limit_resp, "额度接口返回业务错误"),
                None => "额度接口响应格式异常".to_string(),
            });
        }
        Err(e) => best_err = Some(e),
    }

    let url = format!("{BILLING_BALANCE_URL}?app_version={}", zcode_app_version());
    match http_get_json(client, &url, token, true).await {
        Ok(balance) if business_ok(&balance) => {
            let mut ov = normalize_balance(&balance);
            ov.source = "zcode.z.ai/billing".into();
            return Ok(ov);
        }
        Ok(resp) => {
            if best_err.is_none() {
                best_err = Some(biz_err_message(&resp, "额度接口响应格式异常"));
            }
        }
        Err(e) => {
            if best_err.is_none() {
                best_err = Some(e);
            }
        }
    }

    Err(best_err.unwrap_or_else(|| "额度查询失败".to_string()))
}

async fn query_tokens(client: &Client, tokens: &[String]) -> Result<QuotaOverview, String> {
    if tokens.is_empty() {
        return Err("没有可用的 ZCode 凭证 token".to_string());
    }
    let mut last_err: Option<String> = None;
    let mut first_business: Option<String> = None;
    let mut auth_fail = 0usize;
    for t in tokens {
        match query_with_token(client, t).await {
            Ok(ov) => return Ok(ov),
            Err(e) => {
                if e.contains("401") {
                    auth_fail += 1;
                } else if first_business.is_none() {
                    first_business = Some(e.clone());
                }
                last_err = Some(e);
            }
        }
    }
    if auth_fail > 0 && auth_fail == tokens.len() {
        tokio::time::sleep(Duration::from_millis(1500)).await;
        if let Ok(ov) = query_with_token(client, &tokens[0]).await {
            return Ok(ov);
        }
        return Err("ZCode 登录凭证已全部过期".to_string());
    }
    Err(first_business
        .or(last_err)
        .unwrap_or_else(|| "额度查询失败".to_string()))
}

async fn query_channels(client: &Client, channels: &[Channel]) -> Result<QuotaOverview, String> {
    let mut best_err: Option<String> = None;
    let mut saw_no_plan = false;
    let mut parts: Vec<QuotaOverview> = vec![];
    for ch in channels {
        match ch {
            Channel::Monitor(key) => {
                match http_get_json(client, QUOTA_LIMIT_URL, key, true).await {
                    Ok(resp) => {
                        if business_ok(&resp) {
                            let sub = http_get_json(client, SUBSCRIPTION_URL, key, true)
                                .await
                                .ok();
                            let mut ov = normalize_quota_limit(&resp, sub.as_ref());
                            ov.source = "bigmodel.cn/api/monitor".into();
                            parts.push(ov);
                        } else {
                            let msg = ["msg", "message", "error"]
                                .iter()
                                .find_map(|k| {
                                    resp.get(k).and_then(|x| x.as_str()).map(String::from)
                                })
                                .unwrap_or_default();
                            if is_no_plan_message(&msg) {
                                saw_no_plan = true;
                            } else if best_err.is_none() {
                                best_err = Some(biz_err_message(&resp, "额度接口返回业务错误"));
                            }
                        }
                    }
                    Err(e) => {
                        if best_err.is_none() {
                            best_err = Some(e);
                        }
                    }
                }
            }
            Channel::ZaiBilling(tok) => {
                let url = format!("{BILLING_BALANCE_URL}?app_version={}", zcode_app_version());
                match http_get_json(client, &url, tok, true).await {
                    Ok(balance) if business_ok(&balance) => {
                        let mut ov = normalize_balance(&balance);
                        ov.source = "zcode.z.ai/billing".into();
                        if ov.plan_tier.is_some() || !ov.plans.is_empty() {
                            parts.push(ov);
                        } else {
                            saw_no_plan = true;
                        }
                    }
                    Ok(resp) => {
                        if best_err.is_none() {
                            best_err = Some(biz_err_message(&resp, "额度接口响应格式异常"));
                        }
                    }
                    Err(e) => {
                        if best_err.is_none() {
                            best_err = Some(e);
                        }
                    }
                }
            }
        }
    }
    if parts.is_empty() {
        if saw_no_plan {
            return Ok(QuotaOverview {
                is_empty: true,
                source: "no_plan".into(),
                ..Default::default()
            });
        }
        return Err(best_err.unwrap_or_else(|| "额度查询失败".to_string()));
    }
    Ok(merge_parts(parts))
}

fn merge_parts(parts: Vec<QuotaOverview>) -> QuotaOverview {
    let mut slots: Vec<PlanSlot> = vec![];
    let mut slot_src: Vec<String> = vec![];
    let mut sources: Vec<String> = vec![];
    for p in &parts {
        if !sources.contains(&p.source) {
            sources.push(p.source.clone());
        }
        for s in &p.plans {
            let dup = slot_src.iter().enumerate().any(|(i, src)| {
                src != &p.source
                    && slots[i].tier == s.tier
                    && slots[i].name == s.name
                    && !slots[i].items.is_empty()
                    && !s.items.is_empty()
            });
            if !dup {
                slots.push(s.clone());
                slot_src.push(p.source.clone());
            }
        }
    }
    let items: Vec<ZcodeQuotaItem> = slots.iter().flat_map(|s| s.items.iter().cloned()).collect();
    let has_content = |s: &PlanSlot| s.tier.is_some() || !s.items.is_empty() || s.total.is_some();
    let mut pri_idx: Option<usize> = None;
    for (i, s) in slots.iter().enumerate() {
        if !has_content(s) {
            continue;
        }
        pri_idx = match pri_idx {
            None => Some(i),
            Some(p) if tier_rank(&s.tier) > tier_rank(&slots[p].tier) => Some(i),
            _ => pri_idx,
        };
    }
    let (total, used, remaining, percent_used) = match pri_idx.map(|i| &slots[i]) {
        Some(p) => (p.total, p.used, p.remaining, p.percent_used),
        None => (None, None, None, None),
    };
    QuotaOverview {
        total,
        used,
        remaining,
        percent_used,
        plan_tier: pri_idx.and_then(|i| slots[i].tier.clone()),
        plan_expire: pri_idx.and_then(|i| slots[i].expire.clone()),
        is_empty: slots.is_empty(),
        items,
        source: sources.join(" + "),
        plans: slots,
    }
}

/// 查询一组 ZCode 凭证对应的额度（通道优先，候选 token 兜底）。
pub async fn query_credentials_quota(
    creds: &Value,
    config: Option<&Value>,
) -> Result<QuotaOverview, String> {
    let home = crate::modules::zcode_credentials::home_dir();
    let secret = default_secret(&home);
    let new_gen = new_gen_provider_config();
    let client = crate::utils::http::create_client(20);

    let channels = pick_channels(creds, config, &secret, new_gen);
    if !channels.is_empty() {
        if let Ok(ov) = query_channels(&client, &channels).await {
            return Ok(ov);
        }
    }
    let tokens = candidate_tokens(creds, config, &secret, new_gen);
    query_tokens(&client, &tokens).await
}

// ---------------------------------------------------------------------------
// 响应解析
// ---------------------------------------------------------------------------

fn unit_label(unit: Option<i64>, number: Option<i64>) -> String {
    match unit {
        Some(3) => format!("每 {} 小时", number.unwrap_or(5)),
        Some(4) => "每天".into(),
        Some(5) => "每月".into(),
        Some(6) => "每周".into(),
        _ => "每周期".into(),
    }
}

fn fmt_reset_time(ms: Option<i64>) -> Option<String> {
    let ms = ms?;
    if ms <= 0 {
        return None;
    }
    Some(
        Local
            .timestamp_millis_opt(ms)
            .single()?
            .format("%m-%d %H:%M")
            .to_string(),
    )
}

fn safe_prefix(s: &str, n: usize) -> &str {
    if s.len() <= n {
        return s;
    }
    let mut end = n;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn safe_suffix(s: &str, n: usize) -> &str {
    if s.len() <= n {
        return s;
    }
    let mut start = s.len() - n;
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    &s[start..]
}

fn looks_like_date(d: &str) -> bool {
    d.len() == 10 && d.as_bytes().get(4) == Some(&b'-') && d.as_bytes().get(7) == Some(&b'-')
}

fn looks_like_dt(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() < 16 {
        return false;
    }
    let p = &b[..16];
    p[4] == b'-'
        && p[7] == b'-'
        && p[10] == b' '
        && p[13] == b':'
        && (0..16).all(|i| match i {
            0..=3 | 5..=6 | 8..=9 | 11..=12 | 14..=15 => p[i].is_ascii_digit(),
            _ => true,
        })
}

fn format_epoch(n: i64) -> Option<String> {
    if n > 1_000_000_000_000 {
        return Local
            .timestamp_millis_opt(n)
            .single()
            .map(|t| t.format("%Y-%m-%d %H:%M").to_string());
    }
    if n > 1_000_000_000 {
        return Local
            .timestamp_opt(n, 0)
            .single()
            .map(|t| t.format("%Y-%m-%d %H:%M").to_string());
    }
    None
}

fn extract_expire(obj: &Value) -> Option<String> {
    const KEYS: [&str; 12] = [
        "nextRenewTime",
        "expireTime",
        "expire_time",
        "endTime",
        "end_time",
        "expireAt",
        "expiredTime",
        "validEndTime",
        "expires_at",
        "expiresAt",
        "expired_at",
        "period_end",
    ];
    for k in KEYS {
        let Some(v) = obj.get(k) else { continue };
        if let Some(n) = v.as_i64() {
            if let Some(s) = format_epoch(n) {
                return Some(s);
            }
        }
        if let Some(s) = v.as_str() {
            let t = s.trim();
            if t.is_empty() {
                continue;
            }
            if let Ok(n) = t.parse::<i64>() {
                if let Some(f) = format_epoch(n) {
                    return Some(f);
                }
            }
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(t) {
                return Some(
                    dt.with_timezone(&Local)
                        .format("%Y-%m-%d %H:%M")
                        .to_string(),
                );
            }
            let t = if t.as_bytes().get(10) == Some(&b'T') {
                format!("{} {}", &t[..10], &t[11..])
            } else {
                t.to_string()
            };
            let d = safe_prefix(&t, 10);
            if looks_like_date(d) {
                if looks_like_dt(&t) {
                    return Some(safe_prefix(&t, 16).to_string());
                }
                return Some(d.to_string());
            }
            return Some(t.to_string());
        }
    }
    if let Some(s) = obj.get("valid").and_then(|v| v.as_str()) {
        let tail = safe_suffix(s, 19);
        if looks_like_dt(tail) {
            return Some(safe_prefix(tail, 16).to_string());
        }
        if looks_like_date(safe_prefix(tail, 10)) {
            return Some(safe_prefix(tail, 10).to_string());
        }
        let d = safe_prefix(s, 10);
        if looks_like_date(d) {
            return Some(d.to_string());
        }
    }
    None
}

fn expiry_field(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => {
            let t = s.trim();
            (!t.is_empty()).then(|| t.to_string())
        }
        Value::Number(_) => {
            let mut o = serde_json::Map::new();
            o.insert("expires_at".to_string(), v.clone());
            extract_expire(&Value::Object(o))
        }
        _ => None,
    }
}

fn tier_from_level(level: &str) -> String {
    let l = level.to_lowercase();
    if l.contains("max") {
        "Max".into()
    } else if l.contains("pro") {
        "Pro".into()
    } else if l.contains("lite") {
        "Lite".into()
    } else {
        level.to_string()
    }
}

fn tier_rank(tier: &Option<String>) -> u8 {
    let Some(t) = tier else { return 0 };
    let t = t.to_lowercase();
    if t.contains("max") {
        5
    } else if t.contains("pro") {
        4
    } else if t.contains("lite") {
        3
    } else if t.contains("start") {
        2
    } else if t.contains("trial") || t.contains("体验") {
        1
    } else {
        0
    }
}

fn is_active_coding_plan_entry(s: &Value) -> bool {
    let coding_name = ["productId", "productName"].iter().any(|k| {
        s.get(k)
            .and_then(|v| v.as_str())
            .map(|v| v.to_lowercase().contains("coding"))
            .unwrap_or(false)
    });
    if !coding_name {
        return false;
    }
    s.get("inCurrentPeriod").and_then(|v| v.as_bool()) == Some(true)
        && s.get("status").and_then(|v| v.as_str()) == Some("VALID")
}

fn normalize_quota_limit(limit_resp: &Value, sub_resp: Option<&Value>) -> QuotaOverview {
    let data = limit_resp.get("data").cloned().unwrap_or(Value::Null);
    let limits = data
        .get("limits")
        .and_then(|l| l.as_array())
        .cloned()
        .unwrap_or_default();

    let mut items = vec![];
    let mut main: Option<ZcodeQuotaItem> = None;
    for l in &limits {
        let typ = l.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let unit = l.get("unit").and_then(|u| u.as_i64());
        let number = l.get("number").and_then(|n| n.as_i64());
        let total = l.get("usage").and_then(|v| v.as_f64());
        let used = l.get("currentValue").and_then(|v| v.as_f64());
        let remaining = l.get("remaining").and_then(|v| v.as_f64());
        let percentage = l.get("percentage").and_then(|v| v.as_f64());
        let reset_ms = l.get("nextResetTime").and_then(|v| v.as_i64());
        let period = unit_label(unit, number);
        let (name, unit_str) = match typ {
            "TOKENS_LIMIT" => (format!("提示次数（{period}）"), "次".to_string()),
            "TIME_LIMIT" => (format!("使用时长（{period}）"), "分钟".to_string()),
            other => (format!("{other}（{period}）"), String::new()),
        };
        let percent_used = match (total, used) {
            (Some(t), Some(u)) if t > 0.0 => Some((u / t * 100.0).clamp(0.0, 100.0)),
            _ => percentage.map(|p| p.clamp(0.0, 100.0)),
        };
        let item = ZcodeQuotaItem {
            name,
            total,
            used,
            remaining,
            percent_used,
            unit: unit_str,
            window: String::new(),
            period_end: fmt_reset_time(reset_ms).map(|r| format!("{r} 重置")),
        };
        if typ == "TIME_LIMIT" && total.is_some() {
            main = main.or(Some(item.clone()));
        }
        items.push(item);
    }

    let level = data.get("level").and_then(|l| l.as_str()).map(String::from);
    let mut plan_tier = level.as_deref().map(tier_from_level);
    let mut plan_expire: Option<String> = None;
    let mut product_name: Option<String> = None;
    if let Some(sub) = sub_resp {
        if business_ok(sub) {
            if let Some(arr) = sub.get("data").and_then(|d| d.as_array()) {
                let current = arr
                    .iter()
                    .find(|s| is_active_coding_plan_entry(s))
                    .or_else(|| arr.first());
                if let Some(s) = current {
                    if let Some(pn) = s.get("productName").and_then(|x| x.as_str()) {
                        if !pn.trim().is_empty() {
                            product_name = Some(pn.to_string());
                            plan_tier = Some(tier_from_level(pn));
                        }
                    }
                    plan_expire = extract_expire(s);
                }
            }
        }
    }

    let main = main.or_else(|| items.iter().find(|i| i.total.is_some()).cloned());
    let (total, used, remaining, percent_used) = match &main {
        Some(m) => (m.total, m.used, m.remaining, m.percent_used),
        None => (None, None, None, items.iter().find_map(|i| i.percent_used)),
    };

    let plans = if plan_tier.is_some() || !items.is_empty() {
        vec![PlanSlot {
            pid: String::new(),
            tier: plan_tier.clone(),
            name: product_name,
            expire: plan_expire.clone(),
            total,
            used,
            remaining,
            percent_used,
            items: items.clone(),
        }]
    } else {
        vec![]
    };

    QuotaOverview {
        total,
        used,
        remaining,
        percent_used,
        plan_tier,
        plan_expire,
        is_empty: limits.is_empty(),
        items,
        source: String::new(),
        plans,
    }
}

fn unwrap_data(data: &Value) -> Value {
    let mut cur = data.clone();
    for _ in 0..4 {
        if !cur.is_object() {
            return cur;
        }
        if let Some(d) = cur.get("data") {
            cur = d.clone();
            continue;
        }
        if let Some(r) = cur.get("result") {
            cur = r.clone();
            continue;
        }
        break;
    }
    cur
}

fn to_number(v: &Value) -> Option<f64> {
    if let Some(n) = v.as_f64() {
        return if n.is_finite() { Some(n) } else { None };
    }
    if let Some(s) = v.as_str() {
        let t = s.replace(',', "");
        return t.trim().parse::<f64>().ok();
    }
    None
}

fn flatten_numbers(obj: &Value, prefix: &str, out: &mut Vec<(String, f64)>) {
    if let Some(m) = obj.as_object() {
        for (k, v) in m {
            let p = if prefix.is_empty() {
                k.clone()
            } else {
                format!("{prefix}.{k}")
            };
            if let Some(n) = to_number(v) {
                out.push((p, n));
            } else {
                flatten_numbers(v, &p, out);
            }
        }
    } else if let Some(arr) = obj.as_array() {
        for (i, v) in arr.iter().enumerate() {
            let p = format!("{prefix}.{i}");
            if let Some(n) = to_number(v) {
                out.push((p, n));
            } else {
                flatten_numbers(v, &p, out);
            }
        }
    }
}

fn sum_numbers(pool: &[(String, f64)], keys: &[&str]) -> Option<f64> {
    let mut total = 0.0;
    let mut count = 0;
    for (path, v) in pool {
        let name = path.rsplit('.').next().unwrap_or("");
        if keys.contains(&name) {
            total += v;
            count += 1;
        }
    }
    (count > 0).then_some(total)
}

fn first_number(pool: &[(String, f64)], keys: &[&str]) -> Option<f64> {
    for (path, v) in pool {
        let name = path.rsplit('.').next().unwrap_or("");
        if keys.contains(&name) {
            return Some(*v);
        }
    }
    None
}

fn extract_plan_tier(current_data: &Value) -> Option<String> {
    let cur = unwrap_data(current_data);
    let plans = cur.get("plans").and_then(|p| p.as_array())?;
    let active: Vec<&Value> = plans
        .iter()
        .filter(|p| {
            p.get("status")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_lowercase()
                == "active"
        })
        .collect();
    if active.is_empty() {
        return None;
    }
    let matches = |kw: &[&str]| {
        active.iter().any(|p| {
            let id = p
                .get("plan_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase();
            let name = p
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase();
            kw.iter().any(|k| id.contains(k) || name.contains(k))
        })
    };
    if matches(&["max"]) {
        Some("Max".into())
    } else if matches(&["pro"]) {
        Some("Pro".into())
    } else if matches(&["lite"]) {
        Some("Lite".into())
    } else if matches(&["start-plan", "start plan", "start"]) {
        Some("Start Plan".into())
    } else {
        None
    }
}

fn plan_tier_from_id(plan_id: &str, name: Option<&str>) -> String {
    let mut hay = plan_id.to_lowercase();
    if let Some(n) = name {
        hay.push(' ');
        hay.push_str(&n.to_lowercase());
    }
    if hay.contains("max") {
        "Max".into()
    } else if hay.contains("pro") {
        "Pro".into()
    } else if hay.contains("lite") {
        "Lite".into()
    } else if hay.contains("start") {
        "Start Plan".into()
    } else if [
        "trial",
        "taste",
        "experience",
        "gift",
        "weekend",
        "promo",
        "activity",
        "体验",
    ]
    .iter()
    .any(|k| hay.contains(k))
    {
        "体验".into()
    } else {
        plan_id.to_string()
    }
}

fn normalize_balance(balance_data: &Value) -> QuotaOverview {
    let balance = unwrap_data(balance_data);
    let mut pool = vec![];
    flatten_numbers(&balance, "", &mut pool);

    let mut slots: Vec<PlanSlot> = balance
        .get("plans")
        .and_then(|p| p.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|pl| {
                    pl.get("status")
                        .and_then(|s| s.as_str())
                        .map(|s| s.eq_ignore_ascii_case("active"))
                        .unwrap_or(false)
                })
                .map(|pl| {
                    let pid = pl
                        .get("plan_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let pname = pl.get("name").and_then(|v| v.as_str()).map(str::to_string);
                    PlanSlot {
                        pid: pid.clone(),
                        tier: Some(plan_tier_from_id(&pid, pname.as_deref())),
                        name: Some(pname.filter(|s| !s.trim().is_empty()).unwrap_or(pid)),
                        expire: extract_expire(pl),
                        ..Default::default()
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let mut loose: Vec<ZcodeQuotaItem> = vec![];
    let any_pid = balance
        .get("balances")
        .and_then(|b| b.as_array())
        .map(|arr| {
            arr.iter().any(|item| {
                ["plan_id", "planId", "entitlement_id"].iter().any(|k| {
                    item.get(k)
                        .and_then(Value::as_str)
                        .map(|s| !s.is_empty())
                        .unwrap_or(false)
                })
            })
        })
        .unwrap_or(false);
    if let Some(arr) = balance.get("balances").and_then(|b| b.as_array()) {
        for item in arr {
            let it_total = item.get("total_units").and_then(to_number);
            let it_used = item.get("used_units").and_then(to_number);
            let it_remaining = item
                .get("remaining_units")
                .and_then(to_number)
                .or_else(|| item.get("available_units").and_then(to_number));
            let it = ZcodeQuotaItem {
                name: ["show_name", "name", "entitlement_id", "plan_id"]
                    .iter()
                    .find_map(|k| item.get(k).and_then(Value::as_str))
                    .unwrap_or("Unknown")
                    .to_string(),
                total: it_total,
                used: it_used,
                remaining: it_remaining,
                percent_used: match (it_total, it_used) {
                    (Some(t), Some(u)) if t > 0.0 => Some((u / t * 100.0).clamp(0.0, 100.0)),
                    _ => None,
                },
                unit: item
                    .get("unit_type")
                    .or_else(|| item.get("meter"))
                    .and_then(Value::as_str)
                    .unwrap_or("quota")
                    .to_string(),
                window: String::new(),
                period_end: ["period_end", "expires_at"]
                    .iter()
                    .find_map(|k| item.get(k).and_then(expiry_field)),
            };
            let bpid = ["plan_id", "planId", "entitlement_id"]
                .iter()
                .find_map(|k| item.get(k).and_then(Value::as_str))
                .unwrap_or("");
            let target = if !bpid.is_empty() {
                slots.iter_mut().find(|s| s.pid == bpid)
            } else if slots.len() == 1 && !any_pid {
                slots.first_mut()
            } else {
                None
            };
            match target {
                Some(s) => {
                    if s.expire.is_none() {
                        s.expire = extract_expire(item);
                    }
                    s.items.push(it);
                }
                None => loose.push(it),
            }
        }
    }

    if slots.is_empty() {
        if !loose.is_empty() {
            let t = extract_plan_tier(balance_data);
            slots.push(PlanSlot {
                tier: t,
                items: std::mem::take(&mut loose),
                ..Default::default()
            });
        } else {
            let ptot = sum_numbers(&pool, &["total_units"]).or_else(|| {
                first_number(
                    &pool,
                    &[
                        "total",
                        "totalQuota",
                        "totalCredits",
                        "quotaTotal",
                        "amountTotal",
                        "creditTotal",
                    ],
                )
            });
            let pused = sum_numbers(&pool, &["used_units"]).or_else(|| {
                first_number(
                    &pool,
                    &[
                        "used",
                        "usedQuota",
                        "usedCredits",
                        "quotaUsed",
                        "amountUsed",
                        "consumed",
                        "totalUsed",
                    ],
                )
            });
            let prem = sum_numbers(&pool, &["remaining_units"]).or_else(|| {
                first_number(
                    &pool,
                    &[
                        "remaining",
                        "remain",
                        "balance",
                        "available",
                        "availableQuota",
                        "left",
                        "quotaRemaining",
                    ],
                )
            });
            if ptot.is_some() || pused.is_some() || prem.is_some() {
                let t = extract_plan_tier(balance_data);
                slots.push(PlanSlot {
                    tier: t,
                    total: ptot,
                    used: pused,
                    remaining: prem,
                    ..Default::default()
                });
            }
        }
    } else if !loose.is_empty() {
        slots.push(PlanSlot {
            name: Some("其他额度".into()),
            items: loose,
            ..Default::default()
        });
    }

    for s in &mut slots {
        let sum = |f: fn(&ZcodeQuotaItem) -> Option<f64>| -> Option<f64> {
            let vals: Vec<f64> = s.items.iter().filter_map(f).collect();
            (!vals.is_empty()).then(|| vals.iter().sum())
        };
        if s.items.is_empty() && s.total.is_none() {
            continue;
        }
        s.total = sum(|i| i.total).or(s.total);
        s.used = sum(|i| i.used).or(s.used);
        s.remaining = sum(|i| i.remaining).or(s.remaining);
        if s.total.is_none() {
            if let (Some(u), Some(r)) = (s.used, s.remaining) {
                s.total = Some(u + r);
            }
        }
        if s.used.is_none() {
            if let (Some(t), Some(r)) = (s.total, s.remaining) {
                s.used = Some((t - r).max(0.0));
            }
        }
        if s.remaining.is_none() {
            if let (Some(t), Some(u)) = (s.total, s.used) {
                s.remaining = Some((t - u).max(0.0));
            }
        }
        if s.percent_used.is_none() {
            s.percent_used = match (s.total, s.used) {
                (Some(t), Some(u)) if t > 0.0 => Some((u / t * 100.0).clamp(0.0, 100.0)),
                _ => None,
            };
        }
    }

    let mut pri_idx = 0usize;
    for (i, s) in slots.iter().enumerate() {
        if tier_rank(&s.tier) > tier_rank(&slots[pri_idx].tier) {
            pri_idx = i;
        }
    }

    let items: Vec<ZcodeQuotaItem> = slots.iter().flat_map(|s| s.items.iter().cloned()).collect();

    let plan_expire_chain = extract_expire(&balance)
        .or_else(|| {
            balance
                .get("plans")
                .and_then(|p| p.as_array())
                .and_then(|arr| {
                    arr.iter()
                        .find(|pl| {
                            pl.get("status")
                                .and_then(|s| s.as_str())
                                .map(|s| s.eq_ignore_ascii_case("active"))
                                .unwrap_or(false)
                        })
                        .and_then(extract_expire)
                        .or_else(|| arr.first().and_then(extract_expire))
                })
        })
        .or_else(|| {
            balance
                .get("balances")
                .and_then(|b| b.as_array())
                .and_then(|arr| {
                    arr.iter()
                        .filter_map(|it| it.get("expires_at").and_then(|v| v.as_i64()))
                        .filter(|n| *n > 1_000_000_000)
                        .max()
                        .and_then(|n| {
                            Local
                                .timestamp_opt(n, 0)
                                .single()
                                .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                        })
                })
        })
        .or_else(|| {
            items
                .iter()
                .find_map(|i| i.period_end.clone().filter(|s| !s.is_empty()))
        });

    if slots.len() == 1 && slots[0].expire.is_none() {
        slots[0].expire = plan_expire_chain.clone();
    }
    let (total, used, remaining, percent_used) = match slots.get(pri_idx) {
        Some(p) => (p.total, p.used, p.remaining, p.percent_used),
        None => (None, None, None, items.iter().find_map(|i| i.percent_used)),
    };

    QuotaOverview {
        total,
        used,
        remaining,
        percent_used,
        plan_tier: slots.get(pri_idx).and_then(|p| p.tier.clone()),
        plan_expire: slots
            .get(pri_idx)
            .and_then(|p| p.expire.clone())
            .or(plan_expire_chain),
        is_empty: balance
            .get("balances")
            .and_then(|b| b.as_array())
            .map(|a| a.is_empty())
            .unwrap_or(false),
        items,
        source: String::new(),
        plans: slots,
    }
}

// ---------------------------------------------------------------------------
// 自检测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn business_envelope() {
        assert!(business_ok(&json!({"code": 200, "data": {}})));
        assert!(business_ok(&json!({"data": {}})));
        assert!(business_ok(&json!({"code": 0})));
        assert!(!business_ok(&json!({"code": 401})));
        assert!(!business_ok(&json!({"success": false})));
    }

    #[test]
    fn bigmodel_limit_basic() {
        let resp = json!({
            "code": 200,
            "data": {
                "level": "GLM Max",
                "limits": [
                    {
                        "type": "TOKENS_LIMIT",
                        "unit": 3,
                        "number": 5,
                        "usage": 600,
                        "currentValue": 120,
                        "remaining": 480,
                        "nextResetTime": 1750000000000i64
                    },
                    {
                        "type": "TIME_LIMIT",
                        "unit": 4,
                        "usage": 300,
                        "currentValue": 30
                    }
                ]
            }
        });
        let ov = normalize_quota_limit(&resp, None);
        assert_eq!(ov.plan_tier.as_deref(), Some("Max"));
        assert_eq!(ov.items.len(), 2);
        assert!(ov.items[0].name.contains("提示次数"));
        assert!(ov.items[0].name.contains("每 5 小时"));
        assert_eq!(ov.items[0].total, Some(600.0));
        assert_eq!(ov.items[0].used, Some(120.0));
        assert!((ov.items[0].percent_used.unwrap() - 20.0).abs() < 1e-6);
        // 主摘要取 TIME_LIMIT（第一个带 total 的项）
        assert_eq!(ov.total, Some(300.0));
        assert_eq!(ov.used, Some(30.0));
        assert!(ov.items[0].period_end.as_deref().unwrap().contains("重置"));
    }

    #[test]
    fn balance_plans_and_units() {
        let resp = json!({
            "code": 200,
            "data": {
                "plans": [
                    {"plan_id": "zcode-plan-pro", "name": "GLM Pro", "status": "active"}
                ],
                "balances": [
                    {
                        "plan_id": "zcode-plan-pro",
                        "show_name": "GLM-4.6",
                        "total_units": 1000,
                        "used_units": 250,
                        "remaining_units": 750,
                        "expires_at": 1760000000i64
                    }
                ]
            }
        });
        let ov = normalize_balance(&resp);
        assert_eq!(ov.plan_tier.as_deref(), Some("Pro"));
        assert_eq!(ov.total, Some(1000.0));
        assert_eq!(ov.used, Some(250.0));
        assert_eq!(ov.remaining, Some(750.0));
        assert!((ov.percent_used.unwrap() - 25.0).abs() < 1e-6);
        assert_eq!(ov.items.len(), 1);
        assert_eq!(ov.items[0].name, "GLM-4.6");
        assert!(ov.plan_expire.is_some());
    }

    #[test]
    fn balance_flat_fallback() {
        let resp = json!({"code": 200, "data": {"totalQuota": 500, "usedQuota": 100, "quotaRemaining": 400}});
        let ov = normalize_balance(&resp);
        assert_eq!(ov.total, Some(500.0));
        assert_eq!(ov.used, Some(100.0));
        assert_eq!(ov.remaining, Some(400.0));
    }

    #[test]
    fn candidate_tokens_order_plain() {
        let creds = json!({
            "zcodejwttoken": "jwt-token-abcdefghijklmnopqrstuvwxyz",
            "oauth:active_provider": "zai",
            "oauth:zai:access_token": "zai-token-abcdefghijklmnopqrstuvwxyz"
        });
        let tokens = candidate_tokens(&creds, None, "secret", false);
        // 明文值顺序: zcodejwttoken → oauth:zai:access_token
        assert_eq!(tokens.len(), 2);
        assert!(tokens[0].starts_with("jwt-"));
        assert!(tokens[1].starts_with("zai-"));
    }

    #[test]
    fn expire_extraction_variants() {
        // epoch 秒：与 format_epoch 输出一致（同机器时区）
        assert_eq!(
            extract_expire(&json!({"expires_at": 1760000000i64})).as_deref(),
            format_epoch(1760000000i64).as_deref()
        );
        // 纯日期字符串原样返回（与时区无关）
        assert_eq!(
            extract_expire(&json!({"endTime": "2026-01-02"})).as_deref(),
            Some("2026-01-02")
        );
        // RFC3339 只断言可解析出结果
        assert!(extract_expire(&json!({"expireTime": "2026-01-02T03:04:05Z"})).is_some());
        assert_eq!(extract_expire(&json!({})), None);
    }

    #[test]
    fn zai_headers_shape() {
        let h = zai_billing_headers("tok", Some("mid-1".into()));
        let get = |k: &str| h.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
        assert_eq!(get("Authorization").as_deref(), Some("Bearer tok"));
        assert_eq!(get("X-Device-Mid").as_deref(), Some("mid-1"));
        assert_eq!(
            get("X-Platform").as_deref().map(str::to_string),
            Some(client_platform())
        );
        assert!(get("User-Agent").unwrap().starts_with("ZCode/"));
        assert!(h.iter().any(|(k, _)| k == "X-ZCode-App-Version"));
    }

    #[test]
    fn tier_detection() {
        assert_eq!(tier_from_level("GLM Max"), "Max");
        assert_eq!(plan_tier_from_id("zcode-plan-pro", Some("GLM Pro")), "Pro");
        assert_eq!(plan_tier_from_id("start-plan", None), "Start Plan");
        assert_eq!(tier_rank(&Some("Max".into())), 5);
        assert_eq!(tier_rank(&None), 0);
    }
}
