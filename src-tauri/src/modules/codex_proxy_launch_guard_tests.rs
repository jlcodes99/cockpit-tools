use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[test]
fn country_parser_and_mismatch_never_guess_from_an_ip_or_node_name() {
    assert_eq!(parse_country(b"ip=192.0.2.1\nloc=JP\n").unwrap(), "JP");
    for body in [
        b"".as_slice(),
        b"Tokyo",
        b"loc=XX",
        b"loc=JP\nloc=SG",
        b"<html>blocked</html>",
    ] {
        assert_eq!(parse_country(body).unwrap_err(), UNKNOWN);
    }
    assert!(compare_country("JP", "JP").is_ok());
    assert_eq!(
        compare_country("JP", "SG").unwrap_err(),
        "PROXY_COUNTRY_MISMATCH:JP:SG"
    );
}
#[tokio::test]
async fn country_probe_uses_only_the_selected_proxy_without_account_credentials() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut data = Vec::new();
        while !data.ends_with(b"\r\n\r\n") {
            data.push(socket.read_u8().await.unwrap());
            assert!(data.len() < 8192);
        }
        let request = String::from_utf8(data).unwrap();
        assert!(request.starts_with("GET http://unresolvable.invalid/ HTTP/1.1"));
        assert!(!request.to_ascii_lowercase().contains("authorization"));
        assert!(!request.to_ascii_lowercase().contains("cookie:"));
        let body = "ip=192.0.2.1\nloc=JP\n";
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    });
    let country = query_country(
        &probe_client(Some(&proxy), None).unwrap(),
        "http://unresolvable.invalid/",
    )
    .await
    .unwrap();
    assert_eq!(country, "JP");
    server.await.unwrap();
}
#[tokio::test]
async fn unavailable_proxy_cannot_fall_back_to_a_reachable_direct_origin() {
    let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = format!("http://{}", dead.local_addr().unwrap());
    drop(dead);
    let url = format!("http://{}/", origin.local_addr().unwrap());
    assert_eq!(
        query_country(&probe_client(Some(&proxy), None).unwrap(), &url)
            .await
            .unwrap_err(),
        UNKNOWN
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), origin.accept())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn redirects_fail_without_contacting_a_different_target() {
    let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_url = format!("http://{}", proxy.local_addr().unwrap());
    let redirect = format!("http://{}/", target.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = proxy.accept().await.unwrap();
        let mut data = vec![0; 8192];
        let _ = socket.read(&mut data).await.unwrap();
        socket.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {redirect}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
    });
    assert_eq!(
        query_country(
            &probe_client(Some(&proxy_url), None).unwrap(),
            "http://unresolvable.invalid/"
        )
        .await
        .unwrap_err(),
        UNKNOWN
    );
    server.await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), target.accept())
            .await
            .is_err()
    );
}
#[tokio::test]
async fn country_policy_is_optional_per_account_and_normalizes_gateway_bindings() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let root = std::env::temp_dir().join(format!("country-policy-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let keys = ["COCKPIT_TOOLS_TEST_DATA_DIR", "COCKPIT_TOOLS_DATA_DIR"];
    let previous = keys.map(std::env::var_os);
    struct Cleanup(PathBuf, [Option<std::ffi::OsString>; 2]);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            for (key, value) in ["COCKPIT_TOOLS_TEST_DATA_DIR", "COCKPIT_TOOLS_DATA_DIR"]
                .into_iter()
                .zip(&self.1)
            {
                match value {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone(), previous);
    for key in keys {
        std::env::set_var(key, &root);
    }
    assert_eq!(policy("a").unwrap().expected_country, None);
    // No account file and no profile exists: disabled checking performs no
    // route resolution, credential lookup, profile write or network request.
    check_for_launch(Some("not-an-account"), &root.join("missing-profile"))
        .await
        .unwrap();
    assert!(!policy_path().unwrap().exists());
    assert_eq!(
        save_policy("a", Some(" jp ".into()))
            .unwrap()
            .expected_country
            .as_deref(),
        Some("JP")
    );
    let wrapped = super::super::codex_instance::provider_gateway_bind_account_id("a").unwrap();
    assert_eq!(
        policy(&wrapped).unwrap().expected_country.as_deref(),
        Some("JP")
    );
    save_policy("b", Some("SG".into())).unwrap();
    assert!(save_policy("a", Some("XX".into())).is_err());
    assert_eq!(policy("a").unwrap().expected_country.as_deref(), Some("JP"));
    save_policy("a", None).unwrap();
    assert_eq!(policy("a").unwrap().expected_country, None);
    assert_eq!(policy("b").unwrap().expected_country.as_deref(), Some("SG"));
    assert!(!root.join("missing-profile").exists());
}

#[test]
fn country_check_history_is_account_scoped_bounded_and_invalidated_by_network_changes() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let root = std::env::temp_dir().join(format!("country-evidence-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("home")).unwrap();
    let keys = ["COCKPIT_TOOLS_TEST_DATA_DIR", "COCKPIT_TOOLS_DATA_DIR"];
    let previous = keys.map(std::env::var_os);
    struct Cleanup(PathBuf, [Option<std::ffi::OsString>; 2]);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            for (key, value) in ["COCKPIT_TOOLS_TEST_DATA_DIR", "COCKPIT_TOOLS_DATA_DIR"]
                .into_iter()
                .zip(&self.1)
            {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone(), previous);
    for key in keys {
        std::env::set_var(key, root.join("app"));
    }
    let home = root.join("home");
    let account = format!("evidence-{}", uuid::Uuid::new_v4());
    let signature = configuration_signature(&account, &home).unwrap();
    for index in 0..15 {
        record_check(
            None,
            &CountryCheck {
                check_id: format!("{index}"),
                account_id: account.clone(),
                scope: "launch".into(),
                expected_country: Some("JP".into()),
                actual_country: Some("SG".into()),
                status: "blocked".into(),
                checked_at: index,
                elapsed_ms: 12,
                source: "account".into(),
                reason: Some("PROXY_COUNTRY_MISMATCH:JP:SG".into()),
                stale: false,
            },
            &home,
            &signature,
        );
    }
    let items = recent_checks(&account).unwrap();
    assert_eq!(items.len(), 12);
    assert!(!items.last().unwrap().stale);
    assert!(recent_checks("different-account").unwrap().is_empty());
    std::fs::write(
        home.join(".env"),
        "HTTPS_PROXY=http://changed.invalid:7890\n",
    )
    .unwrap();
    assert!(recent_checks(&account).unwrap().last().unwrap().stale);
    // Dropping a timed-out/cancelled probe records a closed failure state.
    {
        let mut result = items.last().unwrap().clone();
        result.check_id = "cancelled-check".into();
        result.status = "checking".into();
        let _cancelled = CountryCheckGuard {
            app: None,
            result,
            home: home.clone(),
            signature: signature.clone(),
            started: std::time::Instant::now(),
            completed: false,
        };
    }
    assert_eq!(
        recent_checks(&account).unwrap().last().unwrap().status,
        "blocked"
    );
    invalidate_report(&account);
    assert!(recent_checks(&account).unwrap().is_empty());
}

#[test]
fn managed_country_probe_uses_the_same_https_route_and_bypass_as_launch() {
    let proxy = CodexLaunchProxy::Managed(41001);
    let (url, bypass, source) = route_for_check("fixture", Path::new("unused"), &proxy).unwrap();
    let overrides = proxy.dotenv_overrides().unwrap();
    let https = overrides
        .iter()
        .find(|(key, _)| *key == "HTTPS_PROXY")
        .unwrap();
    let no_proxy = overrides
        .iter()
        .find(|(key, _)| *key == "NO_PROXY")
        .unwrap();
    assert_eq!(url.as_deref(), Some(https.1.as_str()));
    assert_eq!(bypass.as_deref(), Some(no_proxy.1.as_str()));
    assert_eq!(source, "account");
}

// Protect the entry-point ordering as well as the probe's isolated behavior.
#[test]
fn country_gate_precedes_switch_mutations_and_cannot_be_disabled_by_autolaunch() {
    let source = include_str!("../commands/codex_account_commands.rs").replace("\r\n", "\n");
    let body = source
        .split_once("async fn switch_codex_account_internal(")
        .unwrap()
        .1
        .split_once("\n}\n")
        .unwrap()
        .0;
    let gate = body
        .find("codex_proxy_launch_guard::check_launch(")
        .unwrap();
    for later in [
        "if launch_after_switch",
        "disable_default_model_routing_for_non_oauth_switch().await",
        "read_current_codex_launch_credential_snapshot()",
        "codex_account::switch_account_managed_after_reauth_with_before_commit(",
        "codex_account::switch_account_managed_with_before_commit_and_revalidation_options(",
    ] {
        assert!(
            gate < body.find(later).unwrap(),
            "country check must precede {later}"
        );
    }
    let guarded = &body[gate..body
        .find("disable_default_model_routing_for_non_oauth_switch().await")
        .unwrap()];
    assert!(
        guarded.contains("return Err(error)"),
        "failed checks must stop the switch"
    );

    let source = include_str!("../commands/codex_instance.rs").replace("\r\n", "\n");
    let body = source
        .split_once("async fn codex_start_instance_internal(")
        .unwrap()
        .1
        .split_once("\n}\n")
        .unwrap()
        .0;
    let gate = body
        .find("preflight_desktop_proxy(Some(&app)).await?")
        .unwrap();
    assert!(body.find("start_flow_lock.lock().await").unwrap() < gate);
    assert!(
        gate < body
            .find("prepare_account_for_instance_launch_preflight(")
            .unwrap()
    );
    assert!(gate < body.find("skip_failed_step ==").unwrap());
}
