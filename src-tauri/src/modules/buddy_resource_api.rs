//! WorkBuddy 5.9 resource API, verified against main/account.js and authority-snapshot.js.
//! Refresh only: bounded pages/bytes and deadline; failures never return partial balances.
use reqwest::{header::HeaderMap, Client};
use serde_json::{json, Value};
use std::{collections::HashSet, time::Duration};

const PAGE_SIZE: usize = 200;
const MAX_PAGES: usize = 10;
const RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const GROUP_BYTES: usize = 4 * 1024 * 1024;
const PAID: &[&str] = &[
    "TCACA_code_002_AkiJS3ZHF5",
    "TCACA_code_005_maRGyrHhw1",
    "TCACA_code_003_FAnt7lcmRT",
    "TCACA_code_023_4xbGhMrE6q",
    "TCACA_code_026_BaESVICNoi",
    "TCACA_code_027_0FCGVA6vSa",
    "TCACA_code_043_6oIkxT7Z0S",
    "TCACA_code_009_0XmEQc2xOf",
    "TCACA_code_038_OhvqZtiPKr",
    "TCACA_code_036_lupO5WgNdG",
];
const FREE: &[&str] = &[
    "TCACA_code_001_PqouKr6QWV",
    "TCACA_code_008_cfWoLwvjU4",
    "TCACA_code_035_ArVxJcGDsm",
    "TCACA_code_006_DbXS0lrypC",
    "TCACA_code_039_KRcQj7wUat",
    "TCACA_code_040_mi9rCYg46x",
    "TCACA_code_007_nzdH5h4Nl0",
    "TCACA_code_028_NtpWi0jzXs",
    "TCACA_code_037_WxOD3MpI2o",
    "TCACA_code_029_6wCGEWquYy",
    "TCACA_code_030_BjSt89qTvr",
];

async fn post(
    client: &Client,
    base: &str,
    route: &str,
    headers: &HeaderMap,
    body: &Value,
) -> Result<(Value, usize), String> {
    let mut response = client
        .post(format!(
            "{}/billing/meter/{}",
            base.trim_end_matches('/'),
            route
        ))
        .headers(headers.clone())
        .json(body)
        .send()
        .await
        .map_err(|e| format!("BUDDY_RESOURCE_NETWORK: {}", e))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "BUDDY_RESOURCE_HTTP: {} {}",
            route,
            status.as_u16()
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if bytes.len() + chunk.len() > RESPONSE_BYTES {
            return Err("BUDDY_RESOURCE_RESPONSE_LIMIT".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("BUDDY_RESOURCE_JSON: {}", e))?;
    if let Some(code) = value.get("code").and_then(Value::as_i64) {
        if code != 0 && code != 200 {
            return Err(format!("BUDDY_RESOURCE_API: {} {}", route, code));
        }
    }
    let data = value
        .get("data")
        .filter(|v| v.is_object())
        .ok_or("BUDDY_RESOURCE_DATA_MISSING")?;
    Ok((data.clone(), bytes.len()))
}

fn page_body(paid: bool, page: usize, date: &str) -> Value {
    let mut body = json!({"PageNumber": page, "PageSize": PAGE_SIZE, "PackageCodes": if paid { PAID } else { FREE }, "Status": [0, 3]});
    if paid {
        body["NeedRenewInfo"] = json!(true);
    } else {
        body["SlicePeriodStartTime"] = json!(format!("{} 00:00:00", date));
        body["SlicePeriodEndTime"] = json!(format!("{} 23:59:59", date));
    }
    body
}

fn package_page(data: &Value) -> Result<(&[Value], Option<u64>), String> {
    let accounts = match data.get("Accounts") {
        None | Some(Value::Null) => &[][..],
        Some(Value::Array(items)) => items.as_slice(),
        _ => return Err("BUDDY_RESOURCE_ACCOUNTS_INVALID".into()),
    };
    let total = data
        .get("TotalCount")
        .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()))
        .filter(|count| *count > 0);
    Ok((accounts, total))
}

fn append_page(
    items: &mut Vec<Value>,
    total: &mut Option<u64>,
    page: &Value,
) -> Result<bool, String> {
    let (accounts, page_total) = package_page(page)?;
    *total = page_total.or(*total);
    if accounts.len() > PAGE_SIZE
        || total.is_some_and(|count| count > (PAGE_SIZE * MAX_PAGES) as u64)
    {
        return Err("BUDDY_RESOURCE_PAGE_LIMIT".into());
    }
    items.extend(accounts.iter().cloned());
    match *total {
        Some(count) if items.len() as u64 >= count => Ok(true),
        Some(_) if accounts.is_empty() => Err("BUDDY_RESOURCE_INCOMPLETE".into()),
        Some(_) => Ok(false),
        None => Ok(accounts.len() < PAGE_SIZE),
    }
}

async fn group(
    client: &Client,
    base: &str,
    headers: &HeaderMap,
    paid: bool,
    date: &str,
) -> Result<Vec<Value>, String> {
    let route = if paid {
        "get-user-resource-paid-packages"
    } else {
        "get-user-resource-free-packages"
    };
    let mut items = Vec::new();
    let mut total = None;
    let mut bytes = 0;
    for page in 1..=MAX_PAGES {
        let (data, size) = post(client, base, route, headers, &page_body(paid, page, date)).await?;
        bytes += size;
        if bytes > GROUP_BYTES {
            return Err("BUDDY_RESOURCE_GROUP_LIMIT".into());
        }
        if append_page(&mut items, &mut total, &data)? {
            return Ok(items);
        }
        tokio::task::yield_now().await;
    }
    Err("BUDDY_RESOURCE_INCOMPLETE".into())
}

fn merge(summary: Value, paid: Vec<Value>, free: Vec<Value>) -> Value {
    let mut seen = HashSet::new();
    let resources: Vec<_> = paid
        .into_iter()
        .chain(free)
        .filter(|item| {
            // Keep distinct anonymous records; official de-duplication only uses ResourceId.
            let id = item.get("ResourceId").and_then(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .or_else(|| v.as_u64().map(|n| n.to_string()))
            });
            match id.filter(|id| !id.is_empty()) {
                Some(id) => seen.insert(id),
                None => true,
            }
        })
        .collect();
    json!({"code": 0, "data": {"resources": resources, "resourceSummary": summary}})
}

pub async fn fetch(client: &Client, base: &str, headers: HeaderMap) -> Result<Value, String> {
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    tokio::time::timeout(Duration::from_secs(30), async {
        let summary_request = json!({});
        let ((summary, _), paid, free) = tokio::try_join!(
            post(
                client,
                base,
                "get-user-resource-summary",
                &headers,
                &summary_request
            ),
            group(client, base, &headers, true, &date),
            group(client, base, &headers, false, &date),
        )?;
        if summary
            .get("Packages")
            .is_some_and(|value| !value.is_array() && !value.is_null())
        {
            return Err("BUDDY_RESOURCE_SUMMARY_MISSING".into());
        }
        Ok(merge(summary, paid, free))
    })
    .await
    .map_err(|_| "BUDDY_RESOURCE_TIMEOUT".to_string())?
}

fn detail_body(page: u32, expired: bool, now: chrono::NaiveDateTime) -> Value {
    let mut body = json!({
        "PageNumber": page,
        "PageSize": PAGE_SIZE,
        "IsDisplayTotalInfo": true,
        "Status": if expired { [2, 3] } else { [0, 3] },
    });
    if expired {
        // WorkBuddy 5.9.3 uses a 101-year history window and descending end time.
        let begin = now
            .checked_sub_months(chrono::Months::new(101 * 12))
            .expect("the billing date is in the current era");
        body["PackageEndTimeRangeBegin"] = json!(begin.format("%Y-%m-%d %H:%M:%S").to_string());
        body["PackageEndTimeRangeEnd"] = json!(now.format("%Y-%m-%d %H:%M:%S").to_string());
        body["OrderBy"] = json!("endTime");
        body["SortBy"] = json!("desc");
    } else {
        body["OnlyValidPeriod"] = json!(true);
    }
    body
}

async fn detail_group(
    client: &Client,
    base: &str,
    headers: &HeaderMap,
    route: &str,
    mut body: Value,
    codes: Vec<&str>,
) -> Result<(Vec<Value>, bool), String> {
    if codes.is_empty() {
        return Ok((Vec::new(), false));
    }
    let page = body["PageNumber"].as_u64().unwrap_or(1);
    body["PackageCodes"] = json!(codes);
    let (data, _) = post(client, base, route, headers, &body).await?;
    let (accounts, total) = package_page(&data)?;
    let has_more = match total {
        Some(count) => page * (PAGE_SIZE as u64) < count,
        None => accounts.len() >= PAGE_SIZE,
    };
    let accounts = accounts
        .iter()
        .filter(|item| {
            let kind = &item["CapacityType"];
            kind.as_i64() != Some(4) && kind.as_str() != Some("4")
        })
        .cloned()
        .collect();
    Ok((accounts, has_more))
}

pub async fn fetch_credit_packages(
    client: &Client,
    base: &str,
    headers: HeaderMap,
    package_codes: &[String],
    expired: bool,
    page: u32,
) -> Result<Value, String> {
    let select_codes = |available: &'static [&str]| {
        available
            .iter()
            .copied()
            .filter(|code| package_codes.iter().any(|requested| requested == code))
            .collect()
    };
    let body = detail_body(page.max(1), expired, chrono::Local::now().naive_local());
    let ((mut paid, more_paid), (free, more_free)) = tokio::try_join!(
        detail_group(
            client,
            base,
            &headers,
            "get-user-resource-paid-packages",
            body.clone(),
            select_codes(PAID)
        ),
        detail_group(
            client,
            base,
            &headers,
            "get-user-resource-free-packages",
            body,
            select_codes(FREE)
        ),
    )?;
    paid.extend(free);
    Ok(json!({"items": paid, "hasMore": more_paid || more_free}))
}

pub async fn fetch_recent_credit_usage(
    client: &Client,
    base: &str,
    headers: HeaderMap,
) -> Result<Vec<Value>, String> {
    let today = chrono::Local::now().date_naive();
    let start = today - chrono::Days::new(30);
    let body = json!({
        "startTime": format!("{} 00:00:00", start.format("%Y-%m-%d")),
        "endTime": format!("{} 23:59:59", today.format("%Y-%m-%d")),
        "pageNum": 1,
        "pageSize": 5,
    });
    let (data, _) = post(client, base, "get-user-request-usage", &headers, &body).await?;
    let rows = data
        .get("data")
        .and_then(Value::as_array)
        .ok_or("WORKBUDDY_CREDIT_USAGE_MISSING")?;
    Ok(rows
        .iter()
        .take(5)
        .map(|row| {
            json!({
                "requestId": row["requestId"],
                "requestTime": row["requestTime"],
                "inputTrunc": row["inputTrunc"],
                "input": row["input"],
                "credit": row["credit"],
                "model": row["model"],
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requests_current_group_contract_and_daily_free_slice() {
        let paid = page_body(true, 2, "2026-11-01");
        assert_eq!(paid["PageNumber"], 2);
        assert_eq!(paid["PageSize"], 200);
        assert_eq!(paid["NeedRenewInfo"], true);
        assert!(paid.get("OnlyValidPeriod").is_none());
        let free = page_body(false, 1, "2026-11-01");
        assert_eq!(free["SlicePeriodStartTime"], "2026-11-01 00:00:00");
        assert_eq!(free["SlicePeriodEndTime"], "2026-11-01 23:59:59");
        assert!(free["PackageCodes"]
            .as_array()
            .unwrap()
            .contains(&json!("TCACA_code_040_mi9rCYg46x")));
    }
    #[test]
    fn pages_are_complete_or_fail_and_never_silently_truncate() {
        let mut items = Vec::new();
        let mut total = None;
        assert!(!append_page(
            &mut items,
            &mut total,
            &json!({"Accounts":[{"ResourceId":"one"}],"TotalCount":2})
        )
        .unwrap());
        assert!(append_page(&mut items, &mut total, &json!({"Accounts":[]})).is_err());
        assert!(append_page(
            &mut items,
            &mut total,
            &json!({"Accounts":[{"ResourceId":"two"}],"TotalCount":"2"})
        )
        .unwrap());
        assert!(append_page(
            &mut Vec::new(),
            &mut None,
            &json!({"Accounts":[],"TotalCount":2001})
        )
        .is_err());
        assert!(append_page(&mut Vec::new(), &mut None, &json!({"Accounts": null})).unwrap());
        assert!(append_page(&mut Vec::new(), &mut None, &json!({"Accounts": {}})).is_err());
    }

    #[test]
    fn valid_and_expired_package_requests_use_official_filters() {
        let now = "2026-11-05T09:18:00".parse().unwrap();
        let valid = detail_body(2, false, now);
        assert_eq!(valid["PageNumber"], 2);
        assert_eq!(valid["Status"], json!([0, 3]));
        assert_eq!(valid["OnlyValidPeriod"], true);
        assert_eq!(valid["IsDisplayTotalInfo"], true);
        assert!(valid.get("SlicePeriodStartTime").is_none());
        let expired = detail_body(1, true, now);
        assert_eq!(expired["Status"], json!([2, 3]));
        assert_eq!(expired["PackageEndTimeRangeBegin"], "1925-11-05 09:18:00");
        assert_eq!(expired["PackageEndTimeRangeEnd"], "2026-11-05 09:18:00");
        assert_eq!(expired["OrderBy"], "endTime");
        assert_eq!(expired["SortBy"], "desc");
        assert!(expired.get("OnlyValidPeriod").is_none());
    }
    #[test]
    fn merges_groups_without_double_counting_and_preserves_unknown_records() {
        let value = merge(
            json!({"Packages": []}),
            vec![
                json!({"ResourceId":"same"}),
                json!({"PackageCode":"anonymous-a"}),
            ],
            vec![
                json!({"ResourceId":"same"}),
                json!({"PackageCode":"anonymous-b"}),
            ],
        );
        assert_eq!(value["data"]["resources"].as_array().unwrap().len(), 3);
        assert_eq!(value["data"]["resourceSummary"]["Packages"], json!([]));
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn server(fail_free: bool) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    loop {
                        let mut buf = [0; 4096];
                        let size = stream.read(&mut buf).await.unwrap();
                        if size == 0 {
                            return;
                        }
                        request.extend_from_slice(&buf[..size]);
                        if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                            let header = String::from_utf8_lossy(&request[..end]).to_lowercase();
                            let length: usize = header
                                .lines()
                                .find_map(|line| {
                                    line.strip_prefix("content-length:")
                                        .and_then(|v| v.trim().parse().ok())
                                })
                                .unwrap_or(0);
                            if request.len() < end + 4 + length {
                                continue;
                            }
                            assert!(header.contains("authorization: bearer fixture"));
                            assert!(header.starts_with("post /billing/meter/get-user-"));
                            let body: Value =
                                serde_json::from_slice(&request[end + 4..end + 4 + length])
                                    .unwrap();
                            let free = header.contains("get-user-resource-free-packages");
                            let summary = header.contains("get-user-resource-summary");
                            let detail = body["IsDisplayTotalInfo"] == true;
                            let usage = header.contains("get-user-request-usage");
                            if usage {
                                assert_eq!(body["pageNum"], 1);
                                assert_eq!(body["pageSize"], 5);
                                assert!(body["startTime"].as_str().unwrap().ends_with(" 00:00:00"));
                                assert!(body["endTime"].as_str().unwrap().ends_with(" 23:59:59"));
                            } else if !summary {
                                assert_eq!(body["PageSize"], 200);
                            }
                            if detail {
                                assert!(free);
                                assert_eq!(body["PackageCodes"], json!([FREE[6]]));
                                assert_eq!(body["Status"], json!([2, 3]));
                                assert!(body.get("SlicePeriodStartTime").is_none());
                            } else if free {
                                assert!(body.get("SlicePeriodStartTime").is_some());
                            }
                            let data = if summary {
                                json!({"Packages": []})
                            } else if detail {
                                json!({"Accounts": [
                                    {"ResourceId": "daily", "CapacityType": 4},
                                    {"ResourceId": "grant", "CapacitySizePrecise": "100"}
                                ], "TotalCount": "201"})
                            } else if usage {
                                json!({"data": [
                                    {"requestId": "request", "requestTime": "2026-11-05 12:00:00",
                                     "credit": "1.25", "model": "fixture", "internalField": "ignored"}
                                ]})
                            } else {
                                json!({"Accounts": [{"ResourceId": "shared", "CycleCapacityRemainPrecise": "12.5"}], "TotalCount": 1})
                            };
                            let payload = json!({"code": 0, "data": data}).to_string();
                            let status = if free && fail_free {
                                "503 Service Unavailable"
                            } else {
                                "200 OK"
                            };
                            let response = format!("HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", status, payload.len(), payload);
                            let _ = stream.write_all(response.as_bytes()).await;
                            return;
                        }
                    }
                });
            }
        });
        (base, task)
    }

    #[tokio::test]
    async fn merges_all_official_sources_and_rejects_partial_failure() {
        for fail in [false, true] {
            let (base, task) = server(fail).await;
            let mut headers = HeaderMap::new();
            headers.insert("Authorization", "Bearer fixture".parse().unwrap());
            let result = fetch(
                &Client::builder().no_proxy().build().unwrap(),
                &base,
                headers,
            )
            .await;
            if fail {
                assert!(result.unwrap_err().contains("503"));
            } else {
                let value = result.unwrap();
                assert_eq!(value["data"]["resources"].as_array().unwrap().len(), 1);
                assert_eq!(
                    value["data"]["resources"][0]["CycleCapacityRemainPrecise"],
                    "12.5"
                );
            }
            task.abort();
        }
    }

    #[tokio::test]
    async fn expired_details_use_free_packages_and_keep_pagination_after_filtering() {
        let (base, task) = server(false).await;
        let mut headers = HeaderMap::new();
        headers.insert("Authorization", "Bearer fixture".parse().unwrap());
        let value = fetch_credit_packages(
            &Client::builder().no_proxy().build().unwrap(),
            &base,
            headers,
            &[FREE[6].to_string()],
            true,
            1,
        )
        .await
        .unwrap();
        assert_eq!(value["hasMore"], true);
        assert_eq!(
            value["items"],
            json!([{"ResourceId":"grant","CapacitySizePrecise":"100"}])
        );
        task.abort();
    }

    #[tokio::test]
    async fn request_usage_uses_official_preview_contract() {
        let (base, task) = server(false).await;
        let mut headers = HeaderMap::new();
        headers.insert("Authorization", "Bearer fixture".parse().unwrap());
        let rows = fetch_recent_credit_usage(
            &Client::builder().no_proxy().build().unwrap(),
            &base,
            headers,
        )
        .await
        .unwrap();
        assert_eq!(rows[0]["requestId"], "request");
        assert_eq!(rows[0]["credit"], "1.25");
        assert!(rows[0].get("internalField").is_none());
        task.abort();
    }
}
