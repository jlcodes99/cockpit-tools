import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';

function elements(value: any): any[] {
  if (Array.isArray(value)) return value.flatMap(elements);
  return value?.props ? [value, ...elements(value.props.children)] : [];
}
function harness(initial: string | null = null, readFails = false) {
  const calls: unknown[][] = [];
  const t = (key: string) => key;
  const h = loadHookModule(new URL('./CodexProxyCountryGuard.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t, i18n: { language: 'en' } }) },
    './CodexProxySelect': { CodexProxySelect: 'country-select' },
    '@tauri-apps/api/event': { listen: async () => () => {} },
    '../ModalErrorMessage': { ModalErrorMessage: 'error-message' },
    '../../utils/codexProxyCountry': {
      codexCountryOptions: () => [{ value: 'JP', label: 'Japan' }, { value: 'SG', label: 'Singapore' }],
      codexCountryErrorValues: () => ({}),
      codexCountryErrorKey: () => 'codex.proxy.country.storage',
    },
    '../../services/codexProxyCountryService': {
      getCodexCountryChecks: async () => [],
      checkCodexCountry: async () => { throw new Error('unexpected check'); },
      getCodexProxyCountryPolicy: async () => { if (readFails) throw new Error('storage'); return { expectedCountry: initial }; },
      setCodexProxyCountryPolicy: async (...args: unknown[]) => { calls.push(args); return { expectedCountry: args[1] }; },
    },
  });
  h.render(() => h.exports.CodexProxyCountryGuard({ accountId: 'unbound-account' }));
  const tree = () => elements(h.flush());
  const checkbox = () => tree().find(node => node.props.role === 'switch').props;
  const select = () => tree().find(node => node.type === 'country-select').props;
  const save = () => tree().find(node => node.type === 'button' && node.props.children === 'codex.proxy.country.save').props;
  return { h, tree, checkbox, select, save, calls };
}
test('unbound accounts can opt in but editing and closing do not save automatically', async () => {
  const h = harness(); await settlePromises();
  assert.equal(h.checkbox()['aria-checked'], false);
  h.checkbox().onClick();
  assert.equal(h.save().disabled, true);
  h.select().onChange('JP');
  assert.equal(h.save().disabled, false);
  assert.equal(h.calls.length, 0);
  h.h.unmount();
  assert.equal(h.calls.length, 0);
});
test('country check saves separately from the proxy binding and can be disabled explicitly', async () => {
  const h = harness('JP'); await settlePromises();
  h.select().onChange('SG'); h.save().onClick(); await settlePromises();
  assert.deepEqual(JSON.parse(JSON.stringify(h.calls)), [['unbound-account', 'SG']]);
  h.checkbox().onClick(); h.save().onClick(); await settlePromises();
  assert.deepEqual(JSON.parse(JSON.stringify(h.calls)), [['unbound-account', 'SG'], ['unbound-account', null]]);
  h.h.unmount();
});
test('failed policy reads cannot silently save disabled defaults', async () => {
  const h = harness(null, true); await settlePromises();
  assert.equal(h.checkbox().disabled, true);
  h.save().onClick(); await settlePromises();
  assert.equal(h.calls.length, 0);
  assert.ok(h.tree().find(node => node.type === 'error-message').props.message);
  h.h.unmount();
});

test('country evidence starts collapsed and uses the existing proxy switch', async () => {
  const h = harness(); await settlePromises();
  const evidence = h.tree().find(node => node.type === 'details' && node.props.className === 'codex-proxy-country-evidence');
  assert.equal(evidence.props.open, undefined);
  assert.match(h.checkbox().className, /codex-resource-network-switch/);
  h.h.unmount();
});
