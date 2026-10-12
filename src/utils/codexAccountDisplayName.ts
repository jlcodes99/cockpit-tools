export interface CodexAccountDisplayIdentity {
  id: string;
  email?: string | null;
  account_name?: string | null;
  auth_mode?: string | null;
  api_base_url?: string | null;
  api_provider_name?: string | null;
  api_provider_mode?: string | null;
}

const generatedApiKeyName = /^api-key-[a-f0-9]{8}$/i;

/** Labels only: persisted credential identities must remain unchanged. */
export function resolveCodexAccountDisplayName(
  account: CodexAccountDisplayIdentity,
  apiKeyLabel: string = 'API',
): string {
  const name = account.account_name?.trim();
  if (name && !generatedApiKeyName.test(name)) return name;

  const email = account.email?.trim();
  const isApiKey = account.auth_mode?.trim().toLowerCase() === 'apikey' ||
    generatedApiKeyName.test(email ?? '') || generatedApiKeyName.test(name ?? '') ||
    generatedApiKeyName.test(account.id) || account.id.startsWith('codex_apikey_');
  if (!isApiKey) return email || account.id;

  const baseUrl = account.api_base_url?.trim();
  if (baseUrl) {
    try {
      const url = new URL(baseUrl);
      if ((url.protocol === 'http:' || url.protocol === 'https:') && url.host) return url.host;
    } catch {
      // Invalid imported addresses cannot provide a trustworthy display label.
    }
  } else if (account.api_provider_mode === 'openai_builtin') {
    return 'api.openai.com';
  }
  const provider = account.api_provider_name?.trim();
  return provider && !generatedApiKeyName.test(provider) ? provider : apiKeyLabel;
}

/** Historical logs can outlive their account; never expose their generated label. */
export function resolveCodexAccountReferenceDisplayName(
  accountId: string | null | undefined,
  email: string | null | undefined,
  displayNames: ReadonlyMap<string, string>,
  apiKeyLabel: string,
): string {
  const id = accountId?.trim() ?? '';
  const address = email?.trim() ?? '';
  return displayNames.get(id) || displayNames.get(address) ||
    resolveCodexAccountDisplayName({ id, email: address }, apiKeyLabel);
}

export function resolveCodexHealthIssueDisplayName(
  accountName: string | null | undefined,
  accountEmail: string | null | undefined,
  healthEmail: string | null | undefined,
  accountId: string,
  account?: CodexAccountDisplayIdentity,
  apiKeyLabel: string = 'API',
): string {
  return resolveCodexAccountDisplayName({
    ...account,
    id: account?.id || accountId,
    account_name: accountName ?? undefined,
    email: accountEmail?.trim() || healthEmail?.trim(),
  }, apiKeyLabel);
}
