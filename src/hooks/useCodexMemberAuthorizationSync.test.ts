import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';
import { rebaseCodexMemberSelection } from '../utils/codexAccountServiceNavigation';

test('explicit same-mode selection restores the row even when persisted membership has no delta', () => {
  const window = new EventTarget();
  let selected = new Set(['unrelated-draft']);
  let policies: any = {};
  let routing: any = {};
  let collection = { accountIds: ['source'], imageGenerationAccountPolicies: {}, customRoutingRules: [] as any[] };
  let isOpen = true;
  const setSelected = (update: any) => { selected = update(selected); };
  const setPolicies = (update: any) => { policies = update(policies); };
  const setRouting = (update: any) => { routing = update(routing); };
  const hook = loadHookModule(new URL('./useCodexMemberAuthorizationSync.ts', import.meta.url), {
    '../utils/codexAccountServiceNavigation': { CODEX_ACCOUNT_SERVICE_APPLIED: 'applied', rebaseCodexMemberSelection },
  }, { window });
  hook.render(() => hook.exports.useCodexMemberAuthorizationSync(isOpen, collection, setSelected, setPolicies, setRouting));
  window.dispatchEvent(new CustomEvent('applied', { detail: { relatedIds: ['source', 'official'], selectedIds: ['source', 'outside-scope'] } }));
  assert.deepEqual([...selected].sort(), ['source', 'unrelated-draft']);
  collection = { accountIds: ['official'], imageGenerationAccountPolicies: { official: 'disabled' }, customRoutingRules: [{ accountId: 'official', priority: 9, weight: 4, isBackup: true }] };
  hook.flush();
  assert.deepEqual([...selected].sort(), ['official', 'unrelated-draft']);
  assert.equal(policies.official, 'disabled');
  assert.equal(routing.official.weight, 4);
  assert.equal(routing.official.isBackup, true);
  isOpen = false; hook.flush();
  window.dispatchEvent(new CustomEvent('applied', { detail: { relatedIds: ['official'], selectedIds: [] } }));
  assert.equal(selected.has('official'), true);
  hook.unmount();
});
