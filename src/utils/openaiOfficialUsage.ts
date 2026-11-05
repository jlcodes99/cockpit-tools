import type { CodexAccount } from '../types/codex';
import type { CodexModelProvider } from '../services/codexModelProviderService';

export const OPENAI_OFFICIAL_USAGE_BASE_URL = 'https://api.openai.com/v1';
export const OPENAI_ADMIN_KEYS_URL = 'https://platform.openai.com/settings/organization/admin-keys';

export function openAIUsageBinding(providerId: string, apiKeyId: string, keyVersion: number): string {
  // Re-query old snapshots that represented missing cost records as zero.
  return `${providerId}:${apiKeyId}:${keyVersion}:costs-v2`;
}

export function formatOpenAIUsageMoney(value: number): string {
  // The official API returns fractional cents; preserve small nonzero charges.
  const amount = value !== 0 && Math.abs(value) < 0.01
    ? Number(value.toPrecision(6)).toString()
    : value.toFixed(2);
  return `$${amount}`;
}

export function resolveOpenAIUsageConfig(provider: CodexModelProvider | null | undefined, apiKey: string) {
  if (!provider || !isOpenAIOfficialUsageUrl(provider.baseUrl)) return null;
  const key = provider.apiKeys.find(item => item.apiKey.trim() === apiKey.trim());
  if (!key?.usageApiKey?.trim()) return null;
  return { apiKey: key.usageApiKey.trim(), binding: openAIUsageBinding(provider.id, key.id, key.usageKeyVersion ?? 0) };
}

export function currentOpenAIUsageState<T extends { sourceBinding?: string }>(
  provider: CodexModelProvider | null | undefined, apiKey: string, state: T | undefined,
): T | undefined {
  const config = resolveOpenAIUsageConfig(provider, apiKey);
  return config && state?.sourceBinding === config.binding ? state : undefined;
}

export function resolveAccountUsageBaseUrl(
  account: Pick<CodexAccount, 'api_provider_mode' | 'api_base_url'>,
  providerBaseUrl?: string,
): string {
  return providerBaseUrl?.trim() || account.api_base_url?.trim() ||
    (account.api_provider_mode === 'openai_builtin' ? OPENAI_OFFICIAL_USAGE_BASE_URL : '');
}

export function isOpenAIOfficialUsageUrl(baseUrl: string): boolean {
  try {
    const url = new URL(baseUrl);
    return url.protocol === 'https:' && url.hostname === 'api.openai.com' &&
      !url.port && !url.username && !url.password && !url.search && !url.hash &&
      ['', '/', '/v1', '/v1/'].includes(url.pathname);
  } catch {
    return false;
  }
}

export function isOpenAIOfficialUsageAccount(
  account: Pick<CodexAccount, 'api_provider_id' | 'api_provider_mode' | 'api_base_url'>,
  providerBaseUrl?: string,
): boolean {
  return account.api_provider_id !== 'cockpit_chatgpt_plan' &&
    isOpenAIOfficialUsageUrl(resolveAccountUsageBaseUrl(account, providerBaseUrl));
}

export function openAIUsageErrorKey(error: string): string {
  if (error.includes('OPENAI_USAGE_NOT_CONFIGURED')) return 'codex.openaiUsage.notConfigured';
  if (error.includes('OPENAI_USAGE_CONFIG_CHANGED')) return 'codex.openaiUsage.configChanged';
  if (error.includes('OPENAI_USAGE_PERMISSION_REQUIRED')) return 'codex.openaiUsage.permissionRequired';
  if (error.includes('OPENAI_USAGE_INVALID_KEY')) return 'codex.openaiUsage.invalidKey';
  if (error.includes('OPENAI_USAGE_NETWORK_FAILED')) return 'codex.openaiUsage.networkFailed';
  return 'codex.openaiUsage.queryFailed';
}
