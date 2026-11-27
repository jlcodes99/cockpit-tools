//! Qoder / Qoder CN 配额预警。
//!
//! 两条通道的账户来源不同（全局走 Qoder IDE 存储，CN 走原生客户端适配层），
//! 但判定、冷却与载荷完全一致，所以判定只写一份，由 [`QoderStore`] 挑账户。

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use crate::models::qoder::QoderAccount;
use crate::modules::account::{self, QuotaAlertPayload};

/// 同一通道、同一阈值在冷却窗口内只提醒一次，避免自动刷新反复弹窗。
const COOLDOWN_SECONDS: i64 = 300;

static LAST_SENT: LazyLock<Mutex<HashMap<String, i64>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 配额预警覆盖的通道。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QoderStore {
    Global,
    Cn,
}

impl QoderStore {
    fn kind(self) -> &'static str {
        match self {
            Self::Global => "qoder",
            Self::Cn => "qoder_cn",
        }
    }

    fn load_accounts(self) -> Result<Vec<QoderAccount>, String> {
        match self {
            Self::Global => super::qoder_account::list_accounts_checked(),
            Self::Cn => Ok(super::qoder_native::list_accounts()),
        }
    }

    fn resolve_current_id(self, accounts: &[QoderAccount]) -> Option<String> {
        match self {
            Self::Global => super::qoder_account::resolve_current_account_id(accounts),
            Self::Cn => super::qoder_native::resolve_current_account_id(accounts),
        }
    }

    /// (是否启用, 阈值)。两条通道各自持有独立的开关与阈值。
    fn settings(self) -> (bool, i32) {
        let config = super::config::get_user_config();
        match self {
            Self::Global => (
                config.qoder_quota_alert_enabled,
                config.qoder_quota_alert_threshold,
            ),
            Self::Cn => (
                config.qoder_cn_quota_alert_enabled,
                config.qoder_cn_quota_alert_threshold,
            ),
        }
    }
}

fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default()
}

/// 剩余额度百分比。有 credits_remaining/total 就按比例算，缺数据时用 used 折算。
fn remaining_quota_percent(account: &QoderAccount) -> Option<f64> {
    if let (Some(remaining), Some(total)) = (account.credits_remaining, account.credits_total) {
        if total > 0.0 {
            return Some(((remaining / total) * 100.0).clamp(0.0, 100.0));
        }
    }
    account
        .credits_usage_percent
        .filter(|value| value.is_finite())
        .map(|used| (100.0 - used).clamp(0.0, 100.0))
}

/// 剩余额度最高的备用账号；额度相同时取最近用过的那个。
fn pick_recommendation<'a>(
    accounts: &'a [QoderAccount],
    current_id: &str,
) -> Option<&'a QoderAccount> {
    accounts
        .iter()
        .filter(|account| account.id != current_id)
        .filter(|account| account.quota_query_last_error.is_none())
        .filter_map(|account| remaining_quota_percent(account).map(|percent| (account, percent)))
        .max_by(|(a, a_percent), (b, b_percent)| {
            a_percent
                .partial_cmp(b_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.last_used.cmp(&b.last_used))
        })
        .map(|(account, _)| account)
}

fn cooldown_key(store: QoderStore, account_id: &str, threshold: i32) -> String {
    format!("{}:{}:{}", store.kind(), account_id, threshold)
}

fn should_emit(key: &str, now: i64) -> bool {
    let Ok(mut state) = LAST_SENT.lock() else {
        return true;
    };
    if let Some(last_sent) = state.get(key) {
        if now - *last_sent < COOLDOWN_SECONDS {
            return false;
        }
    }
    state.insert(key.to_string(), now);
    true
}

fn clear_cooldown(key: &str) {
    if let Ok(mut state) = LAST_SENT.lock() {
        state.remove(key);
    }
}

/// 刷新后调用：当前账号剩余额度不高于阈值时派发一次预警。
pub fn run(store: QoderStore) -> Result<Option<QuotaAlertPayload>, String> {
    let (enabled, raw_threshold) = store.settings();
    if !enabled {
        return Ok(None);
    }
    let threshold = raw_threshold.clamp(0, 100);
    let accounts = store.load_accounts()?;
    let Some(current_id) = store.resolve_current_id(&accounts) else {
        return Ok(None);
    };
    let Some(current) = accounts.iter().find(|account| account.id == current_id) else {
        return Ok(None);
    };
    // 查询失败的账号没有可信额度，宁可漏报也不误报。
    if current.quota_query_last_error.is_some() {
        return Ok(None);
    }
    let Some(remaining) = remaining_quota_percent(current) else {
        return Ok(None);
    };
    let key = cooldown_key(store, &current_id, threshold);
    if remaining > threshold as f64 {
        clear_cooldown(&key);
        return Ok(None);
    }
    let now = now_ts();
    if !should_emit(&key, now) {
        return Ok(None);
    }

    let recommendation = pick_recommendation(&accounts, &current_id);
    let payload = QuotaAlertPayload {
        platform: store.kind().to_string(),
        current_account_id: current.id.clone(),
        current_email: current.email.clone(),
        threshold,
        threshold_display: None,
        lowest_percentage: remaining.round().clamp(0.0, 100.0) as i32,
        low_models: vec!["Credits".to_string()],
        recommended_account_id: recommendation.map(|account| account.id.clone()),
        recommended_email: recommendation.map(|account| account.email.clone()),
        triggered_at: now,
    };
    account::dispatch_quota_alert(&payload);
    Ok(Some(payload))
}
