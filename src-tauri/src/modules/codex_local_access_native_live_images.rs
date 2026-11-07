//! Explicit live acceptance using selected saved accounts and the current host.
//! Credentials stay in the normal account store; responses/history use a private
//! directory. Normal refresh retains the existing cross-process transaction.
use super::*;
use base64::Engine as _;
use native_contract::Host;
use serde::Deserialize;

#[derive(Deserialize)]
struct Config {
    source_data: PathBuf,
    accounts: Vec<String>,
    output: PathBuf,
    #[serde(default)]
    phase: String,
    #[serde(default)]
    cases: Vec<Value>,
    #[serde(default)]
    models: Vec<String>,
}

fn safe_error(body: &Value, secrets: &[String]) -> Value {
    let mut error = body
        .get("error")
        .cloned()
        .unwrap_or_else(|| json!({"code":"invalid_image_response"}));
    let mut text = error.to_string();
    for secret in secrets.iter().filter(|value| !value.is_empty()) {
        text = text.replace(secret, "[redacted]");
    }
    error = serde_json::from_str(&text).unwrap_or(Value::Null);
    error
}

async fn run_case(
    client: &reqwest::Client,
    base: &str,
    key: &str,
    path: &str,
    request: Value,
    name: &str,
    output: &std::path::Path,
    secrets: &[String],
) -> Result<(Value, Option<Vec<u8>>), Box<dyn std::error::Error>> {
    let start = Instant::now();
    let outgoing = client
        .post(format!("{base}{path}"))
        .bearer_auth(key)
        .header(
            "x-agtools-disable-image-generation",
            if request["tools"]
                .as_array()
                .is_some_and(|tools| tools.iter().any(|tool| tool["type"] == "image_generation"))
            {
                "false"
            } else {
                "true"
            },
        );
    let outgoing = if name.ends_with("-multipart") {
        let encoded = request["image"]
            .as_str()
            .and_then(|value| value.split_once(";base64,"))
            .ok_or("Missing live edit image")?
            .1;
        let mut form = reqwest::multipart::Form::new().part(
            "image",
            reqwest::multipart::Part::bytes(
                base64::engine::general_purpose::STANDARD.decode(encoded)?,
            )
            .file_name("input.png")
            .mime_str("image/png")?,
        );
        for (key, value) in request.as_object().unwrap() {
            if key != "image" {
                form = form.text(
                    key.clone(),
                    value
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value.to_string()),
                );
            }
        }
        outgoing.multipart(form)
    } else {
        outgoing.json(&request)
    };
    let response = outgoing.send().await;
    let mut image = None;
    let evidence = match response {
        Err(error) => {
            json!({"name":name,"success":false,"transport_error":error.is_connect(),"timeout":error.is_timeout()})
        }
        Ok(response) => {
            let status = response.status().as_u16();
            let bytes = response.bytes().await?;
            let body = serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null);
            let encoded = body["data"][0]["b64_json"].as_str().or_else(|| {
                body["output"]
                    .as_array()?
                    .iter()
                    .find(|item| item["type"] == "image_generation_call")?["result"]
                    .as_str()
            });
            if let Some(encoded) = encoded {
                let decoded = base64::engine::general_purpose::STANDARD.decode(encoded)?;
                std::fs::write(output.join(format!("{name}.png")), &decoded)?;
                image = Some(decoded);
            }
            let success = status == 200
                && if path.contains("/images/") {
                    image.is_some()
                } else {
                    body["status"] == "completed"
                };
            let mut evidence = json!({"name":name,"status":status,"success":success,"requested_model":request["model"],"returned_model":body["model"],"usage":body["usage"],"image_bytes":image.as_ref().map(Vec::len)});
            for key in [
                "size",
                "quality",
                "aspect_ratio",
                "resolution",
                "width",
                "height",
                "output_format",
            ] {
                if let Some(value) = request.get(key) {
                    evidence[format!("requested_{key}")] = value.clone();
                }
                if let Some(value) = body.get(key) {
                    evidence[format!("returned_{key}")] = value.clone();
                }
                if let Some(value) = body["data"][0].get(key) {
                    evidence[format!("item_{key}")] = value.clone();
                }
            }
            if let Some(bytes) = &image {
                if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.len() >= 24 {
                    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
                    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
                    evidence["actual_size"] = json!(format!("{width}x{height}"));
                }
            }
            if !success {
                evidence["error"] = safe_error(&body, secrets);
            }
            if path == "/responses" && status == 200 {
                evidence["instructions_preserved"] =
                    json!(body["instructions"] == request["instructions"]);
                evidence["response_status"] = body["status"].clone();
            }
            evidence
        }
    };
    let mut evidence = evidence;
    evidence["elapsed_ms"] = json!(start.elapsed().as_millis());
    std::fs::write(
        output.join(format!("{name}.json")),
        serde_json::to_vec_pretty(&evidence)?,
    )?;
    println!("LIVE_IMAGE_CASE {evidence}");
    Ok((evidence, image))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "Explicit COCKPIT_IMAGE_LIVE_CONFIG; consumes the selected real accounts' quota"]
async fn native_image_live_acceptance() -> Result<(), Box<dyn std::error::Error>> {
    let config: Config = serde_json::from_slice(&std::fs::read(
        std::env::var_os("COCKPIT_IMAGE_LIVE_CONFIG")
            .ok_or("Missing explicit live configuration")?,
    )?)?;
    if crate::modules::account::get_data_dir()?.canonicalize()?
        != config.source_data.canonicalize()?
    {
        return Err("Live credential directory mismatch".into());
    }
    let saved: CodexLocalAccessCollection =
        serde_json::from_slice(&std::fs::read(local_access_file_path()?)?)?;
    if config
        .accounts
        .iter()
        .any(|id| !saved.account_ids.contains(id))
    {
        return Err("Selected account is no longer in the saved service".into());
    }
    std::fs::create_dir_all(&config.output)?;
    let mut results = Vec::new();
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(240))
        .build()?;
    for id in &config.accounts {
        let account: CodexAccount =
            crate::modules::secure_account_storage::read_account_file_readonly(
                &config
                    .source_data
                    .join("codex_accounts")
                    .join(format!("{id}.json")),
                &config.source_data.join("secure-account-storage.key"),
            )?;
        let label = account
            .plan_type
            .as_deref()
            .unwrap_or("unknown")
            .to_ascii_lowercase();
        let mut secrets = vec![
            account.tokens.access_token.clone(),
            account.tokens.id_token.clone(),
            account.tokens.refresh_token.clone().unwrap_or_default(),
            saved.api_key.clone(),
        ];
        secrets.extend(saved.api_keys.iter().map(|key| key.key.clone()));
        let mut any_26 = false;
        let mut edited = false;
        let models = if config.models.is_empty() {
            vec![
                "gpt-image-2.6-flare".into(),
                "gpt-image-2.6-sunburst".into(),
                "gpt-image-2.5".into(),
            ]
        } else {
            config.models.clone()
        };
        for (index, model) in models.iter().enumerate() {
            if config.phase == "geometry" && config.models.is_empty() && index != 0 {
                continue;
            }
            if config.phase == "edits" && index != 0 {
                continue;
            }
            if index == 2 && any_26 {
                continue;
            }
            let name = format!("{label}-{}", model.trim_start_matches("gpt-image-"));
            let mut collection = saved.clone();
            collection.api_key = format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            );
            collection.api_keys.clear();
            collection.account_ids = vec![id.clone()];
            collection.image_generation_account_ids = vec![id.clone()];
            // Equivalent to choosing this model in settings, in memory only.
            collection.image_generation_model = model.clone();
            if let Some(main) = config
                .cases
                .iter()
                .find(|case| case["path"] == "/responses")
                .and_then(|case| case["request"]["model"].as_str())
            {
                collection.image_generation_main_model = Some(main.into());
            }
            sanitize_image_model_catalog(&mut collection);
            collection.debug_logs = false;
            collection.request_payload_logging = false;
            let key = collection.api_key.clone();
            let host = Arc::new(NativeHostBridge::for_collection(collection));
            let identity = config.source_data.join("codex_api_account_identity.sqlite");
            if identity.exists() {
                let source = rusqlite::Connection::open_with_flags(
                    identity,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
                )?;
                source.execute(
                    "VACUUM INTO ?1",
                    [host.identity_fixture.path().to_string_lossy().as_ref()],
                )?;
            }
            let caller = host.authorize(&key).await?;
            let route = host
                .image_route(
                    &caller,
                    native_contract::ImageRouteRequest {
                        model: Some(model.clone()),
                        responses_model: None,
                        edit: false,
                        target_account_id: None,
                        force_refresh: false,
                        preferred_account_id: None,
                        excluded_account_ids: vec![],
                    },
                )
                .await?;
            println!(
                "LIVE_IMAGE_ROUTE {}",
                json!({"plan":label,"native_oauth":route.route.chatgpt,"siwc":route.route.chatgpt_plan,"uses_responses_main":route.main_model.is_some(),"base":route.route.base_url,"model":route.image_model})
            );
            drop(route);
            let mut handle = native_api::start(
                "127.0.0.1:0".parse()?,
                host,
                config.output.join(format!("{name}-history.sqlite")),
                native_api::Options::default(),
            )
            .await?;
            let base = format!("http://{}/v1", handle.addr);
            if config.phase == "geometry" {
                for case in &config.cases {
                    let mut request = case["request"].clone();
                    let path = case["path"].as_str().unwrap_or("/images/generations");
                    if path != "/responses" {
                        request["model"] = json!(model);
                    }
                    let name = format!(
                        "{label}-{}",
                        case["name"].as_str().ok_or("Missing geometry case name")?
                    );
                    let (evidence, _) = run_case(
                        &client,
                        &base,
                        &key,
                        path,
                        request,
                        &name,
                        &config.output,
                        &secrets,
                    )
                    .await?;
                    results.push(evidence);
                }
                handle.shutdown(Duration::from_secs(10)).await?;
                continue;
            }
            let image = if config.phase == "edits" {
                Some(std::fs::read(config.output.join(format!("{name}.png")))?)
            } else {
                let (evidence,image) = run_case(&client,&base,&key,"/images/generations",json!({"model":model,"prompt":"A single solid blue circle centered on a plain white background, flat geometric icon, no text.","n":1,"size":"1024x1024","quality":"low","output_format":"png","response_format":"b64_json"}),&name,&config.output,&secrets).await?;
                if index < 2 {
                    any_26 |= evidence["success"] == true;
                }
                results.push(evidence);
                image
            };
            if index < 2 && !edited {
                if let Some(image) = image {
                    let encoded = base64::engine::general_purpose::STANDARD.encode(image);
                    let edit_name = if label == "plus" {
                        format!("{label}-26-edit-multipart")
                    } else {
                        format!("{label}-26-edit")
                    };
                    let (evidence,_) = run_case(&client,&base,&key,"/images/edits",json!({"model":model,"prompt":"Keep the blue circle and add one small red square in the bottom right corner.","image":format!("data:image/png;base64,{encoded}"),"n":1,"size":"1024x1024","quality":"low","complexity":"medium","response_format":"b64_json"}),&edit_name,&config.output,&secrets).await?;
                    results.push(evidence);
                    edited = true;
                }
            }
            if index == 0 && config.phase != "edits" {
                let (evidence,_) = run_case(&client,&base,&key,"/responses",json!({"model":"gpt-6.1-sol","instructions":"","input":"Reply with exactly OK.","reasoning":{"effort":"low"},"max_output_tokens":128,"stream":false}),&format!("{label}-text"),&config.output,&secrets).await?;
                results.push(evidence);
            }
            handle.shutdown(Duration::from_secs(10)).await?;
        }
    }
    std::fs::write(
        config.output.join("summary.json"),
        serde_json::to_vec_pretty(&results)?,
    )?;
    if results.iter().any(|case| case["success"] != true) {
        return Err("Some real upstream cases failed; inspect the private evidence".into());
    }
    Ok(())
}
