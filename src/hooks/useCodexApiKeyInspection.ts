import { useEffect, useLayoutEffect, useState, type Dispatch, type SetStateAction } from 'react';
import {
  subscribeCodexApiKeyInspectionRequests,
  type CodexApiKeyInspectionRequest,
} from '../utils/codexApiKeyInspection';
import { scrollElementIntoView } from '../utils/reducedMotion';

type InspectionTab = 'keys' | 'accounts';
type InspectionNoticeKey =
  | 'codex.apiService.inspection.internalScope'
  | 'codex.apiService.inspection.keyUnavailable'
  | 'codex.apiService.inspection.imageScope';
type InspectionApiKey = { id: string };

export function codexApiKeyInspectionCardId(apiKeyId: string): string {
  // An ID lookup avoids interpolating arbitrary persisted IDs into CSS selectors.
  return `codex-api-service-key:${apiKeyId}`;
}

function usesInternalScope(apiKeyId: string): boolean {
  return apiKeyId === '__cockpit_internal__' || apiKeyId === 'legacy'
    || apiKeyId === 'mixed_model_routing' || apiKeyId.startsWith('provider_gateway_');
}

export function resolveCodexApiKeyInspection(
  request: CodexApiKeyInspectionRequest,
  apiKeys: readonly InspectionApiKey[],
): { tab: InspectionTab; apiKeyId?: string; noticeKey?: InspectionNoticeKey } {
  if (request.requestKind === 'image_generation' || request.requestKind === 'image_edit') {
    return { tab: 'accounts', noticeKey: 'codex.apiService.inspection.imageScope' };
  }
  if (request.apiKeyId !== '__cockpit_internal__'
    && apiKeys.some((key) => key.id === request.apiKeyId)) {
    return { tab: 'keys', apiKeyId: request.apiKeyId };
  }
  return usesInternalScope(request.apiKeyId)
    ? { tab: 'accounts', noticeKey: 'codex.apiService.inspection.internalScope' }
    : { tab: 'keys', noticeKey: 'codex.apiService.inspection.keyUnavailable' };
}

/** Navigation only: retain policy drafts and wait for asynchronous collection loading. */
export function useCodexApiKeyInspection({
  apiKeys,
  activeTab,
  setActiveTab,
  expandedApiKeyPolicyIds,
  setExpandedApiKeyPolicyIds,
}: {
  apiKeys: readonly InspectionApiKey[] | null;
  activeTab: string;
  setActiveTab: (tab: InspectionTab) => void;
  expandedApiKeyPolicyIds: Set<string>;
  setExpandedApiKeyPolicyIds: Dispatch<SetStateAction<Set<string>>>;
}) {
  const [request, setRequest] = useState<CodexApiKeyInspectionRequest | null>(null);
  const [inspectionNoticeKey, setInspectionNoticeKey] = useState<InspectionNoticeKey | null>(null);

  useEffect(() => subscribeCodexApiKeyInspectionRequests((next) => {
    setInspectionNoticeKey(null);
    setRequest(next);
  }), []);

  useEffect(() => {
    if (!request || apiKeys === null) return;
    const target = resolveCodexApiKeyInspection(request, apiKeys);
    setActiveTab(target.tab);
    setInspectionNoticeKey(target.noticeKey ?? null);
    if (!target.apiKeyId) {
      setRequest(null);
      return;
    }
    const apiKeyId = target.apiKeyId;
    setExpandedApiKeyPolicyIds((current) => current.has(apiKeyId)
      ? current : new Set([...current, apiKeyId]));
  }, [request, apiKeys, setActiveTab, setExpandedApiKeyPolicyIds]);

  useLayoutEffect(() => {
    if (!request || apiKeys === null || activeTab !== 'keys') return;
    const target = resolveCodexApiKeyInspection(request, apiKeys);
    if (!target.apiKeyId || !expandedApiKeyPolicyIds.has(target.apiKeyId)) return;
    const card = document.getElementById(codexApiKeyInspectionCardId(target.apiKeyId));
    if (!card) return;
    scrollElementIntoView(card, { block: 'center', behavior: 'auto' });
    card.focus({ preventScroll: true });
    setRequest(null);
  }, [request, apiKeys, activeTab, expandedApiKeyPolicyIds]);

  return { inspectionNoticeKey, dismissInspectionNotice: () => setInspectionNoticeKey(null) };
}
