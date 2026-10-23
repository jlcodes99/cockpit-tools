// Specific upstream revocation is terminal for this credential. Generic
// 401/403, proxy errors and refresh-token ownership conflicts are not revocation.
pub(crate) fn account_has_known_access_token_revocation(account: &CodexAccount) -> bool {
    if account.is_api_key_auth()
        || account.is_agent_identity_auth()
        || account.is_web_session_auth()
    {
        return false;
    }
    let Some(error) = account.quota_error.as_ref() else {
        return false;
    };
    let code = error
        .code
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let message = error.message.to_ascii_lowercase();
    matches!(code.as_str(), "token_revoked" | "token_invalidated")
        && !message.contains("refresh_token")
        && !message.contains("refresh token")
        && !message.contains("刷新 token")
        && !message.contains("token 刷新")
}

pub(crate) fn observe_known_access_token_revocation(account: &mut CodexAccount) {
    if account_has_known_access_token_revocation(account) {
        account.requires_reauth = true;
        account.reauth_reason = Some(crate::modules::i18n::translate(
            &crate::modules::config::get_user_config().language,
            "codex.authError.refreshTokenInvalidated",
            &[],
        ));
    }
}

fn reject_known_access_token_revocation(account: &CodexAccount) -> Result<(), String> {
    if account_has_known_access_token_revocation(account) {
        return Err(crate::modules::i18n::translate(
            &crate::modules::config::get_user_config().language,
            "codex.authError.refreshTokenInvalidated",
            &[],
        ));
    }
    Ok(())
}
