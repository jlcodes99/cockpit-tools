import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';

function nodes(value: any): any[] {
  if (Array.isArray(value)) return value.flatMap(nodes);
  return value && typeof value === 'object' && value.props ? [value, ...nodes(value.props.children)] : [];
}

test('general settings retain payload privacy, refresh scope and account refresh controls after automatic controls are removed', () => {
  const callbacks: string[] = [];
  const hook = loadHookModule(new URL('../../pages/SettingsCodexPlatformPanel.tsx', import.meta.url), {
    '../components/CodexRefreshPlanScopeControl': { CodexRefreshPlanScopeControl: 'Scope' },
    '../components/codex/CodexAppEnhancementSettings': { CodexAppEnhancementSettings: 'Enhancements' },
    '../components/codex/CodexRequestPayloadSetting': { CodexRequestPayloadSetting: 'PayloadPrivacy' },
    '../components/AutoSwitchAccountScopeSelector': { AutoSwitchAccountScopeSelector: 'AccountScope' },
    '../components/codex/CodexContextManagementControl': { CodexContextManagementControl: 'Context' },
    '../components/codex/CodexSshSyncSettingsControl': { CodexSshSyncSettingsControl: 'SshSync' },
    '../stores/useRemoteConfigStore': { useRemoteConfigStore: () => '' },
  });
  const props = new Proxy({
    t: (key: string) => key,
    platformSettingsOrder: { codex: 0 },
    codexAutoRefresh: '5', codexAutoRefreshPlanTypes: ['plus'], codexQuotaAlertThreshold: '20',
    codexGroups: [], codexScopeAccounts: [], codexLaunchCandidates: [], isAppPathResetDetecting: () => false,
    renderCurrentAccountRefreshRow: () => { callbacks.push('current'); return 'CurrentRefresh'; },
    renderAccountLevelRefreshConfig: () => { callbacks.push('accounts'); return 'AccountRefresh'; },
  } as Record<string, any>, {
    get(target, key: string) { return key in target ? target[key] : /^(set|handle|get|sanitize|normalize)/.test(key) ? () => {} : false; },
  });
  const tree = nodes(hook.render(() => hook.exports.SettingsCodexPlatformPanel(props)));
  assert.equal(tree.filter((entry) => entry.type === 'PayloadPrivacy').length, 1);
  assert.equal(tree.filter((entry) => entry.type === 'Scope').length, 1);
  assert.equal(tree.filter((entry) => entry.type === 'Enhancements').length, 1);
  assert.deepEqual(callbacks, ['current', 'accounts']);
  assert.equal(tree.some((entry) => /^(multiAgent|capabilityFallback|codex\.quotaSync)\./.test(String(entry.props.children))), false);
  hook.unmount();
});
