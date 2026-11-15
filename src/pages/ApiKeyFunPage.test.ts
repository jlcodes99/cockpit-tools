import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';
import type { ManagedApiKey } from '../services/apiKeyFunVault';

function nodes(tree: any): any[] {
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  return tree?.props ? [tree, ...nodes(tree.props.children)] : [];
}

function setup() {
  let keys: ManagedApiKey[] = [{ id: 'test-key', name: 'Example key', key: 'synthetic-key', createdAt: 1, lastUsedAt: 1 }];
  let onKeysChanged: (keys: ManagedApiKey[]) => void;
  const writes: ReturnType<typeof deferred<void>>[] = [];
  const t = (key: string) => key;
  const hook = loadHookModule(new URL('./ApiKeyFunPage.tsx', import.meta.url), {
    '@tauri-apps/api/core': { isTauri: () => true },
    '@tauri-apps/plugin-dialog': {},
    '@tauri-apps/plugin-opener': {},
    'react-i18next': { useTranslation: () => ({ t }) },
    '../services/modelProviderUsageService': {},
    '../utils/apikeyFunLinks': { buildApiKeyFunProviderBaseUrl: () => 'https://example.test/v1' },
    '../utils/apiKeyFunPrefill': {},
    '../utils/apikeyFunClientConfigs': { resolveApiKeyFunGrokModel: () => null },
    '../components/apiKeyFun/ApiKeyFunKeyConfigModal': { ApiKeyFunKeyConfigModal: 'KeyConfig' },
    '../components/apiKeyFun/ApiKeyFunPromotionHeader': { ApiKeyFunPromotionHeader: 'PromotionHeader' },
    '../components/ModalErrorMessage': {
      ModalErrorMessage: 'ModalErrorMessage',
      useModalErrorState: () => {
        const [message, setMessage] = hook.hooks.useState(null);
        return { message, clear: () => setMessage(null), report: setMessage };
      },
    },
    '../stores/useSponsorStore': { useSponsorStore: (select: (value: unknown) => unknown) => select({ state: {} }) },
    '../services/apiKeyFunVault': {
      initializeApiKeyFunVault: async () => {},
      subscribeApiKeyFunVault: (callback: typeof onKeysChanged) => {
        onKeysChanged = callback;
        callback(keys);
        return () => {};
      },
      requiresApiKeyFunRecovery: (error: unknown) => error === 'recovery-required',
      updateApiKeyFunVault: (update: (keys: ManagedApiKey[]) => ManagedApiKey[]) => {
        const write = deferred<void>();
        writes.push(write);
        return write.promise.then(() => { keys = update(keys); onKeysChanged(keys); });
      },
    },
  }, { window: { setTimeout: () => 1, clearTimeout() {} } });
  hook.render(() => hook.exports.ApiKeyFunPage());
  const tree = () => nodes(hook.flush());
  const dialog = () => tree().find(n => n.props.role === 'alertdialog');
  const confirm = () => nodes(dialog()).find(n => n.props.className === 'btn btn-danger');
  const error = () => nodes(dialog()).find(n => n.type === 'ModalErrorMessage')?.props.message;
  const open = () => tree().find(n => n.props.className === 'apikey-fun-key-delete').props.onClick({ stopPropagation() {} });
  return { hook, tree, dialog, confirm, error, open, writes, keys: () => keys };
}

test('failed deletion keeps the confirmation open and a successful retry closes it', async () => {
  const h = setup();
  h.open();
  const confirm = h.confirm();
  confirm.props.onClick();
  confirm.props.onClick();
  assert.equal(h.writes.length, 1);
  assert.equal(h.confirm().props.disabled, true);
  assert.ok(h.dialog());

  h.writes[0].reject(new Error('write failed'));
  await settlePromises();
  assert.ok(h.dialog());
  assert.equal(h.error(), 'apiKeyFun.keyManager.vaultWriteFailed');
  assert.equal(h.tree().some(n => n.props.role === 'alert' && n.props.children === h.error()), false);
  assert.equal(h.confirm().props.disabled, false);
  assert.equal(h.keys().length, 1);

  h.confirm().props.onClick();
  assert.equal(h.error(), null);
  h.writes[1].resolve();
  await settlePromises();
  assert.equal(h.dialog(), undefined);
  assert.equal(h.keys().length, 0);
  h.hook.unmount();
});

test('vault recovery is reported inside the confirmation before returning to the recovery action', async () => {
  const h = setup();
  h.open();
  h.confirm().props.onClick();
  h.writes[0].reject('recovery-required');
  await settlePromises();
  assert.equal(h.error(), 'apiKeyFun.keyManager.checkpointAvailable');
  const recovery = () => h.tree().find(n => n.type === 'button' && n.props.children === 'apiKeyFun.keyManager.restoreCheckpoint');
  assert.equal(recovery(), undefined);
  nodes(h.dialog()).find(n => n.type === 'button' && n.props.children === 'common.cancel').props.onClick();
  assert.equal(h.dialog(), undefined);
  assert.ok(recovery());
  assert.equal(h.keys().length, 1);
  h.hook.unmount();
});
