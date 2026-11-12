import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness.ts';

type Element = {
  type: string;
  props: {
    children?: unknown;
    role?: string;
    value?: string;
    onChange?: (event: { target: { value: string } }) => void;
    onSubmit?: (event: { preventDefault: () => void }) => void;
  };
};

function elements(node: unknown): Element[] {
  if (Array.isArray(node)) return node.flatMap(elements);
  if (!node || typeof node !== 'object') return [];
  const element = node as Element;
  return [element, ...elements(element.props?.children)];
}

test('a failed token save keeps the draft and a successful retry clears it', async () => {
  const writes: unknown[] = [];
  const harness = loadHookModule(new URL('./CodexQuotaRecoverySettings.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '@tauri-apps/api/event': { listen: async () => () => {} },
    '@tauri-apps/api/core': {
      invoke: async (command: string, args: unknown) => {
        if (command === 'codex_quota_recovery_enabled') return false;
        assert.equal(command, 'codex_quota_recovery_set_token');
        writes.push(args);
        if (writes.length === 1) throw new Error('fixture storage failure');
        return true;
      },
    },
  });
  harness.render(() => harness.exports.CodexQuotaRecoverySettings());
  await settlePromises();
  const find = (type: string) => elements(harness.flush()).find((element) => element.type === type)!.props;
  find('input').onChange!({ target: { value: 'fixture-token' } });
  const submit = () => find('form').onSubmit!({ preventDefault() {} });
  submit();
  await settlePromises();
  assert.equal(find('input').value, 'fixture-token');
  assert.ok(elements(harness.flush()).some((element) => element.props?.role === 'alert'));
  submit();
  await settlePromises();
  assert.equal(find('input').value, '');
  assert.equal(writes.length, 2);
  assert.equal((writes[1] as { newToken: string }).newToken, 'fixture-token');
  assert.ok(elements(harness.flush()).some((element) => element.props?.children === 'common.enabled'));
  assert.ok(!elements(harness.flush()).some((element) => element.props?.role === 'alert'));
  harness.unmount();
});
