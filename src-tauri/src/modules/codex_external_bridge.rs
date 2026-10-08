//! Opt-in preservation of a verified, profile-wide Codex Web GPT launcher route.
//! No network requests or launcher execution: malformed/unverified state follows
//! the ordinary OAuth projection, which clears the previous endpoint.

use crate::models::codex::CodexAccount;
use serde_json::Value;
use std::io::Read;
use std::path::Path;

const MANAGED_ROUTE_COMMENT: &str =
    "# Managed by codex-chatgpt-web: Responses use the local bridge; Voice stays on ChatGPT.";
const MAX_CONFIG_BYTES: u64 = 256 * 1024;
const MAX_JOURNAL_BYTES: u64 = 64 * 1024;

fn read_bounded(path: &Path, limit: u64) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > limit {
        return None;
    }
    let mut content = String::new();
    file.take(limit + 1).read_to_string(&mut content).ok()?;
    (content.len() as u64 <= limit).then_some(content)
}

fn ordinary_oauth_auth(auth: &Value) -> bool {
    if auth.get("auth_mode").and_then(Value::as_str) != Some("chatgpt")
        || auth.get("OPENAI_API_KEY").is_some_and(|key| !key.is_null())
        || auth.get("agent_identity").is_some_and(|key| !key.is_null())
        || auth
            .get("personal_access_token")
            .is_some_and(|key| !key.is_null())
    {
        return false;
    }
    ["id_token", "access_token", "refresh_token"]
        .iter()
        .all(|key| {
            auth.get("tokens")
                .and_then(|tokens| tokens.get(key))
                .and_then(Value::as_str)
                .is_some_and(|token| !token.trim().is_empty())
        })
}

fn is_loopback_bridge_url(raw: &str) -> bool {
    if raw.split_once("://").is_none_or(|(_, rest)| {
        rest.split('/')
            .next()
            .is_some_and(|authority| authority.contains('@'))
    }) {
        return false;
    }
    let Ok(url) = url::Url::parse(raw) else {
        return false;
    };
    let loopback = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        None => false,
    };
    url.scheme() == "http"
        && loopback
        && url.port().is_some()
        && url.path() == "/v1"
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
}

fn verified_bridge_at(profile: &Path, journal_path: &Path) -> Option<String> {
    let config_path = profile.join("config.toml");
    let content = read_bounded(&config_path, MAX_CONFIG_BYTES)?;
    if !content
        .lines()
        .any(|line| line.trim() == MANAGED_ROUTE_COMMENT)
    {
        return None;
    }
    let doc = crate::modules::codex_config_format::read_codex_config_doc_from_str(&content).ok()?;
    // A selected custom provider owns its own endpoint, even if an old bridge
    // comment remains in the file. Never preserve that stale bridge implicitly.
    if doc
        .get("model_provider")
        .and_then(|item| item.as_str())
        .is_some_and(|provider| provider.trim() != "openai")
    {
        return None;
    }
    let endpoint = doc.get("openai_base_url")?.as_str()?.trim();
    if !is_loopback_bridge_url(endpoint) {
        return None;
    }
    let journal: Value =
        serde_json::from_str(&read_bounded(journal_path, MAX_JOURNAL_BYTES)?).ok()?;
    if journal.get("active").and_then(Value::as_bool) != Some(true)
        || journal.get("installed")?.get("openai_base_url")?.as_str()? != endpoint
    {
        return None;
    }
    let journal_config = Path::new(journal.get("configPath")?.as_str()?);
    if !journal_config.is_absolute()
        || std::fs::canonicalize(journal_config).ok()? != std::fs::canonicalize(config_path).ok()?
    {
        return None;
    }
    Some(endpoint.to_owned())
}

pub(super) fn preserved_url(
    profile: &Path,
    incoming: &CodexAccount,
    existing_auth: Option<&Value>,
    enabled: bool,
) -> Option<String> {
    let journal = dirs::home_dir()?.join(".codex-chatgpt-web/codex/integration-journal.json");
    preserved_url_at(profile, incoming, existing_auth, enabled, &journal)
}

fn preserved_url_at(
    profile: &Path,
    incoming: &CodexAccount,
    existing_auth: Option<&Value>,
    enabled: bool,
    journal: &Path,
) -> Option<String> {
    if !enabled
        || incoming.is_api_key_auth()
        || incoming.is_agent_identity_auth()
        || incoming.is_web_session_auth()
        || incoming.tokens.id_token.trim().is_empty()
        || incoming
            .tokens
            .refresh_token
            .as_deref()
            .is_none_or(|token| token.trim().is_empty())
        || !existing_auth.is_some_and(ordinary_oauth_auth)
    {
        return None;
    }
    verified_bridge_at(profile, journal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .join(format!("codex-external-bridge-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }
        fn seed(&self, endpoint: &str) {
            std::fs::write(self.0.join("config.toml"), format!(
                "{MANAGED_ROUTE_COMMENT}\nopenai_base_url = {endpoint:?}\nmodel = \"chatgpt-web/test\"\n"
            )).unwrap();
            self.journal(
                json!({"active": true, "configPath": self.0.join("config.toml"),
                "installed": {"openai_base_url": endpoint}}),
            );
        }
        fn journal(&self, journal: Value) {
            std::fs::write(self.0.join("journal.json"), journal.to_string()).unwrap();
        }
        fn url(&self) -> Option<String> {
            verified_bridge_at(&self.0, &self.0.join("journal.json"))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn verified_active_bridge_requires_matching_profile_endpoint_and_comment() {
        let fixture = Fixture::new();
        let endpoint = "http://127.0.0.1:17841/v1";
        fixture.seed(endpoint);
        assert_eq!(fixture.url().as_deref(), Some(endpoint));
        for journal in [
            json!({"active": false, "configPath": fixture.0.join("config.toml"), "installed": {"openai_base_url": endpoint}}),
            json!({"active": true, "configPath": fixture.0.join("config.toml"), "installed": {"openai_base_url": "http://127.0.0.1:9999/v1"}}),
            json!({"active": true, "configPath": fixture.0.join("other.toml"), "installed": {"openai_base_url": endpoint}}),
        ] {
            fixture.journal(journal);
            assert_eq!(fixture.url(), None);
        }
        fixture.seed(endpoint);
        std::fs::write(
            fixture.0.join("config.toml"),
            format!("openai_base_url = {endpoint:?}\n"),
        )
        .unwrap();
        assert_eq!(fixture.url(), None);
        fixture.seed(endpoint);
        let config = std::fs::read_to_string(fixture.0.join("config.toml")).unwrap();
        std::fs::write(
            fixture.0.join("config.toml"),
            format!("model_provider = \"other\"\n{config}"),
        )
        .unwrap();
        assert_eq!(fixture.url(), None);
    }

    #[test]
    fn remote_credential_bearing_or_ambiguous_endpoints_are_not_bridges() {
        for endpoint in [
            "https://127.0.0.1:17841/v1",
            "http://example.com:17841/v1",
            "http://127.0.0.1/v1",
            "http://user@127.0.0.1:17841/v1",
            "http://@127.0.0.1:17841/v1",
            "http://127.0.0.1:17841/v1?token=x",
            "http://127.0.0.1:17841/v1#x",
            "http://127.0.0.1:17841/other",
        ] {
            assert!(!is_loopback_bridge_url(endpoint), "{endpoint}");
        }
        assert!(is_loopback_bridge_url("http://[::1]:17841/v1"));
        assert!(is_loopback_bridge_url("http://localhost:17841/v1"));
    }

    #[test]
    fn journal_reads_are_bounded_and_only_oauth_credentials_qualify() {
        let fixture = Fixture::new();
        fixture.seed("http://127.0.0.1:17841/v1");
        std::fs::write(
            fixture.0.join("journal.json"),
            " ".repeat(MAX_JOURNAL_BYTES as usize + 1),
        )
        .unwrap();
        assert_eq!(fixture.url(), None);
        let oauth = json!({"auth_mode": "chatgpt", "OPENAI_API_KEY": null,
            "tokens": {"id_token": "id", "access_token": "access", "refresh_token": "refresh"}});
        assert!(ordinary_oauth_auth(&oauth));
        for auth in [
            json!({"auth_mode": "apikey", "OPENAI_API_KEY": "key"}),
            json!({"personal_access_token": "access"}),
            json!({"auth_mode": "chatgpt", "tokens": {"access_token": "access"}}),
        ] {
            assert!(!ordinary_oauth_auth(&auth));
        }
    }

    #[test]
    fn opt_in_oauth_projections_preserve_bridge_and_default_or_api_projection_clear_it() {
        use crate::models::codex::{CodexApiProviderMode, CodexTokens};
        let _lock = crate::modules::test_support::env_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let fixture = Fixture::new();
        let endpoint = "http://127.0.0.1:17841/v1";
        fixture.seed(endpoint);
        let config_path = fixture.0.join("config.toml");
        let config = std::fs::read_to_string(&config_path).unwrap();
        std::fs::write(&config_path, format!("cli_auth_credentials_store = \"file\"\nexperimental_realtime_webrtc_call_base_url = \"https://chatgpt.com/backend-api/codex\"\n{config}")).unwrap();
        std::fs::write(fixture.0.join("models_cache.json"), "native-and-web-cache").unwrap();
        let oauth = json!({"auth_mode": "chatgpt", "OPENAI_API_KEY": null,
            "tokens": {"id_token": "old-id", "access_token": "old-access", "refresh_token": "old-refresh"}});
        std::fs::write(fixture.0.join("auth.json"), oauth.to_string()).unwrap();
        let journal = fixture.0.join("journal.json");
        for id in ["oauth-b", "oauth-a"] {
            let account = CodexAccount::new(
                id.into(),
                "fixture@example.invalid".into(),
                CodexTokens {
                    id_token: format!("{id}-id"),
                    access_token: format!("{id}-access"),
                    refresh_token: Some(format!("{id}-refresh")),
                },
            );
            let auth: Value = serde_json::from_str(
                &std::fs::read_to_string(fixture.0.join("auth.json")).unwrap(),
            )
            .unwrap();
            let bridge = preserved_url_at(&fixture.0, &account, Some(&auth), true, &journal);
            assert_eq!(bridge.as_deref(), Some(endpoint));
            assert_eq!(
                preserved_url_at(&fixture.0, &account, Some(&auth), false, &journal),
                None
            );
            super::super::write_auth_file_to_dir_with_after_commit_and_bridge(
                &fixture.0,
                &account,
                || {},
                bridge,
            )
            .unwrap();
            let config = std::fs::read_to_string(&config_path).unwrap();
            assert!(config.contains(endpoint));
            assert!(config.contains("chatgpt-web/test"));
            assert!(config.contains("experimental_realtime_webrtc_call_base_url"));
            let auth: Value = serde_json::from_str(
                &std::fs::read_to_string(fixture.0.join("auth.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(auth["tokens"]["access_token"], format!("{id}-access"));
            assert_eq!(
                std::fs::read_to_string(fixture.0.join("models_cache.json")).unwrap(),
                "native-and-web-cache"
            );
        }
        let api = CodexAccount::new_api_key(
            "api".into(),
            "api@example.invalid".into(),
            "fixture-key".into(),
            CodexApiProviderMode::OpenaiBuiltin,
            Some("https://relay.example.invalid/v1".into()),
            None,
            None,
            vec![],
        );
        assert_eq!(
            preserved_url_at(&fixture.0, &api, Some(&oauth), true, &journal),
            None
        );
        super::super::write_auth_file_to_dir_with_after_commit_and_bridge(
            &fixture.0,
            &api,
            || {},
            None,
        )
        .unwrap();
        let config = std::fs::read_to_string(&config_path).unwrap();
        assert!(config.contains("https://relay.example.invalid/v1"));
        assert!(!config.contains(endpoint));
        let account = CodexAccount::new(
            "oauth-default".into(),
            "fixture@example.invalid".into(),
            CodexTokens {
                id_token: "id".into(),
                access_token: "access".into(),
                refresh_token: Some("refresh".into()),
            },
        );
        assert_eq!(
            preserved_url_at(
                &fixture.0,
                &account,
                Some(&json!({"auth_mode":"apikey"})),
                true,
                &journal
            ),
            None
        );
        super::super::write_auth_file_to_dir_with_after_commit_and_bridge(
            &fixture.0,
            &account,
            || {},
            None,
        )
        .unwrap();
        assert!(!std::fs::read_to_string(config_path)
            .unwrap()
            .contains("openai_base_url"));
    }
}
