use crate::models::qoder::QoderAccount;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CredentialSource {
    GlobalIde,
    LegacyCnIde,
    DedicatedIde,
    PublicCn,
    Unsupported,
}

fn marker(metadata: Option<&Value>) -> Result<Option<&str>, String> {
    match metadata
        .and_then(|value| value.get("login_source"))
        .filter(|value| !value.is_null())
    {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .map(str::trim)
            .map(|source| (!source.is_empty()).then_some(source))
            .ok_or_else(|| "QODER_REGION_UNSUPPORTED".into()),
    }
}

/// Resolves a `login_source` marker, if the snapshot carries one. Callers that
/// only have raw metadata (import, native clients) share this with the store.
pub(crate) fn snapshot_marker<'a>(
    info: Option<&'a Value>,
    plan: Option<&'a Value>,
) -> Result<Option<&'a str>, String> {
    let info = marker(info)?;
    let plan = marker(plan)?;
    if matches!((info, plan), (Some(info), Some(plan)) if info != plan) {
        return Err("QODER_PLATFORM_MISMATCH".into());
    }
    Ok(info.or(plan))
}

pub(crate) fn source_of(marker: Option<&str>) -> CredentialSource {
    match marker {
        None | Some("qoder") => CredentialSource::GlobalIde,
        Some("qodercn") => CredentialSource::LegacyCnIde,
        Some("dedicated_qodercn") => CredentialSource::DedicatedIde,
        // Cockpit's public CN marker is distinct from the legacy IDE's qodercn.
        Some("qoder_cn") => CredentialSource::PublicCn,
        _ => CredentialSource::Unsupported,
    }
}

fn recorded_marker(account: &QoderAccount) -> Result<Option<&str>, String> {
    snapshot_marker(
        account.auth_user_info_raw.as_ref(),
        account.auth_user_plan_raw.as_ref(),
    )
}

pub(crate) fn credential_source(account: &QoderAccount) -> Result<CredentialSource, String> {
    Ok(source_of(recorded_marker(account)?))
}

pub(crate) fn ide_source(account: &QoderAccount) -> Result<CredentialSource, String> {
    let source = credential_source(account)?;
    if source == CredentialSource::PublicCn {
        return Err("QODER_PLATFORM_MISMATCH".into());
    }
    Ok(source)
}

pub(crate) fn preserve_refresh_source(
    account: &QoderAccount,
    user_info: &mut Value,
    plan: Option<&Value>,
) -> Result<(), String> {
    let recorded = recorded_marker(account)?;
    if marker(plan)?.is_some_and(|source| source != recorded.unwrap_or("qoder")) {
        return Err("QODER_PLATFORM_MISMATCH".into());
    }
    // A replaced plan may be the only place the old session recorded its source.
    if let Some(source) = recorded {
        user_info["login_source"] = Value::String(source.to_owned());
    }
    Ok(())
}

/// Endpoints of one Qoder realm. The OAuth flow itself stays shared; a channel
/// only contributes its service origins and the marker its sessions carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Channel {
    source: CredentialSource,
    website: &'static str,
    openapi_base_url: &'static str,
    desktop_dir: &'static str,
    cli_dir: &'static str,
}

impl Channel {
    /// Channels Cockpit can log in to. IDE-bound sources have no login flow of
    /// their own, so they resolve to `None`.
    pub(crate) fn for_source(source: CredentialSource) -> Option<Self> {
        match source {
            CredentialSource::GlobalIde => Some(Self {
                source,
                website: "https://qoder.com",
                openapi_base_url: "https://openapi.qoder.sh",
                desktop_dir: "com.qoder.app.stable",
                cli_dir: ".qoder",
            }),
            CredentialSource::PublicCn => Some(Self {
                source,
                website: "https://qoder.cn",
                openapi_base_url: "https://openapi.qoder.com.cn",
                desktop_dir: "com.qodercn.app.stable",
                cli_dir: ".qoder-cn",
            }),
            CredentialSource::LegacyCnIde
            | CredentialSource::DedicatedIde
            | CredentialSource::Unsupported => None,
        }
    }

    pub(crate) fn source(self) -> CredentialSource {
        self.source
    }

    pub(crate) fn marker(self) -> &'static str {
        match self.source {
            CredentialSource::PublicCn => "qoder_cn",
            _ => "qoder",
        }
    }

    pub(crate) fn openapi_base_url(self) -> &'static str {
        self.openapi_base_url
    }

    /// Config directory the channel's desktop client keeps its credentials in.
    pub(crate) fn desktop_dir(self) -> &'static str {
        self.desktop_dir
    }

    /// Home-relative directory the channel's CLI keeps its credentials in.
    pub(crate) fn cli_dir(self) -> &'static str {
        self.cli_dir
    }

    fn select_accounts_url(self) -> Result<url::Url, String> {
        url::Url::parse(&format!("{}/device/selectAccounts", self.website))
            .map_err(|error| format!("解析 Qoder 登录地址失败: {error}"))
    }

    /// Browser entry point for a channel login. The public CN client signs in
    /// through the website and hands the PKCE challenge back to the desktop
    /// client, so it needs no redirect URI of its own.
    pub(crate) fn sign_in_url(
        self,
        nonce: &str,
        challenge: &str,
        challenge_method: &str,
        machine_id: Option<&str>,
    ) -> Result<String, String> {
        let mut callback = self.select_accounts_url()?;
        {
            let mut query = callback.query_pairs_mut();
            query.append_pair("nonce", nonce);
            query.append_pair("challenge", challenge);
            query.append_pair("challenge_method", challenge_method);
            query.append_pair("client_id", "732aef47-9cf2-46a2-95fe-4cebb5d0d1fa");
            if let Some(machine_id) = machine_id.filter(|value| !value.trim().is_empty()) {
                query.append_pair("machine_id", machine_id);
            }
        }
        let mut sign_in = url::Url::parse(&format!("{}/users/sign-in", self.website))
            .map_err(|error| format!("解析 Qoder 登录地址失败: {error}"))?;
        {
            let mut query = sign_in.query_pairs_mut();
            query.append_pair("biz_variant", "qoder");
            query.append_pair("oauth_callback", callback.as_str());
        }
        Ok(sign_in.to_string())
    }
}

#[cfg(test)]
pub(crate) fn account_fixture(info: Option<&str>, plan: Option<&str>) -> QoderAccount {
    use serde_json::json;
    let mut user_info = json!({ "id": "fixture-user", "token": "synthetic-access-token" });
    if let Some(source) = info {
        user_info["login_source"] = json!(source);
    }
    serde_json::from_value(json!({
        "id": "fixture-account",
        "email": "fixture@example.invalid",
        "created_at": 1,
        "last_used": 1,
        "auth_user_info_raw": user_info,
        "auth_user_plan_raw": plan.map(|source| json!({ "login_source": source })),
    }))
    .expect("Qoder fixture")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_public_cn_from_supported_ide_sources() {
        use CredentialSource::*;
        for (info, plan, expected) in [
            (None, None, GlobalIde),
            (Some("qoder"), Some("qoder"), GlobalIde),
            (None, Some("qodercn"), LegacyCnIde),
            (Some("dedicated_qodercn"), None, DedicatedIde),
            (None, Some("qoder_cn"), PublicCn),
            (Some("qoder_cn"), None, PublicCn),
            (Some("unknown"), None, Unsupported),
        ] {
            assert_eq!(
                credential_source(&account_fixture(info, plan)),
                Ok(expected)
            );
        }
    }

    #[test]
    fn rejects_conflicting_or_invalid_markers() {
        assert_eq!(
            credential_source(&account_fixture(Some("qoder"), Some("qoder_cn"))),
            Err("QODER_PLATFORM_MISMATCH".into())
        );
        let mut account = account_fixture(None, Some("qodercn"));
        account.auth_user_info_raw.as_mut().unwrap()["login_source"] = serde_json::json!(false);
        assert_eq!(
            credential_source(&account),
            Err("QODER_REGION_UNSUPPORTED".into())
        );
    }

    #[test]
    fn public_cn_login_never_points_at_the_global_service() {
        let channel = Channel::for_source(CredentialSource::PublicCn).expect("CN channel");
        assert_eq!(channel.marker(), "qoder_cn");
        assert_eq!(channel.openapi_base_url(), "https://openapi.qoder.com.cn");

        // 用真实生成器，避免把占位字面量当 nonce 塞进登录地址（那正是
        // rust/hard-coded-cryptographic-value 会报的写法）。
        let nonce = crate::modules::qoder_oauth::generate_login_nonce();
        let challenge = crate::modules::qoder_oauth::generate_pkce_challenge(&nonce);
        let sign_in = url::Url::parse(
            &channel
                .sign_in_url(&nonce, &challenge, "S256", Some("machine-test"))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(sign_in.host_str(), Some("qoder.cn"));
        let callback = sign_in
            .query_pairs()
            .find(|(key, _)| key == "oauth_callback")
            .expect("oauth_callback")
            .1
            .into_owned();
        let callback = url::Url::parse(&callback).unwrap();
        assert_eq!(callback.host_str(), Some("qoder.cn"));
        assert!(callback
            .query_pairs()
            .any(|(key, value)| key == "challenge" && value == challenge));
        assert!(!callback.query_pairs().any(|(key, _)| key == "redirect_uri"));
    }

    #[test]
    fn ide_bound_sources_have_no_login_channel() {
        for source in [
            CredentialSource::LegacyCnIde,
            CredentialSource::DedicatedIde,
            CredentialSource::Unsupported,
        ] {
            assert!(Channel::for_source(source).is_none());
        }
        assert_eq!(
            Channel::for_source(CredentialSource::GlobalIde)
                .unwrap()
                .openapi_base_url(),
            "https://openapi.qoder.sh"
        );
    }
}
