import { resolveOpenAIUsageConfig, resolveAccountUsageBaseUrl, isOpenAIOfficialUsageAccount } from '../utils/openaiOfficialUsage';
import type { CodexAccount } from '../types/codex';
import {
  isCodexApiKeyUsageQueryEligible,
  isApiKeyFunUsageAccount,
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
  sourceBinding?: string;
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
        sourceBinding: item.sourceBinding,
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
              sourceBinding: item.sourceBinding,
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
    if (!isUsageEligibleApiKey(account) && !initialCache[account.id]?.summary?.cockpit) return false;
    if (options?.force || isDeepSeekAccount(account) || isOpenAIOfficialUsageAccount(account) || isApiKeyFunUsageAccount(account)) return true;
    return !initialCache[account.id]?.unavailable;
  });
  if (eligibleAccounts.length === 0) return;

  const providers = await listCodexModelProviders();
  const updates: Record<string, CodexApiKeyUsageState> = {};

  for (const account of eligibleAccounts) {
    const provider =
      findCodexModelProviderById(providers, account.api_provider_id) ??
      findCodexModelProviderByBaseUrl(providers, resolveAccountUsageBaseUrl(account));
    const baseUrl = resolveAccountUsageBaseUrl(account, provider?.baseUrl);
    if (!baseUrl) continue;
    const apiKey = account.openai_api_key!.trim();
    const usageConfig = resolveOpenAIUsageConfig(provider, apiKey);
    if (isOpenAIOfficialUsageAccount(account, provider?.baseUrl) && !usageConfig) continue;

    try {
      const summary = await queryCodexModelProviderUsage({
        baseUrl,
        apiKey,
        usageApiKey: usageConfig?.apiKey,
        integrationType: provider?.integrationType ?? null,
      });
      updates[account.id] = { loading: false, sourceBinding: usageConfig?.binding, summary, updatedAt: Date.now() };
    } catch (error) {
      const unavailable = isModelProviderUsageUnavailableError(error);
      updates[account.id] = {
        loading: false,
        sourceBinding: usageConfig?.binding,
        summary: initialCache[account.id]?.sourceBinding === usageConfig?.binding ? initialCache[account.id]?.summary : undefined,
        error: unavailable ? undefined : String(error).replace(/^Error:\s*/, ''),
        unavailable,
        updatedAt: Date.now(),
      };
    }
  }

  if (Object.keys(updates).length === 0) return;

  const latestCache = readCodexApiKeyUsageCache();
  const latestProviders = await listCodexModelProviders();
  let changed = false;
  for (const [accountId, update] of Object.entries(updates)) {
    const account = accounts.find(item => item.id === accountId);
    if (update.sourceBinding && account) {
      const provider = findCodexModelProviderByBaseUrl(latestProviders, resolveAccountUsageBaseUrl(account));
      if (resolveOpenAIUsageConfig(provider, account.openai_api_key ?? '')?.binding !== update.sourceBinding) continue;
    }
    const latest = latestCache[accountId];
    if ((latest?.updatedAt ?? 0) > (update.updatedAt ?? 0)) continue;
    latestCache[accountId] = {
      ...update,
      summary: update.summary ?? (latest?.sourceBinding === update.sourceBinding ? latest?.summary : undefined),
    };
    changed = true;
  }
  if (!changed) return;

  writeCodexApiKeyUsageCache(latestCache);
  notifyCodexApiKeyUsageRefreshed();
}
