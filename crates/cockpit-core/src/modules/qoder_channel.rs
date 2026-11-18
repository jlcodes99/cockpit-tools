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

fn recorded_marker(account: &QoderAccount) -> Result<Option<&str>, String> {
    let info = marker(account.auth_user_info_raw.as_ref())?;
    let plan = marker(account.auth_user_plan_raw.as_ref())?;
    if matches!((info, plan), (Some(info), Some(plan)) if info != plan) {
        return Err("QODER_PLATFORM_MISMATCH".into());
    }
    Ok(info.or(plan))
}

pub(crate) fn credential_source(account: &QoderAccount) -> Result<CredentialSource, String> {
    Ok(match recorded_marker(account)? {
        None | Some("qoder") => CredentialSource::GlobalIde,
        Some("qodercn") => CredentialSource::LegacyCnIde,
        Some("dedicated_qodercn") => CredentialSource::DedicatedIde,
        // Cockpit's public CN marker is distinct from the legacy IDE's qodercn.
        Some("qoder_cn") => CredentialSource::PublicCn,
        _ => CredentialSource::Unsupported,
    })
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
}
