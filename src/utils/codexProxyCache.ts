import { CODEX_ACCOUNT_DISPLAY_CACHE, isCodexPendingOAuthAccount, type CodexAccount } from '../types/codex';

/** Proxy credentials belong to encrypted backend storage, never browser cache. */
export function withoutCachedProxySecret(account: CodexAccount): CodexAccount {
  const { egress_proxy_url: _secret, ...cached } = account;
  return cached;
}

export function toCodexAccountDisplayCache(account: CodexAccount): CodexAccount {
  const {
    openai_api_key: _apiKey,
    two_factor_secret: _mfa,
    account_password: _password,
    account_note: _note,
    phone_number: _phone,
    mail_url: _mail,
    agent_identity: agent,
    ...cached
  } = withoutCachedProxySecret(account);
  return {
    ...cached,
    [CODEX_ACCOUNT_DISPLAY_CACHE]: true,
    tokens: { id_token: '', access_token: '' },
    agent_identity: agent ? { ...agent, agent_private_key: '' } : undefined,
    authorization_status: isCodexPendingOAuthAccount(account) ? 'pending' : cached.authorization_status,
  } as CodexAccount;
}
