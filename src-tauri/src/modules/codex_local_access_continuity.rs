// Both protocol engines share identity storage and credential ownership.
fn native_global_identity_path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?
        .join("codex_native_api")
        .join("identity.sqlite"))
}

fn continuity_account_owner(mut account: CodexAccount) -> Result<String, String> {
    let base = if codex_account::is_grok_upstream_provider(&account) {
        let id = account
            .upstream_grok_account_id
            .as_deref()
            .unwrap_or_default();
        let current = crate::modules::grok_account::load_account(id)
            .ok_or_else(|| "Continuity owner requires the linked Grok account".to_string())?;
        native_grok::bind_owner(&mut account, &current);
        codex_account::GROK_CLI_CHAT_PROXY_BASE_URL.to_string()
    } else if account.is_api_key_auth() {
        account
            .api_base_url
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "https://api.openai.com/v1".into())
    } else {
        "https://chatgpt.com/backend-api/codex".to_string()
    };
    Ok(native_account_owner(&account, &base))
}
