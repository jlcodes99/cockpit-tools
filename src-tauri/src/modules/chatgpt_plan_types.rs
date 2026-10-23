use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatgptPlanAccountSummary {
    pub id: String,
    pub label: String,
    pub email: Option<String>,
    pub subject: String,
    pub client_id: String,
    /// Explicit local association; neither email nor cross-client subject equality proves a workspace match.
    #[serde(default)]
    pub source_account_id: Option<String>,
    /// Derived from the current local source identity, independently of token availability.
    #[serde(default)]
    pub source_binding_valid: bool,
    #[serde(default)]
    pub source_binding_error: Option<String>,
    pub connected: bool,
    pub needs_reauth: bool,
    #[serde(default)]
    pub last_checked_at: Option<i64>,
    #[serde(default)]
    pub check_error: Option<String>,
    /// Last successfully committed service mode; membership removal retains it.
    #[serde(default)]
    pub last_service_mode: Option<String>,
    pub verified_at: Option<i64>,
    pub verified_model: Option<String>,
    pub expires_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatgptPlanModel {
    pub id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatgptPlanLogin {
    pub login_id: String,
    pub authorize_url: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatgptPlanLoginStatus {
    pub status: String,
    pub account: Option<ChatgptPlanAccountSummary>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatgptPlanDisconnectResult {
    pub remote_revoked: bool,
}

// Never implement Serialize for this type through a Tauri command. Only the
// protected per-registration envelope contains these values.
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Credentials {
    pub client_id: String,
    pub subject: String,
    pub access_token: String,
    pub refresh_token: String,
    pub id_token: String,
    pub scopes: Vec<String>,
    pub expires_at: i64,
    pub earliest_refresh_at: Option<i64>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Registration {
    pub summary: ChatgptPlanAccountSummary,
    pub revision: String,
    #[serde(default)]
    pub authorization_revision: String,
    pub encrypted_credentials: Option<String>,
    #[serde(default)]
    pub revocation_confirmed: bool,
    #[serde(default)]
    pub source_binding: Option<SourceBinding>,
    #[serde(default)]
    pub source_binding_required: bool,
    #[serde(default)]
    pub model_catalog: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct SourceBinding {
    pub source_account_id: String,
    pub identity_fingerprint: String,
}

impl Registration {
    pub fn credentials(&self) -> Result<Credentials, String> {
        let encrypted = self
            .encrypted_credentials
            .as_deref()
            .ok_or("CHATGPT_PLAN_DISCONNECTED")?;
        let credentials: Credentials =
            crate::modules::secure_account_storage::deserialize_encrypted_snapshot(encrypted)
                .map_err(|_| "CHATGPT_PLAN_STORAGE".to_string())?;
        if credentials.client_id != self.summary.client_id
            || credentials.subject != self.summary.subject
        {
            return Err("CHATGPT_PLAN_IDENTITY_MISMATCH".into());
        }
        Ok(credentials)
    }

    pub fn set_credentials(&mut self, credentials: &Credentials) -> Result<(), String> {
        self.encrypted_credentials = Some(
            crate::modules::secure_account_storage::serialize_account_file(
                "chatgpt_plan",
                credentials,
            )
            .map_err(|_| "CHATGPT_PLAN_STORAGE".to_string())?,
        );
        self.summary.connected = true;
        self.summary.needs_reauth = false;
        self.summary.expires_at = Some(credentials.expires_at);
        self.summary.updated_at = chrono::Utc::now().timestamp();
        self.revision = uuid::Uuid::new_v4().to_string();
        self.revocation_confirmed = false;
        Ok(())
    }
}
