import assert from 'node:assert/strict';
import test from 'node:test';
import type { CodexAccount } from '../types/codex';
import type { ModelProviderUsageSummary } from '../services/modelProviderUsageService';
import { codexRemoteBindingValue } from './codexRemoteBinding';
import { isCodexRemoteGateway, withCodexRemotePresentation } from './codexRemotePresentation';

const account = { id: 'gateway', auth_mode: 'apikey', api_base_url: 'http://192.168.10.7:58554/v1', openai_api_key: 'fixture-key' } as CodexAccount;
const summary = { cockpit: { product: 'cockpit-tools' }, sourceBinding: codexRemoteBindingValue(account.api_base_url!, account.openai_api_key!) } as ModelProviderUsageSummary;
const original = { displayName: '192.168.10.7', planLabel: 'API', planClass: 'api', quotaItems: [] };

test('gateway identity is shared across cards, instance options and selected account badges', () => {
  const presentation = withCodexRemotePresentation(account, original, summary, 'Cockpit 网关');
  assert.equal(presentation.displayName, 'Cockpit 网关');
  assert.equal(presentation.planLabel, 'Cockpit');
  assert.equal(presentation.planClass, 'codex-cockpit');
  assert.equal(presentation.quotaItems, original.quotaItems);
  assert.equal(original.displayName, '192.168.10.7');
  assert.equal(withCodexRemotePresentation(account, original, summary, 'Cockpit Gateway').displayName, 'Cockpit Gateway');
});

test('offline cache retains identity but changed credentials, endpoints or account type do not', () => {
  assert.equal(isCodexRemoteGateway(account, { ...summary, status: 'offline' }), true);
  for (const changed of [
    { ...account, api_base_url: 'http://192.168.10.8/v1' },
    { ...account, openai_api_key: 'different-key' },
    { ...account, auth_mode: 'oauth' as const },
  ]) {
    assert.equal(isCodexRemoteGateway(changed, summary), false);
    assert.equal(withCodexRemotePresentation(changed, original, summary, 'Cockpit 网关'), original);
  }
  assert.equal(isCodexRemoteGateway(account, undefined), false);
  assert.equal(isCodexRemoteGateway(account, { ...summary, sourceBinding: undefined }), false);
});
