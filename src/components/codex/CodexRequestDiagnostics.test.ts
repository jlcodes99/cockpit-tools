import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';
import * as diagnostics from '../../utils/codexRequestDiagnostics';

type Element = { type: unknown; props: Record<string, any> };
function nodes(value: any): Element[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(nodes);
  return [{ type: value.type, props: value.props ?? {} }, ...nodes(value.props?.children)];
}
const t = (key: string) => key;
const modalImports = {
  'react-i18next': { useTranslation: () => ({ t }) },
  '../ModalErrorMessage': { ModalErrorMessage: 'ModalErrorMessage' },
  '../SingleSelectDropdown': { SingleSelectDropdown: 'SingleSelectDropdown' },
};

test('payload setting retains old preference on save failure, deduplicates saves and requires confirmation before clearing', async () => {
  const saves: ReturnType<typeof deferred<any>>[] = [];
  let savedEnabled = false;
  let clearCalls = 0;
  const win = new EventTarget();
  const hook = loadHookModule(new URL('./CodexRequestPayloadSetting.tsx', import.meta.url), {
    ...modalImports,
    '../../utils/codexRequestDiagnostics': diagnostics,
    '../../utils/tauriEventListener': { async listenSafely() { return () => {}; } },
    '../../services/codexLocalAccessService': {
      async getCodexLocalAccessRequestPayloadLogging() { return savedEnabled; },
      async getCodexLocalAccessRequestPayloadLoggingStatus() { return { pending: false }; },
      updateCodexLocalAccessRequestPayloadLogging() { const task = deferred<any>(); saves.push(task); return task.promise.then((state) => { savedEnabled = state.collection.requestPayloadLogging; return state; }); },
      async clearCodexLocalAccessRequestPayloads() { clearCalls++; return 1; },
    },
  }, { window: win, Event });
  hook.render(() => hook.exports.CodexRequestPayloadSetting());
  await settlePromises();
  const render = () => nodes(hook.flush());
  let select = render().find((node) => node.type === 'SingleSelectDropdown')!;
  assert.equal(select.props.value, 'false');
  assert.equal(saves.length, 0);
  select.props.onChange('true');
  select.props.onChange('true');
  assert.equal(saves.length, 1);
  saves[0].reject(new Error('disk full'));
  await settlePromises();
  assert.match(render().find((node) => node.type === 'ModalErrorMessage')!.props.message, /disk full/);
  select = render().find((node) => node.type === 'SingleSelectDropdown')!;
  assert.equal(select.props.value, 'false');
  select.props.onChange('true');
  assert.equal(render().find((node) => node.type === 'ModalErrorMessage')!.props.message, '');
  saves[1].resolve({ collection: { requestPayloadLogging: true } });
  await settlePromises();
  assert.equal(render().find((node) => node.type === 'SingleSelectDropdown')!.props.value, 'true');
  render().find((node) => node.type === 'button' && node.props.children === 'codex.requestDiagnostics.clearPayloads')!.props.onClick();
  assert.equal(clearCalls, 0);
  render().find((node) => node.type === 'button' && node.props.children === 'common.confirm')!.props.onClick();
  await settlePromises();
  assert.equal(clearCalls, 1);
  assert.equal(render().find((node) => node.type === 'SingleSelectDropdown')!.props.value, 'true');
  hook.unmount();
});

test('request details open before IPC completes, reject stale responses and keep copy failures inside the dialog', async () => {
  const reads: { id: string; task: ReturnType<typeof deferred<any>> }[] = [];
  let current = { requestId: 'A', latencyMs: 100, firstResponseMs: null };
  const hook = loadHookModule(new URL('./CodexRequestDetailModal.tsx', import.meta.url), {
    ...modalImports,
    'react-dom': { createPortal: (value: unknown) => value },
    '../../utils/codexRequestDiagnostics': diagnostics,
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap() {} },
    '../../hooks/useModalScrollLock': { useModalScrollLock() {} },
    '../../services/codexLocalAccessService': { getCodexLocalAccessRequestDetail(id: string) { const task = deferred<any>(); reads.push({ id, task }); return task.promise; } },
  }, { document: { body: {} }, navigator: { clipboard: { async writeText() { throw new Error('clipboard denied'); } } } });
  hook.render(() => hook.exports.CodexRequestDetailModal({ event: current, maskAccountText: (name: string) => name, onClose() {} }));
  assert.equal(nodes(hook.flush()).some((node) => node.props.role === 'dialog'), true);
  current = { ...current, requestId: 'B' };
  hook.flush();
  reads[0].task.resolve({ requestId: 'A', firstResponseMs: 999, attempts: [], payloads: [], truncated: false });
  await settlePromises();
  assert.equal(nodes(hook.flush()).some((node) => node.props.children === '999 ms'), false);
  reads[1].task.resolve({ requestId: 'B', firstResponseMs: 0, attempts: [], payloads: [], truncated: false });
  await settlePromises();
  const tree = nodes(hook.flush());
  assert.equal(tree.some((node) => node.props.children === '0 ms'), true);
  await tree.find((node) => node.type === 'button' && Array.isArray(node.props.children) && node.props.children.includes('common.copy'))!.props.onClick();
  await settlePromises();
  const failed = nodes(hook.flush());
  assert.match(failed.find((node) => node.type === 'ModalErrorMessage')!.props.message, /clipboard denied/);
  assert.equal(failed.some((node) => node.props.role === 'dialog'), true);
  hook.unmount();
});

test('a saved setting with a gateway application failure stays visible and retries without toggling the preference', async () => {
  let failure: string | null = 'connection refused';
  const applied: boolean[] = [];
  const hook = loadHookModule(new URL('./CodexRequestPayloadSetting.tsx', import.meta.url), {
    ...modalImports,
    '../../utils/codexRequestDiagnostics': diagnostics,
    '../../utils/tauriEventListener': { async listenSafely() { return () => {}; } },
    '../../services/codexLocalAccessService': {
      async getCodexLocalAccessRequestPayloadLogging() { return true; },
      async getCodexLocalAccessRequestPayloadLoggingStatus() { return { pending: false, error: failure }; },
      async updateCodexLocalAccessRequestPayloadLogging(enabled: boolean) { applied.push(enabled); failure = null; return { collection: { requestPayloadLogging: enabled } }; },
      async clearCodexLocalAccessRequestPayloads() { return 0; },
    },
  }, { window: new EventTarget(), Event });
  hook.render(() => hook.exports.CodexRequestPayloadSetting());
  await settlePromises();
  let tree = nodes(hook.flush());
  assert.equal(tree.find((node) => node.type === 'SingleSelectDropdown')!.props.value, 'true');
  assert.equal(tree.find((node) => node.type === 'ModalErrorMessage')!.props.message, 'codex.requestDiagnostics.applyFailed');
  tree.find((node) => node.type === 'button' && node.props.children === 'common.retry')!.props.onClick();
  await settlePromises();
  tree = nodes(hook.flush());
  assert.deepEqual(applied, [true]);
  assert.equal(tree.find((node) => node.type === 'ModalErrorMessage')!.props.message, '');
  hook.unmount();
});
