import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';

function nodes(value: any): any[] {
  if (Array.isArray(value)) return value.flatMap(nodes);
  return value?.props ? [value, ...nodes(value.props.children)] : [];
}

test('probe avoids duplicate submissions and keeps cleanup failures visible inside the dialog', async () => {
  const probe = deferred<any>();
  let calls = 0;
  let closed = 0;
  const h = loadHookModule(new URL('./WorkbuddyModelProbeModal.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../types/workbuddy': { getWorkbuddyAccountDisplayEmail: (account: any) => account.email },
    '../SingleSelectFilterDropdown': { SingleSelectFilterDropdown: 'picker' },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../../services/workbuddyService': {
      fetchWorkbuddyCloudModels: async () => [],
      probeWorkbuddyModel: () => { calls++; return probe.promise; },
    },
    '../../utils/workbuddyProbeStatus': {
      clearWorkbuddyProbeRecord() {}, setWorkbuddyProbeRecord() {}, parseProbeResetAt: () => null,
    },
  });
  h.render(() => h.exports.WorkbuddyModelProbeModal({
    accounts: [{ id: 'a', email: 'a@example.invalid' }], currentAccountId: 'a', onClose: () => { closed++; },
  }));
  await settlePromises();
  nodes(h.flush()).find((node) => node.type === 'input' && node.props.id === 'workbuddy-probe-model')
    .props.onChange({ target: { value: 'chosen' } });
  const tree = nodes(h.flush());
  const submit = tree.find((node) => node.type === 'button' && node.props.className === 'btn btn-primary');
  assert.equal(submit.props.disabled, false);
  submit.props.onClick();
  submit.props.onClick();
  assert.equal(calls, 1);
  assert.equal(nodes(h.flush()).find((node) => node.props.className === 'modal-overlay').props.onClick, undefined);
  const cancel = nodes(h.flush()).find((node) => node.type === 'button' && node.props.className === 'btn btn-secondary');
  assert.equal(cancel.props.disabled, undefined);
  probe.resolve({ status: 'available', message: '', reply: 'OK', conversationId: 'created',
    conversationDeleted: false, cleanupError: 'HTTP 503', keepConversation: false });
  await settlePromises();
  const result = nodes(h.flush());
  assert.ok(result.some((node) => node.props.children === 'workbuddy.modelProbe.retained'));
  assert.ok(!result.some((node) => node.props.children === 'workbuddy.modelProbe.deleted'));
  assert.ok(result.some((node) => Array.isArray(node.props.children) && node.props.children.includes('workbuddy.modelProbe.cleanupFailed')));
  cancel.props.onClick();
  assert.equal(closed, 1);
  h.unmount();
});
