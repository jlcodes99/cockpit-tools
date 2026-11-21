import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';

function nodes(value: any): any[] {
  if (Array.isArray(value)) return value.flatMap(nodes);
  return value && typeof value === 'object' && value.props ? [value, ...nodes(value.props.children)] : [];
}

function loadSettingsPanel() {
  const callbacks: string[] = [];
  const hook = loadHookModule(new URL('../../pages/SettingsCodexPlatformPanel.tsx', import.meta.url), {
    '../components/CodexRefreshPlanScopeControl': { CodexRefreshPlanScopeControl: 'Scope' },
    '../components/codex/CodexAppEnhancementSettings': { CodexAppEnhancementSettings: 'Enhancements' },
    '../components/codex/CodexRequestPayloadSetting': { CodexRequestPayloadSetting: 'PayloadPrivacy' },
    '../components/codex/CodexOverloadRetrySetting': { CodexOverloadRetrySetting: 'OverloadRetry' },
    '../components/codex/CodexQuotaRecoverySettings': { CodexQuotaRecoverySettings: 'QuotaRecovery' },
    '../components/codex/CodexQuotaHistorySettings': { CodexQuotaHistorySettings: 'QuotaHistoryStorage' },
    '../components/AutoSwitchAccountScopeSelector': { AutoSwitchAccountScopeSelector: 'AccountScope' },
    '../components/codex/CodexContextManagementControl': { CodexContextManagementControl: 'Context' },
    '../components/codex/CodexSshSyncSettingsControl': { CodexSshSyncSettingsControl: 'SshSync' },
    '../components/SingleSelectDropdown': { SingleSelectDropdown: 'SingleSelectDropdown' },
    '../stores/useRemoteConfigStore': { useRemoteConfigStore: () => '' },
  });
  const props = new Proxy({
    t: (key: string) => key,
    platformSettingsOrder: { codex: 0 },
    codexAutoRefresh: '5', codexAutoRefreshPlanTypes: ['plus'],
    codexGroups: [], codexScopeAccounts: [], codexLaunchCandidates: [], isAppPathResetDetecting: () => false,
    renderCurrentAccountRefreshRow: () => { callbacks.push('current'); return 'CurrentRefresh'; },
    renderAccountLevelRefreshConfig: () => { callbacks.push('accounts'); return 'AccountRefresh'; },
  } as Record<string, any>, {
    get(target, key: string) { return key in target ? target[key] : /^(set|handle|get|sanitize|normalize)/.test(key) ? () => {} : false; },
  });
  return {
    callbacks,
    render: (overrides: Record<string, unknown> = {}) => {
      Object.assign(props, overrides);
      return nodes(hook.render(() => hook.exports.SettingsCodexPlatformPanel(props)));
    },
    unmount: () => hook.unmount(),
  };
}

test('general settings retain payload privacy, refresh scope and account refresh controls after automatic controls are removed', () => {
  const { callbacks, render, unmount } = loadSettingsPanel();
  const tree = render();
  assert.equal(tree.filter((entry) => entry.type === 'PayloadPrivacy').length, 1);
  assert.equal(tree.filter((entry) => entry.type === 'OverloadRetry').length, 1);
  assert.equal(tree.filter((entry) => entry.type === 'Scope').length, 1);
  assert.equal(tree.filter((entry) => entry.type === 'Enhancements').length, 1);
  assert.equal(tree.filter((entry) => entry.type === 'QuotaHistoryStorage').length, 1);
  assert.deepEqual(callbacks, ['current', 'accounts']);
  assert.equal(tree.some((entry) => /^(multiAgent|capabilityFallback|codex\.quotaSync)\./.test(String(entry.props.children))), false);
  unmount();
});

test('Codex general settings edit each quota alert window independently', () => {
  const { render, unmount } = loadSettingsPanel();
  const updates: [string, number][] = [];
  const tree = render({
    codexQuotaAlertEnabled: true,
    codexQuotaAlertPrimaryThreshold: 10,
    codexQuotaAlertSecondaryThreshold: 30,
    setCodexQuotaAlertPrimaryThreshold: (value: number) => updates.push(['short', value]),
    setCodexQuotaAlertSecondaryThreshold: (value: number) => updates.push(['weekly', value]),
  });
  const short = tree.find((entry) => entry.props['aria-label'] === 'codex.thresholds.shortCycle quickSettings.quotaAlert.threshold');
  const weekly = tree.find((entry) => entry.props['aria-label'] === 'codex.thresholds.weekly quickSettings.quotaAlert.threshold');
  assert.equal(short?.props.defaultValue, 10);
  assert.equal(weekly?.props.defaultValue, 30);
  short.props.onBlur({ currentTarget: { valueAsNumber: 0 } });
  weekly.props.onBlur({ currentTarget: { valueAsNumber: 105 } });
  short.props.onBlur({ currentTarget: { valueAsNumber: NaN } });
  assert.deepEqual(updates, [['short', 0], ['weekly', 100], ['short', 10]]);
  unmount();
});
