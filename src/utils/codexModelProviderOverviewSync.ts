import type { CodexAccount } from '../types/codex';
import type { CodexModelProvider, CodexModelProviderApiKey } from '../services/codexModelProviderService';

/** Import each saved credential once; a deleted overview account stays deleted. */
export async function reconcileCodexModelProviderOverview(
  providers: CodexModelProvider[],
  accounts: CodexAccount[],
  operations: {
    createAccount: (provider: CodexModelProvider, key: CodexModelProviderApiKey) => Promise<CodexAccount>;
    rememberAccount: (provider: CodexModelProvider, key: CodexModelProviderApiKey, accountId: string) => Promise<void>;
  },
): Promise<{ accounts: CodexAccount[]; failedProviders: string[] }> {
  const result = [...accounts];
  const failedProviders = new Set<string>();
  for (const provider of providers) {
    for (const key of provider.apiKeys) {
      const secret = key.apiKey.trim();
      if (!secret) continue;
      // The backend account id is derived from the key, not the provider URL.
      // Reuse it even if the same credential was saved under another provider.
      let account = result.find((item) =>
        item.auth_mode?.toLowerCase() === 'apikey' && item.openai_api_key?.trim() === secret,
      );
      if (key.overviewAccountId && !account) continue;
      if (account && account.id === key.overviewAccountId) continue;
      try {
        if (!account) {
          account = await operations.createAccount(provider, key);
          result.push(account);
        }
        await operations.rememberAccount(provider, key, account.id);
      } catch {
        // Never include credentials or backend error text in this result.
        failedProviders.add(provider.name);
      }
    }
  }
  return { accounts: result, failedProviders: [...failedProviders] };
}
