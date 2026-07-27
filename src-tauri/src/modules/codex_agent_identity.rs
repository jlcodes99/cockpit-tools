use crate::models::codex::CodexAccount;
use crate::modules::{codex_account, codex_local_access, logger};
use base64::{engine::general_purpose, Engine as _};
use ed25519_dalek::{pkcs8::DecodePrivateKey, pkcs8::EncodePrivateKey, Signer, SigningKey};
use rand::{rngs::OsRng, RngCore};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;
use tokio::sync::Mutex;

pub(crate) const AGENT_IDENTITY_AUTH_API_BASE_URL: &str = "https://auth.openai.com/api/accounts";
const AGENT_IDENTITY_REGISTRATION_TIMEOUT: Duration = Duration::from_secs(15);
const AGENT_IDENTITY_TASK_REGISTRATION_TIMEOUT: Duration = Duration::from_secs(30);
const AGENT_IDENTITY_REGISTRATION_ATTEMPTS: usize = 3;
const AGENT_IDENTITY_KEY_SEED_BYTES: usize = 64;
const AGENT_IDENTITY_KEY_DERIVATION_CONTEXT: &[u8] = b"codex-agent-identity-ed25519-v1";

static AGENT_IDENTITY_TASK_LOCKS: OnceLock<StdMutex<HashMap<String, Arc<Mutex<()>>>>> =
    OnceLock::new();

#[derive(Clone)]
struct AgentIdentityKey {
    runtime_id: String,
    private_key: SigningKey,
    task_id: String,
}

#[derive(Debug, Deserialize)]
struct AgentIdentityTaskRegistrationResponse {
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default, rename = "taskId")]
    task_id_camel: Option<String>,
    #[serde(default)]
    encrypted_task_id: Option<String>,
    #[serde(default, rename = "encryptedTaskId")]
    encrypted_task_id_camel: Option<String>,
}

#[derive(Debug, Serialize)]
struct AgentIdentityBillOfMaterials {
    agent_version: String,
    agent_harness_id: String,
    running_location: String,
}

#[derive(Debug, Serialize)]
struct AgentIdentityRegistrationRequest {
    abom: AgentIdentityBillOfMaterials,
    agent_public_key: String,
    capabilities: Vec<String>,
    ttl: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct AgentIdentityRegistrationResponse {
    agent_runtime_id: String,
}

struct GeneratedAgentIdentityKeyMaterial {
    private_key_pkcs8_base64: String,
    public_key_ssh: String,
}

pub(crate) struct RegisteredAgentIdentity {
    pub agent_runtime_id: String,
    pub agent_private_key: String,
}

struct AgentIdentityRegistrationAttemptError {
    message: String,
    retryable: bool,
}

fn append_ssh_string(target: &mut Vec<u8>, value: &[u8]) -> Result<(), String> {
    let len =
        u32::try_from(value.len()).map_err(|_| "Agent Identity SSH 公钥字段过长".to_string())?;
    target.extend_from_slice(&len.to_be_bytes());
    target.extend_from_slice(value);
    Ok(())
}

fn encode_ssh_ed25519_public_key(signing_key: &SigningKey) -> Result<String, String> {
    let verifying_key = signing_key.verifying_key();
    let mut blob = Vec::with_capacity(4 + 11 + 4 + 32);
    append_ssh_string(&mut blob, b"ssh-ed25519")?;
    append_ssh_string(&mut blob, verifying_key.as_bytes())?;
    Ok(format!(
        "ssh-ed25519 {}",
        general_purpose::STANDARD.encode(blob)
    ))
}

fn generate_agent_identity_key_material() -> Result<GeneratedAgentIdentityKeyMaterial, String> {
    let mut seed_material = [0u8; AGENT_IDENTITY_KEY_SEED_BYTES];
    OsRng.fill_bytes(&mut seed_material);
    let mut digest = Sha512::new();
    digest.update(AGENT_IDENTITY_KEY_DERIVATION_CONTEXT);
    digest.update(seed_material);
    let digest = digest.finalize();
    let mut secret_key_bytes = [0u8; 32];
    secret_key_bytes.copy_from_slice(&digest[..32]);
    let signing_key = SigningKey::from_bytes(&secret_key_bytes);
    let private_key = signing_key
        .to_pkcs8_der()
        .map_err(|_| "Agent Identity 私钥编码失败".to_string())?;
    Ok(GeneratedAgentIdentityKeyMaterial {
        private_key_pkcs8_base64: general_purpose::STANDARD.encode(private_key.as_bytes()),
        public_key_ssh: encode_ssh_ed25519_public_key(&signing_key)?,
    })
}

async fn register_agent_identity_once(
    client: &reqwest::Client,
    url: &str,
    access_token: &str,
    is_fedramp_account: bool,
    key_material: &GeneratedAgentIdentityKeyMaterial,
) -> Result<String, AgentIdentityRegistrationAttemptError> {
    let request = AgentIdentityRegistrationRequest {
        abom: AgentIdentityBillOfMaterials {
            agent_version: env!("CARGO_PKG_VERSION").to_string(),
            agent_harness_id: "codex-cli".to_string(),
            running_location: format!("cli-{}", std::env::consts::OS),
        },
        agent_public_key: key_material.public_key_ssh.clone(),
        capabilities: vec!["responsesapi".to_string()],
        ttl: None,
    };
    let mut request_builder = client
        .post(url)
        .bearer_auth(access_token)
        .header("Accept", "application/json")
        .json(&request);
    if is_fedramp_account {
        request_builder = request_builder.header("X-OpenAI-Fedramp", "true");
    }
    let response =
        request_builder
            .send()
            .await
            .map_err(|error| AgentIdentityRegistrationAttemptError {
                message: format!("Agent Identity runtime 注册请求失败: {}", error),
                retryable: error.is_timeout() || error.is_connect() || error.is_request(),
            })?;
    let status = response.status();
    if !status.is_success() {
        return Err(AgentIdentityRegistrationAttemptError {
            message: format!("Agent Identity runtime 注册返回 HTTP {}", status),
            retryable: status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error(),
        });
    }
    let response_body =
        response
            .bytes()
            .await
            .map_err(|error| AgentIdentityRegistrationAttemptError {
                message: format!("读取 Agent Identity runtime 注册响应失败: {}", error),
                retryable: error.is_timeout() || error.is_connect() || error.is_request(),
            })?;
    if response_body.len() > 64 * 1024 {
        return Err(AgentIdentityRegistrationAttemptError {
            message: "Agent Identity runtime 注册响应过大".to_string(),
            retryable: false,
        });
    }
    let result = serde_json::from_slice::<AgentIdentityRegistrationResponse>(&response_body)
        .map_err(|_| AgentIdentityRegistrationAttemptError {
            message: "Agent Identity runtime 注册响应格式无效".to_string(),
            retryable: false,
        })?;
    let runtime_id = result.agent_runtime_id.trim().to_string();
    if runtime_id.is_empty() {
        return Err(AgentIdentityRegistrationAttemptError {
            message: "Agent Identity runtime 注册响应缺少 agent_runtime_id".to_string(),
            retryable: false,
        });
    }
    Ok(runtime_id)
}

pub(crate) async fn register_agent_identity_with_base_url(
    access_token: &str,
    is_fedramp_account: bool,
    base_url: &str,
) -> Result<RegisteredAgentIdentity, String> {
    let access_token = access_token.trim();
    if access_token.is_empty() {
        return Err("Agent Identity runtime 注册缺少 accessToken".to_string());
    }
    let key_material = generate_agent_identity_key_material()?;
    let client = reqwest::Client::builder()
        .timeout(AGENT_IDENTITY_REGISTRATION_TIMEOUT)
        .connect_timeout(Duration::from_secs(10))
        .user_agent(format!("cockpit-tools/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|error| format!("创建 Agent Identity runtime 注册客户端失败: {}", error))?;
    let url = format!("{}/v1/agent/register", base_url.trim_end_matches('/'));

    let mut last_error = None;
    for attempt in 1..=AGENT_IDENTITY_REGISTRATION_ATTEMPTS {
        match register_agent_identity_once(
            &client,
            &url,
            access_token,
            is_fedramp_account,
            &key_material,
        )
        .await
        {
            Ok(agent_runtime_id) => {
                return Ok(RegisteredAgentIdentity {
                    agent_runtime_id,
                    agent_private_key: key_material.private_key_pkcs8_base64,
                });
            }
            Err(error) if error.retryable && attempt < AGENT_IDENTITY_REGISTRATION_ATTEMPTS => {
                last_error = Some(error.message);
            }
            Err(error) => return Err(error.message),
        }
    }

    Err(last_error.unwrap_or_else(|| "Agent Identity runtime 注册失败".to_string()))
}

fn task_lock_for(account_id: &str) -> Arc<Mutex<()>> {
    let locks = AGENT_IDENTITY_TASK_LOCKS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut locks = locks.lock().unwrap_or_else(|error| error.into_inner());
    locks
        .entry(account_id.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

fn agent_identity_key(account: &CodexAccount) -> Result<AgentIdentityKey, String> {
    let identity = account
        .agent_identity
        .as_ref()
        .ok_or_else(|| "Agent Identity 凭据不存在".to_string())?;
    let encoded = identity.agent_private_key.trim();
    let der = general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| "Agent Identity agent_private_key 不是有效 Base64".to_string())?;
    let private_key = SigningKey::from_pkcs8_der(&der).map_err(|_| {
        "Agent Identity agent_private_key 不是有效的 PKCS#8 Ed25519 私钥".to_string()
    })?;
    let runtime_id = identity.agent_runtime_id.trim().to_string();
    if runtime_id.is_empty() {
        return Err("Agent Identity agent_runtime_id 为空".to_string());
    }
    Ok(AgentIdentityKey {
        runtime_id,
        private_key,
        task_id: identity
            .task_id
            .clone()
            .unwrap_or_default()
            .trim()
            .to_string(),
    })
}

fn build_agent_assertion(
    key: &AgentIdentityKey,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<String, String> {
    if key.runtime_id.is_empty() || key.task_id.is_empty() {
        return Err("Agent Identity runtime 或 task_id 为空".to_string());
    }
    let timestamp = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let payload = format!("{}:{}:{}", key.runtime_id, key.task_id, timestamp);
    let signature = key.private_key.sign(payload.as_bytes());
    let envelope = serde_json::json!({
        "agent_runtime_id": key.runtime_id,
        "task_id": key.task_id,
        "timestamp": timestamp,
        "signature": general_purpose::STANDARD.encode(signature.to_bytes()),
    });
    let encoded = serde_json::to_vec(&envelope)
        .map_err(|_| "序列化 Agent Identity assertion 失败".to_string())?;
    Ok(format!(
        "AgentAssertion {}",
        general_purpose::URL_SAFE_NO_PAD.encode(encoded)
    ))
}

fn sign_task_registration(
    key: &AgentIdentityKey,
    now: chrono::DateTime<chrono::Utc>,
) -> (String, String) {
    let timestamp = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let payload = format!("{}:{}", key.runtime_id, timestamp);
    let signature = key.private_key.sign(payload.as_bytes());
    (
        timestamp,
        general_purpose::STANDARD.encode(signature.to_bytes()),
    )
}

fn decrypt_agent_task_id(key: &AgentIdentityKey, encoded: &str) -> Result<String, String> {
    let ciphertext = general_purpose::STANDARD
        .decode(encoded.trim())
        .map_err(|_| "加密 Agent Identity task_id 不是有效 Base64".to_string())?;
    let digest = Sha512::digest(key.private_key.to_bytes());
    let mut curve_private = [0u8; 32];
    curve_private.copy_from_slice(&digest[..32]);
    let secret_key = crypto_box::SecretKey::from_bytes(curve_private);
    let plaintext = secret_key
        .unseal(&ciphertext)
        .map_err(|_| "解密 Agent Identity task_id 失败".to_string())?;
    let task_id = String::from_utf8(plaintext)
        .map_err(|_| "解密后的 Agent Identity task_id 不是有效 UTF-8".to_string())?
        .trim()
        .to_string();
    if task_id.is_empty() {
        return Err("解密后的 Agent Identity task_id 为空".to_string());
    }
    Ok(task_id)
}

async fn register_agent_identity_task_with_base_url(
    account: &CodexAccount,
    base_url: &str,
) -> Result<String, String> {
    let key = agent_identity_key(account)?;
    let (timestamp, signature) = sign_task_registration(&key, chrono::Utc::now());
    let client = reqwest::Client::builder()
        .timeout(AGENT_IDENTITY_TASK_REGISTRATION_TIMEOUT)
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| format!("创建 Agent Identity task 注册客户端失败: {}", error))?;
    let url = format!(
        "{}/v1/agent/{}/task/register",
        base_url.trim_end_matches('/'),
        key.runtime_id
    );
    let response = client
        .post(url)
        .header("Accept", "application/json")
        .json(&serde_json::json!({
            "timestamp": timestamp,
            "signature": signature,
        }))
        .send()
        .await
        .map_err(|error| format!("Agent Identity task 注册请求失败: {}", error))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("Agent Identity task 注册返回 HTTP {}", status));
    }
    let response_body = response
        .bytes()
        .await
        .map_err(|_| "读取 Agent Identity task 注册响应失败".to_string())?;
    if response_body.len() > 64 * 1024 {
        return Err("Agent Identity task 注册响应过大".to_string());
    }
    let result = serde_json::from_slice::<AgentIdentityTaskRegistrationResponse>(&response_body)
        .map_err(|_| "Agent Identity task 注册响应格式无效".to_string())?;
    if let Some(task_id) = result
        .task_id
        .or(result.task_id_camel)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return Ok(task_id);
    }
    let encrypted = result
        .encrypted_task_id
        .or(result.encrypted_task_id_camel)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Agent Identity task 注册响应缺少 task_id".to_string())?;
    decrypt_agent_task_id(&key, &encrypted)
}

async fn ensure_agent_identity_task_with_base_url(
    account: &CodexAccount,
    expected_task_id: Option<&str>,
    base_url: &str,
) -> Result<CodexAccount, String> {
    if !account.is_agent_identity_auth() {
        return Ok(account.clone());
    }
    let expected_task_id = expected_task_id.unwrap_or_default().trim();
    let current_task_id = account
        .agent_identity
        .as_ref()
        .and_then(|identity| identity.task_id.as_deref())
        .unwrap_or_default()
        .trim();
    if !current_task_id.is_empty()
        && (expected_task_id.is_empty() || current_task_id != expected_task_id)
    {
        return Ok(account.clone());
    }

    let lock = task_lock_for(&account.id);
    let _guard = lock.lock().await;
    let mut current = codex_account::load_account(&account.id).unwrap_or_else(|| account.clone());
    let current_task_id = current
        .agent_identity
        .as_ref()
        .and_then(|identity| identity.task_id.as_deref())
        .unwrap_or_default()
        .trim();
    if !current_task_id.is_empty()
        && (expected_task_id.is_empty() || current_task_id != expected_task_id)
    {
        return Ok(current);
    }

    let task_id = register_agent_identity_task_with_base_url(&current, base_url).await?;
    let identity = current
        .agent_identity
        .as_mut()
        .ok_or_else(|| "Agent Identity 凭据不存在".to_string())?;
    identity.task_id = Some(task_id);
    if codex_account::load_account(&current.id).is_some() {
        codex_account::save_account(&current)?;
        if let Err(error) =
            codex_local_access::sync_sidecar_auth_file_for_account_with_current_task(&current)
        {
            logger::log_warn(&format!(
                "Agent Identity task 已保存，但同步 API 服务凭据失败: account_id={}, error={}",
                current.id, error
            ));
        }
    }
    Ok(current)
}

pub(crate) async fn build_authentication_headers_with_base_url(
    account: &CodexAccount,
    expected_task_id: Option<&str>,
    base_url: &str,
) -> Result<(CodexAccount, HeaderMap, String), String> {
    if !account.is_agent_identity_auth() {
        return Err("需要 Agent Identity 账号".to_string());
    }
    let current =
        ensure_agent_identity_task_with_base_url(account, expected_task_id, base_url).await?;
    let key = agent_identity_key(&current)?;
    let expected_task_id = key.task_id.clone();
    let assertion = build_agent_assertion(&key, chrono::Utc::now())?;
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&assertion).map_err(|_| "构建 AgentAssertion 头失败".to_string())?,
    );
    Ok((current, headers, expected_task_id))
}

pub fn is_task_invalid_response(status: reqwest::StatusCode, body: &str) -> bool {
    if status != reqwest::StatusCode::UNAUTHORIZED {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    let compact = lower
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect::<String>();
    [
        "\"code\":\"invalid_task_id\"",
        "\"code\":\"task_not_found\"",
        "\"code\":\"task_expired\"",
        "\"error\":\"invalid_task_id\"",
    ]
    .iter()
    .any(|marker| compact.contains(marker))
        || [
            "invalid task_id",
            "invalid task id",
            "task_id is invalid",
            "task id is invalid",
            "task not found",
            "task expired",
            "unknown task_id",
            "unknown task id",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
}

pub fn redact_sensitive_body(account: &CodexAccount, body: &str) -> String {
    if !account.is_agent_identity_auth() || body.is_empty() {
        return body.to_string();
    }
    let Some(identity) = account.agent_identity.as_ref() else {
        return body.to_string();
    };
    let mut redacted = body.to_string();
    for value in [
        Some(identity.agent_private_key.as_str()),
        Some(identity.agent_runtime_id.as_str()),
        identity.task_id.as_deref(),
        Some(account.tokens.access_token.as_str()),
        account.tokens.refresh_token.as_deref(),
        Some(account.tokens.id_token.as_str()),
        account.openai_api_key.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .filter(|value| !value.is_empty())
    {
        redacted = redacted.replace(value, "[redacted]");
    }
    let prefix = "AgentAssertion ";
    let mut offset = 0;
    while let Some(relative_start) = redacted[offset..].find(prefix) {
        let start = offset + relative_start;
        let value_start = start + prefix.len();
        let end = redacted[value_start..]
            .find(|character: char| character.is_ascii_whitespace() || "\"',}".contains(character))
            .map(|offset| value_start + offset)
            .unwrap_or(redacted.len());
        redacted.replace_range(value_start..end, "[redacted]");
        offset = value_start + "[redacted]".len();
    }
    redacted
}

#[cfg(test)]
mod tests {
    use super::{
        agent_identity_key, build_agent_assertion, decrypt_agent_task_id, redact_sensitive_body,
        register_agent_identity_with_base_url,
    };
    use crate::models::codex::{CodexAccount, CodexAgentIdentity, CodexTokens};
    use base64::{engine::general_purpose, Engine as _};
    use crypto_box::PublicKey;
    use ed25519_dalek::{pkcs8::DecodePrivateKey, pkcs8::EncodePrivateKey, SigningKey};
    use rand::rngs::OsRng;
    use sha2::Digest;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn read_test_http_request(stream: &mut tokio::net::TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut chunk = [0u8; 2048];
        let header_end = loop {
            let read = stream.read(&mut chunk).await.expect("read request");
            assert!(read > 0, "connection closed before request headers");
            bytes.extend_from_slice(&chunk[..read]);
            if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let headers = String::from_utf8_lossy(&bytes[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        while bytes.len() < header_end + content_length {
            let read = stream.read(&mut chunk).await.expect("read request body");
            assert!(read > 0, "connection closed before request body");
            bytes.extend_from_slice(&chunk[..read]);
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn test_account() -> CodexAccount {
        let signing_key = SigningKey::generate(&mut OsRng);
        let private_key = signing_key.to_pkcs8_der().expect("encode PKCS#8");
        let mut account = CodexAccount::new(
            "agent-test".to_string(),
            "agent@example.com".to_string(),
            CodexTokens {
                id_token: String::new(),
                access_token: String::new(),
                refresh_token: None,
            },
        );
        account.agent_identity = Some(CodexAgentIdentity {
            agent_runtime_id: "runtime-test".to_string(),
            agent_private_key: general_purpose::STANDARD.encode(private_key.as_bytes()),
            task_id: Some("task-test".to_string()),
            account_id: "team-test".to_string(),
            chatgpt_user_id: "user-test".to_string(),
            email: Some("agent@example.com".to_string()),
            plan_type: Some("k12".to_string()),
            chatgpt_account_is_fedramp: false,
        });
        account
    }

    #[test]
    fn assertion_uses_agent_identity_envelope_and_signature() {
        let account = test_account();
        let key = agent_identity_key(&account).expect("load key");
        let assertion = build_agent_assertion(
            &key,
            chrono::DateTime::parse_from_rfc3339("2026-07-21T10:11:12Z")
                .expect("timestamp")
                .with_timezone(&chrono::Utc),
        )
        .expect("build assertion");
        assert!(assertion.starts_with("AgentAssertion "));
    }

    #[tokio::test]
    async fn runtime_registration_uses_official_payload_and_reuses_key_on_retry() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock registration server");
        let base_url = format!("http://{}", listener.local_addr().expect("local address"));
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for attempt in 0..2 {
                let (mut stream, _) = listener.accept().await.expect("accept request");
                let request = read_test_http_request(&mut stream).await;
                requests.push(request);
                let (status, body) = if attempt == 0 {
                    ("429 Too Many Requests", r#"{"error":"retry"}"#)
                } else {
                    ("200 OK", r#"{"agent_runtime_id":"runtime-registered"}"#)
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("write response");
            }
            requests
        });

        let registered =
            register_agent_identity_with_base_url("session-access-token", true, &base_url)
                .await
                .expect("register runtime");
        assert_eq!(registered.agent_runtime_id, "runtime-registered");
        let private_key = general_purpose::STANDARD
            .decode(&registered.agent_private_key)
            .expect("decode private key");
        SigningKey::from_pkcs8_der(&private_key).expect("valid PKCS#8 Ed25519 private key");

        let requests = server.await.expect("mock registration server");
        assert_eq!(requests.len(), 2);
        let request_bodies =
            requests
                .iter()
                .map(|request| {
                    assert!(request.starts_with("POST /v1/agent/register HTTP/1.1"));
                    assert!(request.lines().any(|line| line
                        .eq_ignore_ascii_case("authorization: Bearer session-access-token")));
                    assert!(request
                        .lines()
                        .any(|line| line.eq_ignore_ascii_case("x-openai-fedramp: true")));
                    let (_, body) = request.split_once("\r\n\r\n").expect("request body");
                    serde_json::from_str::<serde_json::Value>(body).expect("registration JSON")
                })
                .collect::<Vec<_>>();
        assert_eq!(
            request_bodies[0]["agent_public_key"],
            request_bodies[1]["agent_public_key"]
        );
        assert!(request_bodies[0]["agent_public_key"]
            .as_str()
            .is_some_and(|value| value.starts_with("ssh-ed25519 ")));
        assert_eq!(
            request_bodies[0]["capabilities"],
            serde_json::json!(["responsesapi"])
        );
        assert!(request_bodies[0]["ttl"].is_null());
        assert_eq!(
            request_bodies[0].pointer("/abom/agent_harness_id"),
            Some(&serde_json::json!("codex-cli"))
        );
        assert!(!requests[0].contains(&registered.agent_private_key));
    }

    #[test]
    fn encrypted_task_uses_ed25519_seed_derived_curve_key() {
        let account = test_account();
        let key = agent_identity_key(&account).expect("load key");
        let digest = sha2::Sha512::digest(key.private_key.to_bytes());
        let mut curve_private = [0u8; 32];
        curve_private.copy_from_slice(&digest[..32]);
        let secret_key = crypto_box::SecretKey::from_bytes(curve_private);
        let public_key = PublicKey::from(&secret_key);
        let ciphertext = public_key
            .seal(&mut OsRng, b"task-encrypted")
            .expect("encrypt task");
        let decoded = decrypt_agent_task_id(&key, &general_purpose::STANDARD.encode(ciphertext))
            .expect("decrypt task");
        assert_eq!(decoded, "task-encrypted");
    }

    #[test]
    fn redacts_agent_identity_values_and_multiple_assertions() {
        let account = test_account();
        let private_key = account
            .agent_identity
            .as_ref()
            .expect("identity")
            .agent_private_key
            .clone();
        let body = format!(
            "runtime-test {} AgentAssertion first-value and AgentAssertion second-value",
            private_key
        );
        let redacted = redact_sensitive_body(&account, &body);
        assert!(!redacted.contains("runtime-test"));
        assert!(!redacted.contains(&private_key));
        assert!(!redacted.contains("first-value"));
        assert!(!redacted.contains("second-value"));
        assert_eq!(redacted.matches("AgentAssertion [redacted]").count(), 2);
    }
}
