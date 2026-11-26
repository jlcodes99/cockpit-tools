//! Resolve native SSH aliases to the same endpoint as a configured sync target.
use serde_json::Value;
use std::collections::BTreeSet;

pub(super) const CONNECTIONS: &str = "codex-managed-remote-connections";
pub(super) const AUTO_CONNECT: &str = "remote-connection-auto-connect-by-host-id";
pub(super) const SIGNED_OUT_OWNER: &str = "signed-out";
pub(super) const RENDERER_OWNER: &str = "electron-renderer-content-owner";
pub(super) const OWNER_VALUE_PREFIX: &str = "electron-owner-value:";
pub(super) const OWNER_ATOMS_PREFIX: &str = "electron-owner-atoms:";
pub(super) const PERSISTED_ATOMS: &str = "electron-persisted-atom-state";

pub(super) const LOCAL_OWNER_VALUE_SUBKEYS: &[&str] = &[
    "open-in-target-preferences",
    "worktree-repository-preferences",
    "active-workspace-roots",
    "electron-saved-workspace-roots",
    "electron-workspace-root-labels",
    "local-projects",
    "app-server-project-id-by-legacy-project-id-by-host",
    "app-server-projects-migration-by-host",
    "app-server-pending-project-deletions-by-host",
    "selected-project",
    "project-writable-roots",
    "project-appearances",
    "project-files",
    "pinned-thread-ids",
    "app-server-migrated-pinned-thread-ids-by-host",
    "pinned-project-ids",
    "sidebar-project-thread-orders",
    "sidebar-thread-metadata",
    "thread-project-assignments",
    "thread-project-membership-host-ids",
    "thread-writable-roots",
    "thread-workspace-root-hints",
    "thread-projectless-output-directories",
    "projectless-thread-ids",
    "project-order",
];

pub(super) const SHARED_OWNER_ATOMS: &[&str] = &[
    "last_completed_onboarding",
    "electron:onboarding-projectless-completed",
    "electron:bedrock-govcloud-guidance-acknowledged",
    "ctrl-tab-shortcut-notice-shown",
    "number-shortcut-notice-shown",
    "thread-sort-key",
];

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

pub(super) fn scoped_owner_value_key(subkey: &str, owner: &str) -> String {
    format!("{OWNER_VALUE_PREFIX}{subkey}:{owner}")
}

pub(super) fn scoped_owner_atoms_key(owner: &str) -> String {
    format!("{OWNER_ATOMS_PREFIX}{owner}")
}

pub(super) fn has_scoped_owner_state(state: &Value) -> bool {
    state.as_object().is_some_and(|map| {
        map.keys().any(|key| {
            key.starts_with(OWNER_VALUE_PREFIX) || key.starts_with(OWNER_ATOMS_PREFIX)
        })
    })
}

fn merge_owner_json_objects(
    subkey: &str,
    target: &mut serde_json::Map<String, Value>,
    donor: &serde_json::Map<String, Value>,
) {
    for (k, v) in donor {
        match target.get_mut(k) {
            None => {
                target.insert(k.clone(), v.clone());
            }
            Some(existing) if existing.is_null() && !v.is_null() => {
                *existing = v.clone();
            }
            Some(Value::Array(existing_arr)) if v.is_array() => {
                if let Some(donor_arr) = v.as_array() {
                    for item in donor_arr {
                        if !existing_arr.contains(item) {
                            existing_arr.push(item.clone());
                        }
                    }
                }
            }
            Some(Value::Object(existing_obj)) if v.is_object() => {
                let donor_obj = v.as_object().expect("checked object");
                if subkey == "app-server-projects-migration-by-host" {
                    for flag in ["projectsMigrated", "threadAssignmentsMigrated"] {
                        if donor_obj.get(flag) == Some(&Value::Bool(true)) {
                            existing_obj.insert(flag.to_owned(), Value::Bool(true));
                        }
                    }
                    for (fk, fv) in donor_obj {
                        if !existing_obj.contains_key(fk) || existing_obj[fk].is_null() {
                            existing_obj.insert(fk.clone(), fv.clone());
                        }
                    }
                } else {
                    merge_owner_json_objects(subkey, existing_obj, donor_obj);
                }
            }
            Some(_) => {}
        }
    }
}

fn merge_local_owner_subkey_values(subkey: &str, donors: &[&Value]) -> Option<Value> {
    if donors.is_empty() {
        return None;
    }
    let Some(first_non_null) = donors.iter().copied().find(|v| !v.is_null()) else {
        return Some(Value::Null);
    };
    match first_non_null {
        Value::Array(_) => {
            let mut merged = Vec::new();
            for donor in donors {
                if let Some(items) = donor.as_array() {
                    for item in items {
                        if !merged.contains(item) {
                            merged.push(item.clone());
                        }
                    }
                }
            }
            Some(Value::Array(merged))
        }
        Value::Object(_) => {
            let mut merged = serde_json::Map::new();
            for donor in donors {
                if let Some(obj) = donor.as_object() {
                    merge_owner_json_objects(subkey, &mut merged, obj);
                }
            }
            Some(Value::Object(merged))
        }
        _ => donors
            .iter()
            .copied()
            .find(|v| match v {
                Value::Null => false,
                Value::String(s) => !s.trim().is_empty(),
                _ => true,
            })
            .or_else(|| Some(first_non_null))
            .cloned(),
    }
}

pub(super) fn inherit_scoped_owner_local_state(
    state: &mut Value,
    next_owner: &str,
) -> Vec<String> {
    let Some(map) = state.as_object() else {
        return Vec::new();
    };
    let active_owner = renderer_owner(state).map(str::to_owned);
    let mut owners = BTreeSet::new();
    let mut has_atoms = false;
    let mut present_subkeys = BTreeSet::new();
    for key in map.keys() {
        if let Some(owner) = key.strip_prefix(OWNER_ATOMS_PREFIX) {
            if !owner.is_empty() {
                owners.insert(owner.to_owned());
                has_atoms = true;
            }
        } else if let Some(rest) = key.strip_prefix(OWNER_VALUE_PREFIX) {
            if let Some((subkey, owner)) = rest.rsplit_once(':') {
                if !subkey.is_empty() && !owner.is_empty() {
                    owners.insert(owner.to_owned());
                    present_subkeys.insert(subkey.to_owned());
                }
            }
        }
    }
    if owners.is_empty() {
        return Vec::new();
    }

    let mut donor_owners = Vec::new();
    if let Some(ref active) = active_owner {
        donor_owners.push(active.clone());
    }
    if !donor_owners.iter().any(|o| o == next_owner) {
        donor_owners.push(next_owner.to_owned());
    }
    for owner in &owners {
        if !donor_owners.iter().any(|o| o == owner) {
            donor_owners.push(owner.clone());
        }
    }

    let mut updates: Vec<(String, Value)> = Vec::new();
    for &subkey in LOCAL_OWNER_VALUE_SUBKEYS {
        let has_scoped = present_subkeys.contains(subkey);
        if !has_scoped {
            continue;
        }
        let mut donors = Vec::new();
        if let Some(ref active) = active_owner {
            if let Some(val) = map.get(&scoped_owner_value_key(subkey, active)) {
                donors.push(val);
            }
        }
        if let Some(top_val) = map.get(subkey) {
            donors.push(top_val);
        }
        for owner in &donor_owners {
            if Some(owner.as_str()) == active_owner.as_deref() {
                continue;
            }
            if let Some(val) = map.get(&scoped_owner_value_key(subkey, owner)) {
                donors.push(val);
            }
        }
        let Some(merged) = merge_local_owner_subkey_values(subkey, &donors) else {
            continue;
        };
        let next_key = scoped_owner_value_key(subkey, next_owner);
        if map.get(&next_key) != Some(&merged) {
            updates.push((next_key, merged.clone()));
        }
        if let Some(ref active) = active_owner {
            if active != next_owner {
                let active_key = scoped_owner_value_key(subkey, active);
                if map.contains_key(&active_key) && map.get(&active_key) != Some(&merged) {
                    updates.push((active_key, merged.clone()));
                }
            }
        }
        if map.contains_key(subkey) && map.get(subkey) != Some(&merged) {
            updates.push((subkey.to_owned(), merged));
        }
    }

    if has_atoms {
        let mut atom_donors = Vec::new();
        if let Some(ref active) = active_owner {
            if let Some(val) = map.get(&scoped_owner_atoms_key(active)) {
                atom_donors.push(val);
            }
        }
        if let Some(top_atoms) = map.get(PERSISTED_ATOMS) {
            atom_donors.push(top_atoms);
        }
        for owner in &donor_owners {
            if Some(owner.as_str()) == active_owner.as_deref() {
                continue;
            }
            if let Some(val) = map.get(&scoped_owner_atoms_key(owner)) {
                atom_donors.push(val);
            }
        }
        let next_atoms_key = scoped_owner_atoms_key(next_owner);
        let mut next_atoms = map
            .get(&next_atoms_key)
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mut shared_found = serde_json::Map::new();
        for &atom_key in SHARED_OWNER_ATOMS {
            if let Some(val) = atom_donors
                .iter()
                .find_map(|donor| donor.get(atom_key).filter(|v| !v.is_null()))
            {
                next_atoms.insert(atom_key.to_owned(), val.clone());
                shared_found.insert(atom_key.to_owned(), val.clone());
            }
        }
        let next_atoms_val = Value::Object(next_atoms);
        if (!next_atoms_val.as_object().unwrap().is_empty() || map.contains_key(&next_atoms_key))
            && map.get(&next_atoms_key) != Some(&next_atoms_val)
        {
            updates.push((next_atoms_key, next_atoms_val));
        }
        if let Some(top_atoms) = map.get(PERSISTED_ATOMS).and_then(Value::as_object) {
            let mut updated_top = top_atoms.clone();
            for (k, v) in shared_found {
                updated_top.insert(k, v);
            }
            let updated_top_val = Value::Object(updated_top);
            if map.get(PERSISTED_ATOMS) != Some(&updated_top_val) {
                updates.push((PERSISTED_ATOMS.to_owned(), updated_top_val));
            }
        }
    }

    let mut changed_keys = Vec::with_capacity(updates.len());
    if let Some(map_mut) = state.as_object_mut() {
        for (key, value) in updates {
            map_mut.insert(key.clone(), value);
            changed_keys.push(key);
        }
    }
    changed_keys
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
