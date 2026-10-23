import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../../tests/helpers/reactHookHarness';

function nodes(value: any): any[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(nodes);
  return [value, ...nodes(value.props?.children)];
}

function harness(initialMode = 'sidecar') {
  const changes: { mode: string; task: ReturnType<typeof deferred<any>> }[] = [];
  const reads: ReturnType<typeof deferred<any>>[] = [];
  const busy: boolean[] = [];
  const notices: string[] = [];
  const props: any = {
    state: { collection: { gatewayMode: initialMode } },
    onState: (state: any) => { props.state = state; },
    onSavingChange: (value: boolean) => busy.push(value),
    onNotice: (value: string) => notices.push(value),
    onError() {},
  };
  const hook = loadHookModule(new URL('./CodexGatewayModeControl.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string, values?: any) => values?.error ? `${key}: ${values.error}` : key }) },
    'lucide-react': { RefreshCw: 'RefreshCw' },
    '../SingleSelectDropdown': { SingleSelectDropdown: 'SingleSelectDropdown' },
    '../../services/codexLocalAccessService': {
      updateCodexLocalAccessGatewayMode(mode: string) {
        const task = deferred<any>(); changes.push({ mode, task }); return task.promise;
      },
      getCodexLocalAccessState() { const task = deferred<any>(); reads.push(task); return task.promise; },
    },
  }, { setTimeout, clearTimeout });
  hook.render(() => hook.exports.CodexGatewayModeControl(props));
  const tree = () => nodes(hook.flush());
  const select = () => tree().find((node) => node.type === 'SingleSelectDropdown').props;
  const error = () => tree().find((node) => node.props?.role === 'alert')?.props.children;
  return { hook, props, changes, reads, busy, notices, tree, select, error };
}

test('engine selection is deduplicated and only changes after backend confirmation', async () => {
  const h = harness();
  assert.equal(h.select().value, 'sidecar');
  assert.equal(h.changes.length, 0);
  const select = h.select();
  select.onChange('native'); select.onChange('native');
  assert.equal(h.changes.length, 1);
  assert.equal(h.select().value, 'sidecar');
  assert.equal(h.select().disabled, true);
  h.changes[0].task.resolve({ collection: { gatewayMode: 'native' } });
  await settlePromises();
  assert.equal(h.select().value, 'native');
  assert.equal(Boolean(h.select().disabled), false);
  assert.deepEqual(h.busy, [true, false]);
  assert.equal(h.notices[h.notices.length - 1], 'codex.localAccess.gatewayModeSaveSuccess');
});

test('missing engine selection displays native without changing an explicit Go choice', () => {
  const h = harness();
  h.props.state = { collection: {} };
  assert.equal(h.select().value, 'native');
  assert.equal(h.select().options[0].value, 'native');
  h.props.state = { collection: { gatewayMode: 'sidecar' } };
  assert.equal(h.select().value, 'sidecar');
  assert.equal(h.changes.length, 0);
  assert.equal(h.reads.length, 0);
  h.hook.unmount();
});

test('failed switching restores backend state and preserves an inline retryable error', async () => {
  const h = harness();
  h.select().onChange('native');
  h.changes[0].task.reject(Error('port occupied'));
  await settlePromises();
  assert.match(h.error(), /port occupied/);
  assert.equal(h.reads.length, 1);
  h.reads[0].resolve({ collection: { gatewayMode: 'sidecar' } });
  await settlePromises();
  assert.equal(h.select().value, 'sidecar');
  assert.equal(Boolean(h.select().disabled), false);
  assert.match(h.error(), /port occupied/);
  h.select().onChange('native');
  assert.equal(h.error(), undefined);
  h.changes[1].task.resolve({ collection: { gatewayMode: 'native' } });
  await settlePromises();
});

test('unconfirmed backend state blocks further switching until explicit refresh succeeds', async () => {
  const h = harness();
  h.select().onChange('native');
  h.changes[0].task.reject(Error('switch failed'));
  await settlePromises();
  h.reads[0].reject(Error('IPC disconnected'));
  await settlePromises();
  assert.equal(h.select().value, '');
  assert.equal(h.select().disabled, true);
  assert.match(h.error(), /IPC disconnected/);
  h.select().onChange('native');
  assert.equal(h.changes.length, 1);
  h.tree().find((node) => node.type === 'button').props.onClick();
  assert.equal(h.error(), undefined);
  h.reads[1].resolve({ collection: { gatewayMode: 'sidecar' } });
  await settlePromises();
  assert.equal(h.select().value, 'sidecar');
  assert.equal(Boolean(h.select().disabled), false);
});

test('legacy values stay readable but cannot be newly selected; busy and absent services cannot switch', () => {
  const h = harness('legacy');
  assert.equal(h.select().value, 'legacy');
  assert.deepEqual(Array.from(h.select().options, (option: any) => option.value), ['native', 'sidecar']);
  h.select().onChange('legacy');
  h.props.disabled = true;
  h.select().onChange('native');
  h.props.disabled = false;
  h.props.state = null;
  h.select().onChange('native');
  assert.equal(h.changes.length, 0);
});

test('engine selection stays disabled while either engine prepares or recovers', () => {
  for (const mode of ['native', 'sidecar', 'legacy']) {
    const h = harness(mode);
    assert.equal(Boolean(h.select().disabled), false);
    h.props.disabled = true;
    assert.equal(h.select().disabled, true);
    h.props.disabled = false;
    h.props.state.preparing = true;
    assert.equal(h.select().disabled, true);
    h.props.state.preparing = false;
    h.props.state.recovering = true;
    assert.equal(h.select().disabled, true);
    h.props.state.recovering = false;
    h.props.state = null;
    assert.equal(h.select().disabled, true);
    h.hook.unmount();
  }
});

test('native service failures use localized messages and retain backend diagnostics', async () => {
  for (const [code, key, detail] of [
    ['NATIVE_API_BUSY', 'gatewayEngineRequestsBusy', ''],
    ['NATIVE_API_SWITCH_BUSY', 'gatewayEngineOperationBusy', ''],
    ['NATIVE_API_SWITCH_FAILED', 'gatewayEngineSwitchRestored', 'port occupied'],
    ['NATIVE_API_ROLLBACK_FAILED', 'gatewayEngineRestoreFailed', 'port occupied; restore unavailable'],
    ['NATIVE_API_STOPPED', 'gatewayEngineStopped', ''],
  ]) {
    const h = harness();
    h.select().onChange('native');
    h.changes[0].task.reject(Error(code + (detail ? `: ${detail}` : '')));
    await settlePromises();
    h.reads[0].resolve({ collection: { gatewayMode: 'sidecar' } });
    await settlePromises();
    assert.match(h.error(), new RegExp(key));
    assert.equal(h.error().includes(code), false);
    if (detail) assert.equal(h.error().includes(detail), true);
  }
});

test('plan membership blocks another engine with a localized remedy and preserves the running native service', async () => {
  const h = harness('native');
  const current = { running: true, collection: { gatewayMode: 'native', enabled: true } };
  h.props.state = current;
  h.select().onChange('sidecar');
  assert.equal(h.changes.length, 1);
  h.changes[0].task.reject(Error('CHATGPT_PLAN_NATIVE_REQUIRED'));
  await settlePromises();
  assert.equal(h.props.state, current);
  assert.match(h.error(), /chatgptPlan\.error\.nativeSwitch/);
  assert.equal(h.error().includes('CHATGPT_PLAN_NATIVE_REQUIRED'), false);
  h.reads[0].resolve(current);
  await settlePromises();
  assert.equal(h.select().value, 'native');
  assert.equal(h.props.state.running, true);
  assert.equal(h.props.state.collection.enabled, true);
  assert.equal(Boolean(h.select().disabled), false);
  assert.equal(h.notices.some((notice) => notice === 'codex.localAccess.gatewayModeSaveSuccess'), false);
  assert.deepEqual(h.busy, [true, false]);
  assert.equal(h.hook.exports.formatCodexGatewayModeError((key: string) => key, 'CHATGPT_PLAN_NATIVE_REQUIRED: hidden upstream detail'), 'chatgptPlan.error.nativeSwitch');
  h.hook.unmount();
});

test('gateway settings retain only engine selection and the restart notice without extra service reads', () => {
  for (const mode of ['native', 'sidecar', 'legacy']) {
    const h = harness(mode);
    const assertSettings = () => {
      assert.deepEqual(h.tree().filter((node) => node.type === 'p').map((node) => node.props.children), ['codex.localAccess.gatewayEngineHint']);
      assert.equal(h.tree().filter((node) => node.type === 'SingleSelectDropdown').length, 1);
      assert.equal(h.tree().some((node) => node.type === 'button' || node.type === 'input'), false);
      assert.equal(h.tree().filter((node) => typeof node.type === 'function').length, 0);
    };
    assertSettings();
    h.props.disabled = true;
    h.props.state.preparing = true;
    assertSettings();
    h.props.state = null;
    assertSettings();
    assert.equal(h.changes.length, 0);
    assert.equal(h.reads.length, 0);
    assert.deepEqual(h.busy, []);
    h.hook.unmount();
  }
});
