//! Real loopback HTTP tests with synthetic credentials and an isolated data
//! directory. No browser, external OAuth service, or user credential is used.
use super::*;
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// A generated, disposable test-only RSA key. It has never authenticated to any
// service and is used solely to exercise JWKS signature verification offline.
const RSA_KEY: &str = "MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDUErxQ4xxBLJkcsUWmUNjQMAiicTo07cdtkozKp8PSjCcr0n10UjrY9UwHNzemLO2x3wNwvbmlWUFPYNuLSSvbhWfByVQOvttpkalZB1KlOqaTiEq8bzVp25baBWabCnbG2yjOam0nVzpN3h/Wq0fA1ECOZJUfZOwVPumV4wTKqjyIeX3EGF95Dr9dCazy4HZw70uOS2EcVxMMNqQwaU/9zXkqP6eIeNW7rNv3Zst3hfrZu8UTX5qSZM5lx62ps+9eFulUjYP7H95E/6BrCU+E2aKazoyeS0orSxpWDTTzHkXrvPSVcw6kqA2T0ysXs2zUS0LB/5fuk1IWAKoPeYw3AgMBAAECggEBAJWmGujhOkDrXagXl9SC3FwcaRtn1vEekUnKzjr7I33hlIqh4BFDLW+HuPymhEhpD9oiSuU1nFkHoMSIzqN9IMupWcwva3d0SpQ2S1AdkL4zovQcitBpkUF/yPVUNlFVkvdEE4mFJD1MmkH0ancrKljx7hQM+gXLS6S6Xk58ghcXu5YPt0C3TSNMJ68q7DkcmAI+vuV9jIsXQDH1LWRv0zw2glDIhJF4jWhwxNDql4g/g57vRWls1T6CnSCmclB5896JdpLgZigRGZjOr9SileJh19VHT2qs+okFLlsBaTb46fCW0x7t+ZN7fNg+ueXQNdGP7Fv1XYuD79pUKNTSMAECgYEA/8Op6PY4rW5Ck+r+Lof+KtA4eIA3Md1HpoC5FR0vJEIRFQ5YTL/of9pnKDAtv69qktq94r3iRKOM9Dg+zkxL87XFF+nCFV8cUFsFyh7aLtYP0TNhPh517fMrlk8t47+H5xiOSLk8UEpeSLt5HoCPy9Jt9Bw8bTJzVtXPt6yjzAECgYEA1ETD0Pwo2AkKh0vQBcYvEXR1Uhtqzj9tPtu6JO0irfPyKTxy0o1TmAZqTs9zuJZ64p5VwSkjC829sWVCLWlzQGsOHIAXHs4xW54uRFPD/Q+vbEjGhpvAvSiuKS+3I3P87eEHNw9j2yQRBBi2D7Wg9Q+xobRbE/ew5XBfhF2ouDcCgYAcA0g21dD7Xdo47UINwqNZ+RhWSmTBN2VDnVGoOFwTcpEAZ2PQdCx45GjsA+re1c2sa/zkOrvvnKIxlX97hOvEs6H0routX49wVafarWpkOtncpe3+qcNs3ndlrSJvkWjHLO9Yk8FBYgPVn4Lyjk4xmsikNfuAUkhDuINZsLT4AQKBgQCCZGNtQxjL7HHdPoLgqSdMr59J+rGXVHsFO/CSsmC1Pr8o+YsHL+2D25z+II1TfIysHNtMtrpii9XU3QDT6Iz1q02V0FFjiQHk57xAGLyx/Sqjd+kL0y2IhS7G+hqj2qIqdir6dae/e95AB/X+nHHtV+CkFqhEEclGh8sFAoub7QKBgD/2UiCLyK683LrtBDSbESainK0041lusHeTqxVUrNaD9B1MolByEZmtqBSJe0/zO47gABF6GSy+hBXTKS67btEu77T1jdNkkPbhgXzPlIjT7f15ekA2oRdvfbbn73lsDaCcCnxoRMX3mvD4LppXKw3uyz1HAkzuOHo18jomf9xu";
const RSA_N: &str = "1BK8UOMcQSyZHLFFplDY0DAIonE6NO3HbZKMyqfD0ownK9J9dFI62PVMBzc3piztsd8DcL25pVlBT2Dbi0kr24VnwclUDr7baZGpWQdSpTqmk4hKvG81aduW2gVmmwp2xtsozmptJ1c6Td4f1qtHwNRAjmSVH2TsFT7pleMEyqo8iHl9xBhfeQ6_XQms8uB2cO9LjkthHFcTDDakMGlP_c15Kj-niHjVu6zb92bLd4X62bvFE1-akmTOZcetqbPvXhbpVI2D-x_eRP-gawlPhNmims6MnktKK0saVg008x5F67z0lXMOpKgNk9MrF7Ns1EtCwf-X7pNSFgCqD3mMNw";

fn signed_id_token(nonce: &str, subject: &str) -> String {
    let header =
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&json!({"alg":"RS256","kid":"test"})).unwrap());
    let claims = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&json!({
            "sub":subject,"email":"test@example.invalid","iss":http::ISSUER,
            "aud":"oaiapp_test","nonce":nonce,"exp":chrono::Utc::now().timestamp()+3600
        }))
        .unwrap(),
    );
    let message = format!("{header}.{claims}");
    let key = ring::signature::RsaKeyPair::from_pkcs8(&STANDARD.decode(RSA_KEY).unwrap()).unwrap();
    let mut signature = vec![0; key.public().modulus_len()];
    key.sign(
        &ring::signature::RSA_PKCS1_SHA256,
        &ring::rand::SystemRandom::new(),
        message.as_bytes(),
        &mut signature,
    )
    .unwrap();
    format!("{message}.{}", URL_SAFE_NO_PAD.encode(signature))
}

#[derive(Default)]
struct MockState {
    nonce: String,
    redirect_uri: String,
    challenge: String,
    refresh_token: String,
    refresh_count: usize,
    refresh_started: usize,
    refresh_release: Option<Arc<tokio::sync::Notify>>,
    revoke_count: usize,
    fail_revoke: bool,
    no_plan_permission: bool,
    invalid_code_once: bool,
    code_started: usize,
    code_delay: bool,
    subject: Option<String>,
    response_started: usize,
    response_delay: bool,
    response_requests: Vec<(String, Value)>,
    response_status: Option<u16>,
    model_count: usize,
    model_status: Option<u16>,
    model_delay: bool,
    expected_model: Option<String>,
}

struct Fixture {
    directory: std::path::PathBuf,
    previous_directory: Option<std::ffi::OsString>,
    previous_server: Option<std::ffi::OsString>,
    server: tokio::task::JoinHandle<()>,
    state: Arc<Mutex<MockState>>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
        for (key, value) in [
            (
                "COCKPIT_TOOLS_TEST_DATA_DIR",
                self.previous_directory.take(),
            ),
            ("CHATGPT_PLAN_TEST_SERVER", self.previous_server.take()),
        ] {
            if let Some(value) = value {
                std::env::set_var(key, value);
            } else {
                std::env::remove_var(key);
            }
        }
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

impl Fixture {
    async fn new() -> Self {
        let directory =
            std::env::temp_dir().join(format!("chatgpt-plan-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let previous_directory = std::env::var_os("COCKPIT_TOOLS_TEST_DATA_DIR");
        let previous_server = std::env::var_os("CHATGPT_PLAN_TEST_SERVER");
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &directory);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::env::set_var("CHATGPT_PLAN_TEST_SERVER", &base);
        let state = Arc::new(Mutex::new(MockState::default()));
        let server_state = state.clone();
        let server = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let state = server_state.clone();
                let base = base.clone();
                tokio::spawn(async move {
                    serve(stream, &base, state).await;
                });
            }
        });
        Self {
            directory,
            previous_directory,
            previous_server,
            server,
            state,
        }
    }

    async fn login(&self) -> ChatgptPlanAccountSummary {
        let login = start_login(None).await.unwrap();
        self.finish_login(login).await
    }

    async fn finish_login(&self, login: ChatgptPlanLogin) -> ChatgptPlanAccountSummary {
        let status = self.complete_status(&login).await;
        assert_eq!(status.status, "complete", "{status:?}");
        status.account.unwrap()
    }

    fn callback_url(&self, login: &ChatgptPlanLogin) -> String {
        let url = url::Url::parse(&login.authorize_url).unwrap();
        let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert!(query["ext_agent_host_id"].starts_with("urn:uuid:"));
        uuid::Uuid::parse_str(query["ext_agent_host_id"].trim_start_matches("urn:uuid:")).unwrap();
        {
            let mut state = self.state.lock().unwrap();
            state.nonce = query["nonce"].clone();
            state.redirect_uri = query["redirect_uri"].clone();
            state.challenge = query["code_challenge"].clone();
        }
        let mut callback = url::Url::parse(&query["redirect_uri"]).unwrap();
        callback.query_pairs_mut().extend_pairs([
            ("state", query["state"].as_str()),
            ("code", "synthetic-code"),
            ("client_id", "oaiapp_test"),
        ]);
        callback.to_string()
    }

    async fn complete_status(&self, login: &ChatgptPlanLogin) -> ChatgptPlanLoginStatus {
        let response = http::client(5)
            .unwrap()
            .get(self.callback_url(login))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
        self.wait_status(login).await
    }

    async fn wait_status(&self, login: &ChatgptPlanLogin) -> ChatgptPlanLoginStatus {
        for _ in 0..200 {
            let status = poll_login(&login.login_id).await.unwrap();
            if matches!(
                status.status.as_str(),
                "complete" | "error" | "expired" | "cancelled"
            ) {
                return status;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("mock authorization did not finish");
    }
}

async fn serve(mut stream: tokio::net::TcpStream, base: &str, state: Arc<Mutex<MockState>>) {
    let mut bytes = Vec::new();
    let header_end;
    loop {
        let mut chunk = [0; 2048];
        let count = stream.read(&mut chunk).await.unwrap();
        if count == 0 {
            return;
        }
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            header_end = end + 4;
            break;
        }
        assert!(bytes.len() < 65536);
    }
    let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
    let length = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.trim().parse::<usize>().unwrap())
        .unwrap_or(0);
    while bytes.len() < header_end + length {
        let mut chunk = [0; 2048];
        let count = stream.read(&mut chunk).await.unwrap();
        if count == 0 {
            return;
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    let path = headers
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let form: HashMap<_, _> = url::form_urlencoded::parse(&bytes[header_end..])
        .into_owned()
        .collect();
    let (status, content_type, body) = match path {
        "/.well-known/openid-configuration" => (200,"application/json",json!({"issuer":http::ISSUER,"jwks_uri":format!("{base}/jwks"),"revocation_endpoint":format!("{base}/revoke")}).to_string()),
        "/jwks" => (200,"application/json",json!({"keys":[{"kty":"RSA","kid":"test","use":"sig","alg":"RS256","n":RSA_N,"e":"AQAB"}]}).to_string()),
        "/api/accounts/oauth/token" => {
            assert_eq!(form["client_id"],"oaiapp_test");
            assert_eq!(form["resource"],http::RESOURCE);
            let is_refresh = form["grant_type"] == "refresh_token";
            if !is_refresh {
                let delayed={let mut state=state.lock().unwrap();state.code_started+=1;state.code_delay};
                if delayed { tokio::time::sleep(std::time::Duration::from_millis(100)).await; }
            }
            if is_refresh {
                let release = { let mut state = state.lock().unwrap(); state.refresh_started += 1; state.refresh_release.clone() };
                if let Some(release) = release { release.notified().await; }
                else { tokio::time::sleep(std::time::Duration::from_millis(80)).await; }
            }
            let mut state = state.lock().unwrap();
            if (!is_refresh && state.invalid_code_once) || (is_refresh && form["refresh_token"] != state.refresh_token) {
                state.invalid_code_once=false;
                (400,"application/json",json!({"error":"invalid_grant"}).to_string())
            } else {
                if is_refresh {
                    assert!(!form.contains_key("scope")); state.refresh_count+=1;
                } else {
                    assert_eq!(form["grant_type"],"authorization_code"); assert_eq!(form["code"],"synthetic-code");
                    assert_eq!(form["redirect_uri"],state.redirect_uri);
                    assert_eq!(URL_SAFE_NO_PAD.encode(Sha256::digest(form["code_verifier"].as_bytes())),state.challenge);
                }
                state.refresh_token=format!("synthetic-refresh-{}",state.refresh_count);
                let scope=if state.no_plan_permission { "openid profile email offline_access" } else { "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct" };
                (200,"application/json",json!({"access_token":format!("synthetic-access-{}",state.refresh_count),"refresh_token":state.refresh_token,"id_token":signed_id_token(&state.nonce,state.subject.as_deref().unwrap_or("subject-test")),"scope":scope,"token_type":"Bearer","expires_in":3600}).to_string())
            }
        }
        "/v1/models" => {
            let (status, delay) = { let mut state = state.lock().unwrap(); state.model_count += 1; (state.model_status.unwrap_or(200), state.model_delay) };
            if delay { tokio::time::sleep(std::time::Duration::from_millis(150)).await; }
            (status, "application/json", if status == 200 {
                json!({"models":[{"slug":"test-model","display_name":"Test Model","visibility":"list"}]}).to_string()
            } else { json!({"error":{"message":"synthetic-model-check"}}).to_string() })
        },
        "/v1/responses" => {
            let body:Value=serde_json::from_slice(&bytes[header_end..]).unwrap();
            let expected = state.lock().unwrap().expected_model.clone().unwrap_or_else(|| "test-model".into());
            assert_eq!(body["model"],expected); assert_eq!(body["store"],false); assert_eq!(body["stream"],true);
            let (delayed, response_status) = {
                let mut state = state.lock().unwrap();
                state.response_started += 1;
                state.response_requests.push((headers.clone(), body));
                (state.response_delay, state.response_status.unwrap_or(200))
            };
            if delayed { tokio::time::sleep(std::time::Duration::from_millis(150)).await; }
            if response_status != 200 {
                (response_status, "application/json", json!({"error":{"message":"synthetic-official-failure"}}).to_string())
            } else {
                (200,"text/event-stream",format!("data: {}\n\ndata: {}\n\n",
                    json!({"type":"response.output_text.delta","delta":"OK"}),
                    json!({"type":"response.completed","response":{"id":"synthetic-response","model":"test-model","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"OK"}]}],"usage":{"input_tokens":2,"output_tokens":1,"total_tokens":3}}})))
            }
        }
        "/revoke" => {
            let mut state=state.lock().unwrap(); state.revoke_count+=1;
            assert_eq!(form["client_id"],"oaiapp_test"); assert_eq!(form["token_type_hint"],"refresh_token"); assert_eq!(form["token"],state.refresh_token);
            (if state.fail_revoke{503}else{200},"application/json",String::new())
        }
        _ => panic!("unexpected mock route: {path}"),
    };
    let response=format!("HTTP/1.1 {status} Test\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
    let _ = stream.write_all(response.as_bytes()).await;
}

#[test]
fn mock_bound_service_mode_transaction_and_cancel_keep_credentials_and_scopes_separate() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        crate::modules::codex_local_access::with_chatgpt_plan_test_runtime(|| async {
            use crate::models::{
                codex::{CodexAccount, CodexTokens},
                codex_local_access::{
                    CodexLocalAccessAccountModelRule, CodexLocalAccessCustomRoutingRule,
                    CodexLocalAccessImageGenerationPolicy,
                },
            };
            use crate::modules::{
                chatgpt_plan_service as service, codex_account, codex_local_access as access,
            };
            let fixture = Fixture::new().await;
            let mut source = CodexAccount::new(
                "bound-service-source".into(),
                "test@example.invalid".into(),
                CodexTokens {
                    access_token: "synthetic-codex-access".into(),
                    id_token: "synthetic-codex-id".into(),
                    refresh_token: Some("synthetic-codex-refresh".into()),
                },
            );
            source.user_id = Some("codex-sub-independent-from-siwc-sub".into());
            source.account_id = Some("codex-workspace".into());
            source.plan_type = Some("plus".into());
            codex_account::save_account(&source).unwrap();
            let mut index = codex_account::load_account_index();
            index
                .accounts
                .push(crate::models::codex::CodexAccountSummary {
                    id: source.id.clone(),
                    email: source.email.clone(),
                    plan_type: source.plan_type.clone(),
                    subscription_active_until: None,
                    created_at: source.created_at,
                    last_used: source.last_used,
                });
            codex_account::save_account_index(&index).unwrap();
            let source_tokens =
                serde_json::to_value(&codex_account::load_account(&source.id).unwrap().tokens)
                    .unwrap();
            let profile = fixture.directory.join("untouched-client-profile");
            std::fs::create_dir_all(&profile).unwrap();
            let auth = b"{\"tokens\":{\"access_token\":\"synthetic-existing-client\"}}";
            let config = b"model_provider = \"openai\"\n";
            std::fs::write(profile.join("auth.json"), auth).unwrap();
            std::fs::write(profile.join("config.toml"), config).unwrap();
            let registration = fixture.login().await;
            bind_account(&registration.id, &source.id).await.unwrap();
            service::set_service_mode(&source.id, Some(&registration.id), "codex", None)
                .await
                .unwrap();
            access::save_local_access_accounts(
                vec![source.id.clone()],
                true,
                None,
                None,
                None,
                None,
                Some(HashMap::from([(
                    source.id.clone(),
                    CodexLocalAccessImageGenerationPolicy::Enabled,
                )])),
            )
            .await
            .unwrap();
            access::update_local_access_custom_routing(vec![CodexLocalAccessCustomRoutingRule {
                account_id: source.id.clone(),
                priority: 6,
                weight: 3,
                is_backup: true,
                is_preferred: false,
            }])
            .await
            .unwrap();
            access::update_local_access_account_model_rules(vec![
                CodexLocalAccessAccountModelRule {
                    account_id: source.id.clone(),
                    excluded_models: vec!["blocked-model".into()],
                },
            ])
            .await
            .unwrap();
            let before = serde_json::to_value(
                access::get_local_access_state()
                    .await
                    .unwrap()
                    .collection
                    .unwrap(),
            )
            .unwrap();
            let mut invalid = storage::read(&registration.id).await.unwrap();
            invalid.summary.needs_reauth = true;
            storage::save(&invalid).unwrap();
            assert_eq!(service::set_service_mode(&source.id, Some(&registration.id), "official", None).await.unwrap_err(), "CHATGPT_PLAN_REAUTH_REQUIRED");
            assert_eq!(serde_json::to_value(access::get_local_access_state().await.unwrap().collection.unwrap()).unwrap(), before);
            invalid.summary.needs_reauth = false;
            storage::save(&invalid).unwrap();
            let registration_lock = storage::lock(&registration.id).await.unwrap();
            let source_id = source.id.clone();
            let registration_id = registration.id.clone();
            let pending = tokio::spawn(async move {
                service::set_service_mode(&source_id, Some(&registration_id), "official", None).await
            });
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            pending.abort();
            assert!(pending.await.unwrap_err().is_cancelled());
            drop(registration_lock);
            assert_eq!(fixture.state.lock().unwrap().response_started, 0);
            assert_eq!(serde_json::to_value(access::get_local_access_state().await.unwrap().collection.unwrap()).unwrap(), before);
            let official = service::set_service_mode(
                &source.id,
                Some(&registration.id),
                "official",
                Some("test-model"),
            )
            .await
            .unwrap();
            let official = official.collection.unwrap();
            let projection_id =
                codex_account::chatgpt_plan_projection_id(&registration.id).unwrap();
            assert_eq!(official.account_ids, vec![source.id.clone()]);
            assert_eq!(official.official_account_bindings[&source.id], registration.id);
            assert_eq!(fixture.state.lock().unwrap().response_started, 0);
            service::list_models(&registration.id).await.unwrap();
            access::assert_chatgpt_plan_mixed_key_real_host(&source.id).await;
            assert_eq!(official.account_model_rules[0].account_id, source.id);
            assert_eq!(
                official.account_model_rules[0].excluded_models,
                vec!["blocked-model"]
            );
            assert_eq!(official.custom_routing_rules[0].priority, 6);
            assert_eq!(official.image_generation_account_policies[&source.id], CodexLocalAccessImageGenerationPolicy::Enabled);
            assert!(official.chatgpt_plan_codex_image_policies.is_empty());
            let restored = service::set_service_mode(&source.id, None, "codex", None)
                .await
                .unwrap()
                .collection
                .unwrap();
            assert_eq!(restored.account_ids, vec![source.id.clone()]);
            assert_eq!(
                restored.image_generation_account_policies[&source.id],
                CodexLocalAccessImageGenerationPolicy::Enabled
            );
            assert_eq!(restored.custom_routing_rules[0].account_id, source.id);
            assert_eq!(restored.account_model_rules[0].account_id, source.id);
            // The legacy command must select the same card's official member,
            // rather than silently leave both credentials in the pool.
            service::join(&registration.id, "test-model").await.unwrap();
            assert_eq!(
                access::chatgpt_plan_pool_snapshot().await.unwrap().1,
                vec![source.id.clone()]
            );
            // A replaced source principal cannot acquire this member's selected grant.
            // Release pauses the same business member without transferring its ACL.
            access::append_local_access_accounts(vec![source.id.clone()])
                .await
                .unwrap();
            let keys = access::create_local_access_api_key(Some("Mode scope".into()))
                .await
                .unwrap();
            let scoped_key = keys
                .collection
                .unwrap()
                .api_keys
                .into_iter()
                .find(|key| key.label == "Mode scope")
                .unwrap()
                .id;
            access::update_local_access_api_key(
                scoped_key.clone(),
                None,
                None,
                None,
                None,
                None,
                None,
                Some(vec![source.id.clone()]),
                Some(false),
            )
            .await
            .unwrap();
            source.account_id = Some("replaced-codex-workspace".into());
            codex_account::save_account(&source).unwrap();
            let detached = service::unbind(&registration.id).await.unwrap();
            assert!(detached.source_account_id.is_none());
            assert!(detached.connected);
            assert!(get_account(&registration.id).await.unwrap().connected);
            let state = access::get_local_access_state().await.unwrap();
            let collection = state.collection.unwrap();
            assert_eq!(collection.account_ids, vec![source.id.clone()]);
            assert_eq!(collection.paused_account_ids, vec![source.id.clone()]);
            let scoped_key = collection
                .api_keys
                .iter()
                .find(|key| key.id == scoped_key)
                .unwrap();
            assert_eq!(scoped_key.account_ids, vec![source.id.clone()]);
            assert_eq!(scoped_key.inherit_account_pool, Some(false));
            assert!(codex_account::load_account(&projection_id).is_none());
            assert!(service::unbind(&registration.id)
                .await
                .unwrap()
                .source_account_id
                .is_none());
            service::disconnect(&registration.id).await.unwrap();
            assert!(access::chatgpt_plan_pool_snapshot()
                .await
                .unwrap()
                .1
                .contains(&source.id));
            assert_eq!(
                serde_json::to_value(codex_account::load_account(&source.id).unwrap().tokens)
                    .unwrap(),
                source_tokens
            );
            assert_eq!(std::fs::read(profile.join("auth.json")).unwrap(), auth);
            assert_eq!(std::fs::read(profile.join("config.toml")).unwrap(), config);
        })
        .await;
    });
}

async fn expire(id: &str) {
    let _lock = storage::lock(id).await.unwrap();
    let mut record = storage::read(id).await.unwrap();
    storage::blocking(move || {
        let mut credentials = record.credentials()?;
        credentials.expires_at = chrono::Utc::now().timestamp() - 1;
        record.set_credentials(&credentials)?;
        storage::save(&record)
    })
    .await
    .unwrap();
}

async fn assert_service_json_contract(id: &str, in_service: bool, connected: bool, verified: bool) {
    // Exercise the actual command DTO, rather than constructing its flattened
    // JSON by hand; a casing/nesting regression would otherwise pass silently.
    let accounts = crate::commands::chatgpt_plan::chatgpt_plan_list_accounts()
        .await
        .unwrap();
    let value = serde_json::to_value(accounts).unwrap();
    let account = value
        .as_array()
        .unwrap()
        .iter()
        .find(|account| account["id"] == id)
        .unwrap();
    assert_eq!(account["in_service"], in_service);
    assert_eq!(account["connected"], connected);
    assert_eq!(account["verified_at"].is_number(), verified);
    assert!(account.get("account").is_none());
    assert!(account.get("inService").is_none());
    assert!(account.get("client_id").is_some());
    assert!(account.get("clientId").is_none());
    assert!(account.get("access_token").is_none());
    assert!(account.get("refresh_token").is_none());
    let ids = crate::modules::codex_local_access::chatgpt_plan_pool_snapshot()
        .await
        .unwrap()
        .1;
    let selected = crate::modules::codex_local_access::chatgpt_plan_selected_bindings().await.unwrap();
    assert_eq!(selected.iter().any(|(source, grant)| grant == id && ids.contains(source)), in_service);
}

async fn assert_reloaded_service_json_contract(
    id: &str,
    in_service: bool,
    connected: bool,
    verified: bool,
) {
    // The helper replaces the in-memory gateway runtime with an unloaded one;
    // both registrations and membership must survive solely from persisted data.
    crate::modules::codex_local_access::with_chatgpt_plan_test_runtime(|| async {
        assert_service_json_contract(id, in_service, connected, verified).await;
        let state = crate::modules::codex_local_access::get_local_access_state()
            .await
            .unwrap();
        let state = serde_json::to_value(state).unwrap();
        let bindings = state["collection"]["officialAccountBindings"].as_object();
        assert_eq!(bindings.is_some_and(|bindings| bindings.values().any(|grant| grant.as_str() == Some(id))), in_service);
        assert_eq!(state["running"], false);
    })
    .await;
}

#[test]
fn mock_oauth_refresh_inference_disconnect_and_cancellation() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let fixture = Fixture::new().await;
        let sentinel = fixture.directory.join("codex-auth-sentinel.json");
        std::fs::write(&sentinel, "existing Codex credentials").unwrap();
        let account = fixture.login().await;
        assert!(account.connected);
        assert!(!account.needs_reauth);
        assert!(account.label.contains("test@example.invalid"));
        assert_eq!(list_models(&account.id).await.unwrap()[0].id, "test-model");
        let verified = verify(&account.id, "test-model").await.unwrap();
        assert!(verified.verified_at.is_some());
        let raw = std::fs::read_to_string(
            fixture
                .directory
                .join("chatgpt-plan")
                .join(format!("{}.json", account.id)),
        )
        .unwrap();
        assert!(!raw.contains("synthetic-access"));
        assert!(!raw.contains("synthetic-refresh"));
        assert!(raw.contains("AES-256-GCM"));
        expire(&account.id).await;
        let mut calls = Vec::new();
        for _ in 0..8 {
            let id = account.id.clone();
            calls.push(tokio::spawn(async move { access_token(&id, false).await }));
        }
        for call in calls {
            assert_eq!(call.await.unwrap().unwrap(), "synthetic-access-1");
        }
        assert_eq!(fixture.state.lock().unwrap().refresh_count, 1);
        expire(&account.id).await;
        let id = account.id.clone();
        let refreshing = tokio::spawn(async move { access_token(&id, false).await });
        while fixture.state.lock().unwrap().refresh_started < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let result = disconnect(&account.id).await.unwrap();
        assert!(result.remote_revoked);
        assert_eq!(refreshing.await.unwrap().unwrap(), "synthetic-access-2");
        assert_eq!(
            access_token(&account.id, false).await.unwrap_err(),
            "CHATGPT_PLAN_DISCONNECTED"
        );
        assert_eq!(fixture.state.lock().unwrap().revoke_count, 1);
        let disconnected = get_account(&account.id).await.unwrap();
        assert!(!disconnected.connected);
        assert_eq!(disconnected.client_id, "oaiapp_test");
        let login = start_login(Some(&account.id)).await.unwrap();
        let url = url::Url::parse(&login.authorize_url).unwrap();
        let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query["client_id"], "oaiapp_test");
        assert!(!query.contains_key("agent_name_hint"));
        assert!(!query.contains_key("id_token_hint"));
        cancel_login(&login.login_id).await.unwrap();
        assert_eq!(
            poll_login(&login.login_id).await.unwrap().status,
            "cancelled"
        );
        assert_eq!(
            std::fs::read_to_string(sentinel).unwrap(),
            "existing Codex credentials"
        );
    });
}

#[test]
fn mock_permission_missing_and_unconfirmed_revocation_remain_explicit() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let fixture = Fixture::new().await;
        fixture.state.lock().unwrap().no_plan_permission = true;
        let account = fixture.login().await;
        assert!(account.connected);
        assert!(account.needs_reauth);
        assert_eq!(
            access_token(&account.id, false).await.unwrap_err(),
            "CHATGPT_PLAN_PERMISSION_REQUIRED"
        );
        fixture.state.lock().unwrap().fail_revoke = true;
        assert!(!disconnect(&account.id).await.unwrap().remote_revoked);
        assert!(!disconnect(&account.id).await.unwrap().remote_revoked);
        assert!(!get_account(&account.id).await.unwrap().connected);
    });
}

#[test]
fn mock_invalid_grant_keeps_issued_client_for_fresh_authorization() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let fixture = Fixture::new().await;
        fixture.state.lock().unwrap().invalid_code_once = true;
        let login = start_login(None).await.unwrap();
        let failed = fixture.complete_status(&login).await;
        assert_eq!(failed.status, "error");
        assert_eq!(
            failed.error.as_deref(),
            Some("CHATGPT_PLAN_REAUTH_REQUIRED")
        );
        let draft = failed.account.unwrap();
        assert_eq!(draft.client_id, "oaiapp_test");
        assert!(draft.subject.is_empty());
        assert!(!draft.connected);
        assert!(inference_token(&draft.id, false).await.is_err());
        let login = start_login(Some(&draft.id)).await.unwrap();
        let url = url::Url::parse(&login.authorize_url).unwrap();
        let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query["client_id"], "oaiapp_test");
        assert!(!query.contains_key("agent_name_hint"));
        let account = fixture.finish_login(login).await;
        assert_eq!(account.id, draft.id);
        assert_eq!(account.subject, "subject-test");
        assert!(account.connected);
        assert!(inference_token(&account.id, false).await.is_ok());
        verify(&account.id, "test-model").await.unwrap();
        assert!(inference_token(&account.id, false).await.is_ok());
        let login = start_login(Some(&account.id)).await.unwrap();
        fixture.finish_login(login).await;
        assert!(inference_token(&account.id, false).await.is_ok());
        disconnect(&account.id).await.unwrap();
    });
}

#[test]
fn mock_service_join_remove_rejoin_disconnect_preserves_client_and_listener_state() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        crate::modules::codex_local_access::with_chatgpt_plan_test_runtime(|| async {
            use crate::modules::{
                chatgpt_plan_service as service, codex_account, codex_local_access as access,
            };
            let fixture = Fixture::new().await;
            let profile = fixture.directory.join("chatgpt-plan-test-codex-home");
            std::fs::create_dir_all(&profile).unwrap();
            let auth = b"{\"OPENAI_API_KEY\":\"synthetic-existing-client-key\"}";
            let config = b"model = \"existing-model\"\n";
            std::fs::write(profile.join("auth.json"), auth).unwrap();
            std::fs::write(profile.join("config.toml"), config).unwrap();
            let account = fixture.login().await;
            let source = authorization_lifecycle_tests::lifecycle_source("service-original");
            bind_account(&account.id, &source.id).await.unwrap();
            let login_json = serde_json::to_value(&account).unwrap();
            assert!(login_json.get("in_service").is_none());
            assert!(login_json["verified_at"].is_null());
            assert!(!service::list_accounts().await.unwrap()[0].in_service);
            assert_service_json_contract(&account.id, false, true, false).await;
            assert!(access::chatgpt_plan_pool_snapshot()
                .await
                .unwrap()
                .1
                .is_empty());
            service::join(&account.id, "test-model").await.unwrap();
            assert_service_json_contract(&account.id, true, true, false).await;
            assert_reloaded_service_json_contract(&account.id, true, true, false).await;
            let projection_id = source.id.clone();
            assert!(codex_account::load_account(&codex_account::chatgpt_plan_projection_id(&account.id).unwrap()).is_none());
            assert_eq!(codex_account::load_account(&source.id).unwrap().tokens.access_token, source.tokens.access_token);
            use crate::models::codex_local_access::CodexLocalAccessImageGenerationPolicy;
            let state = access::save_local_access_accounts_with_official_bindings(
                vec![source.id.clone()], true, None, None, None, None,
                Some(HashMap::from([(source.id.clone(), CodexLocalAccessImageGenerationPolicy::Enabled)])),
                Some(HashMap::from([(source.id.clone(), account.id.clone())])),
            ).await.unwrap();
            assert!(!state.running);
            assert_eq!(state.collection.as_ref().unwrap().image_generation_account_policies[&source.id], CodexLocalAccessImageGenerationPolicy::Enabled);
            assert_eq!(state.collection.as_ref().unwrap().official_account_bindings[&source.id], account.id);
            // Actual official capability is tested by the real Host path; local
            // image preferences remain available when the member returns local.
            assert_reloaded_service_json_contract(&account.id, true, true, false).await;
            let login = start_login(Some(&account.id)).await.unwrap();
            let reauthorized = fixture.finish_login(login).await;
            let reauthorized_json = serde_json::to_value(&reauthorized).unwrap();
            assert!(reauthorized_json.get("in_service").is_none());
            assert!(reauthorized_json["verified_at"].is_null());
            // Reauthorization clears only historical test metadata; consent remains directly usable.
            assert_service_json_contract(&account.id, true, true, false).await;
            assert_reloaded_service_json_contract(&account.id, true, true, false).await;
            assert!(inference_token(&account.id, false).await.is_ok());
            let verified_json = serde_json::to_value(
                crate::commands::chatgpt_plan::chatgpt_plan_verify(
                    account.id.clone(),
                    "test-model".into(),
                )
                .await
                .unwrap(),
            )
            .unwrap();
            assert!(verified_json.get("in_service").is_none());
            assert!(verified_json["verified_at"].is_number());
            assert_service_json_contract(&account.id, true, true, true).await;
            service::remove_from_service(&account.id).await.unwrap();
            assert!(get_account(&account.id).await.unwrap().connected);
            assert!(!service::list_accounts().await.unwrap()[0].in_service);
            assert_reloaded_service_json_contract(&account.id, false, true, true).await;
            service::join(&account.id, "test-model").await.unwrap();
            let state = access::create_local_access_api_key(Some("Plan mock".into()))
                .await
                .unwrap();
            let key = state
                .collection
                .unwrap()
                .api_keys
                .into_iter()
                .find(|key| key.label == "Plan mock")
                .unwrap();
            access::update_local_access_api_key(
                key.id.clone(),
                None,
                None,
                None,
                None,
                None,
                None,
                Some(vec![projection_id.clone()]),
                Some(false),
            )
            .await
            .unwrap();
            access::set_local_access_api_key_account_priority(key.id, projection_id.clone(), true)
                .await
                .unwrap();
            let result = service::disconnect(&account.id).await.unwrap();
            assert!(result.remote_revoked);
            let state = access::get_local_access_state().await.unwrap();
            let collection = state.collection.unwrap();
            assert!(!state.running);
            assert!(!collection.enabled);
            assert!(collection.account_ids.contains(&projection_id));
            assert!(collection.paused_account_ids.contains(&projection_id));
            assert!(collection.api_keys.iter().any(|key| key.account_ids.contains(&projection_id)
                && key.priority_account_ids.contains(&projection_id)));
            assert!(!get_account(&account.id).await.unwrap().connected);
            assert_service_json_contract(&account.id, false, false, false).await;
            assert_reloaded_service_json_contract(&account.id, false, false, false).await;
            assert_eq!(std::fs::read(profile.join("auth.json")).unwrap(), auth);
            assert_eq!(std::fs::read(profile.join("config.toml")).unwrap(), config);
        })
        .await;
    });
}

#[test]
fn mock_identity_mismatch_and_inflight_cancellation_never_replace_credentials() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let fixture = Fixture::new().await;
        let account = fixture.login().await;
        let path = fixture
            .directory
            .join("chatgpt-plan")
            .join(format!("{}.json", account.id));
        let original = std::fs::read(&path).unwrap();
        fixture.state.lock().unwrap().subject = Some("other-subject".into());
        let login = start_login(Some(&account.id)).await.unwrap();
        let status = fixture.complete_status(&login).await;
        assert_eq!(status.status, "error");
        assert_eq!(
            status.error.as_deref(),
            Some("CHATGPT_PLAN_IDENTITY_MISMATCH")
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(
            get_account(&account.id).await.unwrap().subject,
            "subject-test"
        );
        let before = {
            let mut state = fixture.state.lock().unwrap();
            state.subject = None;
            state.code_delay = true;
            state.code_started
        };
        let login = start_login(None).await.unwrap();
        let (status, ()) = tokio::join!(fixture.complete_status(&login), async {
            while fixture.state.lock().unwrap().code_started <= before {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            cancel_login(&login.login_id).await.unwrap();
        });
        assert_eq!(status.status, "cancelled");
        assert_eq!(list_accounts().await.unwrap().len(), 1);
        assert_eq!(std::fs::read(path).unwrap(), original);
    });
}

#[test]
fn mock_manual_callback_is_singleflight_with_loopback_and_preserves_pending_on_errors() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let fixture = Fixture::new().await;
        let login = start_login(None).await.unwrap();
        let callback = fixture.callback_url(&login);
        let invalid = callback.replace("client_id=oaiapp_test", "client_id=dynamic_agent_client");
        assert!(submit_callback_url(&login.login_id, &invalid)
            .await
            .is_err());
        let response = http::client(5).unwrap().get(&invalid).send().await.unwrap();
        assert_eq!(response.status(), 400);
        assert_eq!(poll_login(&login.login_id).await.unwrap().status, "pending");
        assert_eq!(fixture.state.lock().unwrap().code_started, 0);
        fixture.state.lock().unwrap().code_delay = true;
        let (manual, loopback) = tokio::join!(
            submit_callback_url(&login.login_id, &callback),
            http::client(5).unwrap().get(&callback).send(),
        );
        assert!(manual.is_ok() || loopback.is_ok_and(|response| response.status().is_success()));
        let completed = fixture.wait_status(&login).await;
        assert_eq!(completed.status, "complete", "{completed:?}");
        assert_eq!(fixture.state.lock().unwrap().code_started, 1);
        assert_eq!(
            submit_callback_url(&login.login_id, &callback)
                .await
                .unwrap_err(),
            "CHATGPT_PLAN_BUSY"
        );
        assert_eq!(fixture.state.lock().unwrap().code_started, 1);

        let invalid_nonce_login = start_login(None).await.unwrap();
        let invalid_nonce_callback = fixture.callback_url(&invalid_nonce_login);
        fixture.state.lock().unwrap().nonce = "wrong-nonce".into();
        submit_callback_url(&invalid_nonce_login.login_id, &invalid_nonce_callback)
            .await
            .unwrap();
        let invalid_nonce_status = fixture.wait_status(&invalid_nonce_login).await;
        assert_eq!(invalid_nonce_status.status, "error");
        assert_eq!(
            invalid_nonce_status.error.as_deref(),
            Some("CHATGPT_PLAN_INVALID_ID_TOKEN")
        );
        assert!(invalid_nonce_status.account.is_none());

        // Existing registration identities remain bound when using pasted URLs.
        let account = completed.account.unwrap();
        let login = start_login(Some(&account.id)).await.unwrap();
        let callback = fixture.callback_url(&login);
        let wrong = callback.replace("client_id=oaiapp_test", "client_id=other");
        assert_eq!(
            submit_callback_url(&login.login_id, &wrong)
                .await
                .unwrap_err(),
            "CHATGPT_PLAN_REGISTRATION_CHANGED"
        );
        assert_eq!(poll_login(&login.login_id).await.unwrap().status, "pending");
        submit_callback_url(&login.login_id, &callback)
            .await
            .unwrap();
        cancel_login(&login.login_id).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert_eq!(
            poll_login(&login.login_id).await.unwrap().status,
            "cancelled"
        );
        assert_eq!(
            get_account(&account.id).await.unwrap().subject,
            account.subject
        );
    });
}

#[test]
fn mock_valid_oauth_denial_is_terminal_for_manual_and_loopback_callbacks() {
    let _env = crate::modules::test_support::env_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let fixture = Fixture::new().await;
        for manual in [true, false] {
            let login = start_login(None).await.unwrap();
            let valid = fixture.callback_url(&login);
            let mut denied = url::Url::parse(&valid).unwrap();
            let state = denied
                .query_pairs()
                .find(|(key, _)| key == "state")
                .unwrap()
                .1
                .into_owned();
            denied.set_query(None);
            denied
                .query_pairs_mut()
                .extend_pairs([("state", state.as_str()), ("error", "access_denied")]);
            let mut forged = denied.clone();
            forged.set_query(Some("state=wrong-state&error=access_denied"));
            assert_eq!(
                submit_callback_url(&login.login_id, forged.as_str())
                    .await
                    .unwrap_err(),
                "CHATGPT_PLAN_INVALID_CALLBACK"
            );
            assert_eq!(poll_login(&login.login_id).await.unwrap().status, "pending");
            if manual {
                assert_eq!(
                    submit_callback_url(&login.login_id, denied.as_str())
                        .await
                        .unwrap_err(),
                    "CHATGPT_PLAN_AUTH_DENIED"
                );
            } else {
                let response = http::client(5).unwrap().get(denied).send().await.unwrap();
                assert_eq!(response.status(), 400);
            }
            let status = fixture.wait_status(&login).await;
            assert_eq!(status.status, "error");
            assert_eq!(status.error.as_deref(), Some("CHATGPT_PLAN_AUTH_DENIED"));
            assert_eq!(
                submit_callback_url(&login.login_id, &valid)
                    .await
                    .unwrap_err(),
                "CHATGPT_PLAN_BUSY"
            );
            assert_eq!(fixture.state.lock().unwrap().code_started, 0);
        }
    });
}

#[path = "chatgpt_plan_background_tests.rs"]
mod background_tests;
#[path = "chatgpt_plan_authorization_lifecycle_tests.rs"]
mod authorization_lifecycle_tests;
