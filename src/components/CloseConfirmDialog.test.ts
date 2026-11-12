import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness.ts';

interface RenderedNode {
  props?: {
    children?: unknown;
    className?: string;
    role?: string;
    onClick?: () => void;
  };
}

function findNode(node: unknown, matches: (element: RenderedNode) => boolean): RenderedNode | undefined {
  if (Array.isArray(node)) {
    for (const child of node) {
      const match = findNode(child, matches);
      if (match) return match;
    }
  } else if (node && typeof node === 'object') {
    const element = node as RenderedNode;
    return matches(element) ? element : findNode(element.props?.children, matches);
  }
}

test('the close dialog offers system minimize when the tray is hidden and allows retrying a failure', async () => {
  let closed = false;
  let attempts = 0;
  let trayVisible = false;
  const harness = loadHookModule(new URL('./CloseConfirmDialog.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../hooks/useEscClose': { useEscClose: () => {} },
    '@tauri-apps/api/core': {
      invoke: async (command: string, args: { action: string }) => {
        assert.equal(command, 'handle_window_close');
        assert.equal(args.action, 'minimize');
        if (++attempts === 1) throw new Error('minimize failed');
      },
    },
  });
  const tree = harness.render(() => harness.exports.CloseConfirmDialog({
    trayVisible,
    onClose: () => { closed = true; },
  }));
  assert.ok(findNode(tree, (node) => node.props?.children === 'closeDialog.minimizeWindow'));
  const clickMinimize = () => {
    const button = findNode(harness.flush(), (node) => node.props?.className === 'close-option-btn minimize');
    assert.ok(button?.props?.onClick);
    button.props.onClick();
  };
  clickMinimize();
  await settlePromises();
  assert.ok(findNode(harness.flush(), (node) => node.props?.role === 'alert'));
  assert.equal(closed, false);

  clickMinimize();
  await settlePromises();
  assert.equal(closed, true);
  assert.equal(findNode(harness.flush(), (node) => node.props?.role === 'alert'), undefined);

  trayVisible = true;
  assert.ok(findNode(harness.flush(), (node) => node.props?.children === 'closeDialog.minimize'));
});
