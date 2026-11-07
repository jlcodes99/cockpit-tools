import assert from 'node:assert/strict';
import test from 'node:test';
import { mergeChatGptPlanOverviewAccounts, codexOverviewMutableAccountIds, linkedChatGptPlanAccount, chatGptPlanSourceMemberIds, codexServiceMemberAccounts, resolveCodexServiceMemberViewAccounts, codexServiceSourceAccount, implicitCodexServiceAccountIds, reconcileChatGptPlanDisplaySources, projectCodexServiceAccountDisplays, getCodexServiceAccountDisplay, codexServiceDisplayRows, codexServiceRelatedEndpointIds, toggleCodexServiceDisplaySelection } from './chatgptPlanOverview';
import { createCodexOverviewAccountComparator, filterAndSortCodexOverviewAccounts } from './codexAccountOverview';
import { isCodexChatgptPlanAccount, getCodexPlanFilterKey, type CodexAccount } from '../types/codex';
import { resolveCodexMemberImagePolicy } from './codexLocalAccessAccounts';
import type { ChatGptPlanAccount } from '../services/chatgptPlanService';

const grant = { id: 'registration', email: 'same@example.com', label: 'same@example.com · registration', connected: true, needs_reauth: false, in_service: false, created_at: 2, updated_at: 3 } as ChatGptPlanAccount;
const ordinary = { id: 'codex', email: 'same@example.com', plan_type: 'TEAM', tokens: { access_token: 'codex-secret', id_token: '' }, created_at: 1, last_used: 1 } as CodexAccount;
const projection = { ...ordinary, id: 'chatgpt_plan_registration', auth_mode: 'apikey', api_provider_id: 'cockpit_chatgpt_plan', plan_type: 'Original Tier', openai_api_key: 'managed-placeholder', quota: { hourly_percentage: 100 } } as CodexAccount;

test('implicit import preserves the explicitly selected official transport, including pending identity refreshes', () => {
  const linked = { ...grant, source_account_id: ordinary.id, source_binding_valid: false, in_service: true };
  assert.deepEqual(implicitCodexServiceAccountIds([ordinary.id, 'other'], [linked]), ['other']);
  assert.deepEqual(implicitCodexServiceAccountIds([ordinary.id], [{ ...linked, in_service: false }]), [ordinary.id]);
  assert.deepEqual(implicitCodexServiceAccountIds([ordinary.id], [grant]), [ordinary.id]);
});

test('independent grants join the same overview once, including unjoined grants, without copying credentials or quota', () => {
  const rows = mergeChatGptPlanOverviewAccounts([ordinary, projection], [grant, grant]);
  assert.equal(rows.length, 2);
  assert.equal(rows[0], ordinary);
  assert.equal(rows[1].id, projection.id);
  assert.equal(rows[1].plan_type, 'Original Tier');
  assert.equal(rows[1].quota, undefined);
  assert.equal(rows[1].openai_api_key, undefined);
  assert.equal(rows[1].tokens.access_token, '');
  assert.equal(getCodexPlanFilterKey(rows[1]), 'CHATGPT_PLAN');
  assert.equal(mergeChatGptPlanOverviewAccounts([], [grant])[0].id, projection.id);
  assert.deepEqual(codexOverviewMutableAccountIds(rows), ['codex']);
});

test('only explicit valid links fold the grant into its original card, keeping original credentials and tier', () => {
  const linked = { ...grant, source_account_id: ordinary.id, source_binding_valid: true };
  const rows = mergeChatGptPlanOverviewAccounts([ordinary, projection], [linked]);
  assert.deepEqual(rows, [ordinary]);
  assert.equal(rows[0].tokens.access_token, 'codex-secret');
  assert.equal(rows[0].plan_type, 'TEAM');
  assert.equal(mergeChatGptPlanOverviewAccounts([ordinary], [{ ...linked, source_binding_valid: false }]).length, 2);
  assert.equal(mergeChatGptPlanOverviewAccounts([ordinary], [{ ...linked, source_binding_valid: undefined } as unknown as ChatGptPlanAccount]).length, 2);
  assert.equal(mergeChatGptPlanOverviewAccounts([], [linked])[0].id, projection.id);
  assert.equal(linkedChatGptPlanAccount([linked], ordinary.id), linked);
  assert.equal(linkedChatGptPlanAccount([linked, { ...linked, id: 'other' }], ordinary.id), undefined);
});

test('service editor keeps real endpoint IDs and never hides existing dual memberships', () => {
  const linked = { ...grant, source_account_id: ordinary.id, source_binding_valid: true };
  const accounts = [ordinary, projection];
  assert.deepEqual(codexServiceMemberAccounts(accounts, [linked], [projection.id]).map((row) => row.id), [projection.id]);
  assert.deepEqual(codexServiceMemberAccounts(accounts, [linked], [ordinary.id]), [ordinary]);
  assert.deepEqual(codexServiceMemberAccounts(accounts, [linked], [ordinary.id, projection.id]).map((row) => row.id), accounts.map((row) => row.id));
  assert.deepEqual(codexServiceMemberAccounts(accounts, [{ ...linked, source_binding_valid: false }], [projection.id]), accounts);
  assert.equal(codexServiceSourceAccount(projection, accounts, [linked]), ordinary);
  assert.equal(codexServiceSourceAccount(projection, accounts, [grant]), undefined);
  assert.deepEqual(chatGptPlanSourceMemberIds(ordinary.id, [linked], new Set([projection.id, 'unrelated'])), [projection.id]);
});

test('ordinary overview search, type filters and sorting include plan rows without treating them as Free', () => {
  const rows = mergeChatGptPlanOverviewAccounts([ordinary], [grant]);
  const options = { accounts: rows, groups: [], searchQuery: 'same@', filterTypes: [] as string[], tagFilter: [], groupFilter: [], activeGroupId: null, resolveDisplayName: (row: CodexAccount) => row.account_name || row.email,
    compareAccounts: createCodexOverviewAccountComparator({ sortBy: 'created_at', sortDirection: 'desc', customSortOrder: [], currentAccountId: null, resolveSubscriptionTimestamp: () => null }) };
  assert.deepEqual(filterAndSortCodexOverviewAccounts(options).map((row) => row.id), ['chatgpt_plan_registration', 'codex']);
  assert.deepEqual(filterAndSortCodexOverviewAccounts({ ...options, filterTypes: ['CHATGPT_PLAN'] }).map((row) => row.id), ['chatgpt_plan_registration']);
  assert.deepEqual(filterAndSortCodexOverviewAccounts({ ...options, filterTypes: ['FREE'] }), []);
  assert.deepEqual(filterAndSortCodexOverviewAccounts({ ...options, searchQuery: 'missing' }), []);
});


test('a source-only overview filter still displays its selected official service endpoint', () => {
  const linked = { ...grant, source_account_id: ordinary.id, source_binding_valid: true };
  const source = { ...ordinary, account_name: 'My account', tags: ['work'] };
  const endpoint = { ...projection, account_name: 'independent grant suffix', quota: undefined, tokens: { access_token: '', id_token: '' } };
  const endpoints = codexServiceMemberAccounts([source, endpoint], [linked], [endpoint.id]);
  const rows = resolveCodexServiceMemberViewAccounts([source], endpoints, [linked]);
  assert.equal(rows.length, 1, 'authorization must not remove the original account from the picker');
  assert.equal(rows[0].id, endpoint.id, 'selection and saving use the official endpoint');
  assert.equal(rows[0].account_name, 'My account');
  assert.equal(rows[0].email, source.email);
  assert.equal(rows[0].plan_type, 'TEAM');
  assert.deepEqual(rows[0].tags, ['work']);
  assert.equal(rows[0].quota, undefined, 'Codex quota is not copied into official access');
  assert.equal(rows[0].tokens.access_token, '');
  assert.equal(isCodexChatgptPlanAccount(rows[0]), true);
  assert.equal(resolveCodexMemberImagePolicy(rows[0], 'enabled'), 'disabled');
  assert.equal(new Set([endpoint.id]).has(rows[0].id), true, 'the existing member remains checked');
  assert.equal(source.tokens.access_token, 'codex-secret');
  assert.equal(endpoint.account_name, 'independent grant suffix', 'display mapping never mutates the persisted endpoint');
});

test('member-view mapping preserves source filters, order, deduplication and existing dual memberships', () => {
  const linked = { ...grant, source_account_id: ordinary.id, source_binding_valid: true };
  const other = { ...ordinary, id: 'other', email: 'other@example.com' };
  const endpoints = codexServiceMemberAccounts([ordinary, projection, other], [linked], [projection.id]);
  assert.deepEqual(resolveCodexServiceMemberViewAccounts([], endpoints, [linked]), [], 'a filtered-out account must stay filtered out');
  assert.deepEqual(resolveCodexServiceMemberViewAccounts([other, ordinary], endpoints, [linked]).map((row) => row.id), ['other', projection.id]);
  assert.deepEqual(resolveCodexServiceMemberViewAccounts([projection, ordinary], endpoints, [linked]).map((row) => row.id), [projection.id]);
  const dual = codexServiceMemberAccounts([ordinary, projection], [linked], [ordinary.id, projection.id]);
  assert.deepEqual(resolveCodexServiceMemberViewAccounts([ordinary], dual, [linked]).map((row) => row.id), [ordinary.id, projection.id]);
  assert.deepEqual(resolveCodexServiceMemberViewAccounts([ordinary], [ordinary, projection], [{ ...linked, source_binding_valid: false }]), [ordinary]);
  assert.deepEqual(resolveCodexServiceMemberViewAccounts([ordinary], [ordinary, projection], [{ ...linked, id: 'first' }, linked]), [ordinary], 'ambiguous bindings never choose a grant');
});


test('pending rechecks retain only an unchanged previously verified display association', () => {
  const linked = { ...grant, source_account_id: ordinary.id, source_binding_valid: true, subject: 'subject', client_id: 'client' };
  const verified = reconcileChatGptPlanDisplaySources([linked]);
  const pending = { ...linked, source_binding_valid: false, source_binding_error: null };
  const cached = reconcileChatGptPlanDisplaySources([pending], verified.confirmed);
  assert.equal(cached.displaySourceIds.get(linked.id), ordinary.id);
  assert.equal(linkedChatGptPlanAccount([pending], ordinary.id), undefined, 'pending metadata does not become a valid authorization');
  const members = codexServiceMemberAccounts([ordinary, projection], [pending], [projection.id], cached.displaySourceIds);
  assert.deepEqual(members.map((row) => row.id), [projection.id]);
  assert.equal(members[0].plan_type, ordinary.plan_type);
  assert.equal(members[0].service_source_plan_badge?.label, 'TEAM');
  assert.equal(members[0].tokens, projection.tokens);
  assert.equal(members[0].quota, projection.quota);
  assert.equal(reconcileChatGptPlanDisplaySources([pending]).displaySourceIds.size, 0, 'cold pending reads never invent a verified link');
  for (const rows of [
    [{ ...pending, source_binding_error: 'CHATGPT_PLAN_SOURCE_CHANGED' }],
    [{ ...pending, source_account_id: 'other' }], [{ ...pending, subject: 'other' }],
    [{ ...pending, client_id: 'other' }], [{ ...pending, source_account_id: undefined }],
    [pending, { ...pending, id: 'duplicate' }], [],
  ]) {
    assert.equal(reconcileChatGptPlanDisplaySources(rows, verified.confirmed).displaySourceIds.size, 0);
  }
});

test('official display rows reuse all source badge variants without copying credentials or quota', () => {
  for (const raw of ['plus', 'team', 'pro-100', 'pro-500']) {
    const source = { ...ordinary, plan_type: raw };
    const linked = { ...grant, source_account_id: source.id, source_binding_valid: true };
    const rows = projectCodexServiceAccountDisplays([source, projection], [linked]);
    assert.equal(rows[1].tokens, projection.tokens);
    assert.equal(rows[1].quota, projection.quota);
    assert.equal(rows[1].api_provider_id, projection.api_provider_id);
    assert.equal(rows[1].plan_type, raw);
    assert.equal(rows[1].service_source_plan_badge?.className.includes('api-key'), false);
    assert.equal(source.tokens.access_token, 'codex-secret');
  }
});

test('source details are available for display without replacing endpoint health, route or credentials', () => {
  const source = { ...ordinary, account_name: 'Original account', tags: ['work'], account_note: 'Primary account',
    quota: { hourly_percentage: 31, weekly_percentage: 48 }, quota_error: { message: '401 invalid_token', timestamp: 1 },
    egress_proxy_url: 'http://source:secret@proxy.test', egress_proxy: { protocol: 'http', name: 'US node', server: 'proxy.test' },
    subscription_active_until: '2027-01-01T00:00:00Z', subscription_query_last_success_at: 5 };
  const endpoint = { ...projection, quota: undefined, requires_reauth: true, reauth_reason: 'grant expired',
    api_base_url: 'https://official.test/v1', quota_error: { message: '429 official limit', timestamp: 4 },
    egress_proxy_url: 'http://endpoint:secret@official-proxy.test' };
  const linked = { ...grant, source_account_id: source.id, source_binding_valid: true };
  const row = projectCodexServiceAccountDisplays([source, endpoint], [linked])[1];
  const display = getCodexServiceAccountDisplay(row);
  assert.equal(display.account_note, source.account_note);
  assert.deepEqual(display.quota, source.quota);
  assert.equal(display.subscription_active_until, source.subscription_active_until);
  assert.deepEqual(display.egress_proxy, source.egress_proxy);
  for (const key of ['id', 'tokens', 'openai_api_key', 'api_base_url', 'auth_mode', 'api_provider_id',
    'requires_reauth', 'reauth_reason', 'quota_error', 'egress_proxy_url'] as const) {
    assert.equal(row[key], endpoint[key], `${key} must retain the endpoint value`);
    assert.equal(display[key], endpoint[key], `${key} must retain the endpoint value in render data`);
  }
  assert.equal(row.quota, endpoint.quota, 'source quota never enters operational endpoint data');
  const invalid = projectCodexServiceAccountDisplays([source, row], [{ ...linked, source_binding_error: 'CHATGPT_PLAN_SOURCE_CHANGED' }])[1];
  assert.equal(invalid, endpoint, 'revoked display associations discard every old projected field');
  assert.equal(getCodexServiceAccountDisplay(invalid).account_note, endpoint.account_note);
});

test('dual transports render once while group selection preserves or removes every real member', () => {
  const linked = { ...grant, source_account_id: ordinary.id, source_binding_valid: true };
  const endpoints = projectCodexServiceAccountDisplays([ordinary, projection], [linked]);
  const rows = codexServiceDisplayRows(endpoints);
  assert.deepEqual(rows.map((row) => row.id), [projection.id]);
  assert.deepEqual(new Set(codexServiceRelatedEndpointIds(rows[0], endpoints)), new Set([ordinary.id, projection.id]));
  const selected = new Set([ordinary.id, projection.id, 'other']);
  assert.deepEqual(toggleCodexServiceDisplaySelection(rows[0], endpoints, selected, true), selected);
  assert.deepEqual(toggleCodexServiceDisplaySelection(rows[0], endpoints, selected, false), new Set(['other']));
  assert.deepEqual(toggleCodexServiceDisplaySelection(rows[0], endpoints, new Set(), true), new Set([projection.id]));
  assert.equal(selected.size, 3, 'a draft toggle never mutates persisted selection');
});

test('a verified pending display link folds into the source card without granting authorization', () => {
  const linked = { ...grant, source_account_id: ordinary.id, source_binding_valid: true };
  const pending = { ...linked, source_binding_valid: false };
  const previous = reconcileChatGptPlanDisplaySources([linked]);
  const cache = reconcileChatGptPlanDisplaySources([pending], previous.confirmed);
  assert.deepEqual(mergeChatGptPlanOverviewAccounts([ordinary, projection], [pending], cache.displaySourceIds), [ordinary]);
  assert.equal(linkedChatGptPlanAccount([pending], ordinary.id), undefined);
  assert.equal(linkedChatGptPlanAccount([linked, { ...pending, id: 'second' }], ordinary.id), undefined);
});

test('backend source attribution keeps one original row after unlink without granting authorization', () => {
  const endpoint = { ...projection, api_service_source_account_id: ordinary.id };
  const disconnected = { ...grant, connected: false, source_account_id: undefined };
  const accounts = [ordinary, endpoint];
  assert.deepEqual(mergeChatGptPlanOverviewAccounts(accounts, [disconnected]), [ordinary]);
  const rows = projectCodexServiceAccountDisplays(accounts, [disconnected]);
  const selected = rows.find((account) => account.id === endpoint.id)!;
  assert.equal(getCodexServiceAccountDisplay(selected).email, ordinary.email);
  assert.equal(selected.id, endpoint.id);
  assert.equal(selected.openai_api_key, 'managed-placeholder');
  assert.equal(linkedChatGptPlanAccount([disconnected], ordinary.id), undefined);
  assert.deepEqual(codexServiceDisplayRows(rows).map((account) => account.id), [endpoint.id]);
  assert.deepEqual(chatGptPlanSourceMemberIds(ordinary.id, [disconnected], new Set([endpoint.id]), accounts), [endpoint.id]);
  assert.deepEqual(resolveCodexServiceMemberViewAccounts([ordinary], [endpoint], [disconnected]).map((account) => account.id), [endpoint.id]);
  assert.deepEqual(codexServiceMemberAccounts(accounts, [disconnected], [ordinary.id]).map((account) => account.id), [ordinary.id]);
});

test('missing historical source stays isolated and cannot borrow another same-email account', () => {
  const endpoint = { ...projection, api_service_source_account_id: 'deleted' };
  assert.equal(codexServiceSourceAccount(endpoint, [ordinary, endpoint], []), undefined);
  assert.equal(getCodexServiceAccountDisplay(projectCodexServiceAccountDisplays([ordinary, endpoint], [])[1]).plan_type, projection.plan_type);
});
