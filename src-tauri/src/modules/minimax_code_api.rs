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
    pub message: Option<String>,
    pub credit: Option<i64>,
    pub streak_days: Option<i32>,
}

/// MiniMax tokens are short-lived (~1h) and the client renews them; retry 401/403.
async fn request_with_retry(
    client: &reqwest::Client,
    url: &str,
    access_token: &str,
    is_post: bool,
) -> Result<Value, String> {
    let backoff = [2000u64, 5000, 10000];
    let mut last_err = "请求失败".to_string();
    let mut attempt = 0usize;
    loop {
        let result = if is_post {
            client.post(url).bearer_auth(access_token)
                .header("accept", "application/json")
                .header("content-type", "application/json")
                .json(&serde_json::json!({})).send().await
        } else {
            client.get(url).bearer_auth(access_token)
                .header("accept", "application/json")
                .header("accept-language", "zh-CN,zh;q=0.9").send().await
        };
        match result {
            Ok(resp) if resp.status().is_success() => {
                return resp.json::<Value>().await.map_err(|e| format!("解析响应失败: {e}"));
            }
            Ok(resp) => {
                let code = resp.status().as_u16();
                last_err = format!("HTTP {code}");
                let auth_error = code == 401 || code == 403;
                if !auth_error && attempt >= 1 { break; }
                if auth_error && attempt >= backoff.len() { break; }
            }
            Err(e) => { last_err = e.to_string(); if attempt >= 1 { break; } }
        }
        let delay = backoff.get(attempt).copied().unwrap_or(10000);
        attempt += 1;
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
    }
    Err(last_err)
}

fn check_base_resp(body: &Value) -> Result<(), String> {
    if let Some(code) = body.get("base_resp").and_then(|v| v.get("status_code")).and_then(|v| v.as_i64()) {
        if code == 0 { return Ok(()); }
        let msg = body.get("base_resp").and_then(|v| v.get("status_msg")).and_then(|v| v.as_str()).unwrap_or("未知错误");
        return Err(format!("上游返回错误 {code}: {msg}"));
    }
    Ok(())
}

pub async fn get_checkin_status(access_token: &str, region: &str) -> Result<MinimaxCodeCheckinStatus, String> {
    let origin = super::minimax_code_oauth::resolve_agent_origin(region);
    let tz = iana_timezone();
    let url = format!("{origin}{STATUS_PATH}?timezone_id={tz}");
    let client = super::minimax_code_oauth::build_client()?;
    let body = request_with_retry(&client, &url, access_token, false).await?;
    check_base_resp(&body)?;

    let empty = Vec::new();
    let days = body.get("data").and_then(|d| d.get("days")).and_then(|v| v.as_array()).unwrap_or(&empty);
    let Some(today) = days.iter().find(|d| d.get("is_today").and_then(|v| v.as_bool()).unwrap_or(false)) else {
        return Ok(MinimaxCodeCheckinStatus {
            today_checked_in: false, active: true, points: 0, bonus_points: 0,
            streak_days: 0, day_index: None, message: "上游没有返回今天的签到信息".to_string(),
        });
    };
    let status = today.get("status").and_then(|v| v.as_i64()).unwrap_or(0);
    let points = today.get("points").and_then(|v| v.as_i64()).unwrap_or(0);
    let bonus = today.get("bonus_points").and_then(|v| v.as_i64()).unwrap_or(0);
    let day_index = today.get("day_no").and_then(|v| v.as_i64()).map(|v| v as i32);

    let (today_checked_in, active, message) = match status {
        STATUS_CLAIMABLE => (false, true, format!("可签到 +{} 积分", points + bonus)),
        STATUS_CLAIMED => (true, true, "今日已签到".to_string()),
        STATUS_UPCOMING => (false, false, "今日暂不可签".to_string()),
        _ => (false, true, "签到状态未知".to_string()),
    };

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
    let tz = iana_timezone();
    let url = format!("{origin}{CLAIM_PATH}?timezone_id={tz}");
    let client = super::minimax_code_oauth::build_client()?;
    let body = request_with_retry(&client, &url, access_token, true).await?;
    check_base_resp(&body)?;

    let data = &body["data"];
    let result = data.get("claim_result").and_then(|v| v.as_i64()).unwrap_or(0);
    let points = data.get("points").and_then(|v| v.as_i64()).unwrap_or(0);
    match result {
        1 => Ok(MinimaxCodeCheckinResult { success: true, message: Some(format!("签到成功 +{points} 积分")), credit: Some(points), streak_days: None }),
        2 => Ok(MinimaxCodeCheckinResult { success: true, message: Some("今日已签到".to_string()), credit: Some(points), streak_days: None }),
        _ => Ok(MinimaxCodeCheckinResult { success: false, message: Some("上游未返回领取结果".to_string()), credit: None, streak_days: None }),
    }
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
}
