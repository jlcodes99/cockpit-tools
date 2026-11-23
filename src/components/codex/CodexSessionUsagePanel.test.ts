import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
import { isStandardCodexOAuthAccount } from '../../types/codex';
import * as usageFormat from '../../utils/codexSessionUsageFormat';

function nodes(tree: any): any[] {
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  return tree && typeof tree === 'object' ? [tree, ...nodes(tree.props?.children)] : [];
}

test('official lifetime usage follows the chosen OAuth account and distinguishes zero from missing', async () => {
  const requests: string[] = [];
  const pending = deferred<number | null>();
  const accounts = [
    { id: 'a', auth_mode: 'oauth', tokens: { access_token: 'fixture-a', refresh_token: 'fixture-a' } },
    { id: 'b', auth_mode: 'oauth', tokens: { access_token: 'fixture-b', refresh_token: 'fixture-b' } },
    { id: 'api', auth_mode: 'apikey', tokens: { access_token: 'api-key' } },
  ];
  let nextValue: number | null = null;
  let shouldFail = false;
  const h = loadHookModule(new URL('./CodexSessionUsagePanel.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '@tauri-apps/plugin-dialog': {},
    '../SingleSelectDropdown': { SingleSelectDropdown: 'account-picker' },
    '../../services/codexInstanceService': {},
    '../../services/codexService': { getCodexProfileLifetimeTokens: async (accountId: string) => {
      requests.push(accountId);
      if (accountId === 'a') return pending.promise;
      if (shouldFail) throw new Error('upstream rejected the lookup');
      return nextValue;
    } },
    '../../stores/useCodexAccountStore': { useCodexAccountStore: (select: any) => select({ accounts, currentAccount: accounts[0] }) },
    '../../types/codex': { isStandardCodexOAuthAccount },
    '../../utils/codexSessionUsageQuery': {},
    '../../utils/codexSessionUsageFormat': usageFormat,
    './CodexUsageTrend': {},
    '../../utils/codexStatsRangePreference': {},
    '../../hooks/useDurableUiSetting': {},
    '../../utils/durableUiSettings': {},
    './useCodexProxyExitEditor': { useCodexProxyAccountName: () => (account: { id: string }) => `masked ${account.id}` },
  });
  h.render(() => h.exports.CodexProfileUsageTotal({ lang: 'en' }));
  const find = (predicate: (node: any) => boolean) => nodes(h.flush()).find(predicate);
  const status = () => find((node) => node.props?.role === 'status')?.props.children;
  const refresh = () => find((node) => node.type === 'button').props.onClick();
  const picker = () => find((node) => node.type === 'account-picker');
  assert.deepEqual(Array.from(picker().props.options, (option: any) => option.value), ['a', 'b']);
  assert.equal(picker().props.value, 'a');
  assert.deepEqual(requests, []);

  refresh();
  assert.equal(status(), 'common.loading');
  picker().props.onChange('b');
  refresh();
  await settlePromises();
  assert.equal(status(), 'codex.sessionUsage.comparison.unavailable');

  pending.resolve(630_000_000);
  await settlePromises();
  assert.equal(status(), 'codex.sessionUsage.comparison.unavailable');
  nextValue = 0;
  refresh();
  await settlePromises();
  assert.equal(status().props.value, 0);

  shouldFail = true;
  refresh();
  await settlePromises();
  assert.equal(status(), 'codex.sessionUsage.comparison.loadError');
  assert.deepEqual(requests, ['a', 'b', 'b', 'b']);
  h.unmount();
});
