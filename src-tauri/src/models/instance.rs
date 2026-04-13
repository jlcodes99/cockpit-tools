use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InstanceLaunchMode {
    App,
    Cli,
}

impl Default for InstanceLaunchMode {
    fn default() -> Self {
        Self::App
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceProfile {
    pub id: String,
    pub name: String,
    pub user_data_dir: String,
    #[serde(default)]
    pub working_dir: Option<String>,
    pub extra_args: String,
    pub bind_account_id: Option<String>,
    #[serde(default)]
    pub launch_mode: InstanceLaunchMode,
    pub created_at: i64,
    pub last_launched_at: Option<i64>,
    #[serde(default)]
    pub last_pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceStore {
    pub instances: Vec<InstanceProfile>,
    #[serde(default)]
    pub default_settings: DefaultInstanceSettings,
}

impl InstanceStore {
    pub fn new() -> Self {
        Self {
            instances: Vec::new(),
            default_settings: DefaultInstanceSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefaultInstanceSettings {
    #[serde(default)]
    pub bind_account_id: Option<String>,
    #[serde(default)]
    pub extra_args: String,
    #[serde(default)]
    pub launch_mode: InstanceLaunchMode,
    #[serde(default = "default_follow_local_account")]
    pub follow_local_account: bool,
    #[serde(default)]
    pub last_pid: Option<u32>,
}

fn default_follow_local_account() -> bool {
    true
}

impl Default for DefaultInstanceSettings {
    fn default() -> Self {
        Self {
            bind_account_id: None,
            extra_args: String::new(),
            launch_mode: InstanceLaunchMode::App,
            follow_local_account: true,
            last_pid: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceProfileView {
    pub id: String,
    pub name: String,
    pub user_data_dir: String,
    pub working_dir: Option<String>,
    pub extra_args: String,
    pub bind_account_id: Option<String>,
    pub created_at: i64,
    pub last_launched_at: Option<i64>,
    pub last_pid: Option<u32>,
    pub running: bool,
    pub initialized: bool,
    pub is_default: bool,
    pub follow_local_account: bool,
}

impl InstanceProfileView {
    pub fn from_profile(profile: InstanceProfile, running: bool, initialized: bool) -> Self {
        Self {
            id: profile.id,
            name: profile.name,
            user_data_dir: profile.user_data_dir,
            working_dir: profile.working_dir,
            extra_args: profile.extra_args,
            bind_account_id: profile.bind_account_id,
            created_at: profile.created_at,
            last_launched_at: profile.last_launched_at,
            last_pid: profile.last_pid,
            running,
            initialized,
            is_default: false,
            follow_local_account: false,
        }
    }
}
