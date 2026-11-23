import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
function elements(value: any): any[] {
  if (Array.isArray(value)) return value.flatMap(elements);
  return value?.props ? [value, ...elements(value.props.children)] : [];
}
function harness(recognitionFailed = false, readFails = false) {
  const calls: unknown[][] = []; let saved = 0;
  const t = (key: string) => key;
  const view = { document: recognitionFailed ? '' : 'HTTPS_PROXY=http://127.0.0.1:7890', revision: 'fixture-revision',
    source: recognitionFailed ? 'unknown' : 'env', recognitionFailed };
  const h = loadHookModule(new URL('./CodexProxyEnvEditor.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t }) },
    '../ModalErrorMessage': { ModalErrorMessage: 'error-message' },
    '../../services/codexProxyCountryService': {
      getCodexProxyEnv: async () => { if (readFails) throw new Error('storage'); return view; },
      setCodexProxyEnv: async (...args: unknown[]) => { calls.push(args); return { ...view, document: args[1], source: 'saved' }; },
    },
  });
  h.render(() => h.exports.CodexProxyEnvEditor({ accountId: 'fixture-account', onSaved: () => saved++ }));
  const tree = () => elements(h.flush());
  return { h, calls, tree, saved: () => saved,
    input: () => tree().find(node => node.type === 'textarea').props,
    save: () => tree().find(node => node.type === 'button' && node.props.children === 'codex.proxy.env.save').props };
}
test('one proxy text area supports manual input without a new checkbox or automatic writes', async () => {
  const h = harness(true); await settlePromises();
  assert.equal(h.input().disabled, false);
  assert.equal(h.tree().filter(node => node.type === 'input').length, 0);
  h.input().onChange({ target: { value: 'HTTP_PROXY=http://127.0.0.1:7890' } });
  assert.equal(h.calls.length, 0);
  h.save().onClick(); await settlePromises();
  assert.deepEqual(JSON.parse(JSON.stringify(h.calls)), [['fixture-account', 'HTTP_PROXY=http://127.0.0.1:7890', 'fixture-revision']]);
  assert.equal(h.saved(), 1); h.h.unmount();
});
test('editing and closing does not save, and unreadable storage cannot overwrite a preference', async () => {
  const h = harness(); await settlePromises();
  h.input().onChange({ target: { value: 'HTTPS_PROXY=http://other.invalid' } });
  h.h.unmount(); assert.equal(h.calls.length, 0);
  const broken = harness(false, true); await settlePromises();
  assert.equal(broken.input().disabled, true); broken.save().onClick(); await settlePromises();
  assert.equal(broken.calls.length, 0); broken.h.unmount();
});
