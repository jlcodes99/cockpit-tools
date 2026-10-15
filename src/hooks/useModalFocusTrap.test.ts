import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

function harness() {
  const documentListeners = new Set<(event: any) => void>();
  const document: any = { activeElement: null,
    addEventListener(_type: string, callback: (event: any) => void) { documentListeners.add(callback); },
    removeEventListener(_type: string, callback: (event: any) => void) { documentListeners.delete(callback); },
    tab(shiftKey = false) {
      let prevented = false;
      for (const callback of [...documentListeners]) callback({ key: 'Tab', shiftKey, preventDefault() { prevented = true; } });
      return prevented;
    },
  };
  class FakeElement {
    isConnected = true;
    controls: FakeElement[] = [];
    listeners = new Map<string, (event: any) => void>();
    constructor(readonly name: string, readonly layer = 10000) {}
    focus() { document.activeElement = this; }
    contains(element: unknown) { return element === this || this.controls.includes(element as FakeElement); }
    querySelectorAll() { return this.controls; }
    addEventListener(type: string, fn: (event: any) => void) { this.listeners.set(type, fn); }
    removeEventListener(type: string, fn: (event: any) => void) { if (this.listeners.get(type) === fn) this.listeners.delete(type); }
    tab(shiftKey = false) {
      let prevented = false;
      this.listeners.get('keydown')?.({ key: 'Tab', shiftKey, preventDefault() { prevented = true; } });
      return prevented;
    }
  }
  document.body = new FakeElement('body');
  document.documentElement = new FakeElement('html');
  const h = loadHookModule(new URL('./useModalFocusTrap.ts', import.meta.url), {}, { document, HTMLElement: FakeElement, window: { getComputedStyle: (node: FakeElement) => ({ zIndex: String(node.layer) }) } });
  const page = new FakeElement('page-trigger');
  const parent = new FakeElement('task');
  const trigger = new FakeElement('preset-trigger');
  const lastParent = new FakeElement('save');
  parent.controls = [trigger, lastParent];
  const child = new FakeElement('preset');
  const firstChild = new FakeElement('close');
  const lastChild = new FakeElement('save-preset');
  child.controls = [firstChild, lastChild];
  const confirmation = new FakeElement('global-confirmation', 14020);
  const confirmationFirst = new FakeElement('cancel-confirmation');
  const confirmationLast = new FakeElement('approve-confirmation');
  confirmation.controls = [confirmationFirst, confirmationLast];
  const confirmationRef: { current: FakeElement | null } = { current: confirmation };
  let confirmationOpen = false;
  const parentRef: { current: FakeElement | null } = { current: parent };
  const childRef: { current: FakeElement | null } = { current: child };
  let parentOpen = true;
  let childOpen = false;
  page.focus();
  h.render(() => {
    h.exports.useModalFocusTrap(parentRef, parentOpen, true, !childOpen);
    h.exports.useModalFocusTrap(childRef, childOpen);
    h.exports.useModalFocusTrap(confirmationRef, confirmationOpen);
  });
  return { h, document, page, parent, trigger, lastParent, child, firstChild, lastChild, confirmation, confirmationFirst, confirmationLast,
    openChild() { trigger.focus(); childOpen = true; h.flush(); },
    activateChild() {
      childOpen = true; child.isConnected = true;
      child.controls.forEach((control) => { control.isConnected = true; }); h.flush();
    },
    closeChild() {
      childOpen = false; child.isConnected = false;
      child.controls.forEach((control) => { control.isConnected = false; }); h.flush();
    },
    closeParent() { parentOpen = false; h.flush(); },
    openConfirmation() { confirmationOpen = true; h.flush(); },
    closeConfirmation() { confirmationOpen = false; h.flush(); },
    removeConfirmationBeforeCleanup() {
      confirmationOpen = false; confirmationRef.current = null; confirmation.isConnected = false;
      confirmation.controls.forEach((control) => { control.isConnected = false; }); h.flush();
    },
    removeAllBeforeCleanup() {
      parentRef.current = null; childRef.current = null;
      for (const node of [parent, child, ...parent.controls, ...child.controls]) node.isConnected = false;
      h.unmount();
    },
  };
}

test('an opened dialog receives focus and cycles Tab in both directions', () => {
  const h = harness();
  assert.equal(h.document.activeElement, h.parent);
  assert.equal(h.parent.tab(), true);
  assert.equal(h.document.activeElement, h.trigger);
  assert.equal(h.parent.tab(true), true);
  assert.equal(h.document.activeElement, h.lastParent);
  assert.equal(h.parent.tab(), true);
  assert.equal(h.document.activeElement, h.trigger);
  h.closeParent();
  assert.equal(h.document.activeElement, h.page);
  assert.equal(h.parent.listeners.size, 0);
  h.h.unmount();
});

test('a nested dialog suspends its parent trap and restores the exact parent trigger', () => {
  const h = harness();
  h.openChild();
  assert.equal(h.document.activeElement, h.child);
  assert.equal(h.parent.listeners.size, 0);
  assert.equal(h.child.tab(true), true);
  assert.equal(h.document.activeElement, h.lastChild);
  assert.equal(h.child.tab(), true);
  assert.equal(h.document.activeElement, h.firstChild);
  h.closeChild();
  assert.equal(h.document.activeElement, h.trigger);
  assert.equal(h.child.listeners.size, 0);
  assert.equal(h.parent.listeners.size, 1);
  h.closeParent();
  assert.equal(h.document.activeElement, h.page);
  h.h.unmount();
});

test('closing a nested dialog skips a disconnected return target and keeps parent usable', () => {
  const h = harness(); h.openChild();
  h.trigger.isConnected = false;
  h.closeChild();
  assert.equal(h.document.activeElement, h.parent);
  assert.equal(h.parent.tab(), true);
  h.h.unmount();
});


test('a background confirmation owns Tab and lower dialog reactivation cannot steal its focus', () => {
  const h = harness(); h.openChild(); h.firstChild.focus(); h.openConfirmation();
  assert.equal(h.document.activeElement, h.confirmation);
  assert.equal(h.child.tab(), false);
  assert.equal(h.document.activeElement, h.confirmation);
  h.closeChild();
  assert.equal(h.document.activeElement, h.confirmation);
  // Reopening a lower dialog while confirmation is visible must not bring it forward.
  h.activateChild();
  assert.equal(h.document.activeElement, h.confirmation);
  h.closeChild();
  assert.equal(h.document.activeElement, h.confirmation);
  assert.equal(h.parent.tab(), false);
  assert.equal(h.confirmation.tab(true), true);
  assert.equal(h.document.activeElement, h.confirmationLast);
  h.closeConfirmation();
  assert.equal(h.document.activeElement, h.parent);
  h.h.unmount();
});


test('confirmation cleanup restores the parent after React clears its ref and disconnects its node', () => {
  const h = harness(); h.trigger.focus(); h.openConfirmation();
  assert.equal(h.document.activeElement, h.confirmation);
  h.removeConfirmationBeforeCleanup();
  assert.equal(h.document.activeElement, h.trigger);
  assert.equal(h.confirmation.listeners.size, 0);
  h.closeParent(); h.h.unmount();
});

test('a parent and child detached in one commit restore the original page trigger', () => {
  const h = harness(); h.openChild(); h.firstChild.focus();
  h.removeAllBeforeCleanup();
  assert.equal(h.document.activeElement, h.page);
  assert.equal(h.parent.listeners.size, 0);
  assert.equal(h.child.listeners.size, 0);
});


test('Tab after a backdrop blurs focus to body is recovered by only the highest dialog', () => {
  const h = harness(); h.openChild(); h.openConfirmation();
  h.document.activeElement = h.document.body;
  assert.equal(h.document.tab(), true);
  assert.equal(h.document.activeElement, h.confirmationFirst);
  h.document.activeElement = h.document.documentElement;
  assert.equal(h.document.tab(true), true);
  assert.equal(h.document.activeElement, h.confirmationLast);
  h.closeConfirmation();
  h.document.activeElement = h.document.body;
  assert.equal(h.document.tab(), true);
  assert.equal(h.document.activeElement, h.firstChild);
  h.h.unmount();
});

test('the document fallback leaves portaled menu focus and its keyboard handler alone', () => {
  const h = harness();
  const portalOption = { isConnected: true };
  h.document.activeElement = portalOption;
  assert.equal(h.document.tab(), false);
  assert.equal(h.document.activeElement, portalOption);
  h.h.unmount();
});
