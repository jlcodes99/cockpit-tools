use serde::Serialize;
use tabled::{Table, Tabled};

use cockpit_core::models::codex::CodexAccount;
use cockpit_core::models::{DefaultInstanceSettings, InstanceLaunchMode, InstanceProfile};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize)]
pub struct OutputEnvelope<T> {
    pub schema_version: u32,
    pub data: T,
}

impl<T> OutputEnvelope<T> {
    pub const fn new(data: T) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            data,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CommandOutput {
    pub provider: String,
    pub operation: String,
    pub status: String,
    pub message: String,
}

impl CommandOutput {
    pub fn new(
        provider: impl Into<String>,
        operation: impl Into<String>,
        status: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            provider: provider.into(),
            operation: operation.into(),
            status: status.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Serialize, Tabled)]
pub struct AccountDisplay {
    #[tabled(rename = "ID")]
    pub id: String,
    #[tabled(rename = "Email")]
    pub email: String,
    #[tabled(rename = "Plan")]
    pub plan: String,
    #[tabled(rename = "Tags")]
    pub tags: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CodexQuotaDisplay {
    pub hourly_percentage: i32,
    pub hourly_reset_time: Option<i64>,
    pub hourly_window_minutes: Option<i64>,
    pub hourly_window_present: Option<bool>,
    pub weekly_percentage: i32,
    pub weekly_reset_time: Option<i64>,
    pub weekly_window_minutes: Option<i64>,
    pub weekly_window_present: Option<bool>,
}

impl From<&cockpit_core::models::codex::CodexQuota> for CodexQuotaDisplay {
    fn from(quota: &cockpit_core::models::codex::CodexQuota) -> Self {
        Self {
            hourly_percentage: quota.hourly_percentage,
            hourly_reset_time: quota.hourly_reset_time,
            hourly_window_minutes: quota.hourly_window_minutes,
            hourly_window_present: quota.hourly_window_present,
            weekly_percentage: quota.weekly_percentage,
            weekly_reset_time: quota.weekly_reset_time,
            weekly_window_minutes: quota.weekly_window_minutes,
            weekly_window_present: quota.weekly_window_present,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CodexAccountDisplay {
    pub id: String,
    pub email: String,
    pub plan: Option<String>,
    pub status: &'static str,
    pub tags: Vec<String>,
    pub quota: Option<CodexQuotaDisplay>,
    pub quota_updated_at: Option<i64>,
}

impl From<&CodexAccount> for CodexAccountDisplay {
    fn from(account: &CodexAccount) -> Self {
        Self {
            id: account.id.clone(),
            email: account.email.clone(),
            plan: account.plan_type.clone(),
            status: codex_account_status(account),
            tags: account.tags.clone().unwrap_or_default(),
            quota: account.quota.as_ref().map(CodexQuotaDisplay::from),
            quota_updated_at: account.usage_updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CodexCurrentDisplay {
    pub id: String,
    pub email: String,
    pub plan: Option<String>,
    pub status: &'static str,
}

impl From<&CodexAccount> for CodexCurrentDisplay {
    fn from(account: &CodexAccount) -> Self {
        Self {
            id: account.id.clone(),
            email: account.email.clone(),
            plan: account.plan_type.clone(),
            status: codex_account_status(account),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CodexInstanceDisplay {
    pub id: String,
    pub name: String,
    pub user_data_dir: String,
    pub working_dir: Option<String>,
    pub bind_account_id: Option<String>,
    pub launch_mode: String,
    pub created_at: i64,
    pub last_launched_at: Option<i64>,
    pub last_pid: Option<u32>,
    pub initialized: bool,
    pub is_default: bool,
    pub follow_local_account: bool,
}

impl From<&InstanceProfile> for CodexInstanceDisplay {
    fn from(instance: &InstanceProfile) -> Self {
        Self {
            id: instance.id.clone(),
            name: instance.name.clone(),
            user_data_dir: instance.user_data_dir.clone(),
            working_dir: instance.working_dir.clone(),
            bind_account_id: instance.bind_account_id.clone(),
            launch_mode: instance_launch_mode_name(&instance.launch_mode).to_string(),
            created_at: instance.created_at,
            last_launched_at: instance.last_launched_at,
            last_pid: instance.last_pid,
            initialized: std::path::Path::new(&instance.user_data_dir).exists(),
            is_default: false,
            follow_local_account: false,
        }
    }
}

pub fn codex_default_instance_display(
    user_data_dir: &std::path::Path,
    settings: &DefaultInstanceSettings,
) -> CodexInstanceDisplay {
    CodexInstanceDisplay {
        id: "__default__".to_string(),
        name: "default".to_string(),
        user_data_dir: user_data_dir.to_string_lossy().into_owned(),
        working_dir: None,
        bind_account_id: settings.bind_account_id.clone(),
        launch_mode: instance_launch_mode_name(&settings.launch_mode).to_string(),
        created_at: 0,
        last_launched_at: None,
        last_pid: settings.last_pid,
        initialized: user_data_dir.exists(),
        is_default: true,
        follow_local_account: settings.follow_local_account,
    }
}

fn instance_launch_mode_name(mode: &InstanceLaunchMode) -> &'static str {
    match mode {
        InstanceLaunchMode::App => "app",
        InstanceLaunchMode::Cli => "cli",
    }
}

pub fn codex_account_status(account: &CodexAccount) -> &'static str {
    if account.requires_reauth {
        return "reauth_required";
    }
    if account.quota.as_ref().is_some_and(|quota| {
        let has_presence =
            quota.hourly_window_present.is_some() || quota.weekly_window_present.is_some();
        let hourly_exhausted = (!has_presence || quota.hourly_window_present == Some(true))
            && quota.hourly_percentage <= 0;
        let weekly_exhausted = (!has_presence || quota.weekly_window_present == Some(true))
            && quota.weekly_percentage <= 0;
        hourly_exhausted || weekly_exhausted
    }) {
        return "quota_exhausted";
    }
    if account.quota_error.is_some() {
        return "quota_error";
    }
    if account.quota.is_some() {
        return "ready";
    }
    "unknown"
}

#[derive(Debug, Tabled)]
struct CodexAccountTable {
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "Email")]
    email: String,
    #[tabled(rename = "Plan")]
    plan: String,
    #[tabled(rename = "Status")]
    status: String,
    #[tabled(rename = "Quota")]
    quota: String,
}

#[derive(Debug, Tabled)]
struct CodexInstanceTable {
    #[tabled(rename = "Name")]
    name: String,
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "Account")]
    account: String,
    #[tabled(rename = "Mode")]
    launch_mode: String,
    #[tabled(rename = "Initialized")]
    initialized: String,
    #[tabled(rename = "Last PID")]
    last_pid: String,
}

pub fn print_codex_accounts(accounts: &[CodexAccountDisplay]) {
    if accounts.is_empty() {
        println!("No Codex accounts found.");
        return;
    }

    let rows = accounts
        .iter()
        .map(|account| CodexAccountTable {
            id: account.id.clone(),
            email: account.email.clone(),
            plan: account.plan.clone().unwrap_or_else(|| "-".to_string()),
            status: account.status.to_string(),
            quota: format_quota(account.quota.as_ref()),
        })
        .collect::<Vec<_>>();
    println!("{}", Table::new(rows));
}

pub fn print_codex_current(current: Option<&CodexCurrentDisplay>) {
    match current {
        Some(account) => println!(
            "Current Codex account: {} ({}) [{}]",
            account.email, account.id, account.status
        ),
        None => println!("No current Codex account."),
    }
}

pub fn print_codex_quota(accounts: &[CodexAccountDisplay]) {
    if accounts.is_empty() {
        println!("No cached Codex quota found.");
        return;
    }

    let rows = accounts
        .iter()
        .map(|account| CodexAccountTable {
            id: account.id.clone(),
            email: account.email.clone(),
            plan: account.plan.clone().unwrap_or_else(|| "-".to_string()),
            status: account.status.to_string(),
            quota: format_quota(account.quota.as_ref()),
        })
        .collect::<Vec<_>>();
    println!("{}", Table::new(rows));
}

pub fn print_codex_instances(instances: &[CodexInstanceDisplay]) {
    if instances.is_empty() {
        println!("No Codex instances found.");
        return;
    }

    let rows = instances
        .iter()
        .map(|instance| CodexInstanceTable {
            name: instance.name.clone(),
            id: instance.id.clone(),
            account: instance
                .bind_account_id
                .clone()
                .unwrap_or_else(|| "-".to_string()),
            launch_mode: instance.launch_mode.clone(),
            initialized: instance.initialized.to_string(),
            last_pid: instance
                .last_pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "-".to_string()),
        })
        .collect::<Vec<_>>();
    println!("{}", Table::new(rows));
}

fn format_quota(quota: Option<&CodexQuotaDisplay>) -> String {
    quota
        .map(|quota| {
            format!(
                "5h {}%, week {}%",
                quota.hourly_percentage, quota.weekly_percentage
            )
        })
        .unwrap_or_else(|| "-".to_string())
}

pub fn print_accounts(accounts: Vec<AccountDisplay>) {
    if accounts.is_empty() {
        println!("No accounts found.");
    } else {
        println!("{}", Table::new(accounts));
    }
}

#[cfg(test)]
mod tests {
    use super::{AccountDisplay, OutputEnvelope, SCHEMA_VERSION};

    #[test]
    fn output_envelope_has_stable_schema_version() {
        let output = OutputEnvelope::new(vec![AccountDisplay {
            id: "account-1".into(),
            email: "user@example.com".into(),
            plan: "Plus".into(),
            tags: String::new(),
        }]);

        let value = serde_json::to_value(output).expect("output should serialize");
        assert_eq!(value["schema_version"], SCHEMA_VERSION);
        assert_eq!(value["data"][0]["id"], "account-1");
        assert!(value["data"][0].get("access_token").is_none());
        assert!(value["data"][0].get("refresh_token").is_none());
        assert!(value["data"][0].get("agent_private_key").is_none());
    }
}
