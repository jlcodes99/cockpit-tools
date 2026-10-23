import assert from 'node:assert/strict';
import test from 'node:test';
import type { CodexAccount } from '../types/codex';
import type { ChatGptPlanAccount } from '../services/chatgptPlanService';
import type { CodexLocalAccessState } from '../types/codexLocalAccess';
import { prepareCodexServiceJoins, resolveCodexServiceJoinPlan, reuseCodexServiceJoin } from './codexServiceJoin';

const source = { id: 'source', email: 'a@example.com', auth_mode: 'oauth' } as CodexAccount;
const grant = { id: 'grant', source_account_id: source.id, source_binding_valid: true, connected: true, needs_reauth: false } as ChatGptPlanAccount;

test('saved official authorization is reused without model verification or another login, even after removal', () => {
  assert.deepEqual(resolveCodexServiceJoinPlan(source, [grant], []), { kind: 'saved', mode: 'official', registrationId: grant.id });
  assert.deepEqual(resolveCodexServiceJoinPlan(source, [{ ...grant, last_service_mode: 'official', verified_at: undefined, verified_model: undefined }], []), { kind: 'saved', mode: 'official', registrationId: grant.id });
  assert.deepEqual(resolveCodexServiceJoinPlan(source, [{ ...grant, source_binding_valid: false }], []), { kind: 'saved', mode: 'official', registrationId: grant.id }, 'pending association is rechecked by the backend, not replaced with browser login');
});

test('last successful local choice survives membership removal and an expired official session', () => {
  assert.deepEqual(resolveCodexServiceJoinPlan(source, [{ ...grant, last_service_mode: 'codex', needs_reauth: true }], []), { kind: 'saved', mode: 'codex', registrationId: grant.id });
  assert.deepEqual(resolveCodexServiceJoinPlan(source, [grant], [source.id]), { kind: 'saved', mode: 'codex', registrationId: grant.id });
});

test('unbound accounts join locally without a prompt; invalid saved authorization remains explicit', () => {
  assert.equal(resolveCodexServiceJoinPlan(source, [], []).kind, 'local');
  assert.equal(resolveCodexServiceJoinPlan(source, [grant, { ...grant, id: 'other' }], []).kind, 'reauthorize');
  for (const value of [{ ...grant, connected: false }, { ...grant, needs_reauth: true }, { ...grant, source_binding_error: 'CHATGPT_PLAN_SOURCE_CHANGED' }]) assert.equal(resolveCodexServiceJoinPlan(source, [value], []).kind, 'reauthorize');
  assert.equal(resolveCodexServiceJoinPlan({ ...source, auth_mode: 'apikey' }, [grant], []).kind, 'local');
});

test('only committed real membership is announced; model is never supplied and transient errors preserve the route', async () => {
  const calls: unknown[][] = [], applied: unknown[][] = [];
  const state = { collection: { accountIds: ['source', 'other'], officialAccountBindings: { source: 'grant' } } } as unknown as CodexLocalAccessState;
  const dependencies = { setMode: async (...args: any[]) => { calls.push(args); return state; }, applied: (...args: any[]) => { applied.push(args); } };
  const result = await reuseCodexServiceJoin(source, [grant], [], dependencies);
  assert.equal(result.kind, 'joined');
  assert.deepEqual(calls, [['source', 'official', 'grant', { intent: 'join' }]]);
  assert.deepEqual(applied, [[['source'], ['source']]]);
  dependencies.setMode = async () => { throw new Error('CHATGPT_PLAN_NETWORK'); };
  await assert.rejects(reuseCodexServiceJoin(source, [grant], [], dependencies), /CHATGPT_PLAN_NETWORK/);
  dependencies.setMode = async () => { throw new Error('CHATGPT_PLAN_REAUTH_REQUIRED'); };
  await assert.rejects(reuseCodexServiceJoin(source, [grant], [], dependencies), /CHATGPT_PLAN_REAUTH_REQUIRED/);
  dependencies.setMode = async () => ({ collection: { accountIds: ['source'], officialAccountBindings: {} } }) as CodexLocalAccessState;
  await assert.rejects(reuseCodexServiceJoin(source, [grant], [], dependencies), /CHATGPT_PLAN_SERVICE_CHANGED/);
  assert.equal(applied.length, 1);
});

test('batch joins preserve saved official routes and add unbound OAuth and API key accounts locally', async () => {
  const { prepareCodexServiceJoins } = await import('./codexServiceJoin');
  const states: unknown[] = [];
  const state = { collection: { accountIds: ['source'], officialAccountBindings: { source: 'grant' } } } as unknown as CodexLocalAccessState;
  const result = await prepareCodexServiceJoins([source, { ...source, id: 'new' }, { ...source, id: 'api', auth_mode: 'apikey' }], [grant], [],
    (value) => states.push(value), async (account) => account.id === 'source'
      ? { kind: 'joined', state, selectedIds: ['source'] }
      : { kind: 'local' });
  assert.deepEqual(result, { localIds: ['new', 'api'], resumedIds: ['source'], failures: [] });
  assert.deepEqual(states, [state]);
});

test('draft planning reads canonical mode without committing it or silently replacing a missing bound grant', () => {
  const oldLocal = { ...grant, last_service_mode: 'codex' as const };
  assert.deepEqual(resolveCodexServiceJoinPlan(source, [oldLocal], ['source'], { source: 'grant' }), { kind: 'saved', mode: 'official', registrationId: 'grant' });
  assert.deepEqual(resolveCodexServiceJoinPlan(source, [{ ...grant, last_service_mode: 'official' }], ['source'], {}), { kind: 'saved', mode: 'codex', registrationId: 'grant' });
  assert.equal(resolveCodexServiceJoinPlan(source, [grant], ['source'], { source: 'missing' }).kind, 'reauthorize');
  assert.equal(resolveCodexServiceJoinPlan(source, [], ['source'], { source: 'missing' }).kind, 'reauthorize');
});


test('unbound joins and batch imports never require browser authorization or mutate a route', async () => {
  const { prepareCodexServiceJoins } = await import('./codexServiceJoin');
  const dependencies = { setMode: async () => { throw new Error('unexpected authorization mutation'); }, applied: () => { throw new Error('unexpected event'); } };
  assert.deepEqual(await reuseCodexServiceJoin(source, [], [], dependencies), { kind: 'local' });
  assert.deepEqual(await prepareCodexServiceJoins([source], [], [], () => {}, (account, grants, members) => reuseCodexServiceJoin(account, grants, members, dependencies)), { localIds: ['source'], resumedIds: [], failures: [] });
  await assert.rejects(reuseCodexServiceJoin(source, [{ ...grant, needs_reauth: true }], [], dependencies), /CHATGPT_PLAN_REAUTH_REQUIRED/);
});

test('a stale official grant does not discard local IDs before or after it or change its route', async () => {
  const before = { ...source, id: 'before' }, after = { ...source, id: 'after' };
  const stale = { ...grant, needs_reauth: true, last_service_mode: 'official' as const };
  const visited: string[] = [];
  const dependencies = {
    setMode: async () => { throw new Error('unexpected route mutation'); },
    applied: () => { throw new Error('unexpected membership event'); },
  };
  const result = await prepareCodexServiceJoins([before, source, after], [stale], [], () => {}, (account, grants, members) => {
    visited.push(account.id);
    return reuseCodexServiceJoin(account, grants, members, dependencies);
  });
  assert.deepEqual(visited, ['before', 'source', 'after']);
  assert.deepEqual(result.localIds, ['before', 'after']);
  assert.deepEqual(result.resumedIds, []);
  assert.equal(result.failures.length, 1);
  assert.equal(result.failures[0].accountId, 'source');
  assert.match(String(result.failures[0].error), /CHATGPT_PLAN_REAUTH_REQUIRED/);
  assert.equal(stale.last_service_mode, 'official');
});

test('backend join failures are reported individually while successful membership continues to the next account', async () => {
  const rows = ['first', 'reauth', 'network', 'last'].map((id) => ({ ...source, id }));
  const grants = rows.map((account) => ({ ...grant, id: `grant-${account.id}`, source_account_id: account.id }));
  const reauth = new Error('CHATGPT_PLAN_REAUTH_REQUIRED'), network = new Error('CHATGPT_PLAN_NETWORK');
  const snapshots: string[][] = [], seenMembers: string[][] = [], applied: string[][] = [];
  const committed: string[] = [];
  const result = await prepareCodexServiceJoins(rows, grants, [], (state) => snapshots.push([...state.collection!.accountIds]),
    (account, registrations, members) => {
      seenMembers.push([...(members ?? [])]);
      return reuseCodexServiceJoin(account, registrations, members, {
        setMode: async (id) => {
          if (id === 'reauth') throw reauth;
          if (id === 'network') throw network;
          committed.push(id);
          return { collection: { accountIds: [...committed], officialAccountBindings: Object.fromEntries(committed.map((member) => [member, `grant-${member}`])) } } as unknown as CodexLocalAccessState;
        },
        applied: (_related, selected) => { applied.push(selected); },
      });
    });
  assert.deepEqual(result.localIds, []);
  assert.deepEqual(result.resumedIds, ['first', 'last']);
  assert.deepEqual(result.failures, [{ accountId: 'reauth', error: reauth }, { accountId: 'network', error: network }]);
  assert.deepEqual(seenMembers, [[], ['first'], ['first'], ['first']]);
  assert.deepEqual(snapshots, [['first'], ['first', 'last']]);
  assert.deepEqual(applied, [['first'], ['last']]);
});
