// API 服务账号池合计口径测试：周窗耗尽账号的 5h 剩余不计入池合计。
// 与前端 getCodexEffectiveQuotaPercentages 的 weeklyBlocksHourly 对齐，
// 纯内存构造，不读取真实盘上集合。
use super::{add_api_service_window_sum, effective_pool_hourly_percentage, ApiServicePoolWindowSum};

#[test]
fn effective_hourly_is_zeroed_when_weekly_window_is_exhausted() {
    // 复现用户现场：账号 A 5h 剩余 100 但周剩余 0，实际已不可用。
    assert_eq!(effective_pool_hourly_percentage(true, 0, 100), 0);
    // 周窗仍有额度（或耗尽未到 0）时保留原值。
    assert_eq!(effective_pool_hourly_percentage(true, 42, 86), 86);
    // 周窗口不存在（未上报/未启用）时无法据此判定不可用，保留原值。
    assert_eq!(effective_pool_hourly_percentage(false, 0, 100), 100);
}

#[test]
fn pool_sum_excludes_hourly_of_weekly_exhausted_accounts() {
    // 账号 A：5h 100、周 0（耗尽）；账号 B：5h 86、周 42；账号 C：无 5h 窗口、周 94。
    let mut windows = Vec::new();

    add_api_service_window_sum(
        &mut windows,
        Some(300),
        "5h",
        effective_pool_hourly_percentage(true, 0, 100),
    );
    add_api_service_window_sum(&mut windows, Some(300), "5h", 86);
    add_api_service_window_sum(&mut windows, Some(10080), "Weekly", 0);
    add_api_service_window_sum(&mut windows, Some(10080), "Weekly", 42);
    add_api_service_window_sum(&mut windows, Some(10080), "Weekly", 94);

    windows.sort_by(|left, right| left.window_minutes.cmp(&right.window_minutes));

    let five_hour = windows
        .iter()
        .find(|item| item.key == "5h")
        .expect("5h bucket must exist");
    let weekly = windows
        .iter()
        .find(|item| item.key == "weekly")
        .expect("weekly bucket must exist");

    // 修复前菜单栏 5h 合计为 100+86=186（虚高）；修复后与会话区一致为 86。
    assert_eq!(five_hour.percentage, 86);
    assert_eq!(weekly.percentage, 136);
}

#[test]
fn pool_window_sum_bucket_keeps_min_window_minutes() {
    let mut windows = Vec::new();
    add_api_service_window_sum(&mut windows, Some(300), "5h", 100);
    add_api_service_window_sum(&mut windows, Some(280), "5h", 50);
    let bucket = windows
        .iter()
        .find(|item| item.key == "5h")
        .expect("5h bucket must exist");
    assert_eq!(bucket.percentage, 150);
    assert_eq!(bucket.window_minutes, 280);

    let empty: Vec<ApiServicePoolWindowSum> = Vec::new();
    assert!(empty.is_empty());
}
