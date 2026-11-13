use crate::utils::protobuf::{read_varint, skip_field};
use serde_json::{json, Value};

const DASHBOARD_URL: &str = "https://api2.cursor.sh/aiserver.v1.DashboardService";
const BALANCE_PATH: &str = "/grok_api_v2.GrokBuildBilling/ListPrepaidBalanceChanges";

pub(super) async fn dashboard_read(
    client: &reqwest::Client,
    token: &str,
    method: &str,
    request: Value,
) -> Result<Value, String> {
    let response = client
        .post(format!("{DASHBOARD_URL}/{method}"))
        .bearer_auth(token)
        .header("Connect-Protocol-Version", "1")
        .json(&request)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|err| {
            format!(
                "Cursor {method}: {}",
                super::cursor_account::format_reqwest_error(&err)
            )
        })?;
    let body: Value = response.json().await.map_err(|err| {
        format!(
            "解析 Cursor {method} 失败: {}",
            super::cursor_account::format_reqwest_error(&err)
        )
    })?;
    if !body.is_object() {
        return Err(format!("Cursor {method} 返回的不是对象"));
    }
    Ok(body)
}

pub(super) fn json_integer(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value.as_i64().or_else(|| value.as_str()?.parse().ok())
}

fn additional_usage_settings(billing: &Value, limit: Option<&Value>, credits: &mut Value) {
    if billing["eligible"].as_bool() != Some(true) {
        return;
    }
    if let Some(spent) = json_integer(billing.get("spentThisMonthCents")) {
        credits["monthlyUsedCents"] = json!(spent);
    }
    match billing.get("autoTopUp") {
        None | Some(Value::Null) => credits["autoTopUpEnabled"] = json!(false),
        Some(Value::Object(rule)) => {
            // ProtoJSON may omit a rule's default false scalar.
            let enabled = rule.get("enabled").map_or(Some(false), Value::as_bool);
            if let Some(enabled) = enabled {
                credits["autoTopUpEnabled"] = json!(enabled);
                if enabled {
                    if let Some(threshold) = json_integer(rule.get("thresholdCents")) {
                        credits["autoTopUpThresholdCents"] = json!(threshold);
                    }
                    if let Some(amount) = json_integer(rule.get("topUpAmountCents")) {
                        credits["autoTopUpAmountCents"] = json!(amount);
                    }
                }
            }
        }
        Some(_) => {}
    }

    let Some(limit) = limit else { return };
    // The desktop reads whole dollars here. Its maximum int32 means no cap.
    if limit["noUsageBasedAllowed"].as_bool() == Some(true) {
        credits["monthlyLimitCents"] = json!(0);
    } else if let Some(hard_limit) = json_integer(limit.get("hardLimit")) {
        credits["monthlyLimitCents"] = match hard_limit {
            n if n <= 0 => json!(0),
            n if n >= i64::from(i32::MAX) => Value::Null,
            n => json!(n * 100),
        };
    }
}

pub(super) async fn read_credits(client: &reqwest::Client, token: &str) -> Value {
    let (wallet, billing, limit) = tokio::join!(
        read_wallet_balance(client, token),
        dashboard_read(client, token, "GetUserPrepaidBilling", json!({"limit": 1})),
        dashboard_read(
            client,
            token,
            "GetHardLimit",
            json!({"personalScope": true})
        ),
    );
    let mut credits = json!({});
    match wallet {
        Ok(Some(cents)) => credits["balanceCents"] = json!(cents),
        Ok(None) => {}
        Err(err) => crate::modules::logger::log_warn(&format!("[Cursor Refresh] {err}")),
    }
    if let Err(err) = &limit {
        crate::modules::logger::log_warn(&format!("[Cursor Refresh] {err}"));
    }
    match billing {
        Ok(billing) => additional_usage_settings(&billing, limit.as_ref().ok(), &mut credits),
        Err(err) => crate::modules::logger::log_warn(&format!("[Cursor Refresh] {err}")),
    }
    credits
}

async fn read_wallet_balance(client: &reqwest::Client, token: &str) -> Result<Option<i64>, String> {
    let auth = dashboard_read(
        client,
        token,
        "GetXaiCommerceBearerToken",
        json!({"purpose": 2}),
    )
    .await?;
    if auth.get("refusal").is_some() {
        return Ok(None);
    }
    let credential = &auth["credential"];
    let wallet_token = credential["token"]
        .as_str()
        .filter(|token| !token.is_empty())
        .ok_or("Cursor 钱包响应缺少凭据")?;
    let mut url = credential["edgeBaseUrl"]
        .as_str()
        .and_then(|origin| reqwest::Url::parse(origin).ok())
        .filter(|url| {
            url.scheme() == "https"
                && url.host_str().is_some()
                && url.path() == "/"
                && url.query().is_none()
                && url.fragment().is_none()
                && url.username().is_empty()
                && url.password().is_none()
        })
        .ok_or("Cursor 钱包响应缺少有效 HTTPS 地址")?;
    url.set_path(BALANCE_PATH);
    let mut response = client
        .post(url)
        .bearer_auth(wallet_token)
        .header("Content-Type", "application/grpc-web+proto")
        .header("X-Grpc-Web", "1")
        .body(vec![0_u8; 5]) // gRPC-Web envelope containing google.protobuf.Empty
        .send()
        .await
        .map_err(|_| "请求 Cursor 钱包余额失败")?;
    if !response.status().is_success() {
        return Err(format!("Cursor 钱包余额返回 HTTP {}", response.status()));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "读取 Cursor 钱包余额失败")?
    {
        if body.len() + chunk.len() > 8 * 1024 * 1024 {
            return Err("Cursor 钱包流水响应超出大小限制".into());
        }
        body.extend_from_slice(&chunk);
    }
    decode_wallet_balance(&body)
}

fn decode_wallet_balance(mut body: &[u8]) -> Result<Option<i64>, String> {
    let mut message = None;
    let mut status = None;
    while !body.is_empty() {
        let header = body.get(..5).ok_or("Cursor 钱包响应帧不完整")?;
        let size = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
        let rest = &body[5..];
        let data = rest.get(..size).ok_or("Cursor 钱包响应帧不完整")?;
        match header[0] {
            0 if message.is_none() && status.is_none() => message = Some(data),
            0x80 if status.is_none() => {
                status = std::str::from_utf8(data).ok().and_then(|headers| {
                    headers
                        .lines()
                        .filter_map(|line| line.split_once(':'))
                        .find(|(key, _)| key.eq_ignore_ascii_case("grpc-status"))
                        .and_then(|(_, value)| value.trim().parse::<u16>().ok())
                });
            }
            _ => return Err("Cursor 钱包响应帧格式异常".into()),
        }
        body = &rest[size..];
    }
    if status != Some(0) {
        return Err("Cursor 钱包余额查询未成功".into());
    }

    // ListPrepaidBalanceChangesResponse.total (field 2) is a signed ledger
    // total. The desktop negates Cent.val (field 1) to display the balance.
    let message = message.ok_or("Cursor 钱包响应缺少余额消息")?;
    let Some(total) = protobuf_field(message, 2, 2)? else {
        return Ok(None);
    };
    let encoded = protobuf_field(total, 1, 0)?.unwrap_or_default();
    let cents = if encoded.is_empty() {
        0
    } else {
        read_varint(encoded, 0)?.0 as i64
    };
    cents
        .checked_neg()
        .map(Some)
        .ok_or_else(|| "Cursor 钱包余额超出范围".into())
}

fn protobuf_field(data: &[u8], field: u64, wire: u8) -> Result<Option<&[u8]>, String> {
    let mut pos = 0;
    while pos < data.len() {
        let (tag, value) = read_varint(data, pos)?;
        let actual_wire = (tag & 7) as u8;
        pos = skip_field(data, value, actual_wire)?;
        if pos > data.len() {
            return Err("Cursor 钱包 Protobuf 字段不完整".into());
        }
        if tag >> 3 == field && actual_wire == wire {
            let start = if wire == 2 {
                read_varint(data, value)?.1
            } else {
                value
            };
            return Ok(Some(&data[start..pos]));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::protobuf::{encode_len_delim_field, encode_varint_field};

    #[test]
    fn wallet_balance_reads_signed_total_and_requires_success() {
        let message = encode_len_delim_field(2, &encode_varint_field(1, (-1234_i64) as u64));
        let trailer = b"grpc-status: 0\r\n";
        let mut body = [
            vec![0],
            (message.len() as u32).to_be_bytes().to_vec(),
            message,
            vec![0x80],
            (trailer.len() as u32).to_be_bytes().to_vec(),
            trailer.to_vec(),
        ]
        .concat();
        assert_eq!(decode_wallet_balance(&body).unwrap(), Some(1234));
        let status = body.len() - 3;
        body[status] = b'7';
        assert!(decode_wallet_balance(&body).is_err());
        assert!(decode_wallet_balance(&body[..body.len() - 2]).is_err());
    }

    #[test]
    fn monthly_limit_distinguishes_unset_zero_and_amount() {
        let billing = json!({"eligible": true, "spentThisMonthCents": "0"});
        for (limit, expected) in [(0, json!(0)), (20, json!(2000)), (i32::MAX, Value::Null)] {
            let mut credits = json!({});
            additional_usage_settings(&billing, Some(&json!({"hardLimit": limit})), &mut credits);
            assert_eq!(credits["monthlyUsedCents"], 0);
            assert_eq!(credits["monthlyLimitCents"], expected);
        }
    }
    #[test]
    fn eligible_billing_preserves_top_up_rule_without_a_monthly_cap_response() {
        let mut credits = json!({});
        additional_usage_settings(
            &json!({"eligible": true, "autoTopUp": {
                "enabled": true, "thresholdCents": "500", "topUpAmountCents": "2000"
            }}),
            None,
            &mut credits,
        );
        assert_eq!(
            credits,
            json!({
                "autoTopUpEnabled": true, "autoTopUpThresholdCents": 500, "autoTopUpAmountCents": 2000
            })
        );
        let mut credits = json!({});
        additional_usage_settings(&json!({"eligible": true}), Some(&json!({})), &mut credits);
        assert_eq!(credits, json!({"autoTopUpEnabled": false}));
        let mut credits = json!({});
        additional_usage_settings(
            &json!({"eligible": true, "autoTopUp": {"thresholdCents": "500"}}),
            None,
            &mut credits,
        );
        assert_eq!(credits, json!({"autoTopUpEnabled": false}));
    }
}
