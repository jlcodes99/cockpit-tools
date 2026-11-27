use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

use crate::models::qoder::QoderAccount;
use crate::models::t3code::{
    T3CodeBindableAccountView, T3CodeQuotaRemaining, T3CodeQuotaWindowView,
};
use crate::modules::{
    qoder_account,
    qoder_channel::{self, CredentialSource},
    qoder_cn_account::QoderCnAccountStore,
    qoder_cn_clients,
    qoder_native::{self, NativeRegion},
    qoder_oauth, qoder_usage,
};

use super::{
    cockpit_provider_dir, credit_window, env_item, reported_plan, PLATFORM_QODER, PLATFORM_QODER_CN,
};

#[derive(Clone, Copy)]
pub(in crate::modules::t3code_provider_bindings) struct QoderProvider {
    domestic: bool,
}

impl QoderProvider {
    pub(in crate::modules::t3code_provider_bindings) fn from_platform(
        platform: &str,
    ) -> Option<Self> {
        match platform {
            PLATFORM_QODER => Some(Self { domestic: false }),
            PLATFORM_QODER_CN => Some(Self { domestic: true }),
            _ => None,
        }
    }

    fn platform(self) -> &'static str {
        if self.domestic {
            PLATFORM_QODER_CN
        } else {
            PLATFORM_QODER
        }
    }

    fn source(self) -> CredentialSource {
        if self.domestic {
            CredentialSource::PublicCn
        } else {
            CredentialSource::GlobalIde
        }
    }

    fn config_env(self) -> &'static str {
        if self.domestic {
            "QODERCN_CONFIG_DIR"
        } else {
            "QODER_CONFIG_DIR"
        }
    }

    fn token_env(self) -> &'static str {
        if self.domestic {
            "QODERCN_PERSONAL_ACCESS_TOKEN"
        } else {
            "QODER_PERSONAL_ACCESS_TOKEN"
        }
    }

    fn profile_dir(self, state_dir: &Path, instance_id: &str, account_id: &str) -> PathBuf {
        cockpit_provider_dir(state_dir, self.platform(), instance_id)
            .join("profiles")
            .join(format!("{:x}", Sha256::digest(account_id.as_bytes())))
    }

    pub(in crate::modules::t3code_provider_bindings) fn auth_paths(
        self,
        state_dir: &Path,
        instance_id: &str,
        account_id: &str,
    ) -> [PathBuf; 2] {
        let auth_dir = self
            .profile_dir(state_dir, instance_id, account_id)
            .join(".auth");
        [auth_dir.join("user"), auth_dir.join("machine_id")]
    }

    pub(super) fn matches_binding(
        self,
        state_dir: &Path,
        instance_id: &str,
        account_id: &str,
        entry: Option<&Map<String, Value>>,
    ) -> bool {
        let expected = self.profile_dir(state_dir, instance_id, account_id);
        let environment = entry
            .and_then(|obj| obj.get("environment"))
            .and_then(Value::as_array);
        let configured = environment
            .and_then(|items| {
                items
                    .iter()
                    .rev()
                    .find(|item| item["name"].as_str() == Some(self.config_env()))
            })
            .and_then(|item| item["value"].as_str());
        configured.is_some_and(|path| Path::new(path) == expected)
            && self
                .auth_paths(state_dir, instance_id, account_id)
                .iter()
                .all(|path| path.is_file())
    }

    pub(super) fn list_accounts(self) -> Vec<T3CodeBindableAccountView> {
        let accounts = if self.domestic {
            QoderCnAccountStore::default_store()
                .and_then(|store| store.list_accounts())
                .unwrap_or_default()
        } else {
            qoder_account::list_accounts()
        };
        accounts
            .into_iter()
            .filter_map(|account| self.account_view(account))
            .collect()
    }

    fn account_view(self, account: QoderAccount) -> Option<T3CodeBindableAccountView> {
        if qoder_channel::credential_source(&account).ok()? != self.source() {
            return None;
        }
        qoder_oauth::access_token_from_info(account.auth_user_info_raw.as_ref())?;
        let windows = quota_window(&account).into_iter().collect();
        Some(T3CodeBindableAccountView {
            id: account.id,
            driver: self.platform().to_string(),
            email: account.email,
            display_name: account.display_name.filter(|name| !name.trim().is_empty()),
            plan_badge: reported_plan(account.plan_type.as_deref())
                .unwrap_or("UNKNOWN")
                .to_string(),
            plan_badge_kind: None,
            auth_mode: "oauth".to_string(),
            is_api_service: false,
            workspace_id: None,
            windows,
            quota_summary_groups: None,
            provider_quota: None,
            primary_percent: None,
            secondary_percent: None,
            speed_tok_per_sec: None,
            cache_hit_percent: None,
        })
    }

    pub(in crate::modules::t3code_provider_bindings) async fn prepare(
        self,
        account_id: &str,
    ) -> Result<(), String> {
        qoder_oauth::prepare_account_for_client(account_id, self.source()).await
    }

    pub(super) async fn refresh_quota(self, account_id: &str) -> Result<(), String> {
        let account =
            qoder_oauth::refresh_account_for_source(account_id, Some(self.source())).await?;
        match account.quota_query_last_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub(super) fn write_binding(
        self,
        state_dir: &Path,
        instance_id: &str,
        account_id: &str,
        config: &mut Map<String, Value>,
        environment: &mut Vec<Value>,
    ) -> Result<String, String> {
        let executable = qoder_cn_clients::resolve_t3_cli(self.domestic)?;
        let profile_dir = self.profile_dir(state_dir, instance_id, account_id);
        let region = if self.domestic {
            NativeRegion::Cn
        } else {
            NativeRegion::Global
        };
        let email = qoder_native::write_t3_cli_profile(region, account_id, &profile_dir)?;
        config.insert("source".into(), json!("local"));
        config.insert("commandPath".into(), json!(executable));
        config.insert("commandArgs".into(), json!(["--acp"]));
        environment.push(env_item(self.config_env(), &profile_dir.to_string_lossy()));
        // The CLI prefers a PAT from the environment over its saved login.
        environment.push(env_item(self.token_env(), ""));
        Ok(email)
    }
}

fn quota_window(account: &QoderAccount) -> Option<T3CodeQuotaWindowView> {
    let (used, total, remaining, percent) = account
        .auth_credit_usage_raw
        .as_ref()
        .and_then(qoder_usage::scoped_credits)
        .unwrap_or((
            account.credits_used,
            account.credits_total,
            account.credits_remaining,
            account.credits_usage_percent,
        ));
    let nonnegative = |value: &f64| value.is_finite() && *value >= 0.0;
    if let Some(amount) = remaining
        .filter(nonnegative)
        .or_else(|| Some((total.filter(nonnegative)? - used.filter(nonnegative)?).max(0.0)))
    {
        return Some(credit_window(amount));
    }
    let used_percent = percent.filter(nonnegative)?;
    Some(T3CodeQuotaWindowView {
        id: "credits".to_string(),
        label: "Credits".to_string(),
        remaining: T3CodeQuotaRemaining::Percent {
            remaining_percent: (100.0 - used_percent).clamp(0.0, 100.0).round() as i32,
        },
        reset_time: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qoder_binding_matches_only_the_published_instance_and_account_profile() {
        let state = tempfile::tempdir().unwrap();
        let global = QoderProvider::from_platform(PLATFORM_QODER).unwrap();
        let cn = QoderProvider::from_platform(PLATFORM_QODER_CN).unwrap();
        let keys = global.auth_paths(state.path(), "qoder", "A");
        let profile = global.profile_dir(state.path(), "qoder", "A");
        std::fs::create_dir_all(keys[0].parent().unwrap()).unwrap();
        for file in keys {
            std::fs::write(file, b"fixture").unwrap();
        }
        let entry = json!({
            "environment": [
                env_item(global.config_env(), &profile.to_string_lossy()),
                env_item(global.token_env(), "")
            ],
            "config": {
                "source": "local", "commandPath": "/bin/qodercli", "commandArgs": ["--acp"]
            }
        });
        assert!(global.matches_binding(state.path(), "qoder", "A", entry.as_object()));
        assert!(!global.matches_binding(state.path(), "qoder", "B", entry.as_object()));
        assert!(!global.matches_binding(state.path(), "qoder_parallel", "A", entry.as_object()));
        assert!(!cn.matches_binding(state.path(), "qoder", "A", entry.as_object()));
        assert_ne!(
            profile,
            global.profile_dir(&state.path().join("other-workspace"), "qoder", "A")
        );
        assert_eq!(
            super::super::infer_platform_from_t3_entry(
                "qoder",
                "acpRegistry",
                entry.as_object(),
                &Default::default(),
            )
            .as_deref(),
            Some(PLATFORM_QODER)
        );
        let cn_entry = json!({"config":{"source":"local","commandPath":"C:\\Qoder\\qoderclicn.exe","commandArgs":["--acp"]}});
        assert_eq!(
            super::super::infer_platform_from_t3_entry(
                "qoder",
                "acpRegistry",
                cn_entry.as_object(),
                &Default::default(),
            )
            .as_deref(),
            Some(PLATFORM_QODER_CN)
        );
    }

    #[test]
    fn qoder_quota_distinguishes_zero_percentage_and_enterprise_only_data() {
        let mut account: QoderAccount = serde_json::from_value(json!({
            "id": "fixture", "email": "test@example.test", "created_at": 0, "last_used": 0,
            "credits_remaining": 15.0
        }))
        .unwrap();
        assert_eq!(
            quota_window(&account).unwrap().remaining,
            T3CodeQuotaRemaining::Amount {
                remaining_amount: 15.0
            }
        );
        account.auth_credit_usage_raw = Some(json!({"qoderUsage":{"userQuota":{"remaining":0}}}));
        assert_eq!(
            quota_window(&account).unwrap().remaining,
            T3CodeQuotaRemaining::Amount {
                remaining_amount: 0.0
            }
        );
        account.auth_credit_usage_raw =
            Some(json!({"qoderUsage":{"userQuota":{"percentage":0.7}}}));
        assert_eq!(
            quota_window(&account).unwrap().remaining,
            T3CodeQuotaRemaining::Percent {
                remaining_percent: 30
            }
        );
        account.auth_credit_usage_raw =
            Some(json!({"displayMode":"enterprise","enterpriseUsage":{"remaining":999}}));
        assert!(quota_window(&account).is_none());
    }
}
