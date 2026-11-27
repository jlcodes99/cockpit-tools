//! Credential schemas of the native Qoder clients, adapted from
//! coderDJing/workbuddy-switch-next (MIT, copyright 2026 wb-switch).
//! See resources/licenses/WorkBuddy-Switch-Next.txt.
//!
//! A client profile is a flattened JSON blob whose field names differ per client
//! generation; this module converts between that blob and the snapshot shape the
//! account stores keep, and stamps the channel marker on the way in.
use super::qoder_channel::{snapshot_marker, source_of, Channel};
use serde_json::{json, Map, Value};

fn text(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .or_else(|| v.as_i64().map(|n| n.to_string()))
            })
            .filter(|v| !v.trim().is_empty())
    })
}

pub(crate) fn millis(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    let n = value.as_i64().or_else(|| value.as_str()?.parse().ok());
    if let Some(n) = n {
        return Some(if n > 100_000_000_000 {
            n
        } else {
            n.checked_mul(1000)?
        });
    }
    chrono::DateTime::parse_from_rfc3339(value.as_str()?)
        .ok()
        .map(|t| t.timestamp_millis())
}

/// Converts a decrypted client profile into a snapshot stamped for `channel`.
pub(crate) fn from_client(value: &Value, channel: Channel) -> Result<Value, String> {
    let token = text(
        value,
        &[
            "token",
            "accessToken",
            "access_token",
            "security_oauth_token",
        ],
    )
    .ok_or("QODER_NATIVE_TOKEN_MISSING")?;
    let mut user = value
        .get("user")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if !user.contains_key("id") {
        if let Some(uid) = text(value, &["uid", "user_id", "id"]) {
            user.insert("id".into(), json!(uid));
        }
    }
    let uid = text(&Value::Object(user.clone()), &["id", "user_id"])
        .ok_or("QODER_NATIVE_IDENTITY_MISSING")?;
    user.insert("id".into(), json!(uid));
    for (from, to) in [
        ("name", "name"),
        ("avatar_url", "avatarUrl"),
        ("email", "email"),
        ("data_policy_agreed", "data_policy_agreed"),
        ("is_data_policy_modifiable", "is_data_policy_modifiable"),
        ("user_type", "user_type"),
        ("is_personal_version", "is_personal_version"),
    ] {
        if let Some(v) = value.get(from) {
            user.entry(to).or_insert_with(|| v.clone());
        }
    }
    require_channel(value.get("user"), Some(value), channel)?;
    let mut info = user.clone();
    info.insert("token".into(), json!(token));
    info.insert("login_source".into(), json!(channel.marker()));
    info.insert("cockpit_native_user".into(), Value::Object(user));
    if let Some(refresh) = text(value, &["refreshToken", "refresh_token"]) {
        info.insert("refreshToken".into(), json!(refresh));
    }
    for (keys, dest) in [
        (
            &["expiresAt", "expires_at", "expire_time", "expireTime"][..],
            "expireTime",
        ),
        (
            &[
                "refreshTokenExpiresAt",
                "refresh_expires_at",
                "refresh_token_expire_time",
                "refreshTokenExpireTime",
            ][..],
            "refreshTokenExpireTime",
        ),
    ] {
        if let Some(expiry) = keys.iter().find_map(|key| millis(value.get(*key))) {
            info.insert(dest.into(), json!(expiry.to_string()));
        }
    }
    Ok(Value::Object(info))
}

/// Builds the client profile for one client generation from stored credentials.
pub(crate) fn for_client(info: &Value, desktop: bool) -> Result<Value, String> {
    let uid = text(info, &["id", "uid", "user_id"]).ok_or("QODER_NATIVE_IDENTITY_MISSING")?;
    let token = text(info, &["token", "securityOauthToken"]).ok_or("QODER_NATIVE_TOKEN_MISSING")?;
    let refresh = text(info, &["refreshToken"]).ok_or("QODER_REAUTH_REQUIRED")?;
    let expiry = millis(info.get("expireTime")).ok_or("QODER_NATIVE_EXPIRY_MISSING")?;
    let expiry_iso = chrono::DateTime::from_timestamp_millis(expiry)
        .ok_or("QODER_NATIVE_EXPIRY_INVALID")?
        .to_rfc3339();
    let refresh_expiry = millis(info.get("refreshTokenExpireTime"));
    let mut user = info
        .get("cockpit_native_user")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    user.insert("id".into(), json!(uid));
    for key in [
        "name",
        "email",
        "avatarUrl",
        "data_policy_agreed",
        "is_data_policy_modifiable",
        "user_type",
        "is_personal_version",
    ] {
        if let Some(value) = info.get(key) {
            user.insert(key.into(), value.clone());
        }
    }
    if desktop {
        let mut profile = json!({"schemaVersion":1,"token":token,"refreshToken":refresh,"expiresAt":expiry_iso,"user":user});
        if let Some(ms) = refresh_expiry {
            profile["refreshTokenExpiresAt"] = json!(chrono::DateTime::from_timestamp_millis(ms)
                .ok_or("QODER_NATIVE_EXPIRY_INVALID")?
                .to_rfc3339());
        }
        Ok(profile)
    } else {
        // Do not inherit another account's encrypt_user_info, key, policy or identity.
        let mut profile = Map::new();
        profile.insert("uid".into(), json!(uid));
        profile.insert("security_oauth_token".into(), json!(token));
        profile.insert("access_token".into(), json!(token));
        profile.insert("refresh_token".into(), json!(refresh));
        profile.insert("expire_time".into(), json!(expiry / 1000));
        if let Some(ms) = refresh_expiry {
            profile.insert("refresh_token_expire_time".into(), json!(ms / 1000));
        }
        for (from, to) in [
            ("name", "name"),
            ("avatarUrl", "avatar_url"),
            ("data_policy_agreed", "data_policy_agreed"),
            ("is_data_policy_modifiable", "is_data_policy_modifiable"),
            ("user_type", "user_type"),
            ("is_personal_version", "is_personal_version"),
        ] {
            if let Some(v) = user.get(from) {
                profile.insert(to.into(), v.clone());
            }
        }
        profile.insert("login_method".into(), json!("browser"));
        profile.insert(
            "login_timestamp".into(),
            json!(chrono::Utc::now().timestamp()),
        );
        profile.insert("encrypt_user_info".into(), json!(""));
        profile.insert("key".into(), json!(""));
        Ok(Value::Object(profile))
    }
}

/// Validates the whole payload before the first account is saved. The channel
/// the user picked may fill a legacy omission, never override a scope.
pub(crate) fn prepare_import(value: &mut Value, channel: Channel) -> Result<(), String> {
    if let Some(items) = value.as_array_mut() {
        for item in items {
            prepare_import(item, channel)?;
        }
        return Ok(());
    }
    let obj = value.as_object_mut().ok_or("QODER_IMPORT_INVALID")?;
    if let Some(items) = obj.get_mut("accounts") {
        return prepare_import(items, channel);
    }
    if !obj.contains_key("auth_user_info_raw") && !obj.contains_key("userInfo") {
        let info = from_client(&Value::Object(obj.clone()), channel)?;
        *value = json!({"userInfo":info});
        return Ok(());
    }
    let info_key = if obj.contains_key("auth_user_info_raw") {
        "auth_user_info_raw"
    } else {
        "userInfo"
    };
    let plan = obj
        .get("auth_user_plan_raw")
        .or_else(|| obj.get("userPlan"));
    require_channel(obj.get(info_key), plan, channel)?;
    let info = obj
        .get_mut(info_key)
        .and_then(Value::as_object_mut)
        .ok_or("QODER_IMPORT_IDENTITY_MISSING")?;
    info.insert("login_source".into(), json!(channel.marker()));
    Ok(())
}

/// An unstamped snapshot belongs to the channel that imported it; an explicitly
/// stamped one may only be imported into the channel it names.
fn require_channel(
    info: Option<&Value>,
    other: Option<&Value>,
    channel: Channel,
) -> Result<(), String> {
    let Some(marker) = snapshot_marker(info, other)? else {
        return Ok(());
    };
    if source_of(Some(marker)) != channel.source() {
        return Err("QODER_PLATFORM_MISMATCH".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::qoder_channel::CredentialSource;

    fn cn() -> Channel {
        Channel::for_source(CredentialSource::PublicCn).expect("CN channel")
    }

    #[test]
    fn native_roundtrip_preserves_identity_and_realm_for_both_clients() {
        let info = from_client(&json!({"token":"access-test","refreshToken":"refresh-test","expiresAt":"2027-01-01T00:00:00Z","user":{"id":"u1","name":"test","data_policy_agreed":true}}), cn()).unwrap();
        assert_eq!(info["login_source"], "qoder_cn");
        for desktop in [true, false] {
            let native = for_client(&info, desktop).unwrap();
            let restored = from_client(&native, cn()).unwrap();
            for key in ["id", "token", "refreshToken", "expireTime", "login_source"] {
                assert_eq!(info[key], restored[key], "{key}");
            }
            assert!(native.get("authenticationContext").is_none());
            assert!(native.get("profileOverlay").is_none());
        }
    }

    #[test]
    fn missing_identity_or_refresh_is_not_synthesized() {
        assert!(from_client(&json!({"token":"test"}), cn()).is_err());
        assert!(for_client(
            &json!({"id":"test","token":"test","expireTime":"1800000000000"}),
            true
        )
        .is_err());
    }

    #[test]
    fn import_preserves_plan_only_source_and_rejects_mixed_sources() {
        let mut account = json!({
            "auth_user_info_raw": { "id": "fixture-user" },
            "auth_user_plan_raw": { "login_source": "qoder" },
        });
        assert_eq!(
            prepare_import(&mut account, cn()).unwrap_err(),
            "QODER_PLATFORM_MISMATCH"
        );
        assert!(account["auth_user_info_raw"].get("login_source").is_none());

        let mut legacy = json!({"auth_user_info_raw": { "id": "fixture-user" }});
        prepare_import(&mut legacy, cn()).unwrap();
        assert_eq!(legacy["auth_user_info_raw"]["login_source"], "qoder_cn");

        account["auth_user_plan_raw"]["login_source"] = json!("qoder_cn");
        prepare_import(&mut account, cn()).unwrap();
        assert_eq!(account["auth_user_info_raw"]["login_source"], "qoder_cn");
    }

    #[test]
    fn native_import_does_not_override_an_explicit_source() {
        let profile = json!({
            "token": "fixture", "user": { "id": "fixture-user", "login_source": "qoder" },
        });
        assert_eq!(
            from_client(&profile, cn()).unwrap_err(),
            "QODER_PLATFORM_MISMATCH"
        );
        let legacy = json!({
            "token": "fixture", "user": { "id": "fixture-user", "login_source": "qodercn" },
        });
        assert_eq!(
            from_client(&legacy, cn()).unwrap_err(),
            "QODER_PLATFORM_MISMATCH"
        );
    }

    #[test]
    fn import_rejects_cross_channel_overrides() {
        let mut global = json!({"userInfo":{"id":"u","token":"test","login_source":"qoder"}});
        assert!(prepare_import(&mut global, cn()).is_err());
        assert_eq!(global["userInfo"]["login_source"], "qoder");
    }
}
