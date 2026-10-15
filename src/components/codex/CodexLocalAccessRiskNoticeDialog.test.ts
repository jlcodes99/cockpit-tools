import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';

function nodes(tree: any): any[] {
  if (Array.isArray(tree)) return tree.flatMap(nodes);
  return tree && typeof tree === 'object' ? [tree, ...nodes(tree.props?.children)] : [];
}

test('the risk notice registers its actual dialog root and Escape cancels without accepting or saving memory', () => {
  const responses: boolean[] = [];
  const remembered: boolean[] = [];
  let trap: any;
  let escape: (() => void) | undefined;
  const h = loadHookModule(new URL('./CodexLocalAccessRiskNoticeDialog.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap: (ref: any, open: boolean) => { trap = { ref, open }; } },
    '../../hooks/useEscClose': { useEscCloseTopmost: (open: boolean, close: () => void) => { if (open) escape = close; } },
  });
  const tree = h.render(() => h.exports.CodexLocalAccessRiskNoticeDialog({ action: 'service', remember: false,
    onRememberChange: (value: boolean) => remembered.push(value), onClose: (value: boolean) => responses.push(value) }));
  const dialog = nodes(tree).find((node) => node.props?.role === 'dialog');
  assert.equal(dialog.props.ref, trap.ref);
  assert.equal(dialog.props.tabIndex, -1);
  assert.equal(dialog.props['aria-modal'], 'true');
  assert.equal(trap.open, true);
  assert.equal(tree.props.onClick, undefined);
  escape?.();
  assert.deepEqual(responses, [false]);
  assert.deepEqual(remembered, []);
  const buttons = nodes(tree).filter((node) => node.type === 'button');
  buttons[buttons.length - 1].props.onClick();
  assert.deepEqual(responses, [false, true]);
  h.unmount();
});
