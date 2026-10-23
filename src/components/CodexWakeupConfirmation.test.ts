import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';

function harness() {
  const request = { requestId: 'qa', taskName: 'Task', accountCount: 1, expiresAt: Date.now() / 1000 + 60 };
  const calls: any[] = [];
  let trap: any;
  let escape: any;
  let fail = false;
  const h = loadHookModule(new URL('./CodexWakeupConfirmation.tsx', import.meta.url), {
    'react-dom': { createPortal: (tree: unknown) => tree },
    '@tauri-apps/api/core': { invoke: async (name: string, payload?: unknown) => {
      if (name === 'codex_wakeup_pending_confirmations') return [request];
      calls.push([name, payload]);
      if (fail) throw new Error('request expired');
    } },
    '@tauri-apps/api/event': { listen: async () => () => {} },
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../hooks/useEscClose': { useEscCloseTopmost: (open: boolean, _close: () => void, ref: any) => { escape = { open, ref }; } },
    '../hooks/useModalFocusTrap': { useModalFocusTrap: (ref: any, open: boolean) => { trap = { open, ref }; } },
    '../hooks/useModalScrollLock': { useModalScrollLock: () => {} },
  }, { window: { setInterval: () => 1, clearInterval: () => {} }, document: { body: {} } });
  let tree = h.render(() => h.exports.CodexWakeupConfirmation());
  return { h, calls, fail() { fail = true; }, get tree() { return tree; },
    get trap() { return trap; }, get escape() { return escape; },
    async ready() { await settlePromises(); tree = h.flush(); },
    flush() { tree = h.flush(); },
    dialog() { return tree.props.children; },
  };
}

test('recovered confirmation has an initial focus target and cannot close through the backdrop', async () => {
  const h = harness(); await h.ready();
  assert.equal(h.dialog().props.tabIndex, -1);
  assert.equal(h.dialog().props.role, 'dialog');
  assert.equal(h.trap.ref, h.dialog().props.ref);
  assert.equal(h.escape.ref, h.dialog().props.ref);
  assert.equal(h.escape.open, true);
  assert.equal(h.tree.props.onClick, undefined);
  assert.match(h.tree.props.className, /codex-wakeup-confirmation-overlay/);
  h.dialog().props.children[2].props.children[0].props.onClick();
  await h.ready();
  assert.equal(h.tree, null);
  assert.equal(h.calls[0][0], 'codex_wakeup_respond_confirmation');
  assert.equal(h.calls[0][1].approved, false);
  h.h.unmount();
});

test('a response failure stays in the confirmation dialog and enables retry', async () => {
  const h = harness(); await h.ready(); h.fail();
  h.dialog().props.children[2].props.children[1].props.onClick();
  await h.ready();
  assert.equal(h.dialog().props.children[2].props.children[1].props.disabled, false);
  const error = h.dialog().props.children[1].props.children[4];
  assert.equal(error.props.role, 'alert');
  assert.equal(error.props.children, 'wakeup.confirmation.expired');
  h.h.unmount();
});
