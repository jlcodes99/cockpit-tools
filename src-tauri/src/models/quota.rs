use serde::{Deserialize, Serialize};

/// 模型配额信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelQuota {
    pub name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    pub percentage: i32, // 剩余百分比 0-100
    pub reset_time: String,
}

/// AI 积分信息（来自 loadCodeAssist paidTier.availableCredits）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreditInfo {
    pub credit_type: String,
    #[serde(default)]
    pub credit_amount: Option<String>,
    #[serde(default)]
    pub minimum_credit_amount_for_usage: Option<String>,
}

/// 配额数据结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaData {
    pub models: Vec<ModelQuota>,
    pub last_updated: i64,
    /// Window summary failed; model-level data may still be current.
    #[serde(default)]
    pub quota_summary_stale: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_summary_updated_at: Option<i64>,
    #[serde(default)]
    pub is_forbidden: bool,
    /// 订阅等级 (FREE/PRO/ULTRA)
    #[serde(default)]
    pub subscription_tier: Option<String>,
    /// AI 积分列表（仅付费账号可能有）
    #[serde(default)]
    pub credits: Vec<CreditInfo>,
    /// 账号层级 ID（如 free-tier、g1-pro-tier）
    #[serde(default)]
    pub tier_id: Option<String>,
    /// 是否使用 GCP ToS
    #[serde(default)]
    pub is_gcp_tos: Option<bool>,
    /// GCP / Enterprise 项目 ID
    #[serde(default)]
    pub project_id: Option<String>,
}

impl QuotaData {
    pub fn new() -> Self {
        Self {
            models: Vec::new(),
            last_updated: chrono::Utc::now().timestamp(),
            quota_summary_stale: false,
            quota_summary_updated_at: None,
            is_forbidden: false,
            subscription_tier: None,
            credits: Vec::new(),
            tier_id: None,
            is_gcp_tos: None,
            project_id: None,
        }
    }

    pub fn add_model(
        &mut self,
        name: String,
        display_name: Option<String>,
        percentage: i32,
        reset_time: String,
    ) {
        self.models.push(ModelQuota {
            name,
            display_name,
            percentage,
            reset_time,
        });
    }
}

impl Default for QuotaData {
    fn default() -> Self {
        Self::new()
    }
}
