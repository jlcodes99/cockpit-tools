use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn authorization_url_matches_upstream_consent_and_preserves_voice_scope() {
    let redirect = "http://localhost:12345/oauth-callback";
    let url = url::Url::parse(&get_auth_url(redirect, Some("state & value"))).unwrap();
    let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(url.host_str(), Some("accounts.google.com"));
    assert_eq!(params["client_id"], CLIENT_ID);
    assert_eq!(params["redirect_uri"], redirect);
    assert_eq!(params["state"], "state & value");
    assert_eq!(params["response_type"], "code");
    assert_eq!(params["access_type"], "offline");
    assert_eq!(params["prompt"], "consent");
    assert_eq!(params["include_granted_scopes"], "true");
    assert_eq!(
        params["scope"].split_whitespace().collect::<Vec<_>>(),
        vec![
            "openid",
            "https://www.googleapis.com/auth/cloud-platform",
            "https://www.googleapis.com/auth/userinfo.email",
            "https://www.googleapis.com/auth/userinfo.profile",
            "https://www.googleapis.com/auth/cclog",
            "https://www.googleapis.com/auth/experimentsandconfigs",
            "https://www.googleapis.com/auth/aicode",
        ]
    );
}

#[test]
fn refresh_window_covers_the_first_fifteen_minutes_after_switching() {
    let now = 1_800_000_000;
    assert!(token_needs_refresh(now - 1, now));
    assert!(token_needs_refresh(now + 300, now));
    assert!(token_needs_refresh(now + 900, now));
    assert!(!token_needs_refresh(now + 901, now));
}

async fn refresh_with_responses(
    responses: Vec<(u16, &'static str)>,
) -> (Result<TokenResponse, String>, Vec<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut buffer = [0; 1024];
                let count = stream.read(&mut buffer).await.unwrap();
                assert_ne!(count, 0);
                request.extend_from_slice(&buffer[..count]);
                if let Some(headers_end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n")
                {
                    let headers = String::from_utf8_lossy(&request[..headers_end]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    if request.len() >= headers_end + 4 + length {
                        break;
                    }
                }
            }
            requests.push(String::from_utf8(request).unwrap());
            let response = format!(
                "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len(),
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        }
        requests
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap();
    let result = refresh_access_token_at(&client, &endpoint, "test-refresh", None).await;
    let requests = tokio::time::timeout(std::time::Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
    (result, requests)
}

#[tokio::test]
async fn refresh_retries_invalid_grant_once_and_uses_native_oauth_request() {
    let (result, requests) = refresh_with_responses(vec![
        (400, r#"{"error":"invalid_grant"}"#),
        (
            200,
            r#"{"access_token":"new-access","expires_in":3600,"id_token":"new-id"}"#,
        ),
    ])
    .await;
    let token = result.unwrap();
    assert_eq!(token.access_token, "new-access");
    assert_eq!(token.id_token.as_deref(), Some("new-id"));
    assert_eq!(
        token.oauth_client_key.as_deref(),
        Some(DEFAULT_OAUTH_CLIENT_KEY)
    );
    assert_eq!(requests.len(), 2);
    for request in requests {
        assert!(request
            .to_ascii_lowercase()
            .contains("user-agent: vscode/1.x.x (antigravity/4.3.0)"));
        let form: std::collections::HashMap<_, _> =
            url::form_urlencoded::parse(request.split_once("\r\n\r\n").unwrap().1.as_bytes())
                .into_owned()
                .collect();
        assert_eq!(form["grant_type"], "refresh_token");
        assert_eq!(form["refresh_token"], "test-refresh");
        assert_eq!(form["client_id"], CLIENT_ID);
        assert_eq!(form["client_secret"], CLIENT_SECRET);
    }
}

#[tokio::test]
async fn refresh_preserves_persistent_invalid_grant_failure_after_two_attempts() {
    let (result, requests) = refresh_with_responses(vec![
        (400, r#"{"error":"invalid_grant"}"#),
        (
            400,
            r#"{"error":"invalid_grant","error_description":"revoked"}"#,
        ),
    ])
    .await;
    let error = result.unwrap_err();
    assert!(error.contains("invalid_grant"));
    assert!(error.contains("revoked"));
    assert_eq!(requests.len(), 2);
}

#[tokio::test]
async fn refresh_does_not_retry_unrelated_failures() {
    let (result, requests) =
        refresh_with_responses(vec![(400, r#"{"error":"invalid_client"}"#)]).await;
    assert!(result.unwrap_err().contains("invalid_client"));
    assert_eq!(requests.len(), 1);
}

#[test]
fn refresh_preserves_metadata_and_accepts_a_replacement_refresh_token() {
    let current = crate::models::TokenData::new(
        "old-access".into(),
        "old-refresh".into(),
        60,
        Some("fixture@example.com".into()),
        Some("fixture-project".into()),
        Some("fixture-user".into()),
    )
    .with_oauth_metadata(Some(DEFAULT_OAUTH_CLIENT_KEY.into()), Some("old-id".into()));

    let response = |refresh_token: Option<&str>| TokenResponse {
        access_token: "new-access".into(),
        expires_in: 3600,
        token_type: "Bearer".into(),
        refresh_token: refresh_token.map(str::to_owned),
        id_token: None,
        scope: None,
        oauth_client_key: Some(DEFAULT_OAUTH_CLIENT_KEY.into()),
    };
    let rotated = build_refreshed_token(&current, response(Some("new-refresh")));
    assert_eq!(rotated.refresh_token, "new-refresh");
    assert_eq!(rotated.id_token.as_deref(), Some("old-id"));
    assert_eq!(rotated.project_id.as_deref(), Some("fixture-project"));

    let unchanged = build_refreshed_token(&current, response(None));
    assert_eq!(unchanged.refresh_token, "old-refresh");
}

#[test]
fn detects_missing_aicode_scope_and_persists_granted_scopes() {
    assert!(!scope_lacks_aicode(None));
    assert!(!scope_lacks_aicode(Some("   ")));
    assert!(!scope_lacks_aicode(Some(
        "openid https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/aicode"
    )));
    assert!(scope_lacks_aicode(Some(
        "openid https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email"
    )));

    let current = crate::models::TokenData::new(
        "old-access".into(),
        "old-refresh".into(),
        60,
        Some("fixture@example.com".into()),
        None,
        None,
    );
    let refreshed = build_refreshed_token(
        &current,
        TokenResponse {
            access_token: "new-access".into(),
            expires_in: 3600,
            token_type: "Bearer".into(),
            refresh_token: None,
            id_token: None,
            scope: Some("openid https://www.googleapis.com/auth/cloud-platform".into()),
            oauth_client_key: Some(DEFAULT_OAUTH_CLIENT_KEY.into()),
        },
    );
    assert!(refreshed.lacks_aicode_scope());
    assert_eq!(
        refreshed.granted_scopes.as_deref(),
        Some("openid https://www.googleapis.com/auth/cloud-platform")
    );
}

fn current_token_with_scope_metadata() -> crate::models::TokenData {
    let mut token = crate::models::TokenData::new(
        "fixture-access".into(),
        "fixture-refresh".into(),
        60,
        Some("fixture@example.com".into()),
        Some("fixture-project".into()),
        Some("fixture-session".into()),
    )
    .with_oauth_metadata(
        Some(DEFAULT_OAUTH_CLIENT_KEY.into()),
        Some("fixture-id".into()),
    )
    .with_granted_scopes(Some("openid previous-scope".into()));
    token.is_gcp_tos = Some(true);
    token
}

fn refresh_response_with_scope(scope: Option<&str>) -> TokenResponse {
    TokenResponse {
        access_token: "fixture-access".into(),
        expires_in: 3600,
        token_type: "Bearer".into(),
        refresh_token: None,
        id_token: None,
        scope: scope.map(str::to_owned),
        oauth_client_key: Some(DEFAULT_OAUTH_CLIENT_KEY.into()),
    }
}

#[test]
fn refresh_without_scope_retains_previous_scope_and_other_metadata() {
    let current = current_token_with_scope_metadata();
    let refreshed = build_refreshed_token(&current, refresh_response_with_scope(None));

    assert_eq!(refreshed.access_token, current.access_token);
    assert_eq!(refreshed.granted_scopes, current.granted_scopes);
    assert_eq!(refreshed.refresh_token, current.refresh_token);
    assert_eq!(refreshed.id_token, current.id_token);
    assert_eq!(refreshed.oauth_client_key, current.oauth_client_key);
    assert_eq!(refreshed.email, current.email);
    assert_eq!(refreshed.project_id, current.project_id);
    assert_eq!(refreshed.session_id, current.session_id);
    assert_eq!(refreshed.is_gcp_tos, current.is_gcp_tos);
    assert_eq!(refreshed.expires_in, 3600);
}

#[test]
fn refresh_replaces_and_trims_scope_even_when_access_token_is_unchanged() {
    let current = current_token_with_scope_metadata();
    let refreshed = build_refreshed_token(
        &current,
        refresh_response_with_scope(Some(" \topenid   https://www.googleapis.com/auth/aicode\n")),
    );

    assert_eq!(refreshed.access_token, current.access_token);
    assert_eq!(
        refreshed.granted_scopes.as_deref(),
        Some("openid   https://www.googleapis.com/auth/aicode")
    );
    assert!(!refreshed.lacks_aicode_scope());
}

#[test]
fn explicit_blank_refresh_scope_clears_the_old_scope_instead_of_falling_back() {
    let current = current_token_with_scope_metadata();
    for scope in ["", " \n\t "] {
        let refreshed = build_refreshed_token(&current, refresh_response_with_scope(Some(scope)));
        assert!(refreshed.granted_scopes.is_none());
        assert!(!refreshed.lacks_aicode_scope());
    }
}

#[test]
fn token_json_without_scope_remains_compatible_and_scope_roundtrips() {
    let legacy = serde_json::json!({
        "access_token": "legacy-fixture-access",
        "refresh_token": "legacy-fixture-refresh",
        "expires_in": 3600,
        "expiry_timestamp": 1_800_000_000,
        "token_type": "Bearer",
        "email": "fixture@example.com"
    });
    let token: crate::models::TokenData = serde_json::from_value(legacy).unwrap();
    assert!(token.granted_scopes.is_none());
    assert!(!token.lacks_aicode_scope());
    let without_scope = serde_json::to_value(&token).unwrap();
    assert!(without_scope.get("granted_scopes").is_none());

    let scoped = token.with_granted_scopes(Some(
        " \topenid https://www.googleapis.com/auth/aicode\n".into(),
    ));
    let encoded = serde_json::to_value(&scoped).unwrap();
    assert_eq!(
        encoded["granted_scopes"],
        "openid https://www.googleapis.com/auth/aicode"
    );
    let decoded: crate::models::TokenData = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded.granted_scopes, scoped.granted_scopes);
    assert_eq!(decoded.expiry_timestamp, scoped.expiry_timestamp);
    assert_eq!(decoded.refresh_token, scoped.refresh_token);
    assert!(!decoded.lacks_aicode_scope());
}

#[test]
fn scope_matching_uses_complete_whitespace_delimited_items_case_insensitively() {
    assert!(!scope_lacks_aicode(Some(
        "openid\tHTTPS://WWW.GOOGLEAPIS.COM/AUTH/AICODE\nother-scope"
    )));
    assert!(scope_lacks_aicode(Some(
        "openid https://www.googleapis.com/auth/aicode-extra"
    )));
}
