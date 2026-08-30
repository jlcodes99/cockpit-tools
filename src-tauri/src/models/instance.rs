use serde::{Deserialize, Serialize};

use crate::models::codex::CodexAppSpeed;

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

fn default_model_routing_version() -> u32 {
    1
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexInstanceApiRoute {
    pub id: String,
    pub namespace: String,
    pub provider_account_id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_models: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_models: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CodexInstanceModelRouting {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_model_routing_version")]
    pub version: u32,
    #[serde(default)]
    pub routes: Vec<CodexInstanceApiRoute>,
}

impl Default for CodexInstanceModelRouting {
    fn default() -> Self {
        Self {
            enabled: false,
            version: default_model_routing_version(),
            routes: Vec::new(),
        }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_routing: Option<CodexInstanceModelRouting>,
    #[serde(default)]
    pub launch_mode: InstanceLaunchMode,
    #[serde(default, skip_serializing_if = "is_standard_app_speed")]
    pub app_speed: CodexAppSpeed,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_routing: Option<CodexInstanceModelRouting>,
    #[serde(default)]
    pub extra_args: String,
    #[serde(default)]
    pub working_dir: Option<String>,
    #[serde(default)]
    pub launch_mode: InstanceLaunchMode,
    #[serde(default, skip_serializing_if = "is_standard_app_speed")]
    pub app_speed: CodexAppSpeed,
    #[serde(default = "default_follow_local_account")]
    pub follow_local_account: bool,
    #[serde(default)]
    pub auto_sync_threads: bool,
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
            model_routing: None,
            extra_args: String::new(),
            working_dir: None,
            launch_mode: InstanceLaunchMode::App,
            app_speed: CodexAppSpeed::Standard,
            follow_local_account: true,
            auto_sync_threads: false,
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

fn is_standard_app_speed(speed: &CodexAppSpeed) -> bool {
    matches!(speed, CodexAppSpeed::Standard)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_instance_store_deserializes_without_model_routing() {
        let store: InstanceStore = serde_json::from_str(
            r#"{
                "instances": [{
                    "id": "legacy",
                    "name": "Legacy",
                    "userDataDir": "/tmp/legacy",
                    "extraArgs": "",
                    "bindAccountId": null,
                    "createdAt": 1,
                    "lastLaunchedAt": null
                }],
                "defaultSettings": {}
            }"#,
        )
        .expect("deserialize legacy instance store");

        assert!(store.instances[0].model_routing.is_none());
        assert!(store.default_settings.model_routing.is_none());
    }

    #[test]
    fn model_routing_defaults_version_and_route_enabled_flag() {
        let routing: CodexInstanceModelRouting = serde_json::from_str(
            r#"{
                "enabled": true,
                "routes": [{
                    "id": "route-cpa",
                    "namespace": "cpa",
                    "providerAccountId": "api-account"
                }]
            }"#,
        )
        .expect("deserialize model routing defaults");

        assert_eq!(routing.version, 1);
        assert!(routing.routes[0].enabled);
    }
}
