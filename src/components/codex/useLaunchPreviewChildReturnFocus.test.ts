import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';

function harness() {
  const document: any = { activeElement: null };
  const focusOrder: string[] = [];
  class Element {
    isConnected = true;
    disabled = false;
    owner: Element | null = null;
    controls: Element[] = [];
    constructor(readonly id: string) {}
    focus(options?: any) { document.activeElement = this; focusOrder.push(this.id); if (options) assert.equal(options.preventScroll, true); }
    contains(target: any) { return target === this || this.controls.includes(target); }
    closest() { return this.owner; }
  }
  const parent = new Element('preview'); parent.owner = parent;
  const previous = new Element('previous-close'); previous.owner = parent;
  const opener = new Element('clicked-trigger'); opener.owner = parent;
  parent.controls = [previous, opener];
  const child = new Element('child'); child.owner = child;
  const body = new Element('body');
  const foreground = new Element('global-confirmation'); foreground.owner = foreground;
  const dialog = { current: parent };
  let open = false;
  const h = loadHookModule(new URL('./useLaunchPreviewChildReturnFocus.ts', import.meta.url), {}, { document, HTMLElement: Element });
  let remember = h.render(() => h.exports.useLaunchPreviewChildReturnFocus(dialog, open));
  return { h, document, focusOrder, parent, previous, opener, child, body, foreground,
    openChild() {
      // macOS click does not focus the clicked button; inert then blurs the old focus during commit.
      previous.focus(); remember({ currentTarget: opener }); open = true; body.focus(); remember = h.flush(); child.focus();
    },
    closeChild(global = false) {
      open = false; child.isConnected = false;
      // Simulate trap resume/child cleanup before the owner's final effect.
      if (global) foreground.focus(); else { parent.focus(); body.focus(); }
      remember = h.flush();
    },
  };
}

test('returns to the clicked opener after macOS inert blur, trap resume and child detach', () => {
  const h = harness(); h.openChild();
  assert.equal(h.document.activeElement, h.child);
  h.closeChild();
  assert.equal(h.document.activeElement, h.opener);
  assert.equal(h.focusOrder[h.focusOrder.length - 1], 'clicked-trigger');
  h.h.unmount();
});

test('returning from a child cannot steal focus from a separate foreground confirmation', () => {
  const h = harness(); h.openChild(); h.closeChild(true);
  assert.equal(h.document.activeElement, h.foreground);
  h.h.unmount();
});

test('removed or disabled openers are never focused', () => {
  for (const change of ['removed', 'disabled']) {
    const h = harness(); h.openChild();
    if (change === 'removed') h.opener.isConnected = false; else h.opener.disabled = true;
    h.closeChild();
    assert.equal(h.document.activeElement, h.body);
    h.h.unmount();
  }
});
