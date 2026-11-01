use super::*;
use crate::modules::codex_proxy_catalog_binding as binding;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Fixture {
    root: PathBuf,
    old: Option<std::ffi::OsString>,
    old_data: Option<std::ffi::OsString>,
}
impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("native-profile-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("resources")).unwrap();
        let old = std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR");
        let old_data = std::env::var_os("COCKPIT_TOOLS_DATA_DIR");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", root.join("data"));
        std::env::set_var("COCKPIT_TOOLS_DATA_DIR", root.join("data"));
        Self {
            root,
            old,
            old_data,
        }
    }
    fn resources(&self) -> PathBuf {
        self.root.join("resources")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(old) = &self.old {
            std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", old);
        } else {
            std::env::remove_var("COCKPIT_TOOLS_TEST_DATA_DIR");
        }
        if let Some(old) = &self.old_data {
            std::env::set_var("COCKPIT_TOOLS_DATA_DIR", old);
        } else {
            std::env::remove_var("COCKPIT_TOOLS_DATA_DIR");
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn binary() -> PathBuf {
    std::env::var_os("COCKPIT_TEST_MIHOMO")
        .expect("verified test engine required")
        .into()
}
fn command(binary: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(binary);
    command.kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command
}
fn profile(config: Value) -> NativeProfile {
    NativeProfile {
        snapshot_id: String::new(),
        config,
        files: Default::default(),
        resource_directory: String::new(),
        rules_name: "PROFILE".into(),
        names: Default::default(),
        members: Default::default(),
    }
}
async fn headers(socket: &mut tokio::net::TcpStream) -> String {
    let mut data = Vec::new();
    while !data.ends_with(b"\r\n\r\n") {
        data.push(socket.read_u8().await.unwrap());
        assert!(data.len() < 8192);
    }
    String::from_utf8(data).unwrap()
}

#[test]
fn native_configuration_preserves_rules_dns_options_and_isolates_ingress() {
    let raw = json!({"geo-auto-update":true,"mixed-port":7890,"port":7891,"allow-lan":true,"listeners":[{"type":"socks","port":8888}],
        "external-controller":"0.0.0.0:9090","tun":{"enable":true},"ntp":{"enable":true,"write-to-system":true},
        "dns":{"enable":true,"listen":"0.0.0.0:53","nameserver":["system"]},"hosts":{"synthetic.test":"127.0.0.1"},
        "proxies":[{"name":"account-node","type":"http","server":"localhost","port":9}],
        "proxy-providers":{"A":{"type":"http","url":"https://example.invalid/sub","age-secret-key":"synthetic","override":{"override-expr":[".udp = true"]}}},
        "rules":["DOMAIN,synthetic.test,DIRECT","MATCH,account-node"]});
    let profile = profile(raw.clone());
    let config =
        configuration(&profile, &json!({"rules":true}), 12345, "user", "password").unwrap();
    assert_eq!(config["rules"], raw["rules"]);
    assert_eq!(config["hosts"], raw["hosts"]);
    assert_eq!(config["geo-auto-update"], true);
    assert_eq!(config["proxy-providers"], raw["proxy-providers"]);
    assert_eq!(config["mixed-port"], 12345);
    assert_eq!(config["allow-lan"], false);
    assert!(config.get("port").is_none() && config.get("listeners").is_none());
    assert_eq!(config["dns"]["nameserver"], json!(["system"]));
    assert!(config["dns"].get("listen").is_none());
    assert_eq!(config["tun"]["enable"], false);
    assert_eq!(config["ntp"]["write-to-system"], false);
    let selected =
        configuration(&profile, &json!({"name":"account-node"}), 12345, "u", "p").unwrap();
    assert_eq!(selected["proxy-groups"][0]["name"], "account-node_");
    assert_eq!(selected["rules"], json!(["MATCH,account-node_"]));
}

#[test]
fn resource_relocation_never_mistakes_transport_paths_or_encoded_keys_for_files() {
    let mut config = json!({"proxies":[{"name":"V","type":"vless","ws-opts":{"path":"/ws"}},
        {"name":"WG","type":"wireguard","private-key":"/encoded-key"}],
        "proxy-providers":{"A":{"type":"file","path":"/resources/local.yaml"}}});
    relocate_files(&mut config, Some(Path::new("/resources"))).unwrap();
    assert_eq!(config["proxies"][0]["ws-opts"]["path"], "/ws");
    assert_eq!(config["proxies"][1]["private-key"], "/encoded-key");
    assert_eq!(config["proxy-providers"]["A"]["path"], "local.yaml");
    assert!(!assets::relative("../private"));
    assert!(!assets::relative("/private"));
}

#[test]
fn verge_default_repairs_are_applied_without_replacing_native_provider_semantics() {
    let config = document("Mode: script\nProxies:\n - {name: HY, type: hysteria, alpn: h3}\nproxy-providers:\n P: {type: inline, payload: []}\nproxy-groups:\n - {name: A, type: select, proxies: [Missing, HY, DIRECT]}\n - {name: B, type: select, use: [Gone, P], proxies: [ProviderNode]}\n").unwrap().unwrap();
    let config = verge::normalize(config);
    assert_eq!(config["mode"], "rule");
    assert_eq!(config["proxies"][0]["alpn"], json!(["h3"]));
    assert_eq!(
        config["proxy-groups"][0]["proxies"],
        json!(["HY", "DIRECT"])
    );
    assert_eq!(config["proxy-groups"][1]["use"], json!(["P"]));
    assert_eq!(
        config["proxy-groups"][1]["proxies"],
        json!(["ProviderNode"])
    );
}

#[test]
fn native_snapshot_references_are_small_and_resource_reads_are_not_part_of_validation() {
    let raw =
        json!({"type":KIND,"profileId":"a".repeat(64),"target":{"name":"Node"},"selections":{}});
    assert!(valid_reference(&raw));
    assert!(!valid_reference(
        &json!({"type":KIND,"profileId":"../../secret","target":{},"selections":{}})
    ));
    assert!(
        document("proxy-providers:\n  local: {type: file, path: proxies.yaml}")
            .unwrap()
            .is_some()
    );
    assert!(document("trojan://synthetic@localhost:9")
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn missing_file_directory_is_reported_before_starting_engine() {
    let error = prepare(
        json!({"proxy-providers":{"Local":{"type":"file","path":"local.yaml"}}}),
        String::new(),
        "Rules".into(),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(error, "PROXY_PROFILE_DIRECTORY");
    let inline_fallback = prepare(json!({"proxy-providers":{"Local":{"type":"file","path":"optional.yaml","payload":[{"name":"Node","type":"http","server":"localhost","port":9}]}}}),String::new(),"Rules".into()).await;
    assert!(inline_fallback.is_ok());
}

#[tokio::test]
#[ignore = "verified Mihomo; synthetic encrypted local provider and localhost only"]
async fn native_engine_encrypted_files_expressions_and_snapshot_recovery() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    let binary = binary();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        assert!(headers(&mut socket)
            .await
            .starts_with("CONNECT synthetic.invalid:80 "));
        socket.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
        assert!(headers(&mut socket).await.starts_with("GET / "));
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nnative")
            .await
            .unwrap();
    });
    let keys = command(&binary)
        .args(["age", "keygen"])
        .output()
        .await
        .unwrap();
    assert!(keys.status.success());
    let keys = String::from_utf8(keys.stdout).unwrap();
    let secret = keys
        .lines()
        .find(|line| line.starts_with("AGE-SECRET-KEY-"))
        .unwrap();
    let public = keys
        .lines()
        .find_map(|line| line.strip_prefix("# public key: "))
        .unwrap();
    let plaintext =
        json!({"proxies":[{"name":"Node","type":"http","server":"127.0.0.1","port":port}]})
            .to_string();
    let mut encrypt = command(&binary)
        .args(["age", "encrypt", public, "-", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    encrypt
        .stdin
        .take()
        .unwrap()
        .write_all(plaintext.as_bytes())
        .await
        .unwrap();
    let encrypted = encrypt.wait_with_output().await.unwrap();
    assert!(encrypted.status.success());
    std::fs::write(f.resources().join("nodes.age"), &encrypted.stdout).unwrap();
    let config = json!({"rule-providers":{"Unrelated":{"type":"http","url":"http://127.0.0.1:9/unreachable","behavior":"domain"}},"proxy-providers":{"Local":{"type":"file","path":"nodes.age","age-secret-key":secret,
        "override":{"override-expr":[".name = \"native-\" + .name"]},"filter":"^(?=Node)Node$"}},
        "proxy-groups":[{"name":"Choose","type":"select","use":["Local"]}],"rules":["MATCH,Choose"]});
    let catalog = inspect_with_binary(
        &binary,
        config,
        f.resources().to_string_lossy().into_owned(),
        "Rules".into(),
    )
    .await
    .unwrap();
    let node = catalog
        .nodes
        .iter()
        .find(|n| n.name == "native-Node")
        .unwrap();
    let encoded = binding::encode("s", "Source", &node.id, &catalog, &BTreeMap::new()).unwrap();
    assert!(encoded.len() < 4096);
    assert!(!encoded.contains(secret));
    std::fs::remove_file(f.resources().join("nodes.age")).unwrap();
    let tunnel =
        engine::start_native_inspection(&binary, json!(binding::outbounds(&encoded).unwrap()))
            .await
            .unwrap();
    let client = reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(tunnel.proxy_url()).unwrap())
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let response = client
        .get("http://synthetic.invalid/")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    tunnel.stop().await.unwrap();
    assert_eq!(response, "native");
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
    let persisted = std::fs::read_to_string(f.root.join("data/codex-proxy-assets").join(format!(
        "{}.json",
        catalog.native.as_ref().unwrap().snapshot_id
    )))
    .unwrap();
    assert!(!persisted.contains(secret) && !persisted.contains("native-Node"));
}

#[tokio::test]
#[ignore = "verified Mihomo; synthetic rules, hosts and localhost only"]
async fn native_engine_subscription_rules_use_native_hosts_and_direct_routes() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let f = Fixture::new();
    std::fs::write(
        f.resources().join("rules.yaml"),
        b"payload:\n - native.test\n",
    )
    .unwrap();
    let binary = binary();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        assert!(headers(&mut socket).await.starts_with("GET / "));
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nrules")
            .await
            .unwrap();
    });
    let config = json!({"proxies":[],"hosts":{"native.test":"127.0.0.1"},"rule-providers":{"Local":{"type":"file","behavior":"domain","path":"rules.yaml"}},"rules":["RULE-SET,Local,DIRECT","MATCH,REJECT"]});
    let catalog = inspect_with_binary(
        &binary,
        config,
        f.resources().to_string_lossy().into(),
        "Rules".into(),
    )
    .await
    .unwrap();
    let item = catalog
        .nodes
        .iter()
        .find(|n| n.protocol == "profile")
        .unwrap();
    let raw = binding::encode("s", "s", &item.id, &catalog, &BTreeMap::new()).unwrap();
    let tunnel = engine::start_native_inspection(&binary, json!(binding::outbounds(&raw).unwrap()))
        .await
        .unwrap();
    let rules = engine::native_request(&tunnel.controller(), "providers/rules", None)
        .await
        .unwrap();
    assert_eq!(rules["providers"]["Local"]["ruleCount"], 1);
    let client = reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(tunnel.proxy_url()).unwrap())
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let text = client
        .get(format!("http://native.test:{port}/"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    tunnel.stop().await.unwrap();
    assert_eq!(text, "rules");
    server.await.unwrap();
}

#[tokio::test]
#[ignore = "verified Mihomo; synthetic provider downloaded through localhost proxy"]
async fn native_engine_download_proxy_headers_and_native_filter_are_preserved() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _f = Fixture::new();
    let binary = binary();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        assert!(headers(&mut socket)
            .await
            .starts_with("CONNECT provider.invalid:80 "));
        socket.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
        let request = headers(&mut socket).await;
        assert!(request.starts_with("GET /subscription "));
        assert!(request
            .to_ascii_lowercase()
            .contains("x-sub-token: synthetic"));
        tokio::time::sleep(Duration::from_millis(350)).await;
        let body = "proxies:\n - {name: SG-1, type: http, server: 127.0.0.1, port: 9}\n - {name: US-1, type: http, server: 127.0.0.1, port: 9}\n";
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
    let config = json!({"proxies":[{"name":"Download","type":"http","server":"127.0.0.1","port":port}],
        "proxy-providers":{"Remote":{"type":"http","url":"http://provider.invalid/subscription","proxy":"Download","interval":300,
            "payload":[{"name":"Fallback","type":"http","server":"127.0.0.1","port":9}],
            "header":{"X-Sub-Token":["synthetic"]},"filter":"^(?!US).*", "override":{"override-expr":[".name |= sub(\"^SG-\"; \"native-\")"]}}},
        "proxy-groups":[{"name":"Auto","type":"fallback","use":["Remote"]}],"rules":["MATCH,Auto"]});
    let catalog = inspect_with_binary(&binary, config, String::new(), "Rules".into())
        .await
        .unwrap();
    assert!(catalog.nodes.iter().any(|node| node.name == "native-1"));
    assert!(!catalog.nodes.iter().any(|node| node.name == "US-1"));
    let profile = catalog.native.unwrap();
    assert_eq!(
        profile.config["proxy-providers"]["Remote"]["proxy"],
        "Download"
    );
    assert_eq!(profile.config["proxy-providers"]["Remote"]["interval"], 300);
    assert!(profile
        .files
        .contains_key("cockpit-proxy-providers-0.cache"));
    server.await.unwrap();
}

#[tokio::test]
#[ignore = "verified Mihomo; cancellation of stalled synthetic localhost provider"]
async fn native_engine_cancelled_import_closes_provider_connection() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _f = Fixture::new();
    let binary = binary();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (accepted, signal) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let _ = headers(&mut socket).await;
        let _ = accepted.send(());
        let mut byte = [0];
        tokio::time::timeout(Duration::from_secs(4), socket.read(&mut byte))
            .await
            .unwrap()
            .unwrap()
    });
    let config = json!({"proxy-providers":{"Slow":{"type":"http","url":format!("http://{address}/slow")}},
        "proxy-groups":[{"name":"Choose","type":"select","use":["Slow"]}],"rules":["MATCH,Choose"]});
    let mut import = Box::pin(inspect_with_binary(
        &binary,
        config,
        String::new(),
        "Rules".into(),
    ));
    tokio::select! { _ = &mut import => panic!("stalled provider unexpectedly completed"), result = signal => result.unwrap() }
    drop(import);
    assert_eq!(server.await.unwrap(), 0);
}

#[tokio::test]
#[ignore = "verified Mihomo; localhost provider fallback and failed refresh"]
async fn native_engine_provider_payload_waits_for_failure_and_refresh_rejects_it() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _f = Fixture::new();
    let binary = binary();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            headers(&mut socket).await;
            socket
                .write_all(
                    b"HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        }
    });
    let config = json!({"proxy-providers":{"Remote":{"type":"http","proxy":"DIRECT","url":format!("http://{address}/sub"),
        "payload":[{"name":"Fallback","type":"http","server":"127.0.0.1","port":9}]}},
        "proxy-groups":[{"name":"Choose","type":"select","use":["Remote"]}],"rules":["MATCH,Choose"]});
    let catalog = inspect_with_binary_mode(
        &binary,
        config.clone(),
        String::new(),
        "Rules".into(),
        false,
    )
    .await
    .unwrap();
    assert!(catalog.nodes.iter().any(|n| n.name == "Fallback"));
    let error = inspect_with_binary_mode(&binary, config, String::new(), "Rules".into(), true)
        .await
        .err()
        .expect("refresh must preserve old source");
    assert_eq!(error, "CATALOG_DOWNLOAD");
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
}

#[test]
fn yaml_merge_aliases_and_base64_wrappers_reach_the_native_decoder() {
    use base64::Engine;
    let input = "template: &proxy {type: http, server: 127.0.0.1, port: 9}\nproxies:\n - {<<: *proxy, name: Merged}\n";
    let plain = document(input).unwrap().unwrap();
    assert_eq!(plain["proxies"][0]["server"], "127.0.0.1");
    assert!(plain["proxies"][0].get("<<").is_none());
    let encoded = base64::engine::general_purpose::STANDARD.encode(input);
    assert_eq!(document(&encoded).unwrap().unwrap(), plain);
}

#[tokio::test]
#[ignore = "verified Mihomo; restored group choices and composite native bridges"]
async fn native_engine_global_choices_and_composite_strategy_keep_native_routing() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _f = Fixture::new();
    let binary = binary();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            headers(&mut socket).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
        }
    });
    let config = json!({"proxies":[],"hosts":{"native.test":"127.0.0.1"},
        "proxy-groups":[{"name":"Choose","type":"select","proxies":["REJECT","DIRECT"]}],"rules":["MATCH,Choose"]});
    let catalog = inspect_with_binary(&binary, config, String::new(), "Rules".into())
        .await
        .unwrap();
    let global = catalog.groups.iter().find(|g| g.name == "GLOBAL").unwrap();
    let choose = catalog.groups.iter().find(|g| g.name == "Choose").unwrap();
    let selections = BTreeMap::from([
        (global.id.clone(), "Choose".into()),
        (choose.id.clone(), "DIRECT".into()),
    ]);
    let global_binding = binding::encode("s", "s", &global.id, &catalog, &selections).unwrap();
    let rules = catalog
        .nodes
        .iter()
        .find(|n| n.protocol == "profile")
        .unwrap();
    let rules_binding = binding::encode("s", "s", &rules.id, &catalog, &selections).unwrap();
    let mut reference = binding::outbounds(&rules_binding).unwrap().remove(0);
    reference["tag"] = json!("native-child");
    let composite = json!([{"type":"mihomo-group","tag":"account-node","group":{"type":"select","name":"account-node","proxies":["native-child"]}},reference]);
    for outbound in [
        json!(binding::outbounds(&global_binding).unwrap()),
        composite,
    ] {
        let tunnel = engine::start_native_inspection(&binary, outbound)
            .await
            .unwrap();
        let client = reqwest::Client::builder()
            .no_proxy()
            .proxy(reqwest::Proxy::all(tunnel.proxy_url()).unwrap())
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let response = client
            .get(format!("http://native.test:{port}/"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(response, "ok");
        tunnel.stop().await.unwrap();
    }
    server.await.unwrap();
}

#[test]
fn forced_nodes_skip_routing_only_downloads_but_keep_dns_rule_sets() {
    let raw = json!({"proxies":[],"rule-providers":{"Route":{"type":"http"},"DNS":{"type":"http"},"IPs":{"type":"http"},"FakeIP":{"type":"http"}},
        "dns":{"nameserver-policy":{"rule-set:DNS":["system"]},"fallback-filter":{"ipcidr":["rule-set:IPs"]},"fake-ip-filter":["RULE-SET,FakeIP,real-ip"]},
        "rules":["RULE-SET,Route,REJECT","MATCH,DIRECT"],"sub-rules":{"extra":["RULE-SET,Route,DIRECT"]}});
    let profile = profile(raw.clone());
    for target in [
        json!({"name":"DIRECT"}),
        json!({"name":"GLOBAL"}),
        json!({"rules":true,"inventory":true}),
    ] {
        let config = configuration(&profile, &target, 12345, "u", "p").unwrap();
        assert!(config["rule-providers"].get("Route").is_none());
        for name in ["DNS", "IPs", "FakeIP"] {
            assert!(config["rule-providers"].get(name).is_some());
        }
        assert!(config.get("sub-rules").is_none());
    }
    let rules = configuration(&profile, &json!({"rules":true}), 12345, "u", "p").unwrap();
    assert_eq!(rules["rule-providers"], raw["rule-providers"]);
    assert_eq!(rules["rules"], raw["rules"]);
    assert_eq!(rules["sub-rules"], raw["sub-rules"]);
}

#[test]
fn inventory_preserves_implicit_provider_download_routing() {
    let raw = json!({"proxies":[],"proxy-providers":{"Remote":{"type":"http","url":"https://example.invalid/nodes"}},
        "rule-providers":{"Bootstrap":{"type":"http"}},"rules":["RULE-SET,Bootstrap,DIRECT","MATCH,REJECT"]});
    let config = configuration(
        &profile(raw.clone()),
        &json!({"rules":true,"inventory":true}),
        12345,
        "u",
        "p",
    )
    .unwrap();
    assert_eq!(config["rules"], raw["rules"]);
    assert_eq!(config["rule-providers"], raw["rule-providers"]);
}
