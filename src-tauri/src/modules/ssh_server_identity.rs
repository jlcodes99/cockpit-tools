//! Resolve native SSH aliases to the same endpoint as a configured sync target.
use serde_json::Value;
use std::collections::BTreeSet;

pub(super) const CONNECTIONS: &str = "codex-managed-remote-connections";
pub(super) const AUTO_CONNECT: &str = "remote-connection-auto-connect-by-host-id";
pub(super) const SIGNED_OUT_OWNER: &str = "signed-out";
pub(super) const RENDERER_OWNER: &str = "electron-renderer-content-owner";

pub(super) fn renderer_owner(state: &Value) -> Option<&str> {
    state[RENDERER_OWNER].as_str()
}

pub(super) fn scoped_auto_connect_key(owner: &str) -> String {
    format!("electron-account-remote-preference:{AUTO_CONNECT}:{owner}")
}

pub(super) fn auto_connect(state: &Value) -> &Value {
    match renderer_owner(state) {
        Some(owner) => &state[scoped_auto_connect_key(owner)],
        None => &state[AUTO_CONNECT],
    }
}

pub(super) fn configured_hosts(state: &Value) -> BTreeSet<String> {
    state[CONNECTIONS]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| {
            ["hostname", "alias"]
                .iter()
                .any(|key| entry[*key].as_str().is_some_and(|s| !s.trim().is_empty()))
        })
        .filter_map(|entry| {
            entry["hostId"]
                .as_str()
                .filter(|id| !id.is_empty() && *id != "local")
        })
        .map(str::to_owned)
        .collect()
}

pub(super) fn inherit_scoped_auto_connect(state: &mut Value, next_owner: &str) -> bool {
    if renderer_owner(state).is_none_or(|owner| owner == next_owner) {
        return false;
    }
    let key = scoped_auto_connect_key(next_owner);
    let mut target = match state.get(&key) {
        Some(Value::Object(target)) => target.clone(),
        None => serde_json::Map::new(),
        Some(_) => {
            crate::modules::logger::log_warn("[Codex SSH] 目标账号的自动连接设置格式不受支持");
            return false;
        }
    };
    let current = auto_connect(state);
    for host in configured_hosts(state) {
        if let Some(enabled) = current[&host].as_bool() {
            target.insert(host, Value::Bool(enabled));
        }
    }
    if target.is_empty() || state[&key].as_object() == Some(&target) {
        return false;
    }
    state[key] = Value::Object(target);
    true
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Endpoint {
    pub host: String,
    pub user: String,
    pub port: u16,
}

pub(super) fn endpoint(text: &str) -> Option<Endpoint> {
    let mut host = None;
    let mut user = None;
    let mut port = None;
    for line in text.lines() {
        let (key, value) = line.split_once(char::is_whitespace)?;
        match key {
            "hostname" => host = Some(value.trim().to_ascii_lowercase()),
            "user" => user = Some(value.trim().to_string()),
            "port" => port = value.trim().parse::<u16>().ok().filter(|port| *port > 0),
            _ => {}
        }
    }
    let result = Endpoint {
        host: host?,
        user: user?,
        port: port?,
    };
    (!result.host.is_empty() && !result.user.is_empty()).then_some(result)
}

pub(super) fn aliases(state: &Value) -> Vec<(String, String)> {
    let preferences = auto_connect(state);
    state[CONNECTIONS]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let host = entry["hostId"].as_str()?;
            let alias = entry["alias"].as_str()?.trim();
            (host != "local" && !host.is_empty() && !alias.is_empty() && preferences[host] == true)
                .then(|| (host.to_string(), alias.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ssh_config_identity_requires_host_user_and_port() {
        let a = endpoint("host alias\nhostname Example.INVALID\nuser alice\nport 22\n").unwrap();
        assert_eq!(
            a,
            endpoint("hostname example.invalid\nuser alice\nport 22").unwrap()
        );
        assert_ne!(
            Some(a),
            endpoint("hostname example.invalid\nuser bob\nport 22")
        );
        assert!(endpoint("hostname fixture.invalid\nport 22").is_none());
        assert!(endpoint("hostname fixture.invalid\nuser alice\nport 0").is_none());
    }

    #[test]
    fn only_explicitly_enabled_saved_ssh_aliases_can_be_reconnected() {
        let state = json!({"codex-managed-remote-connections":[
            {"hostId":"ssh-on","alias":"alias"}, {"hostId":"ssh-off","alias":"other"},
            {"hostId":"local","alias":"local"}, {"hostId":"cloud","hostname":"fixture.invalid"}
        ],"remote-connection-auto-connect-by-host-id":{"ssh-on":true,"ssh-off":false,"local":true}});
        assert_eq!(aliases(&state), [("ssh-on".into(), "alias".into())]);
    }

    #[test]
    fn finds_enabled_aliases_in_the_current_desktop_account_only() {
        let state = json!({
            "electron-renderer-content-owner": "current",
            "codex-managed-remote-connections": [
                {"hostId":"first","alias":"one"}, {"hostId":"second","alias":"two"},
                {"hostId":"disabled","alias":"three"}, {"hostId":"missing","alias":"four"}
            ],
            "electron-account-remote-preference:remote-connection-auto-connect-by-host-id:current": {
                "first":true,"second":true,"disabled":false
            },
            "electron-account-remote-preference:remote-connection-auto-connect-by-host-id:old": {
                "disabled":true,"missing":true
            },
            "remote-connection-auto-connect-by-host-id": {"disabled":true,"missing":true}
        });
        assert_eq!(
            aliases(&state),
            [
                ("first".into(), "one".into()),
                ("second".into(), "two".into())
            ]
        );
    }
}
