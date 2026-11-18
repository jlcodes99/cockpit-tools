import { isCodexApiKeyAccount, isCodexChatgptPlanAccount, type CodexAccount } from '../types/codex';
import { listModelProviderModels } from './modelProviderUsageService';

export function codexLaunchModelDiscoveryRequest(account?: CodexAccount | null) {
  if (!account || !isCodexApiKeyAccount(account) || isCodexChatgptPlanAccount(account)
    || account.api_provider_mode !== 'custom' || account.api_model_catalog?.length
    || account.upstream_grok_account_id
    || account.api_instance_access_mode === 'cdp') return null;
  const baseUrl = account.api_base_url?.trim();
  const apiKey = account.openai_api_key?.trim();
  if (!baseUrl || !apiKey) return null;
  return { launchAccountId: account.id, baseUrl, apiKey, wireApi: account.api_wire_api ?? undefined, authScheme: account.api_auth_scheme ?? undefined };
}

export function prefetchCodexLaunchModels(account: CodexAccount) {
  const request = codexLaunchModelDiscoveryRequest(account);
  if (request) void listModelProviderModels(request).catch(() => {
    // The backend logs discovery errors and retries on launch.
  });
}
