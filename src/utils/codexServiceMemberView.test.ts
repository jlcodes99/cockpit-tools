import assert from 'node:assert/strict';
import test from 'node:test';
import type { CodexAccount } from '../types/codex';
import type { CodexAccountGroup } from '../services/codexAccountGroupService';
import type { CodexLocalAccessAccountHealth } from '../types/codexLocalAccess';
import type { ChatGptPlanAccount } from '../services/chatgptPlanService';
import { codexServiceMemberAccounts, projectCodexServiceAccountDisplays, toggleCodexServiceDisplaySelection } from './chatgptPlanOverview';
import { createCodexOverviewAccountComparator } from './codexAccountOverview';
import { VALID_ACCOUNTS_FILTER_VALUE } from './accountValidityFilter';
import { buildCodexServiceMemberView, filterCodexServiceMemberView, countCodexServiceSourceAccounts, needsCodexServiceMemberAuthorizationChoice } from './codexServiceMemberView';

const source = { id: 'source', email: 'original@example.com', plan_type: 'Plus', auth_mode: 'oauth',
  tags: ['work'], tokens: { access_token: 'source-access', id_token: 'source-id' },
  quota: { hourly_percentage: 65, weekly_percentage: 80 }, created_at: 1, last_used: 1 } as CodexAccount;
const endpoint = { id: 'chatgpt_plan_grant', email: 'grant@example.com', plan_type: '', auth_mode: 'apikey',
  api_provider_id: 'cockpit_chatgpt_plan', api_wire_api: 'responses',
  tokens: { access_token: 'endpoint-access', id_token: '' }, created_at: 100, last_used: 100 } as CodexAccount;
const other = { ...source, id: 'other', email: 'other@example.com', plan_type: 'TEAM', created_at: 10 };
const grant = { id: 'grant', source_account_id: source.id, source_binding_valid: true } as ChatGptPlanAccount;
const group = { id: 'work', name: 'Work group', accountIds: [source.id] } as CodexAccountGroup;
const healthy = (accountId: string, available = true) => ({ accountId, available, schedulerAvailable: available, cooldowns: [] }) as unknown as CodexLocalAccessAccountHealth;
const options = { searchQuery: '', filterTypes: [] as string[], tagFilter: [] as string[], groupFilter: [] as string[],
  resolveDisplayName: (account: CodexAccount) => account.email, compareAccounts: (left: CodexAccount, right: CodexAccount) => right.created_at - left.created_at };

function model(accounts: CodexAccount[], memberIds: string[], accountHealth: CodexLocalAccessAccountHealth[] = [], grants = [grant]) {
  return buildCodexServiceMemberView({
    accounts: codexServiceMemberAccounts(accounts, grants, memberIds), sourceAccounts: accounts, groups: [group], memberIds, accountHealth,
  });
}

test('PLUS + valid + group filters retain healthy official access when source OAuth is invalid', () => {
  const failedSource = { ...source, requires_reauth: true, quota_error: { code: 'invalid_token', message: '401 invalid_token', timestamp: 1 } };
  const view = model([failedSource, endpoint, other], [endpoint.id], [healthy(endpoint.id)]);
  assert.equal(view.counts.all, 2);
  assert.equal(view.counts.VALID, 2);
  assert.equal(view.counts.counts.PLUS, 1);
  assert.equal(view.counts.counts.CHATGPT_PLAN, undefined);
  const rows = filterCodexServiceMemberView(view, { ...options,
    filterTypes: ['PLUS', VALID_ACCOUNTS_FILTER_VALUE], groupFilter: ['work'], tagFilter: ['work'], activeGroupId: 'work',
  });
  assert.deepEqual(rows.map((account) => account.id), [endpoint.id]);
  assert.equal(rows[0].tokens, endpoint.tokens);
  assert.equal(rows[0].auth_mode, 'apikey', 'filtering never impersonates the source credential kind');
  assert.equal(rows[0].quota, undefined, 'source quota belongs to the read-only display metadata');
  assert.equal(failedSource.requires_reauth, true);
});

test('only selected real channels affect valid filtering and either selected healthy channel is enough', () => {
  const failedSource = { ...source, requires_reauth: true };
  const failedEndpoint = { ...endpoint, requires_reauth: true };
  assert.equal(model([failedSource, endpoint], [source.id], [healthy(endpoint.id)]).counts.VALID, 0,
    'an unselected official grant cannot make a selected broken gateway healthy');
  assert.equal(model([source, failedEndpoint], [source.id, endpoint.id], [healthy(source.id), healthy(endpoint.id, false)]).counts.VALID, 1);
  const officialOnly = model([source, failedEndpoint], [endpoint.id], [healthy(endpoint.id, false)]);
  assert.equal(officialOnly.counts.ERROR, 1);
  assert.deepEqual(filterCodexServiceMemberView(officialOnly, { ...options, filterTypes: [VALID_ACCOUNTS_FILTER_VALUE] }), []);
});

test('source search, groups, timestamps and custom ordering survive an endpoint with different metadata', () => {
  const view = model([source, endpoint, other], [endpoint.id]);
  assert.deepEqual(filterCodexServiceMemberView(view, { ...options, searchQuery: 'work group' }).map((row) => row.id), [endpoint.id]);
  assert.deepEqual(filterCodexServiceMemberView(view, { ...options, searchQuery: 'original@' }).map((row) => row.id), [endpoint.id]);
  assert.deepEqual(filterCodexServiceMemberView(view, options).map((row) => row.id), [other.id, endpoint.id], 'use source created_at, not newer grant created_at');
  const compareAccounts = createCodexOverviewAccountComparator({ sortBy: 'custom', sortDirection: 'desc',
    customSortOrder: [source.id, other.id], currentAccountId: null, resolveSubscriptionTimestamp: () => null });
  assert.deepEqual(filterCodexServiceMemberView(view, { ...options, compareAccounts }).map((row) => row.id), [endpoint.id, other.id]);
  assert.deepEqual(filterCodexServiceMemberView(view, { ...options, activeGroupId: 'missing' }), []);
});

test('zero quota and expired filters use the source display data without manufacturing official quota', () => {
  const expired = { ...source, subscription_active_until: '2001-01-01T00:00:00Z', quota: { hourly_percentage: 0, weekly_percentage: 0 } };
  const view = model([expired, endpoint, other], [endpoint.id]);
  assert.equal(view.counts.ZERO_QUOTA, 1);
  assert.equal(view.counts.EXPIRED, 1);
  for (const filter of ['ZERO_QUOTA', 'EXPIRED']) {
    assert.deepEqual(filterCodexServiceMemberView(view, { ...options, filterTypes: [filter] }).map((row) => row.id), [endpoint.id]);
  }
  const unbound = model([expired, endpoint], [endpoint.id], [], []);
  assert.equal(unbound.counts.all, 2);
  assert.equal(unbound.counts.ZERO_QUOTA, 1, 'an unbound grant never borrows source allowance');
  assert.equal(unbound.counts.counts.CHATGPT_PLAN, 1);
});

test('scope counts and legacy groups collapse identities without changing actual endpoint arrays', () => {
  const accounts = projectCodexServiceAccountDisplays([source, endpoint, other], [grant]);
  const ids = [source.id, endpoint.id, endpoint.id, other.id, 'missing'];
  const before = [...ids];
  assert.equal(countCodexServiceSourceAccounts(ids, accounts), 3);
  assert.deepEqual(ids, before);
  const view = buildCodexServiceMemberView({ accounts, sourceAccounts: [source, endpoint, other],
    memberIds: [source.id, endpoint.id], groups: [{ ...group, accountIds: [source.id, endpoint.id] }] });
  assert.deepEqual(view.groups[0].accountIds, [source.id]);
  assert.equal(view.rows.length, 2);
});

test('priority and checkbox additions share the recommendation gate while ignored reminders keep local selection', () => {
  const accounts = [source];
  const selected = new Set<string>();
  assert.equal(needsCodexServiceMemberAuthorizationChoice(source, accounts, selected, [], () => true), true);
  assert.equal(selected.size, 0, 'opening the choice does not silently join any channel');
  assert.equal(needsCodexServiceMemberAuthorizationChoice(source, accounts, selected, [], () => false), false);
  assert.deepEqual(toggleCodexServiceDisplaySelection(source, accounts, selected, true), new Set([source.id]));
  assert.equal(needsCodexServiceMemberAuthorizationChoice(source, accounts, new Set([source.id]), [], () => true), false);
  assert.equal(needsCodexServiceMemberAuthorizationChoice(source, accounts, selected, [source.id], () => true), false);
  const dual = projectCodexServiceAccountDisplays([source, endpoint], [grant]);
  assert.equal(needsCodexServiceMemberAuthorizationChoice(dual[1], dual, new Set([source.id]), [source.id], () => true), false,
    'an existing related member is not a fresh account addition');
});

test('original member identity uses official route health independently of original OAuth errors', () => {
  const original = { ...source, requires_reauth: true, quota_error: { code: 'invalid_token', message: '401', timestamp: 1 } };
  const build = (available: boolean) => buildCodexServiceMemberView({ accounts: [original], sourceAccounts: [original],
    groups: [group], memberIds: [source.id], officialAccountBindings: { [source.id]: grant.id },
    accountHealth: [healthy(source.id, available)] });
  const view = build(true);
  assert.equal(view.rows[0], original);
  assert.equal(view.counts.VALID, 1);
  assert.equal(build(false).counts.ERROR, 1);
  assert.equal(original.requires_reauth, true, 'service presentation cannot modify the original client login');
});
