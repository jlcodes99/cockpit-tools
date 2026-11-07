//! Opt-in acceptance against an explicitly authorized, already imported source.
//! Original storage is read-only; all native assets and processes are isolated.
use super::*;
use crate::modules::codex_proxy_engine as engine;
use serde_json::{json, Value};
use std::time::Instant;

fn display_name(value: &str) -> String {
    if value.contains("://") || value.contains("token=") || value.contains("password=") {
        return "[redacted label]".into();
    }
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(120)
        .collect()
}

async fn check(client: &reqwest::Client, url: &str, trace: bool) -> Value {
    let started = Instant::now();
    match client.get(url).send().await {
        Ok(mut response) => {
            let status = response.status().as_u16();
            let mut result = json!({"status":status,"elapsedMs":started.elapsed().as_millis()});
            if trace && status == 200 {
                let mut body = Vec::new();
                while let Ok(Some(chunk)) = response.chunk().await {
                    if body.len() + chunk.len() > 16384 {
                        break;
                    }
                    body.extend_from_slice(&chunk);
                }
                if let Some(country) = String::from_utf8_lossy(&body)
                    .lines()
                    .find_map(|line| line.strip_prefix("loc="))
                {
                    if country.len() == 2 && country.bytes().all(|b| b.is_ascii_uppercase()) {
                        result["country"] = json!(country);
                    }
                }
            }
            result
        }
        Err(error) => {
            json!({"error":if error.is_timeout(){"TIMEOUT"}else if error.is_connect(){"CONNECT_FAILED"}else{"REQUEST_FAILED"},"elapsedMs":started.elapsed().as_millis()})
        }
    }
}

#[tokio::test]
#[ignore = "requires explicit user authorization, source directory/id and verified engine; performs token-free public requests"]
async fn live_imported_subscription_native_nodes_connect() {
    let _lock = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let original_directory = PathBuf::from(
        std::env::var_os("COCKPIT_LIVE_SOURCE_DIR").expect("explicit source directory required"),
    );
    let source_id = std::env::var("COCKPIT_LIVE_SOURCE_ID").expect("explicit source id required");
    let binary =
        PathBuf::from(std::env::var_os("COCKPIT_TEST_MIHOMO").expect("verified engine required"));
    let report_path = PathBuf::from(
        std::env::var_os("COCKPIT_LIVE_REPORT").expect("report destination required"),
    );
    let source_path = original_directory.join("codex-proxy-sources.json");
    let original_bytes = fs::read(&source_path).expect("source read failed");
    let store: Store = secure_account_storage::read_account_file_readonly(
        &source_path,
        &original_directory.join("secure-account-storage.key"),
    )
    .unwrap_or_else(|_| panic!("source decryption failed"));
    let source = store
        .sources
        .into_iter()
        .find(|s| s.id == source_id)
        .expect("source not found");
    let temp = Temp::new();
    let _isolation = DataDir::set(&temp.0);
    println!(
        "LIVE_STAGE download source={} savedNodes={}",
        display_name(&source.name),
        source.catalog.nodes.len()
    );
    let download = fetch(source.url.as_deref().expect("subscription URL required"))
        .await
        .unwrap_or_else(|code| panic!("subscription download: {code}"));
    let config = parser::native::document(&download.body)
        .unwrap()
        .expect("structured subscription expected");
    let resources = source
        .catalog
        .native
        .as_ref()
        .map(|p| p.resource_directory.clone())
        .unwrap_or_default();
    let catalog = parser::native::inspect_with_binary(&binary, config, resources, "Rules".into())
        .await
        .unwrap_or_else(|code| panic!("native import: {code}"));
    let nodes: Vec<_> = catalog
        .nodes
        .iter()
        .filter(|n| n.protocol != "profile" && !binding::is_builtin_name(&n.name))
        .collect();
    assert!(!nodes.is_empty(), "no proxy nodes returned");
    println!(
        "LIVE_STAGE parsed nodes={} groups={}",
        nodes.len(),
        catalog.groups.len()
    );
    let mut results = Vec::new();
    for node in nodes {
        let name = display_name(&node.name);
        let raw = binding::encode(
            &source.id,
            &source.name,
            &node.id,
            &catalog,
            &BTreeMap::new(),
        )
        .unwrap();
        let start =
            engine::start_native_inspection(&binary, json!(binding::outbounds(&raw).unwrap()))
                .await;
        let result = match start {
            Err(code) => json!({"name":name,"protocol":node.protocol,"startError":code}),
            Ok(tunnel) => {
                let client = reqwest::Client::builder()
                    .no_proxy()
                    .proxy(reqwest::Proxy::all(tunnel.proxy_url()).unwrap())
                    .redirect(reqwest::redirect::Policy::none())
                    .connect_timeout(Duration::from_secs(8))
                    .timeout(Duration::from_secs(15))
                    .build()
                    .unwrap();
                let (https, openai, chatgpt) = tokio::join!(
                    check(&client, "https://www.gstatic.com/generate_204", false),
                    check(&client, "https://api.openai.com/v1/models", false),
                    check(&client, "https://chatgpt.com/cdn-cgi/trace", true),
                );
                let stop = tunnel.stop().await;
                json!({"name":name,"protocol":node.protocol,"https":https,"openai":openai,"chatgpt":chatgpt,"stopped":stop.is_ok()})
            }
        };
        println!("LIVE_NODE {result}");
        results.push(result);
    }
    let unchanged = fs::read(&source_path).is_ok_and(|bytes| bytes == original_bytes);
    let report = json!({"source":display_name(&source.name),"engine":engine::ENGINE_VERSION,"checkedAt":chrono::Utc::now().to_rfc3339(),"originalSourceUnchanged":unchanged,"nodes":results});
    fs::write(report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    println!("LIVE_DONE originalSourceUnchanged={unchanged}");
}
