use super::{quota::QuotaData, token::TokenData};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// 账号数据结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// 用户备注
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// 账号 2FA Base32 秘钥（仅存储在本地账号详情文件中）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub two_factor_secret: Option<String>,
    /// 账号登录密码（仅存储在本地账号详情文件中）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_password: Option<String>,
    /// 账号绑定手机号（仅存储在本地账号详情文件中）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phone_number: Option<String>,
    /// 可打开的邮件查询地址（仅存储在本地账号详情文件中）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mail_url: Option<String>,
    /// 辅助邮箱（仅存储在本地账号详情文件中）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aux_email: Option<String>,
    /// 仅保存了邮箱/备注、尚未完成 OAuth 的待授权卡片。
    #[serde(default)]
    pub pending_oauth: bool,
    pub token: TokenData,
    pub quota: Option<QuotaData>,
    /// Disabled accounts are ignored by the proxy token pool (e.g. revoked refresh_token -> invalid_grant).
    #[serde(default)]
    pub disabled: bool,
    /// Optional human-readable reason for disabling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
    /// Unix timestamp when the account was disabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_at: Option<i64>,
    /// 受配额保护禁用的模型列表
    #[serde(default, skip_serializing_if = "HashSet::is_empty")]
    pub protected_models: HashSet<String>,
    /// 最近一次配额错误信息
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_error: Option<QuotaErrorInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_updated_at: Option<i64>,
    pub created_at: i64,
    pub last_used: i64,
}

impl Account {
    pub fn new(id: String, email: String, token: TokenData) -> Self {
        let now = chrono::Utc::now().timestamp();
        Self {
            id,
            email,
            name: None,
            tags: Vec::new(),
            notes: None,
            two_factor_secret: None,
            account_password: None,
            phone_number: None,
            mail_url: None,
            aux_email: None,
            pending_oauth: false,
            token,
            quota: None,
            disabled: false,
            disabled_reason: None,
            disabled_at: None,
            protected_models: HashSet::new(),
            quota_error: None,
            usage_updated_at: None,
            created_at: now,
            last_used: now,
        }
    }

    pub fn update_last_used(&mut self) {
        self.last_used = chrono::Utc::now().timestamp();
    }

    pub fn update_quota(&mut self, quota: QuotaData) {
        self.quota = Some(quota);
    }

    /// Token 失效（invalid_grant）导致的禁用，刷新成功后可自动解除
    pub fn is_invalid_grant_disabled(&self) -> bool {
        self.disabled
            && self
                .disabled_reason
                .as_deref()
                .is_some_and(|r| r.starts_with("invalid_grant"))
    }

    /// 清除禁用状态（三个字段一起重置）
    pub fn clear_disabled(&mut self) {
        self.disabled = false;
        self.disabled_reason = None;
        self.disabled_at = None;
    }
}

/// 配额错误信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaErrorInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<u16>,
    pub message: String,
    pub timestamp: i64,
}

/// 账号索引数据（accounts.json）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountIndex {
    pub version: String,
    pub accounts: Vec<AccountSummary>,
    pub current_account_id: Option<String>,
}

/// 账号摘要信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountSummary {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
    pub created_at: i64,
    pub last_used: i64,
}

impl AccountIndex {
    pub fn new() -> Self {
        Self {
            version: "2.0".to_string(),
            accounts: Vec::new(),
            current_account_id: None,
        }
    }
}

impl Default for AccountIndex {
    fn default() -> Self {
        Self::new()
    }
}
