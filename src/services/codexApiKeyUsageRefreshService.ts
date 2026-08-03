import type { CodexAccount } from '../types/codex';
import {
  isCodexApiKeyAccount,
  isCodexChatCompletionsApiKeyAccount,
  isCodexNewApiAccount,
} from '../types/codex';
import {
  findCodexModelProviderByBaseUrl,
  findCodexModelProviderById,
  listCodexModelProviders,
  queryCodexModelProviderUsage,
} from './codexModelProviderService';
import { isModelProviderUsageUnavailableError } from './modelProviderUsageService';
import {
  readCodexApiKeyUsageCache,
  writeCodexApiKeyUsageCache,
  type CodexApiKeyUsageCacheEntry,
} from './modelProviderUsageHelpers';

export { readCodexApiKeyUsageCache, writeCodexApiKeyUsageCache } from './modelProviderUsageHelpers';

export const CODEX_API_KEY_USAGE_REFRESHED_EVENT = 'codex-api-key-usage-refreshed';

export type CodexApiKeyUsageState = CodexApiKeyUsageCacheEntry;

function notifyCodexApiKeyUsageRefreshed(): void {
  window.dispatchEvent(new CustomEvent(CODEX_API_KEY_USAGE_REFRESHED_EVENT));
}

function isUsageEligibleApiKey(account: CodexAccount): boolean {
  return (
    isCodexApiKeyAccount(account) &&
    !isCodexNewApiAccount(account) &&
    !isCodexChatCompletionsApiKeyAccount(account) &&
    Boolean(account.openai_api_key?.trim())
  );
}

export async function refreshCodexApiKeyUsageForAccounts(
  accounts: CodexAccount[],
  options?: { force?: boolean },
): Promise<void> {
  const initialCache = readCodexApiKeyUsageCache();
  const eligibleAccounts = accounts.filter(
    (account) =>
      isUsageEligibleApiKey(account) &&
      (options?.force || !initialCache[account.id]?.unavailable),
  );
  if (eligibleAccounts.length === 0) return;

  const providers = await listCodexModelProviders();
  const updates: Record<string, CodexApiKeyUsageState> = {};

  for (const account of eligibleAccounts) {
    const provider =
      findCodexModelProviderById(providers, account.api_provider_id) ??
      findCodexModelProviderByBaseUrl(providers, account.api_base_url?.trim() ?? '');
    const baseUrl = provider?.baseUrl.trim() || account.api_base_url?.trim() || '';
    if (!baseUrl) continue;
    const apiKey = account.openai_api_key!.trim();

    try {
      const summary = await queryCodexModelProviderUsage({
        baseUrl,
        apiKey,
        integrationType: provider?.integrationType ?? null,
      });
      updates[account.id] = { loading: false, summary, updatedAt: Date.now() };
    } catch (error) {
      const unavailable = isModelProviderUsageUnavailableError(error);
      updates[account.id] = {
        loading: false,
        summary: initialCache[account.id]?.summary,
        error: unavailable ? undefined : String(error).replace(/^Error:\s*/, ''),
        unavailable,
        updatedAt: Date.now(),
      };
    }
  }

  if (Object.keys(updates).length === 0) return;

  const latestCache = readCodexApiKeyUsageCache();
  let changed = false;
  for (const [accountId, update] of Object.entries(updates)) {
    const latest = latestCache[accountId];
    if ((latest?.updatedAt ?? 0) > (update.updatedAt ?? 0)) continue;
    latestCache[accountId] = {
      ...update,
      summary: update.summary ?? latest?.summary,
    };
    changed = true;
  }
  if (!changed) return;

  writeCodexApiKeyUsageCache(latestCache);
  notifyCodexApiKeyUsageRefreshed();
}
