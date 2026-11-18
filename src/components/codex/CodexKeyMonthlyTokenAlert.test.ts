import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';

type Element = { type: unknown; props: Record<string, unknown> };
function elements(value: unknown): Element[] {
  if (Array.isArray(value)) return value.flatMap(elements);
  if (!value || typeof value !== 'object' || !('props' in value)) return [];
  const element = value as Element;
  return [element, ...elements(element.props.children)];
}

test('monthly token reminder edits one key, preserves a failed draft, and can be disabled', async () => {
  const calls: Array<[string, number | undefined]> = [];
  let rejectSave = true;
  let used = 1_900;
  const harness = loadHookModule(new URL('./CodexKeyMonthlyTokenAlert.tsx', import.meta.url), {
    'react-i18next': {
      useTranslation: () => ({ t: (key: string, options?: { used: string }) => options?.used ?? key, i18n: { language: 'en' } }),
    },
    '../ModalErrorMessage': { ModalErrorMessage: 'mock-error' },
    '../../services/codexLocalAccessService': {
      async getCodexMonthlyTokenAlert(id: string, threshold?: number) {
        calls.push([id, threshold]);
        if (threshold != null && rejectSave) throw new Error('Could not write rule');
        return { threshold: threshold || null, used };
      },
    },
  });
  harness.render(() => harness.exports.CodexKeyMonthlyTokenAlert({ keyId: 'key-a' }));
  await settlePromises();
  const all = () => elements(harness.flush());
  const input = () => all().find(element => element.type === 'input')!;
  const change = (value: string) => {
    (input().props.onChange as (event: { target: { value: string } }) => void)({ target: { value } });
  };
  const click = () => {
    const btn = all().find(element => element.type === 'button' && element.props.children === 'common.save')!;
    (btn.props.onClick as () => void)();
  };
  assert.ok(all().some(element => element.type === 'strong' && element.props.children === '1,900'));
  change('3.5');
  click();
  assert.equal(calls.length, 1);
  change('2000');
  click();
  await settlePromises();
  assert.match(String(all().find(element => element.type === 'mock-error')?.props.message), /Could not write rule/);
  assert.equal(input().props.value, '2000');

  used = 2_300;
  const refresh = all().find(element => element.type === 'button' && element.props.children === 'common.refresh');
  assert.ok(refresh, 'monthly usage can be refreshed from the open key editor');
  (refresh.props.onClick as () => void)();
  await settlePromises();
  assert.ok(all().some(element => element.type === 'strong' && element.props.children === '2,300'));
  assert.equal(input().props.value, '2000');

  rejectSave = false;
  click();
  await settlePromises();
  assert.equal(all().find(element => element.type === 'mock-error')?.props.message, null);
  change('');
  click();
  await settlePromises();
  assert.deepEqual(calls, [['key-a', undefined], ['key-a', 2000], ['key-a', undefined], ['key-a', 2000], ['key-a', 0]]);
  harness.unmount();
});
