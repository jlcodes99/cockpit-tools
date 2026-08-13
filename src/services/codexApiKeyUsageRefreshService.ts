import type { CodexAccount } from '../types/codex';
import {
  isCodexApiKeyUsageQueryEligible,
  isDeepSeekAccount,
} from '../utils/codexDeepSeekAccess';
import {
  findCodexModelProviderByBaseUrl,
  findCodexModelProviderById,
  listCodexModelProviders,
  queryCodexModelProviderUsage,
  type CodexModelProviderUsageSummary,
} from './codexModelProviderService';
import { isModelProviderUsageUnavailableError } from './modelProviderUsageService';

export const CODEX_API_KEY_USAGE_CACHE_KEY = 'agtools.codex.apiKeyUsage.cache.v1';
export const CODEX_API_KEY_USAGE_REFRESHED_EVENT = 'codex-api-key-usage-refreshed';

export type CodexApiKeyUsageState = {
  loading: boolean;
  summary?: CodexModelProviderUsageSummary;
  error?: string;
  unavailable?: boolean;
  updatedAt?: number;
};

export function readCodexApiKeyUsageCache(): Record<string, CodexApiKeyUsageState> {
  try {
    const raw = localStorage.getItem(CODEX_API_KEY_USAGE_CACHE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    if (!parsed || typeof parsed !== 'object') return {};

    const next: Record<string, CodexApiKeyUsageState> = {};
    for (const [accountId, value] of Object.entries(parsed)) {
      if (!value || typeof value !== 'object') continue;
      const item = value as Omit<CodexApiKeyUsageState, 'loading'>;
      next[accountId] = {
        loading: false,
        summary: item.summary,
        error: typeof item.error === 'string' ? item.error : undefined,
        unavailable: item.unavailable === true,
        updatedAt:
          typeof item.updatedAt === 'number' && Number.isFinite(item.updatedAt)
            ? item.updatedAt
            : undefined,
      };
    }
    return next;
  } catch {
    return {};
  }
}

export function writeCodexApiKeyUsageCache(
  value: Record<string, CodexApiKeyUsageState>,
): void {
  try {
    localStorage.setItem(
      CODEX_API_KEY_USAGE_CACHE_KEY,
      JSON.stringify(
        Object.fromEntries(
          Object.entries(value).map(([accountId, item]) => [
            accountId,
            {
              summary: item.summary,
              error: item.error,
              unavailable: item.unavailable === true,
              updatedAt: item.updatedAt,
            },
          ]),
        ),
      ),
    );
  } catch {
    // Ignore cache persistence failures; quota refresh remains available.
  }
}

function notifyCodexApiKeyUsageRefreshed(): void {
  window.dispatchEvent(new CustomEvent(CODEX_API_KEY_USAGE_REFRESHED_EVENT));
}

export function isUsageEligibleApiKey(account: CodexAccount): boolean {
  return isCodexApiKeyUsageQueryEligible(account);
}

export async function refreshCodexApiKeyUsageForAccounts(
  accounts: CodexAccount[],
  options?: { force?: boolean },
): Promise<void> {
  const initialCache = readCodexApiKeyUsageCache();
  const eligibleAccounts = accounts.filter((account) => {
    if (!isUsageEligibleApiKey(account)) return false;
    if (options?.force || isDeepSeekAccount(account)) return true;
    return !initialCache[account.id]?.unavailable;
  });
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
