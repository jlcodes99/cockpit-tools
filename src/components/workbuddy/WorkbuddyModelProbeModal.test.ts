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
  assert.equal(nodes(h.flush()).find((node) => node.props.className === 'modal-overlay wb-probe-overlay').props.onClick, undefined);

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

  const detail = nodes(h.flush());
  assert.ok(detail.some((node) => node.props.className === 'wb-probe-reply' && node.props.children === 'OK'));
  assert.ok(!detail.some((node) => node.props.className === 'wb-probe-model-list'));
  assert.ok(detail.some((node) => node.props.children === 'workbuddy.modelProbe.retained'));
  assert.ok(!detail.some((node) => node.props.children === 'workbuddy.modelProbe.deleted'));
  assert.ok(detail.some((node) => Array.isArray(node.props.children) && node.props.children.includes('workbuddy.modelProbe.cleanupFailed')));

  const backToResults = detail.find((node) => node.props.className === 'wb-probe-quiet');
  backToResults.props.onClick();
  const cell = nodes(h.flush()).find(
    (node) => node.type === 'button' && node.props.className === 'wb-probe-cell ok',
  );
  assert.ok(cell, 'returning to the matrix should retain the result');
  cell.props.onClick();
  assert.ok(nodes(h.flush()).some((node) => node.props.className === 'wb-probe-reply'));

  const backToSetup = nodes(h.flush()).find(
    (node) => node.type === 'button' && node.props.className === 'btn btn-secondary',
  );
  backToSetup.props.onClick();
  assert.ok(nodes(h.flush()).some((node) => node.props.className === 'wb-probe-model-list'));
  assert.equal(closed, 0);
  h.unmount();
});

const PROBE_MODAL_MODULE_MOCKS = {
  'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
  '../../types/workbuddy': { getWorkbuddyAccountDisplayEmail: (account: any) => account.email },
  '../../presentation/platformAccountPresentation': { buildWorkbuddyAccountPresentation: () => ({ planLabel: 'PRO', planClass: 'pro' }) },
  '../../hooks/useEscClose': { useEscCloseTopmost() {} },
};

const PROBE_MODAL_STATUS_MOCKS = {
  clearWorkbuddyProbeRecord() {},
  setWorkbuddyProbeRecord() {},
  parseProbeResetAt: () => null,
  dismissWorkbuddyProbeRecord() {},
  subscribeWorkbuddyProbeStatus: () => () => {},
};

function probeModalHarness(saved: string[], catalog: () => Promise<any[]>, probed: string[] = []) {
  return loadHookModule(new URL('./WorkbuddyModelProbeModal.tsx', import.meta.url), {
    ...PROBE_MODAL_MODULE_MOCKS,
    '../../services/workbuddyService': {
      fetchWorkbuddyCloudModels: catalog,
      fetchCodebuddyCnCloudModels: async () => [],
      probeWorkbuddyModel: (_accountId: string, model: string) => {
        probed.push(model);
        return Promise.resolve({
          status: 'available',
          message: '',
          reply: 'OK',
          conversationId: 'created',
          conversationDeleted: true,
          cleanupError: null,
          keepConversation: false,
        });
      },
      probeCodebuddyCnModel: () => { throw new Error('unused'); },
    },
    '../../utils/workbuddyProbeStatus': PROBE_MODAL_STATUS_MOCKS,
  }, {
    localStorage: {
      getItem: () => JSON.stringify(saved),
      setItem() {},
    },
  });
}

test('retired models stay saved but are excluded from the count and the probe batch', async () => {
  const probed: string[] = [];
  const h = probeModalHarness(
    ['stale-model', 'kept-model'],
    async () => [
      { id: 'kept-model', name: 'Kept', credits: null, description: null, supportsImages: false, isDefault: false },
    ],
    probed,
  );

  h.render(() => h.exports.WorkbuddyModelProbeModal({
    accounts: [{ id: 'a', email: 'a@example.invalid' }],
    currentAccountId: 'a',
    onClose: () => {},
  }));
  await settlePromises();

  const count = nodes(h.flush()).find((node) => node.props?.className === 'wb-probe-selection-count');
  assert.ok(count, 'selection count should render');
  assert.equal(count.props.children, 1, 'the retired model must not count as runnable');

  const submit = nodes(h.flush()).find(
    (node) => node.type === 'button' && node.props.className === 'btn btn-primary',
  );
  assert.ok(submit, 'submit button should render');
  assert.equal(submit.props.disabled, false, 'a live selection keeps the batch runnable');
  submit.props.onClick();
  await settlePromises();

  assert.deepEqual(probed, ['kept-model'], 'the retired model must not be probed');
  h.unmount();
});

test('a failed catalog fetch keeps the saved selection runnable', async () => {
  const h = probeModalHarness(['stale-model', 'kept-model'], async () => {
    throw new Error('offline');
  });

  h.render(() => h.exports.WorkbuddyModelProbeModal({
    accounts: [{ id: 'a', email: 'a@example.invalid' }],
    currentAccountId: 'a',
    onClose: () => {},
  }));
  await settlePromises();

  const count = nodes(h.flush()).find((node) => node.props?.className === 'wb-probe-selection-count');
  assert.ok(count, 'selection count should render even when the catalog fails');
  assert.equal(count.props.children, 2, 'without a loaded catalog the saved selection stays runnable');
  h.unmount();
});
