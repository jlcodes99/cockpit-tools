import type { CodexAccount } from '../types/codex';
import { isCodexApiKeyAccount } from '../types/codex';
import type { ModelProviderUsageSummary } from '../services/modelProviderUsageService';
import { codexRemoteBindingValue } from './codexRemoteBinding';

/** Shared display identity; a cached descriptor belongs to an exact URL/key. */
export function isCodexRemoteGateway(account: CodexAccount, summary?: ModelProviderUsageSummary): boolean {
  return isCodexApiKeyAccount(account) && summary?.cockpit?.product === 'cockpit-tools'
    && summary.sourceBinding === codexRemoteBindingValue(account.api_base_url || '', account.openai_api_key || '');
}

export function withCodexRemotePresentation<T extends { displayName: string; planLabel: string; planClass: string }>(
  account: CodexAccount, presentation: T, summary: ModelProviderUsageSummary | undefined,
  gatewayName: string,
): T {
  return isCodexRemoteGateway(account, summary)
    ? { ...presentation, displayName: gatewayName, planLabel: 'Cockpit', planClass: 'codex-cockpit' }
    : presentation;
}
