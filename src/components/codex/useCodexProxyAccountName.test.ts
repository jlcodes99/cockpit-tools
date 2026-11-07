import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';
import { maskSensitiveValue, PRIVACY_MODE_CHANGED_EVENT } from '../../utils/privacy';
import { codexProxyAccountName } from '../../utils/codexProxyDraft';

test('proxy account labels mask names, emails and dependency identities and react to privacy changes', () => {
  let privateMode = true;
  const listeners = new Map<string, () => void>();
  const h = loadHookModule(new URL('./useCodexProxyExitEditor.ts', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) }, '../../stores/useCodexAccountStore': {}, '../../utils/codexAccountProxy': {},
    '../../services/codexProxyTargetService': {},
    '../../services/codexProxyCatalogService': {}, '../../services/codexAccountProxyService': {},
    '../../utils/codexProxySelection': {}, './CodexProxyWorkspaceContext': {},
    '../../utils/codexProxyDraft': { codexProxyAccountName },
    '../../utils/privacy': { maskSensitiveValue, PRIVACY_MODE_CHANGED_EVENT, isPrivacyModeEnabledByDefault: () => privateMode },
  }, { window: {
    addEventListener: (event: string, listener: () => void) => listeners.set(event, listener),
    removeEventListener: (event: string) => listeners.delete(event),
  } });
  const identities = [
    { id: 'account-id', account_name: 'Private Account', email: 'private@example.com' },
    { id: 'account-id', email: 'private@example.com' },
    { id: 'private-account-id' },
  ];
  const resolveName = h.render(() => h.exports.useCodexProxyAccountName());
  for (const identity of identities) {
    assert.equal(resolveName(identity), maskSensitiveValue(codexProxyAccountName(identity), true));
    assert.notEqual(resolveName(identity), codexProxyAccountName(identity));
  }
  assert.equal(resolveName(null), '');
  assert.equal(resolveName({ id: '__api_service__', account_name: 'API 服务' }), 'API 服务', 'public service label is not an account identity');
  privateMode = false;
  listeners.get(PRIVACY_MODE_CHANGED_EVENT)!();
  for (const identity of identities) assert.equal(h.flush()(identity), codexProxyAccountName(identity));
  h.unmount();
  assert.equal(listeners.size, 0);
});
