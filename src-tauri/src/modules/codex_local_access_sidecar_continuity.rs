// Go keeps protocol execution; its supervised worker shares the native continuity store.
fn sidecar_manifest_needs_continuity(accounts: &[Value], api_keys: &[Value]) -> bool {
    // Fixed provider gateways bypass the SDK Codex executor entirely. Starting
    // an identity worker for those routes would add an unrelated dependency to
    // native Chat/Responses connectivity tests and persistent provider profiles.
    api_keys.iter().any(|key| {
        key.get("enabled") != Some(&Value::Bool(false))
            && key.get("providerGateway").is_none_or(Value::is_null)
            && key
                .get("accountIds")
                .and_then(Value::as_array)
                .is_some_and(|ids| {
                    accounts.iter().any(|account| {
                        account
                            .get("continuityOwnerId")
                            .is_some_and(Value::is_string)
                            && account.get("continuityEligible") != Some(&Value::Bool(false))
                            && account
                                .get("provider")
                                .and_then(Value::as_str)
                                .is_none_or(|provider| provider.is_empty() || provider == "codex")
                            && ids.contains(&account["id"])
                    })
                })
    })
}

fn sidecar_continuity_manifest(
    base_dir: &Path,
    accounts: &[Value],
    api_keys: &[Value],
    session_identity_mapping_enabled: bool,
) -> Result<Value, String> {
    let identity_path = if base_dir == local_access_sidecar_dir()? {
        native_global_identity_path()?
    } else {
        base_dir.join("native-identity.sqlite")
    };
    let config_path = base_dir.join("continuity.json");
    // A stable per-gateway secret avoids restarting active streams on a status
    // refresh. It is never an upstream credential or a public API key.
    let token = std::fs::read(&config_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| {
            value
                .get("token")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .unwrap_or_else(|| {
            format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            )
        });
    let owners: HashMap<String, String> = accounts
        .iter()
        .filter_map(|account| {
            Some((
                account.get("id")?.as_str()?.to_string(),
                account.get("continuityOwnerId")?.as_str()?.to_string(),
            ))
        })
        .collect();
    let mut caller_accounts = HashMap::<String, Vec<String>>::new();
    for key in api_keys {
        if key.get("enabled") == Some(&Value::Bool(false)) {
            continue;
        }
        let Some(id) = key.get("id").and_then(Value::as_str) else {
            continue;
        };
        let caller = match id {
            "legacy" => NATIVE_PRIMARY_CALLER,
            "__cockpit_internal__" => NATIVE_INTERNAL_CALLER,
            other => other,
        };
        let ids = key
            .get("accountIds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|id| owners.contains_key(*id))
            .map(str::to_string)
            .collect();
        caller_accounts.insert(caller.to_string(), ids);
    }
    let config = json!({"identity_path":identity_path,"token":token,"owners":owners,"caller_accounts":caller_accounts,"session_identity_mapping_enabled":session_identity_mapping_enabled});
    write_secret_string_atomic_if_changed(
        &config_path,
        &serde_json::to_string(&config)
            .map_err(|_| "Continuity configuration cannot be encoded")?,
    )?;
    let executable =
        std::env::current_exe().map_err(|_| "Continuity worker executable is unavailable")?;
    // Include the policy in the launch manifest fingerprint so saving it reloads
    // the worker even though its private configuration path remains unchanged.
    Ok(json!({"version":1,"executable":executable,"configPath":config_path,"token":token,"sessionIdentityMappingEnabled":session_identity_mapping_enabled}))
}

#[cfg(test)]
mod sidecar_continuity_config_tests {
    use super::*;
    #[test]
    fn fixed_and_native_provider_routes_do_not_require_the_codex_worker() {
        let accounts = vec![
            json!({"id":"a","continuityOwnerId":"owner-a"}),
            json!({"id":"x","provider":"xai","continuityOwnerId":"owner-x"}),
            json!({"id":"automatic-provider","continuityEligible":false,"continuityOwnerId":"owner-native"}),
        ];
        assert!(!sidecar_manifest_needs_continuity(
            &accounts,
            &[json!({"id":"fixed","accountIds":["a"],"providerGateway":{"wireApi":"responses"}})]
        ));
        assert!(!sidecar_manifest_needs_continuity(
            &accounts,
            &[json!({"id":"native","accountIds":["x"]})]
        ));
        assert!(!sidecar_manifest_needs_continuity(
            &accounts,
            &[json!({"id":"automatic-native","accountIds":["automatic-provider"]})]
        ));
        assert!(!sidecar_manifest_needs_continuity(
            &accounts,
            &[json!({"id":"disabled","accountIds":["a"],"enabled":false})]
        ));
        assert!(sidecar_manifest_needs_continuity(
            &accounts,
            &[json!({"id":"mixed","accountIds":["a","x"],"modelRouting":{"defaultRoute":"auto"}})]
        ));
        assert!(sidecar_manifest_needs_continuity(
            &accounts,
            &[json!({"id":"legacy","accountIds":["a"]})]
        ));
    }
    #[test]
    fn continuity_config_is_private_stable_and_keeps_scopes_and_profile_paths() {
        let dir = tempfile::tempdir().unwrap();
        let accounts = vec![
            json!({"id":"a","continuityOwnerId":"owner-a"}),
            json!({"id":"b","continuityOwnerId":"owner-b"}),
        ];
        let keys = vec![
            json!({"id":"legacy","accountIds":["a"],"enabled":true}),
            json!({"id":"scoped","accountIds":["b"]}),
            json!({"id":"disabled","accountIds":["a"],"enabled":false}),
        ];
        let first = sidecar_continuity_manifest(dir.path(), &accounts, &keys, true).unwrap();
        let second = sidecar_continuity_manifest(dir.path(), &accounts, &keys, true).unwrap();
        assert_eq!(first, second);
        let path = dir.path().join("continuity.json");
        let config: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(config["session_identity_mapping_enabled"], true);
        assert_eq!(
            config["identity_path"],
            dir.path()
                .join("native-identity.sqlite")
                .to_string_lossy()
                .as_ref()
        );
        assert_eq!(
            config["caller_accounts"][NATIVE_PRIMARY_CALLER],
            json!(["a"])
        );
        assert_eq!(config["caller_accounts"]["scoped"], json!(["b"]));
        assert!(config["caller_accounts"].get("disabled").is_none());
        let disabled = sidecar_continuity_manifest(dir.path(), &accounts, &keys, false).unwrap();
        assert_eq!(disabled["sessionIdentityMappingEnabled"], false);
        assert_eq!(first["token"], disabled["token"]);
        assert_ne!(
            sidecar_config_fingerprint("{}", &json!({"continuity":first}).to_string()),
            sidecar_config_fingerprint("{}", &json!({"continuity":disabled}).to_string()),
        );
        let config: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(config["session_identity_mapping_enabled"], false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
