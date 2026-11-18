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

test('WorkBuddy session read errors are visible and refresh can retry an empty list', async () => {
  let failure: string | null = 'invalid SQLite database';
  let partial = false;
  const t = (key: string, options?: { error: string }) => options?.error ?? key;
  const harness = loadHookModule(new URL('./CodebuddySessionManager.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t, i18n: { language: 'en' } }) },
    '../SingleSelectDropdown': { SingleSelectDropdown: 'select' },
    '../ModalErrorMessage': { ModalErrorMessage: 'load-error' },
    '../../services/codebuddySessionService': {
      async codebuddyListSessionOverview() {
        if (failure) throw new Error(failure);
        if (partial) return {
          sessions: [{ conversationId: 'test', cwd: '/healthy-workspace', userId: '', status: '', updatedAt: null, locations: [] }],
          warnings: ['Managed instance: invalid SQLite database'],
        };
        return { sessions: [], warnings: [] };
      },
    },
  });
  harness.render(() => harness.exports.CodebuddySessionManager({ platform: 'workbuddy' }));
  await settlePromises();

  const failed = elements(harness.flush());
  assert.match(String(failed.find(element => element.type === 'load-error')?.props.message), /invalid SQLite database/);
  assert.ok(!failed.some(element => element.props.children === 'codebuddy.sessionManager.noSessions'));

  failure = null;
  const refresh = failed.find(element => element.type === 'button' && element.props.title === 'common.refresh');
  const reload = refresh?.props.onClick as (() => Promise<void>) | undefined;
  assert.ok(reload);
  await reload();

  const recovered = elements(harness.flush());
  assert.equal(recovered.find(element => element.type === 'load-error')?.props.message, null);
  assert.ok(recovered.some(element => element.props.children === 'codebuddy.sessionManager.noSessions'));

  partial = true;
  await reload();
  const partialList = elements(harness.flush());
  assert.match(String(partialList.find(element => element.type === 'load-error')?.props.message), /Managed instance/);
  assert.ok(partialList.some(element => element.props.children === 'healthy-workspace'));
  harness.unmount();
});

test('WorkBuddy rebind selects one instance, keeps failed drafts, and reloads after success', async () => {
  const requests: string[][] = [];
  let loads = 0;
  let finish: ((count: number) => void) | undefined;
  let fail: ((error: Error) => void) | undefined;
  const harness = loadHookModule(new URL('./CodebuddySessionManager.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }) },
    '@tauri-apps/plugin-dialog': {
      async open(opts: { directory: boolean; multiple: boolean }) {
        assert.equal(opts.directory, true);
        assert.equal(opts.multiple, false);
        return '/workspace/renamed';
      },
    },
    '../SingleSelectDropdown': { SingleSelectDropdown: 'select' },
    '../ModalErrorMessage': { ModalErrorMessage: 'rebind-error' },
    '../../services/codebuddySessionService': {
      async codebuddyListSessionOverview() {
        loads += 1;
        return {
          warnings: [],
          sessions: [{
            conversationId: 'session',
            title: 'local history',
            cwd: loads === 1 ? '/workspace/old' : '/workspace/renamed',
            locations: [
              { instanceId: 'workbuddy', instanceName: 'WorkBuddy', local: true },
              { instanceId: 'managed', instanceName: 'Managed instance', local: true },
              { instanceId: 'cloud', instanceName: 'Cloud only', local: false },
            ],
          }],
        };
      },
      workbuddyRebindWorkspace(...args: string[]) {
        requests.push(args);
        return new Promise<number>((resolve, reject) => {
          finish = resolve;
          fail = reject;
        });
      },
    },
  });
  const key = 'codebuddy.sessionManager.rebind';
  const current = () => elements(harness.flush());
  harness.render(() => harness.exports.CodebuddySessionManager({ platform: 'workbuddy' }));
  await settlePromises();
  const launch = current().find((el) => el.type === 'button' && el.props.children === key + '.title');
  assert.ok(launch);
  (launch.props.onClick as () => void)();

  const instance = current().find((el) => el.type === 'select' && el.props.ariaLabel === key + '.instance');
  assert.ok(instance);
  assert.equal((instance.props.options as unknown[]).length, 2);
  (instance.props.onChange as (value: string) => void)('managed');

  const choose = current().find((el) => el.type === 'button' && el.props.children === key + '.choose');
  assert.ok(choose);
  await (choose.props.onClick as () => Promise<void>)();
  assert.ok(current().some((el) => el.props.children === '/workspace/renamed'));

  const submit = () => {
    const form = current().find((el) => el.type === 'form');
    assert.ok(form);
    const onSubmit = form.props.onSubmit as (event: { preventDefault: () => void }) => void;
    onSubmit({ preventDefault() {} });
  };
  submit();
  submit();
  assert.deepEqual(requests, [['managed', '/workspace/old', '/workspace/renamed']]);
  fail?.(new Error('target contains a different transcript'));
  await settlePromises();
  assert.ok(current().some((el) => String(el.props.message).includes('different transcript')));
  assert.ok(current().some((el) => el.props.children === '/workspace/renamed'));
  assert.equal(loads, 1);

  submit();
  finish?.(4);
  await settlePromises();
  assert.equal(requests.length, 2);
  assert.equal(loads, 2);
  assert.ok(!current().some((el) => el.type === 'form'));
  assert.ok(current().some((el) => el.props.role === 'status' && el.props.children === key + '.success'));
  harness.unmount();
});
