import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';
import type { CodexAccount } from '../types/codex';
import type { WorkbuddyAccount } from '../types/workbuddy';
import * as credits from '../utils/codexWorkbuddyCredits';

const linked = [{ id: 'codex', auth_mode: 'apikey', api_credential_kind: 'workbuddy_acp', api_credential_ref: 'source' }] as CodexAccount[];
const source = (value: number) => ({ id: 'source', access_token: 'private-source-token', usage_updated_at: value }) as WorkbuddyAccount;
function setup(read: () => Promise<WorkbuddyAccount[]>) {
  let listener: ((event: any) => void) | undefined;
  let stopped = 0;
  const hook = loadHookModule(new URL('./useCodexWorkbuddyCredits.ts', import.meta.url), {
    '../services/workbuddyService': { listWorkbuddyAccounts: read },
    '../utils/accountSyncEvents': { ACCOUNTS_CHANGED_EVENT: 'accounts' },
    '../utils/codexWorkbuddyCredits': credits,
    '../utils/tauriEventListener': { subscribeTauriEvent(_name: string, callback: (event: any) => void) {
      listener = callback; return () => { stopped++; };
    } },
  });
  return { hook, event: (platformId: string) => listener?.({ payload: { platformId } }), stopped: () => stopped };
}

test('ordinary Codex accounts do not read WorkBuddy accounts or register listeners', () => {
  let reads = 0;
  const { hook, stopped } = setup(async () => { reads++; return []; });
  hook.render(() => hook.exports.useCodexWorkbuddyCredits([{ id: 'oauth' }]));
  assert.equal(reads, 0);
  hook.unmount();
  assert.equal(stopped(), 0);
});

test('local source reads strip credentials, react to WorkBuddy events and clear deleted sources', async () => {
  let data = [source(1)]; let reads = 0;
  const { hook, event, stopped } = setup(async () => { reads++; return data; });
  hook.render(() => hook.exports.useCodexWorkbuddyCredits(linked));
  await settlePromises();
  let value = hook.flush();
  assert.equal(value.loaded, true);
  assert.equal(value.sources[0].usage_updated_at, 1);
  assert.doesNotMatch(JSON.stringify(value.sources), /private-source-token/);
  event('grok'); await settlePromises(); assert.equal(reads, 1);
  data = [];
  event('workbuddy'); await settlePromises(); value = hook.flush();
  assert.equal(value.sources.length, 0);
  assert.equal(value.readFailed, false);
  hook.unmount(); assert.equal(stopped(), 1);
});

test('read failure preserves the last good balance and explicitly marks it stale', async () => {
  let fail = false;
  const { hook, event } = setup(async () => {
    if (fail) throw new Error('local read failed');
    return [source(2)];
  });
  hook.render(() => hook.exports.useCodexWorkbuddyCredits(linked));
  await settlePromises(); hook.flush();
  fail = true; event('workbuddy'); await settlePromises();
  const value = hook.flush();
  assert.equal(value.readFailed, true);
  assert.equal(value.sources[0].usage_updated_at, 2);
  hook.unmount();
});

test('late and unmounted source reads cannot overwrite newer results', async () => {
  const first = deferred<WorkbuddyAccount[]>();
  const second = deferred<WorkbuddyAccount[]>();
  const third = deferred<WorkbuddyAccount[]>();
  const requests = [first, second, third]; let index = 0;
  const { hook, event } = setup(() => requests[index++].promise);
  hook.render(() => hook.exports.useCodexWorkbuddyCredits(linked));
  event('workbuddy');
  second.resolve([source(20)]); await settlePromises(); hook.flush();
  first.resolve([source(10)]); await settlePromises();
  assert.equal(hook.flush().sources[0].usage_updated_at, 20);
  event('workbuddy'); hook.unmount();
  third.resolve([source(30)]); await settlePromises();
  assert.equal(hook.flush().sources[0].usage_updated_at, 20);
});
