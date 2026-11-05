import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';

type Node = { type: any; props: Record<string, any> };
function nodes(value: any): Node[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(nodes);
  return [{ type: value.type, props: value.props ?? {} }, ...nodes(value.props?.children)];
}
function harness(query: () => Promise<unknown> = async () => ({}), save: () => Promise<unknown> = async () => ({})) {
  const error = { message: '', scrollKey: 0, clear() { this.message = ''; }, report(message: string) { this.message = message; this.scrollKey++; } };
  const calls = { queries: [] as any[], saves: [] as any[], closed: 0 };
  const hook = loadHookModule(new URL('./OpenAIUsageSettingsDialog.tsx', import.meta.url), {
    'react-dom': { createPortal: (node: unknown) => node },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../../hooks/useModalScrollLock': { useModalScrollLock() {} },
    '../ModalErrorMessage': { ModalErrorMessage: 'ErrorMessage', useModalErrorState: () => error },
    '../../services/modelProviderUsageService': { queryModelProviderUsage(input: unknown) { calls.queries.push(input); return query(); } },
    '../../services/codexModelProviderService': {
      updateCodexOpenAIUsageKey(...args: unknown[]) { calls.saves.push(args); return save(); },
      upsertCodexModelProviderFromCredential() { throw new Error('unexpected create'); },
    },
  }, { document: { body: {} } });
  hook.render(() => hook.exports.OpenAIUsageSettingsDialog({ apiKey: 'model-key', provider: { id: 'p', apiKeys: [{ id: 'k', apiKey: 'model-key', usageApiKey: 'admin-old' }] }, t: (key: string) => key, onClose() { calls.closed++; } }));
  const tree = () => nodes(hook.flush());
  const button = (key: string) => tree().find(node => node.type === 'button' && (node.props.children === key || Array.isArray(node.props.children) && node.props.children.includes(key)))!;
  const field = () => tree().find(node => node.type === hook.exports.OpenAIUsageKeyField)!;
  return { hook, tree, button, field, calls, error };
}

test('verification uses only the independent query field and prevents duplicate saves', async () => {
  const gate = deferred<unknown>();
  const h = harness(() => gate.promise);
  h.field().props.onChange('admin-new');
  h.button('codex.openaiUsage.saveVerify').props.onClick();
  h.button('codex.openaiUsage.saveVerify').props.onClick();
  assert.equal(h.calls.queries.length, 1);
  assert.equal(h.calls.queries[0].apiKey, 'model-key');
  assert.equal(h.calls.queries[0].usageApiKey, 'admin-new');
  assert.equal(h.calls.saves.length, 0);
  gate.resolve({}); await settlePromises();
  assert.equal(h.calls.saves.length, 1);
  assert.equal(h.calls.saves[0][2], 'admin-new');
  assert.equal(h.calls.saves[0][3].usageApiKey, 'admin-old');
  assert.equal(h.calls.closed, 1);
  h.hook.unmount();
});

test('query failure retains input and original configuration and stays in the dialog', async () => {
  const h = harness(async () => { throw new Error('OPENAI_USAGE_NETWORK_FAILED'); });
  h.field().props.onChange('admin-new');
  h.button('codex.openaiUsage.saveVerify').props.onClick(); await settlePromises();
  assert.equal(h.calls.saves.length, 0);
  assert.equal(h.calls.closed, 0);
  assert.equal(h.field().props.value, 'admin-new');
  assert.equal(h.error.message, 'codex.openaiUsage.networkFailed');
  assert.equal(h.button('codex.openaiUsage.saveVerify').props.disabled, false);
  h.field().props.onChange('admin-retry');
  assert.equal(h.error.message, '');
  h.hook.unmount();
});

test('write failure remains retryable; removing configuration does not verify or replace the model key', async () => {
  const h = harness(undefined, async () => { throw new Error('disk full'); });
  h.button('codex.openaiUsage.remove').props.onClick(); await settlePromises();
  assert.equal(h.calls.queries.length, 0);
  assert.equal(h.calls.saves[0][2], '');
  assert.equal(h.calls.saves[0][3].apiKey, 'model-key');
  assert.equal(h.calls.closed, 0);
  assert.equal(h.error.message, 'codex.openaiUsage.saveFailed');
  h.hook.unmount();
});

test('closing during verification cancels the subsequent save', async () => {
  const gate = deferred<unknown>();
  const h = harness(() => gate.promise);
  h.button('codex.openaiUsage.saveVerify').props.onClick();
  h.button('common.cancel').props.onClick();
  assert.equal(h.calls.closed, 1);
  h.hook.unmount(); gate.resolve({}); await settlePromises();
  assert.equal(h.calls.saves.length, 0);
});

test('empty query keys are reported at the field without network access', async () => {
  const h = harness();
  h.field().props.onChange('  ');
  h.button('codex.openaiUsage.saveVerify').props.onClick(); await settlePromises();
  assert.equal(h.field().props.error, 'codex.openaiUsage.keyRequired');
  assert.equal(h.calls.queries.length, 0);
  assert.equal(h.calls.saves.length, 0);
  h.hook.unmount();
});

test('small viewports retain a scrolling body and accessible explicit close actions', () => {
  const source = readFileSync(new URL('./OpenAIUsageSettingsDialog.tsx', import.meta.url), 'utf8');
  const css = readFileSync(new URL('./OpenAIOfficialUsagePanel.css', import.meta.url), 'utf8');
  assert.doesNotMatch(source, /className="modal-overlay[^"\n]*"\s+onClick/);
  assert.match(css, /\.modal\.openai-usage-settings-modal[^}]*max-height: calc\(100vh - 32px\)/);
  assert.match(css, /\.openai-usage-settings-modal \.modal-body[^}]*min-height: 0; overflow-y: auto/);
  assert.match(css, /\.openai-usage-settings-modal \.modal-footer[^}]*flex-shrink: 0/);
});

test('invalid query keys produce a field error without persisting or closing', async () => {
  const h = harness(async () => { throw new Error('OPENAI_USAGE_INVALID_KEY'); });
  h.button('codex.openaiUsage.saveVerify').props.onClick(); await settlePromises();
  assert.equal(h.field().props.error, 'codex.openaiUsage.invalidKey');
  assert.equal(h.error.message, '');
  assert.equal(h.calls.saves.length, 0);
  assert.equal(h.calls.closed, 0);
  h.hook.unmount();
});
