import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';

type Element = { type: unknown; props: Record<string, any> };

function nodes(tree: unknown): Element[] {
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  if (!tree || typeof tree !== 'object' || !('props' in tree)) return [];
  const node = tree as Element;
  return [node, ...nodes(node.props.children)];
}

function harness() {
  let receive!: (event: { payload: unknown }) => void;
  let escape: (() => void) | undefined;
  let cleanups = 0;
  const registration = deferred<() => void>();
  const copies: { command: string; task: ReturnType<typeof deferred<void>> }[] = [];
  const errors: unknown[][] = [];
  const h = loadHookModule(new URL('./CodexCliDaemonNotice.tsx', import.meta.url), {
    'react-dom': { createPortal: (value: unknown) => value },
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '@tauri-apps/api/event': {
      listen(name: string, callback: typeof receive) {
        assert.equal(name, 'codex:cli-daemon-restart-required');
        receive = callback;
        return registration.promise;
      },
    },
    '../hooks/useEscClose': { useEscCloseTopmost(open: boolean, close: () => void) { escape = open ? close : undefined; } },
    '../hooks/useModalFocusTrap': { useModalFocusTrap() {} },
    '../hooks/useModalScrollLock': { useModalScrollLock() {} },
  }, {
    document: { body: {} },
    console: { warn: (...args: unknown[]) => errors.push(args) },
    navigator: { clipboard: { writeText(command: string) {
      const task = deferred<void>();
      copies.push({ command, task });
      return task.promise;
    } } },
  });
  h.render(() => h.exports.CodexCliDaemonNotice());
  const button = (key: string) => {
    const result = nodes(h.flush()).find((node) => node.type === 'button' && node.props.children === `common.${key}`);
    assert.ok(result, `Missing ${key} button`);
    return result;
  };
  return {
    h, button, copies, registration, errors,
    notify(payload: unknown) { receive({ payload }); return h.flush(); },
    escape() { escape?.(); return h.flush(); },
    register() { registration.resolve(() => { cleanups += 1; }); },
    cleanups: () => cleanups,
  };
}

const command = "CODEX_HOME='/tmp/Alice'\"'\"'s Codex/$profile' codex app-server daemon restart";

test('daemon notice ignores invalid events, copies the exact command and can close and reopen', async () => {
  const h = harness();
  h.register(); await settlePromises();
  for (const invalid of [null, {}, 42, '']) assert.equal(h.notify(invalid), null);
  assert.equal(nodes(h.notify(command)).find((node) => node.type === 'code')?.props.children, command);
  h.button('copy').props.onClick();
  assert.equal(h.copies[0].command, command);
  h.copies[0].task.resolve(); await settlePromises();
  h.button('copied');
  h.button('close').props.onClick(); assert.equal(h.h.flush(), null);
  h.notify(command); h.button('copy');
  assert.equal(h.escape(), null);
  h.h.unmount(); assert.equal(h.cleanups(), 1);
});

test('clipboard failure clears previous success and retry can recover', async () => {
  const h = harness(); h.register(); h.notify(command);
  h.button('copy').props.onClick();
  h.copies[0].task.resolve(); await settlePromises();
  h.button('copied').props.onClick();
  h.copies[1].task.reject(new Error('clipboard unavailable')); await settlePromises();
  assert.equal(nodes(h.h.flush()).find((node) => node.props.role === 'alert')?.props.children, 'common.shared.export.copyFailed');
  h.button('copy').props.onClick();
  h.copies[2].task.resolve(); await settlePromises();
  h.button('copied');
  assert.equal(nodes(h.h.flush()).some((node) => node.props.role === 'alert'), false);
  h.h.unmount();
});

for (const fail of [false, true]) {
  test(`a new notice ignores a previous pending clipboard ${fail ? 'failure' : 'success'}`, async () => {
    const h = harness(); h.register(); h.notify(command);
    h.button('copy').props.onClick();
    h.notify(command); // The same profile can be switched again while copying.
    if (fail) h.copies[0].task.reject(new Error('late clipboard failure'));
    else h.copies[0].task.resolve();
    await settlePromises();
    h.button('copy');
    assert.equal(nodes(h.h.flush()).some((node) => node.props.role === 'alert'), false);
    h.h.unmount();
  });
}

test('only the latest clipboard attempt can update the notice', async () => {
  const h = harness(); h.register(); h.notify(command);
  h.button('copy').props.onClick(); h.button('copy').props.onClick();
  h.copies[1].task.resolve(); await settlePromises();
  h.copies[0].task.reject(new Error('old attempt failed')); await settlePromises();
  h.button('copied');
  assert.equal(nodes(h.h.flush()).some((node) => node.props.role === 'alert'), false);
  h.h.unmount();
});

test('unmount cleans up a late event registration and ignores subsequent events', async () => {
  const h = harness(); h.h.unmount();
  assert.equal(h.notify(command), null);
  h.register(); await settlePromises();
  assert.equal(h.cleanups(), 1);
});

test('event registration failure is handled without an unhandled rejection', async () => {
  const h = harness();
  h.registration.reject(new Error('event API unavailable')); await settlePromises();
  assert.equal(h.h.flush(), null);
  assert.equal(h.errors.length, 1);
  h.h.unmount();
});
