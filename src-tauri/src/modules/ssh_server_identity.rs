//! Resolve native SSH aliases to the same endpoint as a configured sync target.
use serde_json::Value;

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
    state["codex-managed-remote-connections"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let host = entry["hostId"].as_str()?;
            let alias = entry["alias"].as_str()?.trim();
            (host != "local"
                && !host.is_empty()
                && !alias.is_empty()
                && state["remote-connection-auto-connect-by-host-id"][host] == true)
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
}
