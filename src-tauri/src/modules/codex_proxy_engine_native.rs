use super::*;

fn child(
    binary: PathBuf,
    outbound: Value,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<NodeTunnel, String>> + Send>> {
    Box::pin(async move { start_with_binary(&binary, outbound).await })
}

pub(super) async fn dependencies(
    binary: &Path,
    mut outbound: Value,
) -> Result<(Value, Vec<NodeTunnel>), String> {
    let mut children = Vec::new();
    let Some(items) = outbound.as_array_mut().filter(|v| v.len() > 1) else {
        return Ok((outbound, children));
    };
    // Native profiles retain independent DNS, provider and routing namespaces.
    // A composite strategy reaches them through authenticated loopback bridges.
    let mut bridges = BTreeMap::<String, Value>::new();
    for item in items {
        if !native_profile::valid_reference(item) {
            continue;
        }
        let tag = item["tag"].clone();
        let mut key = item.clone();
        key.as_object_mut()
            .ok_or("PROXY_ENGINE_CONFIG_INVALID")?
            .remove("tag");
        let key = key.to_string();
        let bridge = if let Some(bridge) = bridges.get(&key) {
            bridge.clone()
        } else {
            let tunnel = child(binary.to_owned(), item.clone()).await?;
            let url =
                url::Url::parse(tunnel.proxy_url()).map_err(|_| "PROXY_ENGINE_CONFIG_INVALID")?;
            let bridge = json!({"type":"mihomo","proxy":{"type":"socks5","server":"127.0.0.1","port":url.port(),
                "username":url.username(),"password":url.password().unwrap_or_default()}});
            children.push(tunnel);
            bridges.insert(key, bridge.clone());
            bridge
        };
        *item = bridge;
        item["tag"] = tag;
    }
    Ok((outbound, children))
}

pub(crate) async fn start_native_inspection(
    binary: &Path,
    mut outbound: Value,
) -> Result<NodeTunnel, String> {
    // Provider startup has its own bounded import deadline. No listener is
    // published to account consumers while inspection is running.
    if let Some(items) = outbound.as_array_mut() {
        if let Some(item) = items.first_mut() {
            item["inspection"] = json!(true);
        }
    } else {
        outbound["inspection"] = json!(true);
    }
    start_with_binary(binary, outbound).await
}

pub(crate) async fn native_request(
    controller: &EngineController,
    path: &str,
    body: Option<Value>,
) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|_| "PROXY_STATUS_FAILED")?;
    let url = format!("{}/{path}", controller.endpoint);
    let request = if let Some(body) = body {
        client.put(url).json(&body)
    } else {
        client.get(url)
    };
    let mut response = request
        .bearer_auth(&controller.secret)
        .send()
        .await
        .map_err(|_| "PROXY_STATUS_FAILED")?;
    if !response.status().is_success() {
        return Err("PROXY_ENGINE_CONFIG_INVALID".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "PROXY_STATUS_FAILED")? {
        if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
            return Err("CATALOG_LIMIT".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        Ok(Value::Null)
    } else {
        serde_json::from_slice(&bytes).map_err(|_| "PROXY_STATUS_FAILED".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn runtime_directory_survives_cancelled_owner_until_writer_releases_it() {
        let directory = Arc::new(RuntimeDirectory::create().await.unwrap());
        let path = directory.path().to_owned();
        let worker_directory = directory.clone();
        let (release, wait) = std::sync::mpsc::channel();
        let worker = tokio::task::spawn_blocking(move || {
            wait.recv().unwrap();
            std::fs::write(worker_directory.path().join("test"), b"synthetic").unwrap();
        });
        drop(directory);
        assert!(path.is_dir());
        release.send(()).unwrap();
        worker.await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            while path.exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }
}
