import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';

type Element = { type: unknown; props: Record<string, any> };
function nodes(value: any): Element[] {
  if (!value || typeof value !== 'object') return [];
  if (Array.isArray(value)) return value.flatMap(nodes);
  return [value, ...nodes(value.props?.children)];
}
function harness(compact = false) {
  const changes: boolean[] = [];
  let escape: (() => void) | undefined;
  let focusOpen = false;
  let scrollLocked = false;
  const props = { compact, checked: true, disabled: false, onChange: (value: boolean) => {
    changes.push(value); props.checked = value;
  } };
  const h = loadHookModule(new URL('./CodexSessionIdentityMappingControl.tsx', import.meta.url), {
    'react-i18next': { useTranslation: () => ({ t: (key: string) => key }) },
    'react-dom': { createPortal: (value: unknown) => value },
    '../../hooks/useEscClose': { useEscCloseTopmost: (open: boolean, close: () => void) => { escape = open ? close : undefined; } },
    '../../hooks/useModalFocusTrap': { useModalFocusTrap: (_ref: unknown, open: boolean) => { focusOpen = open; } },
    '../../hooks/useModalScrollLock': { useModalScrollLock: (open: boolean) => { scrollLocked = open; } },
  }, { document: { body: {} } });
  h.render(() => h.exports.CodexSessionIdentityMappingControl(props));
  const rendered = () => nodes(h.flush());
  return { h, props, changes, rendered,
    input: () => rendered().find((node) => node.props?.role === 'switch')!,
    dialog: () => rendered().find((node) => node.props?.role === 'alertdialog'),
    button: (label: string) => rendered().find((node) => node.type === 'button' && node.props.children === label)!,
    escape: () => { assert.ok(escape); escape(); },
    guarded: () => focusOpen && scrollLocked,
  };
}

for (const compact of [false, true]) {
  test(`${compact ? 'member' : 'page'} switch explains both directions and changes the draft only after confirmation`, () => {
    const app = harness(compact);
    assert.equal(app.dialog(), undefined);
    assert.equal(app.rendered().some((node) => node.type === 'p'), false, 'no persistent explanation');
    assert.equal(app.input().props.checked, true);
    app.input().props.onChange({ target: { checked: false } });
    assert.deepEqual(app.changes, []);
    assert.equal(app.input().props.checked, true);
    assert.ok(app.dialog());
    assert.ok(app.guarded());
    assert.equal(app.input().props.disabled, false, 'keep the trigger focusable for focus restoration');
    const text = app.rendered().filter((node) => node.type === 'p').map((node) => node.props.children);
    assert.deepEqual(text, ['codex.apiService.routing.sessionIdentityMappingDisableHint',
      'codex.apiService.routing.sessionIdentityMappingCacheNote', 'codex.apiService.routing.sessionIdentityMappingSaveNote']);
    const overlay = app.rendered().find((node) => node.props?.className === 'modal-overlay codex-session-identity-confirm-overlay')!;
    assert.equal(overlay.props.onClick, undefined);
    app.button('common.cancel').props.onClick();
    assert.equal(app.dialog(), undefined);
    assert.deepEqual(app.changes, []);
    assert.equal(app.guarded(), false);

    app.input().props.onChange({ target: { checked: false } });
    const confirm = app.button('common.disable').props.onClick;
    confirm(); confirm();
    assert.deepEqual(app.changes, [false]);
    assert.equal(app.dialog(), undefined);
    assert.equal(app.input().props.checked, false);
    app.input().props.onChange({ target: { checked: true } });
    assert.equal(app.rendered().filter((node) => node.type === 'p')[0].props.children,
      'codex.apiService.routing.sessionIdentityMappingEnableHint');
    assert.deepEqual(app.changes, [false]);
    app.button('common.enable').props.onClick();
    assert.equal(app.input().props.checked, true);
    assert.deepEqual(app.changes, [false, true]);
    app.h.unmount();
  });
}

test('close and Escape cancel without changing the switch', () => {
  const app = harness(true);
  for (const method of ['close', 'escape']) {
    app.input().props.onChange({ target: { checked: false } });
    assert.ok(app.dialog());
    if (method === 'close') app.rendered().find((node) => node.type === 'button' && node.props['aria-label'] === 'common.close')!.props.onClick();
    else app.escape();
    assert.equal(app.dialog(), undefined);
    assert.equal(app.input().props.checked, true);
    assert.deepEqual(app.changes, []);
  }
  app.h.unmount();
});

test('disabling, replacing the draft or unmounting invalidates pending confirmations', () => {
  for (const change of ['disabled', 'checked', 'unmount']) {
    const app = harness();
    app.input().props.onChange({ target: { checked: false } });
    const confirm = app.button('common.disable').props.onClick;
    if (change === 'unmount') app.h.unmount();
    else {
      if (change === 'disabled') app.props.disabled = true;
      else app.props.checked = false;
      assert.equal(app.dialog(), undefined);
    }
    confirm();
    assert.deepEqual(app.changes, []);
    if (change !== 'unmount') app.h.unmount();
  }
});
