use super::*;
use tempfile::TempDir;

const A: &str = "11111111-1111-4111-8111-111111111111";
const B: &str = "22222222-2222-4222-8222-222222222222";

fn write(path: &Path, value: &Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn read(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn connection(url: &str, key: &str) -> Value {
    json!({
        "inferenceProvider": "gateway", "inferenceGatewayBaseUrl": url,
        "inferenceGatewayApiKey": key, "inferenceGatewayAuthScheme": "bearer",
        "inferenceCredentialKind": "static",
        "inferenceModels": [{"name": "claude-sonnet"}]
    })
}

fn gateway_account() -> ClaudeAccount {
    serde_json::from_value(json!({
        "id": "gateway-config-regression", "email": "Test gateway",
        "auth_mode": "desktop_gateway", "created_at": 0, "last_used": 0,
        "api_base_url": "https://example.invalid/anthropic", "api_key": "test-upstream-key",
        "desktop_gateway_auth_scheme": "x-api-key", "desktop_gateway_connection_mode": "direct",
        "desktop_gateway_models": ["claude-sonnet"],
        "desktop_gateway_model_mappings": [{"desktopModel": "claude-sonnet", "upstreamModel": "deepseek-test"}]
    })).unwrap()
}

struct Fixture {
    _temp: TempDir,
    dir: PathBuf,
    modes: Vec<PathBuf>,
    known: BTreeSet<String>,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("Claude-3p/configLibrary");
        let modes = vec![
            temp.path().join("Claude/claude_desktop_config.json"),
            temp.path().join("Claude-3p/claude_desktop_config.json"),
        ];
        for path in &modes {
            write(
                path,
                &json!({"deploymentMode": "1p", "mcpServers": {"local": {"command": "tool"}},
                "preferences": {"keep": true}}),
            );
        }
        Self {
            _temp: temp,
            dir,
            modes,
            known: [A.to_string(), B.to_string()].into(),
        }
    }

    fn seed(&self) {
        let mut config = connection("https://old.invalid", "old-provider-secret");
        let object = config.as_object_mut().unwrap();
        object.extend(
            json!({
                "disableEssentialTelemetry": true, "disableNonessentialTelemetry": true,
                "modelPrefer1mContext": true, "disableAutoUpdates": true,
                "disableDeploymentModeChooser": false,
                "coworkEgressAllowedHosts": ["docs.example.com"],
                "disableBypassPermissionsMode": true, "allowedWorkspaceFolders": [],
                "unknownFuturePolicy": {"nested": [false, null, 0]},
                "inferenceCredentialHelper": "old-helper",
                "inferenceCredentialHelperWindows": "old-helper.exe",
                "inferenceCustomHeaders": {"Authorization": "old-header-secret"},
                "inferenceGatewayOidc": {"clientSecret": "old-oidc-secret"},
                "inferenceModels": [{"name": "claude-sonnet", "supports1m": true,
                    "prefer1m": true, "labelOverride": "My label", "futureCapability": [1,2]},
                    {"name": "claude-obsolete", "supports1m": true}]
            })
            .as_object()
            .unwrap()
            .clone(),
        );
        write(&self.dir.join(format!("{A}.json")), &config);
        write(
            &self.dir.join("_meta.json"),
            &json!({
                "appliedId": A, "futureMeta": {"keep": true},
                "entries": [{"id": A, "name": "Old", "provider": "gateway", "custom": 7},
                    {"id": "manual", "name": "User", "provider": "gateway", "custom": 8},
                    {"id": "other-3p", "name": "Other"}]
            }),
        );
        write(
            &self.dir.join("manual.json"),
            &connection("https://manual.invalid", "manual-secret"),
        );
        write(
            &self.dir.join("other-3p.json"),
            &json!({"deploymentMode": "3p", "inferenceProvider": "bedrock"}),
        );
    }

    fn apply(&self, id: &str, config: &Value) -> Result<(), String> {
        apply(
            &self.dir,
            &self.modes,
            id,
            "New provider",
            config,
            &self.known,
        )
    }

    fn snapshot(&self) -> Vec<(PathBuf, Vec<u8>)> {
        fn collect(dir: &Path, result: &mut Vec<(PathBuf, Vec<u8>)>) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    collect(&path, result);
                } else {
                    result.push((path.clone(), fs::read(path).unwrap()));
                }
            }
        }
        let mut files = Vec::new();
        collect(self._temp.path(), &mut files);
        files.sort();
        files
    }
}

#[test]
fn switching_provider_and_transport_preserves_policy_and_model_metadata() {
    let f = Fixture::new();
    f.seed();
    let original = read(&f.dir.join(format!("{A}.json")));
    let manual = fs::read(f.dir.join("manual.json")).unwrap();
    let other = fs::read(f.dir.join("other-3p.json")).unwrap();
    write(&f.dir.join(format!("{A}.json.bak")), &original);

    for (id, url, key) in [
        (B, "https://new.invalid", "new-secret"),
        (B, "http://127.0.0.1:12345", "ag-local-secret"),
        (A, "https://direct.invalid", "direct-secret"),
    ] {
        f.apply(id, &connection(url, key)).unwrap();
        let config = read(&f.dir.join(format!("{id}.json")));
        for (field, value) in original.as_object().unwrap() {
            if !is_connection_key(field) && field != "inferenceModels" {
                assert_eq!(&config[field], value, "policy changed: {field}");
            }
        }
        assert_eq!(config["inferenceGatewayBaseUrl"], url);
        assert_eq!(config["inferenceGatewayApiKey"], key);
        assert_eq!(config["inferenceCredentialKind"], "static");
        for stale in [
            "inferenceCredentialHelper",
            "inferenceCredentialHelperWindows",
            "inferenceCustomHeaders",
            "inferenceGatewayOidc",
        ] {
            assert!(config.get(stale).is_none(), "retained {stale}");
        }
        assert_eq!(
            config["inferenceModels"],
            json!([{"name": "claude-sonnet",
            "supports1m": true, "prefer1m": true, "labelOverride": "My label", "futureCapability": [1,2]}])
        );
        assert_eq!(fs::read(f.dir.join("manual.json")).unwrap(), manual);
        assert_eq!(fs::read(f.dir.join("other-3p.json")).unwrap(), other);
        let meta = read(&f.dir.join("_meta.json"));
        assert_eq!(meta["appliedId"], id);
        assert_eq!(meta["futureMeta"], json!({"keep": true}));
        assert!(meta["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["id"] == "manual" && entry["custom"] == 8));
        for path in &f.modes {
            let mode = read(path);
            assert_eq!(mode["deploymentMode"], "3p");
            assert_eq!(mode["preferences"], json!({"keep": true}));
            assert_eq!(mode["mcpServers"]["local"]["command"], "tool");
        }
    }
    assert!(!f.dir.join(format!("{B}.json")).exists());
    assert!(!f.dir.join(format!("{A}.json.bak")).exists());
    assert!(!f
        .snapshot()
        .iter()
        .any(|(_, bytes)| String::from_utf8_lossy(bytes).contains("old-provider-secret")));
}

#[test]
fn first_apply_does_not_invent_privacy_permissions_or_network_defaults() {
    let f = Fixture::new();
    f.apply(A, &build_provider_config(&gateway_account()).unwrap())
        .unwrap();
    let config = read(&f.dir.join(format!("{A}.json")));
    assert!(config.get("coworkEgressAllowedHosts").is_none());
    assert!(config.get("disableDeploymentModeChooser").is_none());
    assert!(config.get("disableEssentialTelemetry").is_none());
}

#[test]
fn provider_builder_retains_explicit_false_and_rejects_invalid_accounts() {
    let mut account = gateway_account();
    account.desktop_gateway_model_mappings.as_mut().unwrap()[0].supports_1m = Some(false);
    let generated = build_provider_config(&account).unwrap();
    assert_eq!(generated["inferenceModels"][0]["supports1m"], false);
    assert_eq!(generated["inferenceGatewayAuthScheme"], "x-api-key");
    account.api_key = None;
    assert!(build_provider_config(&account).is_err());
    account.auth_mode = ClaudeAuthMode::DesktopOAuth;
    assert!(build_provider_config(&account).is_err());
}

#[test]
fn actual_local_mapping_config_authenticates_and_routes_without_losing_policy() {
    let f = Fixture::new();
    f.seed();
    let mut account = gateway_account();
    f.apply(B, &build_provider_config(&account).unwrap())
        .unwrap();
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    account.api_base_url = Some(format!("http://{}/anthropic", server.server_addr()));
    account.desktop_gateway_connection_mode = Some("local_mapping".to_string());
    let captured = std::thread::spawn(move || {
        let mut request = server
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .expect("upstream request");
        assert_eq!(request.url(), "/anthropic/v1/messages");
        let auth = request
            .headers()
            .iter()
            .find(|header| header.field.equiv("x-api-key"));
        assert_eq!(auth.unwrap().value.as_str(), "test-upstream-key");
        let mut body = String::new();
        request.as_reader().read_to_string(&mut body).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&body).unwrap()["model"],
            "deepseek-test"
        );
        request
            .respond(tiny_http::Response::from_string("{\"type\":\"message\"}"))
            .unwrap();
    });
    f.apply(B, &build_provider_config(&account).unwrap())
        .unwrap();
    let config = read(&f.dir.join(format!("{B}.json")));
    assert_eq!(config["disableEssentialTelemetry"], true);
    assert_eq!(
        config["coworkEgressAllowedHosts"],
        json!(["docs.example.com"])
    );
    assert_eq!(config["modelPrefer1mContext"], true);
    assert_eq!(config["inferenceModels"][0]["supports1m"], true);
    assert_ne!(config["inferenceGatewayApiKey"], "test-upstream-key");
    assert_eq!(config["inferenceGatewayAuthScheme"], "bearer");
    let response = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap()
        .post(format!(
            "{}/v1/messages",
            config["inferenceGatewayBaseUrl"].as_str().unwrap()
        ))
        .bearer_auth(config["inferenceGatewayApiKey"].as_str().unwrap())
        .json(&json!({"model": "claude-sonnet", "max_tokens": 1, "messages": []}))
        .send()
        .unwrap();
    assert!(response.status().is_success());
    captured.join().unwrap();
    account.desktop_gateway_connection_mode = Some("direct".to_string());
    f.apply(A, &build_provider_config(&account).unwrap())
        .unwrap();
    let direct = read(&f.dir.join(format!("{A}.json")));
    assert_eq!(direct["inferenceGatewayApiKey"], "test-upstream-key");
    assert_eq!(direct["inferenceGatewayAuthScheme"], "x-api-key");
    assert_eq!(direct["modelPrefer1mContext"], true);
    assert!(!f.dir.join(format!("{B}.json")).exists());
}

#[test]
fn explicit_model_capabilities_override_existing_values_and_discovery_clears_old_list() {
    let f = Fixture::new();
    f.seed();
    let mut generated = connection("https://new.invalid", "key");
    generated["inferenceModels"] = json!([{"name": "CLAUDE-SONNET", "supports1m": false,
        "labelOverride": "New label"}, {"name": "claude-haiku"}]);
    f.apply(A, &generated).unwrap();
    let config = read(&f.dir.join(format!("{A}.json")));
    assert_eq!(config["inferenceModels"][0]["supports1m"], false);
    assert_eq!(config["inferenceModels"][0]["labelOverride"], "New label");
    assert_eq!(
        config["inferenceModels"][0]["futureCapability"],
        json!([1, 2])
    );
    assert!(config["inferenceModels"][1].get("supports1m").is_none());
    generated.as_object_mut().unwrap().remove("inferenceModels");
    f.apply(A, &generated).unwrap();
    let config = read(&f.dir.join(format!("{A}.json")));
    assert!(config.get("inferenceModels").is_none());
    assert_eq!(config["modelPrefer1mContext"], true);
}

#[test]
fn reapply_preserves_entry_extensions_and_is_idempotent() {
    let f = Fixture::new();
    f.seed();
    f.apply(A, &connection("https://new.invalid", "key"))
        .unwrap();
    let meta = read(&f.dir.join("_meta.json"));
    assert_eq!(meta["entries"][0]["custom"], 7);
    assert_eq!(meta["entries"][0][OWNER_KEY], true);
    let snapshot = f.snapshot();
    f.apply(A, &connection("https://new.invalid", "key"))
        .unwrap();
    assert_eq!(f.snapshot(), snapshot);
}

#[cfg(windows)]
#[test]
fn differently_cased_owned_ids_do_not_retire_the_selected_file() {
    let mut f = Fixture::new();
    let lower = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    let upper = lower.to_ascii_uppercase();
    f.known.extend([lower.to_string(), upper.clone()]);
    let desired = connection("https://same.invalid", "same-key");
    f.apply(lower, &desired).unwrap();
    let path = f.dir.join(format!("{lower}.json"));
    assert!(same_provider_path(
        &path,
        &f.dir.join(format!("{upper}.json"))
    ));
    f.apply(&upper, &desired).unwrap();
    assert_eq!(read(&path)["inferenceGatewayApiKey"], "same-key");
    let meta = read(&f.dir.join("_meta.json"));
    assert_eq!(meta["appliedId"], upper);
    assert_eq!(meta["entries"].as_array().unwrap().len(), 1);
    remove_managed(&f.dir, &f.modes, &f.known).unwrap();
    assert!(!path.exists());
    assert_eq!(read(&f.dir.join("_meta.json"))["entries"], json!([]));
}

#[test]
fn unchanged_provider_reapply_removes_a_legacy_credential_backup() {
    let f = Fixture::new();
    let desired = connection("https://new.invalid", "new-key");
    f.apply(A, &desired).unwrap();
    let backup = f.dir.join(format!("{A}.json.bak"));
    write(&backup, &connection("https://old.invalid", "old-key"));
    f.apply(A, &desired).unwrap();
    assert!(!backup.exists());
    assert_eq!(
        read(&f.dir.join(format!("{A}.json")))["inferenceGatewayApiKey"],
        "new-key"
    );
}

#[test]
fn cleanup_removes_only_owned_gateways_and_preserves_main_config() {
    let f = Fixture::new();
    f.seed();
    remove_managed(&f.dir, &f.modes, &f.known).unwrap();
    assert!(!f.dir.join(format!("{A}.json")).exists());
    assert!(f.dir.join("manual.json").exists());
    assert!(f.dir.join("other-3p.json").exists());
    let meta = read(&f.dir.join("_meta.json"));
    assert!(meta.get("appliedId").is_none());
    assert_eq!(meta["entries"].as_array().unwrap().len(), 2);
    assert_eq!(meta["futureMeta"], json!({"keep": true}));
    for path in &f.modes {
        assert_eq!(read(path)["preferences"], json!({"keep": true}));
    }
    let snapshot = f.snapshot();
    remove_managed(&f.dir, &f.modes, &f.known).unwrap();
    assert_eq!(snapshot, f.snapshot());
}

#[test]
fn markers_survive_account_removal_but_non_gateway_reuse_is_protected() {
    let f = Fixture::new();
    f.apply(A, &connection("https://first.invalid", "key"))
        .unwrap();
    remove_managed(&f.dir, &f.modes, &BTreeSet::new()).unwrap();
    assert!(!f.dir.join(format!("{A}.json")).exists());
    f.seed();
    write(
        &f.dir.join(format!("{A}.json")),
        &json!({"inferenceProvider": "bedrock", "keep": true}),
    );
    let before = f.snapshot();
    assert!(f
        .apply(A, &connection("https://first.invalid", "key"))
        .is_err());
    assert_eq!(before, f.snapshot());
    remove_managed(&f.dir, &f.modes, &f.known).unwrap();
    assert_eq!(read(&f.dir.join(format!("{A}.json")))["keep"], true);
}

#[test]
fn user_owned_active_provider_is_copied_without_modifying_it_or_transferring_credentials() {
    let f = Fixture::new();
    f.seed();
    let mut manual = connection("https://manual.invalid", "manual-secret");
    manual["inferenceProvider"] = json!("foundry");
    manual["inferenceFoundryApiKey"] = json!("foundry-secret");
    manual["disableEssentialTelemetry"] = json!(true);
    write(&f.dir.join("manual.json"), &manual);
    let before = fs::read(f.dir.join("manual.json")).unwrap();
    let mut meta = read(&f.dir.join("_meta.json"));
    meta["appliedId"] = json!("manual");
    write(&f.dir.join("_meta.json"), &meta);
    f.apply(B, &connection("https://new.invalid", "new-key"))
        .unwrap();
    let config = read(&f.dir.join(format!("{B}.json")));
    assert!(config.get("inferenceFoundryApiKey").is_none());
    assert_eq!(config["disableEssentialTelemetry"], true);
    assert_eq!(fs::read(f.dir.join("manual.json")).unwrap(), before);
}

#[test]
fn invalid_json_shapes_and_path_ids_leave_every_file_unchanged() {
    for bad in ["", "{\"secret\":\"must-not-appear\",", "[]", "null"] {
        let f = Fixture::new();
        f.seed();
        fs::write(f.dir.join(format!("{A}.json")), bad).unwrap();
        let before = f.snapshot();
        let error = f
            .apply(B, &connection("https://new.invalid", "key"))
            .unwrap_err();
        assert!(!error.contains("must-not-appear"));
        assert_eq!(before, f.snapshot());
    }
    for bad_meta in [
        json!({"entries": {}}),
        json!({"entries": [null]}),
        json!({"appliedId": 3}),
        json!({"appliedId": "../outside"}),
        json!({"appliedId": "_meta"}),
        json!({"appliedId": "missing"}),
    ] {
        let f = Fixture::new();
        f.seed();
        write(&f.dir.join("_meta.json"), &bad_meta);
        let before = f.snapshot();
        assert!(f
            .apply(B, &connection("https://new.invalid", "key"))
            .is_err());
        assert_eq!(before, f.snapshot());
    }
}

#[test]
fn malformed_second_deployment_file_aborts_before_changing_anything() {
    let f = Fixture::new();
    f.seed();
    fs::write(&f.modes[1], "[]").unwrap();
    let before = f.snapshot();
    assert!(f
        .apply(B, &connection("https://new.invalid", "key"))
        .is_err());
    assert_eq!(before, f.snapshot());
}

#[test]
fn io_failure_after_provider_write_rolls_back_primary_files_and_new_provider() {
    let f = Fixture::new();
    f.seed();
    // Secret atomic writes must remove a legacy .bak. A directory here forces
    // a real I/O error on _meta.json, after the new provider has been written.
    fs::create_dir(f.dir.join("_meta.json.bak")).unwrap();
    let before = f.snapshot();
    assert!(f
        .apply(B, &connection("https://new.invalid", "key"))
        .is_err());
    assert_eq!(before, f.snapshot());
    assert!(!f.dir.join(format!("{B}.json")).exists());
}

#[test]
fn failure_updating_second_profile_rolls_back_provider_metadata_and_first_profile() {
    let f = Fixture::new();
    f.seed();
    fs::create_dir(f.modes[1].with_extension("json.bak")).unwrap();
    let before = f.snapshot();
    assert!(f
        .apply(B, &connection("https://new.invalid", "key"))
        .is_err());
    assert_eq!(before, f.snapshot());
}

#[cfg(windows)]
#[test]
fn locked_old_provider_causes_switch_to_roll_back_instead_of_leaving_stale_credentials() {
    use std::os::windows::fs::OpenOptionsExt;
    let f = Fixture::new();
    f.seed();
    let _locked = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(f.dir.join(format!("{A}.json")))
        .unwrap();
    let before = f.snapshot();
    assert!(f
        .apply(B, &connection("https://new.invalid", "key"))
        .is_err());
    assert_eq!(before, f.snapshot());
}

#[test]
fn preflight_conflict_keeps_primary_files_and_newer_external_edits() {
    let temp = tempfile::tempdir().unwrap();
    let a = temp.path().join("a.json");
    let b = temp.path().join("b.json");
    write(&a, &json!({"value": "before"}));
    write(&b, &json!({"value": "before"}));
    let mut changes = vec![
        Change::read(a.clone()).unwrap(),
        Change::read(b.clone()).unwrap(),
    ];
    for change in &mut changes {
        change.set_json(&json!({"value": "ours"})).unwrap();
    }
    write(&b, &json!({"value": "newer external edit"}));
    assert!(commit(&changes).is_err());
    assert_eq!(read(&a), json!({"value": "before"}));
    assert_eq!(read(&b), json!({"value": "newer external edit"}));
}

#[test]
fn file_ids_reject_windows_devices_and_directory_traversal() {
    for id in [
        "../outside",
        "x/../../y",
        "x\\y",
        "_meta",
        "_META",
        "CON",
        "nul",
        "COM1",
        "LPT9",
        "x:stream",
    ] {
        assert!(
            provider_path(Path::new("library"), id).is_err(),
            "accepted {id}"
        );
    }
}
