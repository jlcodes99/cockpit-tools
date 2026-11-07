/// Best-effort IANA timezone name for the `timezone_id` query parameter.
/// MiniMax rejects requests without it (`1406010011 invalid timezone_id`).
pub fn iana_timezone() -> String {
    iana_time_zone::get_timezone()
        .unwrap_or_else(|_| "Asia/Shanghai".to_string())
}

use serde::Serialize;
use serde_json::Value;

const STATUS_PATH: &str = "/minimax-cloud/api/v1/signin/status";
const CLAIM_PATH: &str = "/minimax-cloud/api/v1/signin/claim";
const STATUS_UPCOMING: i64 = 1;
const STATUS_CLAIMABLE: i64 = 2;
const STATUS_CLAIMED: i64 = 3;
const STATUS_DISABLED: i64 = 4;

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinimaxCodeCheckinStatus {
    pub today_checked_in: bool,
    pub active: bool,
    pub points: i64,
    pub bonus_points: i64,
    pub streak_days: i32,
    pub day_index: Option<i32>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinimaxCodeCheckinResult {
    pub success: bool,
    #[serde(default)]
    pub already_claimed: bool,
    #[serde(default)]
    pub warning: Option<String>,
    pub message: Option<String>,
    pub credit: Option<i64>,
    pub streak_days: Option<i32>,
}

// A retry cannot repair an expired bearer token. Refresh belongs to the account
// layer; never replay a claim after an ambiguous network failure.
async fn request_json(
    client: &reqwest::Client,
    url: &str,
    access_token: &str,
    is_post: bool,
) -> Result<Value, String> {
    let request = if is_post {
        client.post(url).json(&serde_json::json!({}))
    } else {
        client.get(url)
    };
    let response = request.bearer_auth(access_token)
        .header("accept", "application/json")
        .send().await.map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status().as_u16()));
    }
    response.json::<Value>().await.map_err(|error| format!("解析响应失败: {error}"))
}

fn check_base_resp(body: &Value) -> Result<(), String> {
    if let Some(code) = body.get("base_resp").and_then(|v| v.get("status_code")).and_then(|v| v.as_i64()) {
        if code == 0 { return Ok(()); }
        let msg = body.get("base_resp").and_then(|v| v.get("status_msg")).and_then(|v| v.as_str()).unwrap_or("未知错误");
        return Err(format!("上游返回错误 {code}: {msg}"));
    }
    Ok(())
}

/// Maps an upstream `status` code to `(today_checked_in, active, message)`.
/// `points` is the day's total reward, used only for the claimable message.
fn classify_status(status: i64, points: i64) -> (bool, bool, String) {
    match status {
        STATUS_CLAIMABLE => (false, true, format!("可签到 +{points} 积分")),
        STATUS_CLAIMED => (true, true, "今日已签到".to_string()),
        STATUS_UPCOMING => (false, false, "今日暂不可签".to_string()),
        STATUS_DISABLED => (false, false, "今日签到不可用".to_string()),
        _ => (false, true, "签到状态未知".to_string()),
    }
}

fn encode_timezone() -> String {
    url::form_urlencoded::byte_serialize(iana_timezone().as_bytes()).collect::<String>()
}

pub async fn get_checkin_status(access_token: &str, region: &str) -> Result<MinimaxCodeCheckinStatus, String> {
    let origin = super::minimax_code_oauth::resolve_agent_origin(region);
    let tz = encode_timezone();
    let url = format!("{origin}{STATUS_PATH}?timezone_id={tz}");
    let client = super::minimax_code_oauth::build_client()?;
    let body = request_json(&client, &url, access_token, false).await?;
    parse_checkin_status(&body)
}

fn parse_checkin_status(body: &Value) -> Result<MinimaxCodeCheckinStatus, String> {
    check_base_resp(body)?;
    let days = body.get("data").and_then(|d| d.get("days")).and_then(Value::as_array)
        .ok_or_else(|| "MiniMax 签到响应缺少 days".to_string())?;
    let Some(today) = days.iter().find(|d| d.get("is_today").and_then(|v| v.as_bool()).unwrap_or(false)) else {
        return Ok(MinimaxCodeCheckinStatus {
            today_checked_in: false, active: false, points: 0, bonus_points: 0,
            streak_days: 0, day_index: None, message: "上游没有返回今天的签到信息".to_string(),
        });
    };
    let status = today.get("status").and_then(Value::as_i64).filter(|status| (1..=4).contains(status))
        .ok_or_else(|| "MiniMax 返回了未知签到状态".to_string())?;
    let points = today.get("points").and_then(Value::as_i64).filter(|points| *points >= 0)
        .ok_or_else(|| "MiniMax 签到响应缺少有效积分".to_string())?;
    let bonus = today.get("bonus_points").and_then(|v| v.as_i64()).unwrap_or(0);
    let day_index = today.get("day_no").and_then(|v| v.as_i64()).map(|v| v as i32);

    let (today_checked_in, active, message) = classify_status(status, points + bonus);

    Ok(MinimaxCodeCheckinStatus {
        today_checked_in,
        active,
        points: points + bonus,
        bonus_points: bonus,
        streak_days: compute_streak(days),
        day_index,
        message,
    })
}

pub async fn perform_checkin(access_token: &str, region: &str) -> Result<MinimaxCodeCheckinResult, String> {
    let origin = super::minimax_code_oauth::resolve_agent_origin(region);
    let tz = encode_timezone();
    let url = format!("{origin}{CLAIM_PATH}?timezone_id={tz}");
    let client = super::minimax_code_oauth::build_client()?;
    let body = request_json(&client, &url, access_token, true).await?;
    parse_checkin_result(&body)
}

fn parse_checkin_result(body: &Value) -> Result<MinimaxCodeCheckinResult, String> {
    check_base_resp(body)?;
    let data = &body["data"];
    let result = data.get("claim_result").and_then(Value::as_i64)
        .filter(|value| matches!(value, 1 | 2)).ok_or_else(|| "MiniMax 未返回有效领取结果".to_string())?;
    let points = data.get("points").and_then(Value::as_i64).filter(|value| *value >= 0)
        .ok_or_else(|| "MiniMax 未返回有效签到积分".to_string())?;
    let days = data.get("panel").and_then(|panel| panel.get("days")).and_then(Value::as_array)
        .ok_or_else(|| "MiniMax 未返回签到面板".to_string())?;
    let already_claimed = result == 2;
    Ok(MinimaxCodeCheckinResult {
        success: true, already_claimed, warning: None,
        message: Some(if already_claimed { "今日已签到".to_string() } else { format!("签到成功 +{points} 积分") }),
        credit: Some(points), streak_days: Some(compute_streak(days)),
    })
}

pub async fn get_account_checkin_status(account_id: &str) -> Result<MinimaxCodeCheckinStatus, String> {
    let account = super::minimax_code_account::load_account(account_id).ok_or_else(|| format!("账号不存在: {account_id}"))?;
    match get_checkin_status(&account.access_token, account.region.as_deref().unwrap_or("cn")).await {
        Err(error) if error.contains("HTTP 401") => {
            let updated = super::minimax_code_account::refresh_account_token(account_id).await?;
            get_checkin_status(&updated.access_token, updated.region.as_deref().unwrap_or("cn")).await
        }
        result => result,
    }
}

/// Manual and scheduled claims share the same serialization, persistence and quota refresh.
pub async fn checkin_account(account_id: &str) -> Result<MinimaxCodeCheckinResult, String> {
    use std::sync::{Arc, LazyLock, Mutex};
    static LOCKS: LazyLock<Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
        LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));
    let lock = LOCKS.lock().map_err(|error| error.to_string())?.entry(account_id.into())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))).clone();
    let _guard = lock.lock().await;
    let account = super::minimax_code_account::refresh_account_token(account_id).await?;
    let mut result = perform_checkin(&account.access_token, account.region.as_deref().unwrap_or("cn")).await?;
    if result.success {
        super::minimax_code_account::update_checkin_info(account_id, Some(chrono::Utc::now().timestamp()),
            result.streak_days.unwrap_or(0), result.credit.map(|credit| serde_json::json!({"credit":credit})))
            .map_err(|error| format!("签到已完成，但本地记录保存失败: {error}"))?;
        match super::minimax_code_account::refresh_account_token(account_id).await {
            Ok(updated) if updated.quota_query_last_error.is_none() => {},
            Ok(updated) => result.warning = Some(format!("签到已完成，积分刷新失败: {}",updated.quota_query_last_error.unwrap_or_default())),
            Err(error) => result.warning = Some(format!("签到已完成，积分刷新失败: {error}")),
        }
    }
    Ok(result)
}

fn compute_streak(days: &[Value]) -> i32 {
    let mut sorted: Vec<&Value> = days.iter().collect();
    sorted.sort_by_key(|d| d.get("day_no").and_then(|v| v.as_i64()).unwrap_or(0));
    let Some(today_index) = sorted.iter().position(|d| d.get("is_today").and_then(|v| v.as_bool()).unwrap_or(false)) else { return 0; };
    let today_status = sorted[today_index].get("status").and_then(|v| v.as_i64()).unwrap_or(0);
    let mut index = match today_status {
        STATUS_CLAIMED => today_index as i64,
        STATUS_CLAIMABLE => today_index as i64 - 1,
        _ => -1,
    };
    let mut streak = 0i32;
    while index >= 0 {
        if sorted[index as usize].get("status").and_then(|v| v.as_i64()).unwrap_or(0) != STATUS_CLAIMED { break; }
        streak += 1;
        index -= 1;
    }
    streak
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claimed_panel_and_duplicate_receipt_keep_the_real_streak_and_reward() {
        let days = serde_json::json!([
            {"day_no":1,"is_today":true,"status":3,"points":400,"bonus_points":0},
            {"day_no":2,"is_today":false,"status":1,"points":400,"bonus_points":0}
        ]);
        let status = parse_checkin_status(&serde_json::json!({"base_resp":{"status_code":0},"data":{"scene":2,"days":days}})).unwrap();
        assert!(status.today_checked_in);
        assert_eq!((status.points,status.streak_days),(400,1));
        for claim_result in [1,2] {
            let result = parse_checkin_result(&serde_json::json!({"base_resp":{"status_code":0},"data":{"claim_result":claim_result,"points":400,"panel":{"days":days}}})).unwrap();
            assert_eq!(result.already_claimed,claim_result==2);
            assert_eq!(result.streak_days,Some(1));
            assert_eq!(result.credit,Some(400));
        }
        assert!(parse_checkin_status(&serde_json::json!({"base_resp":{"status_code":0}})).is_err());
        assert!(parse_checkin_result(&serde_json::json!({"data":{"claim_result":0}})).is_err());
        let inactive = parse_checkin_status(&serde_json::json!({"data":{"days":[]}})).unwrap();
        assert!(!inactive.active);
    }

    #[test]
    fn compute_streak_counts_consecutive_claimed_days_ending_today() {
        let days = vec![
            serde_json::json!({"day_no": 1, "status": 3, "is_today": false}),
            serde_json::json!({"day_no": 2, "status": 3, "is_today": false}),
            serde_json::json!({"day_no": 3, "status": 3, "is_today": true}),
        ];
        assert_eq!(compute_streak(&days), 3);
    }

    #[test]
    fn compute_streak_stops_at_first_non_claimed() {
        let days = vec![
            serde_json::json!({"day_no": 1, "status": 3, "is_today": false}),
            serde_json::json!({"day_no": 2, "status": 1, "is_today": false}),
            serde_json::json!({"day_no": 3, "status": 2, "is_today": true}),
        ];
        assert_eq!(compute_streak(&days), 0);
    }

    #[test]
    fn iana_timezone_is_never_empty() {
        assert!(!iana_timezone().is_empty());
    }

    #[test]
    fn classify_status_maps_disabled_to_inactive() {
        let (checked_in, active, _) = classify_status(STATUS_DISABLED, 0);
        assert!(!checked_in);
        assert!(!active);
    }

    #[test]
    fn classify_status_maps_known_states() {
        assert_eq!(classify_status(STATUS_CLAIMABLE, 30).1, true);
        assert_eq!(classify_status(STATUS_CLAIMABLE, 30).2, "可签到 +30 积分");
        assert_eq!(classify_status(STATUS_CLAIMED, 0).0, true);
        assert_eq!(classify_status(STATUS_UPCOMING, 0).1, false);
    }

    #[test]
    fn encode_timezone_is_never_empty_and_has_no_raw_slashes() {
        let encoded = encode_timezone();
        assert!(!encoded.is_empty());
        assert!(!encoded.contains('/'));
    }
}
