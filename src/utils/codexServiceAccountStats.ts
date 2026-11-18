import { isCodexChatgptPlanAccount, type CodexAccount } from '../types/codex';
import type { CodexLocalAccessAccountStats, CodexLocalAccessUsageStats } from '../types/codexLocalAccess';
import type { ChatGptPlanAccount } from '../services/chatgptPlanService';
import { chatGptPlanOverviewId, linkedChatGptPlanDisplayAccount, resolveCodexServiceDisplaySource } from './chatgptPlanOverview';

const USAGE_FIELDS = [
  'requestCount', 'successCount', 'failureCount', 'clientCanceledCount',
  'upstreamResponseFailedCount', 'streamIncompleteCount', 'totalLatencyMs',
  'textRequestCount', 'imageRequestCount', 'imageGenerationRequestCount',
  'imageEditRequestCount', 'imageGenerationCapabilityFailureCount',
  'inputTokens', 'outputTokens', 'totalTokens', 'cachedTokens', 'reasoningTokens', 'estimatedCostUsd',
] as const satisfies readonly (keyof CodexLocalAccessUsageStats)[];

export function codexServiceAccountSources(
  accounts: readonly CodexAccount[],
  grants: readonly ChatGptPlanAccount[],
  displaySourceIds?: ReadonlyMap<string, string>,
): ReadonlyMap<string, string> {
  const sourceByEndpoint = new Map<string, string>();
  for (const grant of grants) {
    const sourceId = grant.source_account_id;
    const sources = accounts.filter((account) => account.id === sourceId);
    if (!sourceId || sources.length !== 1
      || isCodexChatgptPlanAccount(sources[0])
      || linkedChatGptPlanDisplayAccount(grants, sourceId, displaySourceIds)?.id !== grant.id) continue;
    sourceByEndpoint.set(chatGptPlanOverviewId(grant.id), sourceId);
  }
  for (const endpoint of accounts) {
    if (!isCodexChatgptPlanAccount(endpoint) || !endpoint.api_service_source_account_id) continue;
    const source = resolveCodexServiceDisplaySource(endpoint, accounts, grants, displaySourceIds);
    if (source) sourceByEndpoint.set(endpoint.id, source.id);
    else sourceByEndpoint.delete(endpoint.id);
  }
  return sourceByEndpoint;
}

/**
 * Merge historical local/official usage under the original account. Each
 * endpoint snapshot is counted once; this never changes persisted log IDs.
 * Keys remain endpoint IDs so current members can read their source total.
 */
export function aggregateCodexServiceAccountStats(
  stats: readonly CodexLocalAccessAccountStats[],
  accounts: readonly CodexAccount[],
  grants: readonly ChatGptPlanAccount[],
  displaySourceIds?: ReadonlyMap<string, string>,
): Map<string, CodexLocalAccessAccountStats> {
  const accountById = new Map(accounts.map((account) => [account.id, account]));
  const sourceByEndpoint = codexServiceAccountSources(accounts, grants, displaySourceIds);
  const unique = new Map<string, CodexLocalAccessAccountStats>();
  for (const stat of stats) {
    const previous = unique.get(stat.accountId);
    if (!previous || stat.updatedAt > previous.updatedAt) unique.set(stat.accountId, stat);
  }
  const totals = new Map<string, CodexLocalAccessAccountStats>();
  for (const stat of unique.values()) {
    const sourceId = sourceByEndpoint.get(stat.accountId) ?? stat.accountId;
    let total = totals.get(sourceId);
    if (!total) {
      total = { accountId: sourceId, email: accountById.get(sourceId)?.email ?? stat.email,
        updatedAt: stat.updatedAt, usage: Object.fromEntries(USAGE_FIELDS.map((key) => [key, 0])) as unknown as CodexLocalAccessUsageStats };
      totals.set(sourceId, total);
    }
    total.updatedAt = Math.max(total.updatedAt, stat.updatedAt);
    for (const key of USAGE_FIELDS) {
      const value = stat.usage[key];
      if (Number.isFinite(value)) total.usage[key] += value;
    }
    if (stat.usage.inputText) {
      total.usage.inputText ??= { characters: 0, measuredRequests: 0 };
      total.usage.inputText.characters += stat.usage.inputText.characters;
      total.usage.inputText.measuredRequests += stat.usage.inputText.measuredRequests;
    }
  }
  const byEndpoint = new Map(totals);
  for (const [endpointId, sourceId] of sourceByEndpoint) {
    const total = totals.get(sourceId);
    if (total) byEndpoint.set(endpointId, total);
  }
  return byEndpoint;
}
