import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, settlePromises } from '../../tests/helpers/reactHookHarness';
import { createCodexServiceMemberDraftHarness } from '../../tests/helpers/codexServiceMemberDraftHarness';
import type { CodexAccount } from '../types/codex';
const source = { id: 'source', email: 'person@example.com', auth_mode: 'oauth', plan_type: 'PRO 200',
  tokens: { access_token: 'original-token', id_token: 'original-id' } } as CodexAccount;
const grant = { id: 'grant', source_account_id: source.id, connected: true, needs_reauth: false,
  source_binding_valid: true, last_service_mode: 'official' };

test('checkbox and Save do not read grants, open a prompt or mutate service/account identity', async () => {
  const before = structuredClone(source);
  const h = createCodexServiceMemberDraftHarness({ accounts: [source], list: async () => { throw new Error('must not load'); } });
  await h.add(); assert.equal(h.mode(), 'codex'); assert.deepEqual(await h.prepare(), {});
  assert.equal(h.calls.lists, 0); assert.equal(h.calls.choices.length, 0); assert.equal(h.calls.mutations, 0);
  assert.deepEqual(h.collection.accountIds, []); assert.deepEqual(source, before); h.hook.unmount();
});
test('row dropdown reuses a saved bound grant without a chooser and keeps the source ID', async () => {
  const h = createCodexServiceMemberDraftHarness({ accounts: [source], list: async () => [grant] });
  await h.add(); await h.choose('official');
  assert.equal(h.mode(), 'official'); assert.deepEqual(await h.prepare(), { source: 'grant' });
  assert.equal(h.calls.choices.length, 0); assert.equal(h.calls.mutations, 0);
  assert.deepEqual([...h.selected()], ['source']); assert.deepEqual(h.collection.accountIds, []); h.hook.unmount();
});
test('unjoined rows restore saved official access consistently in both dropdown and Save', async () => {
  const h = createCodexServiceMemberDraftHarness({ accounts: [source], grants: [grant] });
  assert.equal(h.mode(), 'official'); await h.add(); assert.deepEqual(await h.prepare(), { source: 'grant' });
  assert.equal(h.calls.lists, 0); await h.choose('codex'); assert.equal(h.mode(), 'codex'); assert.deepEqual(await h.prepare(), {}); h.hook.unmount();
});
test('existing local membership keeps its route even when a bound grant is available', async () => {
  const h = createCodexServiceMemberDraftHarness({ accounts: [source], grants: [grant], collection: { accountIds: ['source'], officialAccountBindings: {} } });
  assert.equal(h.mode(), 'codex'); assert.deepEqual(await h.prepare(), {}); h.hook.unmount();
});
test('official dropdown starts one account authorization and stages only after successful completion', async () => {
  const h = createCodexServiceMemberDraftHarness({ accounts: [source], list: async () => [] }); await h.add();
  const changing = h.choose('official'); await settlePromises();
  assert.equal(h.mode(), 'codex'); assert.equal(await h.prepare(), null); assert.equal(h.calls.choices.length, 1);
  const request = h.calls.choices[0]; assert.deepEqual(request.ids, ['source']);
  h.emit({ requestId: request.requestId, sourceAccountId: 'other', status: 'selected', mode: 'official', registrationId: 'wrong' });
  assert.equal(h.mode(), 'codex');
  h.emit({ requestId: request.requestId, sourceAccountId: 'source', status: 'selected', mode: 'official', registrationId: 'grant' }); await changing;
  assert.deepEqual(await h.prepare(), { source: 'grant' }); assert.equal(h.calls.mutations, 0); h.hook.unmount();
});
test('cancelled authorization retains prior route and ignores late completion', async () => {
  const h = createCodexServiceMemberDraftHarness({ accounts: [source], list: async () => [] }); await h.add();
  const changing = h.choose('official'); await settlePromises(); const request = h.calls.choices[0];
  h.emit({ requestId: request.requestId, sourceAccountId: 'source', status: 'cancelled' }); await changing;
  h.emit({ requestId: request.requestId, sourceAccountId: 'source', status: 'selected', mode: 'official', registrationId: 'late' });
  assert.equal(h.mode(), 'codex'); assert.deepEqual(await h.prepare(), {}); assert.equal(h.hook.flush().busy, false); h.hook.unmount();
});
test('switching existing official member to local clears only draft binding and not saved service', async () => {
  const h = createCodexServiceMemberDraftHarness({ accounts: [source], collection: { accountIds: ['source'], officialAccountBindings: { source: 'grant' } } });
  await h.choose('codex'); assert.deepEqual(await h.prepare(), {});
  assert.deepEqual(h.collection.officialAccountBindings, { source: 'grant' }); assert.equal(h.calls.mutations, 0); h.hook.unmount();
});
test('a metadata failure keeps the route and can be retried without changing membership', async () => {
  let reads = 0;
  const h = createCodexServiceMemberDraftHarness({ accounts: [source], list: async () => { if (++reads === 1) throw new Error('offline'); return [grant]; } });
  await h.choose('official'); assert.equal(h.mode(), 'codex'); assert.equal(h.calls.errors.length, 1);
  await h.choose('official'); assert.equal(h.mode(), 'official'); assert.deepEqual(h.collection.accountIds, []); h.hook.unmount();
});
test('closing during metadata loading fences late responses and reopening remains usable', async () => {
  const metadata = deferred<any[]>(); let reads = 0;
  const h = createCodexServiceMemberDraftHarness({ accounts: [source], list: () => ++reads === 1 ? metadata.promise : Promise.resolve([grant]) });
  const previous = h.choose('official'); await settlePromises(); h.open(false); h.open(true);
  await h.choose('official'); metadata.resolve([]); await previous;
  assert.equal(h.mode(), 'official'); assert.equal(h.calls.choices.length, 0); h.hook.unmount();
});
test('closing pending authorization cancels only its request and fences late results', async () => {
  const h = createCodexServiceMemberDraftHarness({ accounts: [source], list: async () => [] });
  const changing = h.choose('official'); await settlePromises(); const request = h.calls.choices[0]; h.open(false); await changing;
  assert.deepEqual(h.calls.cancelled, [request.requestId]);
  h.emit({ requestId: request.requestId, sourceAccountId: 'source', status: 'selected', mode: 'official', registrationId: 'late' });
  assert.deepEqual(h.bindings(), {}); h.hook.unmount();
});
test('duplicate official changes are single-flight and ambiguous grants never guess an identity', async () => {
  const pending = deferred<any[]>();
  const h = createCodexServiceMemberDraftHarness({ accounts: [source], list: () => pending.promise });
  const first = h.choose('official'); const second = h.choose('official'); pending.resolve([grant, { ...grant, id: 'other' }]);
  await Promise.all([first, second]); assert.equal(h.calls.lists, 1); assert.match(String(h.error()), /BINDING_CONFLICT/);
  assert.equal(h.mode(), 'codex'); h.hook.unmount();
});
test('unselected account choices never enter saved bindings', async () => {
  const h = createCodexServiceMemberDraftHarness({ accounts: [source], list: async () => [grant] });
  await h.choose('official'); assert.deepEqual(await h.prepare(), {}); await h.add(); assert.deepEqual(await h.prepare(), { source: 'grant' }); h.hook.unmount();
});
