import { getCodexPlanFilterKey, getCodexPlanBadgePresentation, isCodexChatgptPlanAccount, type CodexAccount } from '../types/codex';
import type { ChatGptPlanAccount } from '../services/chatgptPlanService';

export const chatGptPlanOverviewId = (id: string) => `chatgpt_plan_${id}`;

/** Member editors use original accounts. Retain only unresolved legacy members
 * so users can explicitly remove them; saved grants are never new candidates. */
export function codexServiceOriginalAccounts(accounts: readonly CodexAccount[], memberIds: readonly string[] = []): CodexAccount[] {
  const members = new Set(memberIds);
  return accounts.filter((account) => !isCodexChatgptPlanAccount(account) || members.has(account.id));
}

/** View-only source metadata. Never feed this object back into account persistence. */
export interface CodexServiceSourceDisplay {
  sourceId: string;
  planFilterKey: string;
  account: Pick<CodexAccount, 'email' | 'account_name' | 'plan_type' | 'auth_file_plan_type'
    | 'tags' | 'account_note' | 'quota' | 'subscription_active_until'
    | 'subscription_query_last_attempt_at' | 'subscription_query_last_success_at'
    | 'subscription_query_next_retry_at' | 'subscription_query_last_error' | 'egress_proxy'>;
}

export type CodexServiceDisplayAccount = CodexAccount & {
  service_source_display?: CodexServiceSourceDisplay;
};

// Re-projecting a row after an invalid binding must recover its real endpoint
// fields, not retain metadata from a previously valid source.
const originalEndpoints = new WeakMap<CodexAccount, CodexAccount>();

export function getCodexServiceEndpoint(account: CodexAccount): CodexAccount {
  return originalEndpoints.get(account) ?? account;
}

/** Render only: this preserves the endpoint's id, credentials and error state. */
export function getCodexServiceAccountDisplay(account: CodexAccount): CodexAccount {
  const display = (account as CodexServiceDisplayAccount).service_source_display;
  return display ? { ...account, ...display.account } : account;
}

export function getCodexServiceSourceId(account: CodexAccount): string {
  return (account as CodexServiceDisplayAccount).service_source_display?.sourceId
    ?? ((isCodexChatgptPlanAccount(account) ? account.api_service_source_account_id?.trim() : undefined) || account.id);
}

export function getCodexServicePlanFilterKey(account: CodexAccount): string {
  return (account as CodexServiceDisplayAccount).service_source_display?.planFilterKey ?? getCodexPlanFilterKey(account);
}

export type ChatGptPlanBindingStatus = 'valid' | 'pending' | 'invalid';
export function chatGptPlanBindingStatus(grant: ChatGptPlanAccount): ChatGptPlanBindingStatus | undefined {
  if (!grant.source_account_id) return undefined;
  if (grant.source_binding_error) return 'invalid';
  return grant.source_binding_valid === true ? 'valid' : 'pending';
}

/** Presentation metadata only. Never substitute this cache for source_binding_valid. */
export function reconcileChatGptPlanDisplaySources(grants: readonly ChatGptPlanAccount[], previous: ReadonlyMap<string, ChatGptPlanAccount> = new Map()) {
  const sourceCounts = new Map<string, number>();
  const idCounts = new Map<string, number>();
  for (const grant of grants) {
    if (grant.source_account_id) sourceCounts.set(grant.source_account_id, (sourceCounts.get(grant.source_account_id) ?? 0) + 1);
    idCounts.set(grant.id, (idCounts.get(grant.id) ?? 0) + 1);
  }
  const confirmed = new Map<string, ChatGptPlanAccount>();
  const displaySourceIds = new Map<string, string>();
  for (const grant of grants) {
    if (!grant.source_account_id || sourceCounts.get(grant.source_account_id) !== 1 || idCounts.get(grant.id) !== 1) continue;
    const status = chatGptPlanBindingStatus(grant);
    const saved = previous.get(grant.id);
    const unchanged = saved?.source_account_id === grant.source_account_id
      && saved.subject === grant.subject && saved.client_id === grant.client_id;
    if (status === 'valid' || (status === 'pending' && unchanged)) {
      confirmed.set(grant.id, status === 'valid' ? grant : saved!);
      displaySourceIds.set(grant.id, grant.source_account_id);
    }
  }
  return { confirmed, displaySourceIds };
}

/** A unique verified display association can survive a pending background recheck. */
export function linkedChatGptPlanDisplayAccount(grants: readonly ChatGptPlanAccount[], sourceId: string, displaySourceIds?: ReadonlyMap<string, string>): ChatGptPlanAccount | undefined {
  const matches = grants.filter((grant) => grant.source_account_id === sourceId);
  if (matches.length !== 1) return undefined;
  const grant = matches[0];
  if (grants.filter((item) => item.id === grant.id).length !== 1) return undefined;
  const status = chatGptPlanBindingStatus(grant);
  return status === 'valid' || (status === 'pending' && displaySourceIds?.get(grant.id) === sourceId) ? grant : undefined;
}

function withSourceDisplay(endpoint: CodexAccount, source: CodexAccount): CodexServiceDisplayAccount {
  endpoint = originalEndpoints.get(endpoint) ?? endpoint;
  const { email, account_name, plan_type, auth_file_plan_type, tags, account_note, quota,
    subscription_active_until, subscription_query_last_attempt_at, subscription_query_last_success_at,
    subscription_query_next_retry_at, subscription_query_last_error, egress_proxy } = source;
  const row: CodexServiceDisplayAccount = { ...endpoint, email: source.email, account_name: source.account_name,
    plan_type: source.plan_type, auth_file_plan_type: source.auth_file_plan_type,
    service_source_plan_badge: getCodexPlanBadgePresentation(source), tags: source.tags,
    service_source_display: { sourceId: source.id, planFilterKey: getCodexPlanFilterKey(source),
      account: { email, account_name, plan_type, auth_file_plan_type, tags, account_note, quota,
        subscription_active_until, subscription_query_last_attempt_at, subscription_query_last_success_at,
        subscription_query_next_retry_at, subscription_query_last_error, egress_proxy } } };
  originalEndpoints.set(row, endpoint);
  return row;
}

/** Source metadata is usable only for one unambiguous, verified association. */
export function resolveCodexServiceDisplaySource(endpoint: CodexAccount, accounts: readonly CodexAccount[], grants: readonly ChatGptPlanAccount[], displaySourceIds?: ReadonlyMap<string, string>): CodexAccount | undefined {
  if (!isCodexChatgptPlanAccount(endpoint)) return endpoint;
  // This attribution survives unlink/revoke, but cannot authorize a route.
  const historicalSource = endpoint.api_service_source_account_id?.trim();
  if (historicalSource) {
    const sources = accounts.filter((source) => source.id === historicalSource && !isCodexChatgptPlanAccount(source));
    return sources.length === 1 ? sources[0] : undefined;
  }
  const matches = grants.filter((grant) => chatGptPlanOverviewId(grant.id) === endpoint.id);
  if (matches.length !== 1) return undefined;
  const grant = matches[0];
  if (!grant.source_account_id || linkedChatGptPlanDisplayAccount(grants, grant.source_account_id, displaySourceIds)?.id !== grant.id) return undefined;
  const sources = accounts.filter((source) => source.id === grant.source_account_id && !isCodexChatgptPlanAccount(source));
  return sources.length === 1 ? sources[0] : undefined;
}

/** Copy public display fields only, retaining each endpoint's credentials and route. */
export function projectCodexServiceAccountDisplays(accounts: readonly CodexAccount[], grants: readonly ChatGptPlanAccount[], displaySourceIds?: ReadonlyMap<string, string>): CodexAccount[] {
  return accounts.map((endpoint) => {
    endpoint = originalEndpoints.get(endpoint) ?? endpoint;
    const source = resolveCodexServiceDisplaySource(endpoint, accounts, grants, displaySourceIds);
    return source && isCodexChatgptPlanAccount(endpoint) ? withSourceDisplay(endpoint, source) : endpoint;
  });
}

/** One visual row per source; every selected endpoint remains available to editors and ACLs. */
export function codexServiceDisplayRows(accounts: readonly CodexAccount[]): CodexAccount[] {
  const rows = new Map<string, CodexAccount>();
  for (const account of accounts) {
    const id = getCodexServiceSourceId(account);
    const previous = rows.get(id);
    // Prefer the selected official endpoint for its live transport health. The
    // source is display metadata, never a substitute route or credential.
    if (!previous || isCodexChatgptPlanAccount(account)) rows.set(id, account);
  }
  return [...rows.values()];
}

/** Feed the complete projected endpoint list, not a search-filtered row list. */
export function codexServiceRelatedEndpointIds(row: CodexAccount, accounts: readonly CodexAccount[]): string[] {
  const sourceId = getCodexServiceSourceId(row);
  return [...new Set([row.id, ...accounts.filter((account) => getCodexServiceSourceId(account) === sourceId).map((account) => account.id)])];
}

/** Checking a legacy dual row preserves both channels; unchecking removes both. */
export function toggleCodexServiceDisplaySelection(row: CodexAccount, accounts: readonly CodexAccount[], selected: ReadonlySet<string>, checked: boolean): Set<string> {
  const next = new Set(selected);
  const ids = codexServiceRelatedEndpointIds(row, accounts);
  if (!checked) ids.forEach((id) => next.delete(id));
  else if (!ids.some((id) => next.has(id))) next.add(row.id);
  return next;
}

/** Read-only overview rows. These never enter the Codex credential store. */
export function mergeChatGptPlanOverviewAccounts(
  accounts: readonly CodexAccount[],
  registrations: readonly ChatGptPlanAccount[],
  displaySourceIds?: ReadonlyMap<string, string>,
): CodexAccount[] {
  const references = new Map(accounts.filter(isCodexChatgptPlanAccount).map((account) => [account.id, account]));
  const result = accounts.filter((account) => !isCodexChatgptPlanAccount(account) && !account.upstream_grok_account_id?.trim());
  const seen = new Set<string>();
  const sourceIds = new Set(result.map((account) => account.id));
  for (const registration of registrations) {
    const savedSource = references.get(chatGptPlanOverviewId(registration.id))?.api_service_source_account_id;
    if (savedSource && sourceIds.has(savedSource)) continue;
    // Only an explicit, valid local link can fold a grant into a Codex card.
    // Identical email addresses are deliberately irrelevant here.
    if (registration.source_account_id
      && sourceIds.has(registration.source_account_id)
      && linkedChatGptPlanDisplayAccount(registrations, registration.source_account_id, displaySourceIds)?.id === registration.id) continue;
    const id = chatGptPlanOverviewId(registration.id);
    if (seen.has(id)) continue;
    seen.add(id);
    const saved = references.get(id);
    result.push({
      id,
      email: registration.email || registration.label,
      account_name: registration.label || registration.email || undefined,
      auth_mode: 'apikey',
      api_provider_mode: 'custom',
      api_provider_id: 'cockpit_chatgpt_plan',
      api_provider_name: 'ChatGPT',
      api_wire_api: 'responses',
      plan_type: saved?.plan_type,
      tags: saved?.tags,
      tokens: { access_token: '', id_token: '' },
      requires_reauth: !registration.connected || registration.needs_reauth,
      created_at: registration.created_at,
      last_used: saved?.last_used ?? registration.updated_at,
    });
  }
  return result;
}

export function linkedChatGptPlanAccount(registrations: readonly ChatGptPlanAccount[], sourceId: string): ChatGptPlanAccount | undefined {
  return linkedChatGptPlanDisplayAccount(registrations, sourceId);
}

/** Preserve the actual selected endpoint IDs in service editors and ACLs. */
export function codexServiceMemberAccounts(accounts: readonly CodexAccount[], grants: readonly ChatGptPlanAccount[], memberIds: readonly string[], displaySourceIds?: ReadonlyMap<string, string>): CodexAccount[] {
  const ids = new Set(accounts.map((account) => account.id));
  const members = new Set(memberIds);
  const hidden = new Set<string>();
  for (const endpoint of accounts) {
    if (!isCodexChatgptPlanAccount(endpoint)) continue;
    const source = resolveCodexServiceDisplaySource(endpoint, accounts, grants, displaySourceIds);
    if (!source || (members.has(endpoint.id) && members.has(source.id))) continue;
    hidden.add(members.has(endpoint.id) ? source.id : endpoint.id);
  }
  for (const grant of grants) {
    if (!grant.source_account_id || !ids.has(grant.source_account_id)) continue;
    if (linkedChatGptPlanDisplayAccount(grants, grant.source_account_id, displaySourceIds)?.id !== grant.id) continue;
    const projection = chatGptPlanOverviewId(grant.id);
    // Existing dual-member configurations remain visible until an explicit switch.
    if (members.has(projection) && members.has(grant.source_account_id)) continue;
    hidden.add(members.has(projection) ? grant.source_account_id : projection);
  }
  return projectCodexServiceAccountDisplays(accounts, grants, displaySourceIds).filter((account) => !hidden.has(account.id));
}

/** Apply the original account's search/group order to its actual service endpoint. */
export function resolveCodexServiceMemberViewAccounts(
  filteredAccounts: readonly CodexAccount[],
  endpointAccounts: readonly CodexAccount[],
  grants: readonly ChatGptPlanAccount[],
  displaySourceIds?: ReadonlyMap<string, string>,
): CodexAccount[] {
  const endpoints = new Map(endpointAccounts.map((account) => [account.id, account]));
  const sources = new Map(filteredAccounts.map((account) => [account.id, account]));
  const sourceByEndpoint = new Map<string, CodexAccount>();
  for (const endpoint of endpointAccounts) {
    const sourceId = isCodexChatgptPlanAccount(endpoint) ? endpoint.api_service_source_account_id : undefined;
    const source = sourceId ? sources.get(sourceId) : undefined;
    if (source && !isCodexChatgptPlanAccount(source)) sourceByEndpoint.set(endpoint.id, source);
  }
  for (const grant of grants) {
    if (!grant.source_account_id
      || linkedChatGptPlanDisplayAccount(grants, grant.source_account_id, displaySourceIds)?.id !== grant.id) continue;
    const source = sources.get(grant.source_account_id);
    if (source) sourceByEndpoint.set(chatGptPlanOverviewId(grant.id), source);
  }
  const result: CodexAccount[] = [];
  const seen = new Set<string>();
  const append = (endpoint: CodexAccount | undefined) => {
    if (!endpoint || seen.has(endpoint.id)) return;
    seen.add(endpoint.id);
    const source = sourceByEndpoint.get(endpoint.id);
    // Source metadata is display-only. Credentials, health, route type and
    // persisted membership keep the real endpoint's identity.
    result.push(source ? withSourceDisplay(endpoint, source) : endpoint);
  };
  for (const account of filteredAccounts) {
    append(endpoints.get(account.id));
    for (const [endpointId, source] of sourceByEndpoint) if (source.id === account.id) append(endpoints.get(endpointId));
    const grant = linkedChatGptPlanDisplayAccount(grants, account.id, displaySourceIds);
    if (grant) append(endpoints.get(chatGptPlanOverviewId(grant.id)));
  }
  return result;
}

export function codexServiceSourceAccount(account: CodexAccount, accounts: readonly CodexAccount[], grants: readonly ChatGptPlanAccount[]): CodexAccount | undefined {
  return resolveCodexServiceDisplaySource(account, accounts, grants);
}

export function chatGptPlanSourceMemberIds(sourceId: string, registrations: readonly ChatGptPlanAccount[], memberIds: ReadonlySet<string>, accounts: readonly CodexAccount[] = []): string[] {
  const grant = linkedChatGptPlanAccount(registrations, sourceId);
  return [...new Set([sourceId, ...(grant ? [chatGptPlanOverviewId(grant.id)] : []),
    ...accounts.filter((account) => isCodexChatgptPlanAccount(account) && account.api_service_source_account_id === sourceId).map((account) => account.id),
  ])].filter((id) => memberIds.has(id));
}

/** A background re-import must not silently re-enable the other transport. */
export function implicitCodexServiceAccountIds(ids: readonly string[], grants: readonly ChatGptPlanAccount[]): string[] {
  const officialSources = new Set(grants.filter((grant) => grant.in_service && grant.source_account_id).map((grant) => grant.source_account_id));
  return ids.filter((id) => !officialSources.has(id));
}

/** Independent grants are managed through their own dialog, never Codex bulk mutations. */
export function codexOverviewMutableAccountIds(accounts: readonly CodexAccount[]): string[] {
  return accounts.filter((account) => !isCodexChatgptPlanAccount(account)).map((account) => account.id);
}
