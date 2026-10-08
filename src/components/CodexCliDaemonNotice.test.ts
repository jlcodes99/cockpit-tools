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
    console,
    navigator: { clipboard: { writeText(command: string) {
      const task = deferred<void>();
      copies.push({ command, task });
      return task.promise;
    } } },
  });
  h.render(() => h.exports.CodexCliDaemonNotice());
  return {
    h, copies, registration,
    button(key: string) {
      const button = nodes(h.flush()).find((node) => node.type === 'button' && node.props.children === `common.${key}`);
      assert.ok(button, `Missing ${key}`);
      return button;
    },
    notify(payload: unknown) { receive({ payload }); return h.flush(); },
    escape() { escape?.(); return h.flush(); },
    register() { registration.resolve(() => { cleanups += 1; }); },
    cleanups: () => cleanups,
  };
}

const command = "CODEX_HOME='/tmp/Alice'\"'\"'s Codex/$profile' codex app-server daemon restart";

test('daemon guidance ignores invalid events, deduplicates and copies the exact command', async () => {
  const h = harness();
  h.register();
  for (const payload of [null, {}, 42, '', '  ']) assert.equal(h.notify(payload), null);
  h.notify(command);
  h.notify(command);
  assert.equal(nodes(h.h.flush()).find((node) => node.type === 'pre')?.props.children, command);
  const overlay = nodes(h.h.flush()).find((node) => node.props.className === 'modal-overlay codex-cli-daemon-overlay');
  assert.equal(overlay?.props.onClick, undefined);
  h.button('copy').props.onClick();
  assert.equal(h.button('copy').props.disabled, true);
  assert.equal(h.copies[0].command, command);
  h.copies[0].task.resolve();
  await settlePromises();
  h.button('copied');
  assert.equal(h.escape(), null);
  h.h.unmount();
  assert.equal(h.cleanups(), 1);
});

test('copy failures remain inside the notice and stale completions do not change the next profile', async () => {
  const h = harness();
  h.notify(command);
  h.button('copy').props.onClick();
  h.copies[0].task.reject(new Error('clipboard unavailable'));
  await settlePromises();
  assert.equal(nodes(h.h.flush()).find((node) => node.props.role === 'alert')?.props.children, 'codex.cliDaemon.copyFailed');
  h.button('copy').props.onClick();
  assert.equal(nodes(h.h.flush()).some((node) => node.props.role === 'alert'), false);
  h.notify('second profile command');
  h.button('close').props.onClick();
  assert.equal(nodes(h.h.flush()).find((node) => node.type === 'pre')?.props.children, 'second profile command');
  h.copies[1].task.resolve();
  await settlePromises();
  h.button('copy');
  assert.equal(h.escape(), null);
});

test('unmount releases late event registration and discards late events', async () => {
  const h = harness();
  h.h.unmount();
  h.register();
  await settlePromises();
  assert.equal(h.cleanups(), 1);
  assert.equal(h.notify(command), null);
});
