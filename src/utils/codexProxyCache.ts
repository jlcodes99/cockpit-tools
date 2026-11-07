import type { CodexAccount } from '../types/codex';

/** Proxy credentials belong to encrypted backend storage, never browser cache. */
export function withoutCachedProxySecret(account: CodexAccount): CodexAccount {
  const { egress_proxy_url: _secret, ...cached } = account;
  return cached;
}
