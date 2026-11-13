import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';

function nodes(value: any): any[] {
  if (Array.isArray(value)) return value.flatMap(nodes);
  return value?.props ? [value, ...nodes(value.props.children)] : [];
}

test('batch probe submits once and keeps cleanup failures visible in the cell detail', async () => {
  const probe = deferred<any>();
  let calls = 0;
  let closed = 0;
  const h = loadHookModule(new URL('./WorkbuddyModelProbeModal.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../types/workbuddy': { getWorkbuddyAccountDisplayEmail: (account: any) => account.email },
    '../../presentation/platformAccountPresentation': { buildWorkbuddyAccountPresentation: () => ({ planLabel: 'PRO', planClass: 'pro' }) },
    '../../hooks/useEscClose': { useEscCloseTopmost() {} },
    '../../services/workbuddyService': {
      fetchWorkbuddyCloudModels: async () => [
        { id: 'chosen', name: 'Chosen', credits: null, description: null, supportsImages: false, isDefault: false },
      ],
      fetchCodebuddyCnCloudModels: async () => [],
      probeWorkbuddyModel: () => { calls++; return probe.promise; },
      probeCodebuddyCnModel: () => { throw new Error('unused'); },
    },
    '../../utils/workbuddyProbeStatus': {
      clearWorkbuddyProbeRecord() {},
      setWorkbuddyProbeRecord() {},
      parseProbeResetAt: () => null,
      dismissWorkbuddyProbeRecord() {},
      subscribeWorkbuddyProbeStatus: () => () => {},
    },
  });

  h.render(() => h.exports.WorkbuddyModelProbeModal({
    accounts: [{ id: 'a', email: 'a@example.invalid' }],
    currentAccountId: 'a',
    onClose: () => { closed++; },
  }));
  await settlePromises();

  // 勾选模型目录里的第一个模型
  const modelLabel = nodes(h.flush()).find((node) => node.props?.className === 'wb-probe-model-item');
  assert.ok(modelLabel, 'model list should render');
  const modelCheckbox = modelLabel.props.children.find((child: any) => child?.type === 'input');
  modelCheckbox.props.onChange({ target: { checked: true } });

  const submit = nodes(h.flush()).find(
    (node) => node.type === 'button' && node.props.className === 'btn btn-primary',
  );
  assert.equal(submit.props.disabled, false);
  submit.props.onClick();
  submit.props.onClick();
  assert.equal(calls, 1);

  // 弹层点击背景不关闭（仅显式关闭）
  assert.equal(nodes(h.flush()).find((node) => node.props.className === 'modal-overlay').props.onClick, undefined);

  probe.resolve({
    status: 'available',
    message: '',
    reply: 'OK',
    conversationId: 'created',
    conversationDeleted: false,
    cleanupError: 'HTTP 503',
    keepConversation: false,
  });
  await settlePromises();

  // 打开单元格详情，确认「已保留 / 清理失败」都可见
  const cell = nodes(h.flush()).find(
    (node) => typeof node.props?.className === 'string' && node.props.className.startsWith('wb-probe-cell'),
  );
  assert.ok(cell, 'result cell should render');
  cell.props.onClick();
  const detail = nodes(h.flush());
  assert.ok(detail.some((node) => node.props.children === 'workbuddy.modelProbe.retained'));
  assert.ok(!detail.some((node) => node.props.children === 'workbuddy.modelProbe.deleted'));
  assert.ok(detail.some((node) => Array.isArray(node.props.children) && node.props.children.includes('workbuddy.modelProbe.cleanupFailed')));

  const cancel = nodes(h.flush()).find(
    (node) => node.type === 'button' && node.props.className === 'btn btn-secondary',
  );
  cancel.props.onClick();
  assert.equal(closed, 1);
  h.unmount();
});
