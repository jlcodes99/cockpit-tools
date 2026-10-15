import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

type Key = { key: string; shiftKey: boolean; prevented: boolean; stopped: boolean;
  preventDefault(): void; stopImmediatePropagation(): void };

function harness(layers = [14020, 10010]) {
  const windowListeners = new Set<(key: Key) => void>();
  const documentListeners = new Set<(key: Key) => void>();
  const document: any = { activeElement: null,
    addEventListener(_type: string, fn: (key: Key) => void) { documentListeners.add(fn); },
    removeEventListener(_type: string, fn: (key: Key) => void) { documentListeners.delete(fn); },
  };
  class Element {
    isConnected = true;
    controls: Element[] = [];
    listeners = new Set<(key: Key) => void>();
    constructor(readonly name: string, public layer = 'auto', readonly parentElement: Element | null = null) {}
    focus() { document.activeElement = this; }
    contains(candidate: Element | null): boolean {
      for (let current = candidate; current; current = current.parentElement) if (current === this) return true;
      return false;
    }
    querySelectorAll() { return this.controls; }
    addEventListener(_type: string, fn: (key: Key) => void) { this.listeners.add(fn); }
    removeEventListener(_type: string, fn: (key: Key) => void) { this.listeners.delete(fn); }
  }
  const window = { getComputedStyle: (node: Element) => ({ zIndex: node.layer }),
    addEventListener(_type: string, fn: (key: Key) => void) { windowListeners.add(fn); },
    removeEventListener(_type: string, fn: (key: Key) => void) { windowListeners.delete(fn); },
  };
  document.body = new Element('body'); document.documentElement = new Element('html');
  const page = new Element('page-trigger'); page.focus();
  const nodes = layers.map((layer, index) => new Element(`dialog-${index}`, 'auto', new Element(`overlay-${index}`, String(layer), document.body)));
  for (const node of nodes) node.controls = [new Element(`${node.name}-close`, 'auto', node), new Element(`${node.name}-confirm`, 'auto', node)];
  const refs: { current: Element | null }[] = nodes.map((node) => ({ current: node }));
  const open = nodes.map(() => false);
  const active = nodes.map(() => true);
  const escapable = nodes.map(() => true);
  const closed: number[] = [];
  // Load the real shared registry and both real hooks in one isolated module graph.
  const stack = loadHookModule(new URL('./modalStack.ts', import.meta.url), {}).exports;
  const globals = { window, document, HTMLElement: Element };
  const imports = { './modalStack': stack };
  const esc = loadHookModule(new URL('./useEscClose.ts', import.meta.url), imports, globals);
  const focus = loadHookModule(new URL('./useModalFocusTrap.ts', import.meta.url), imports, globals);
  esc.render(() => nodes.forEach((_node, index) => {
    esc.exports.useEscCloseTopmost(open[index] && active[index] && escapable[index], () => { closed.push(index); open[index] = false; }, refs[index]);
  }));
  focus.render(() => nodes.forEach((_node, index) => {
    focus.exports.useModalFocusTrap(refs[index], open[index], true, active[index]);
  }));
  const flush = () => { esc.flush(); focus.flush(); };
  function key(key: string, shiftKey = false): Key {
    const event: Key = { key, shiftKey, prevented: false, stopped: false,
      preventDefault() { this.prevented = true; }, stopImmediatePropagation() { this.stopped = true; } };
    for (const listener of windowListeners) { listener(event); if (event.stopped) return event; }
    for (const listener of documentListeners) listener(event);
    for (const listener of nodes.find((node) => node.contains(document.activeElement))?.listeners ?? []) listener(event);
    return event;
  }
  return { nodes, refs, open, active, escapable, closed, document, page, flush, key,
    show(index: number) { open[index] = true; flush(); },
    detach(index: number) {
      refs[index].current = null; nodes[index].isConnected = false;
      nodes[index].controls.forEach((node) => { node.isConnected = false; });
    },
    unmount() { esc.unmount(); focus.unmount(); },
    listenerCounts() { return { window: windowListeners.size, document: documentListeners.size }; },
  };
}

test('high layer opened first retains Tab and Escape when a scan opens a lower layer later', () => {
  const h = harness(); h.show(0); h.show(1);
  assert.equal(h.document.activeElement, h.nodes[0]);
  assert.equal(h.key('Tab').prevented, true);
  assert.equal(h.document.activeElement, h.nodes[0].controls[0]);
  assert.equal(h.key('Escape').stopped, true); h.flush();
  assert.deepEqual(h.closed, [0]); assert.equal(h.open[1], true);
  assert.equal(h.document.activeElement, h.nodes[1]);
  h.key('Escape'); h.flush(); assert.deepEqual(h.closed, [0, 1]);
  h.unmount(); assert.deepEqual(h.listenerCounts(), { window: 0, document: 0 });
});

test('normal order closes the higher layer and restores its lower dialog trigger', () => {
  const h = harness(); h.show(1); h.nodes[1].controls[0].focus(); h.show(0);
  h.key('Escape'); h.flush();
  assert.deepEqual(h.closed, [0]); assert.equal(h.open[1], true);
  assert.equal(h.document.activeElement, h.nodes[1].controls[0]);
  h.unmount();
});

test('same layer follows the latest opened dialog for both keys and restores its opener', () => {
  const h = harness([10010, 10010]); h.show(0); h.nodes[0].controls[1].focus(); h.show(1);
  assert.equal(h.document.activeElement, h.nodes[1]); h.key('Tab');
  assert.equal(h.document.activeElement, h.nodes[1].controls[0]);
  h.key('Escape'); h.flush();
  assert.deepEqual(h.closed, [1]); assert.equal(h.document.activeElement, h.nodes[0].controls[1]);
  h.unmount();
});

test('suspended parent cannot close or trap Tab while its child is open; it resumes on close', () => {
  const h = harness([10010, 10010]); h.show(0); h.nodes[0].controls[0].focus();
  h.active[0] = false; h.show(1);
  assert.equal(h.nodes[0].listeners.size, 0); h.key('Tab');
  assert.equal(h.document.activeElement, h.nodes[1].controls[0]);
  h.key('Escape'); h.active[0] = true; h.flush();
  assert.deepEqual(h.closed, [1]); assert.equal(h.open[0], true);
  assert.equal(h.document.activeElement, h.nodes[0].controls[0]);
  h.key('Escape'); h.flush(); assert.deepEqual(h.closed, [1, 0]);
  h.unmount();
});

test('busy top dialog without an Escape handler consumes the key and preserves background', () => {
  const h = harness(); h.show(1); h.escapable[0] = false; h.show(0);
  assert.equal(h.key('Escape').stopped, true); h.flush();
  assert.deepEqual(h.closed, []); assert.deepEqual(h.open, [true, true]);
  h.escapable[0] = true; h.flush(); h.key('Escape'); h.flush();
  assert.deepEqual(h.closed, [0]); h.unmount();
});

test('same-layer busy dialog blocks a previously registered lower dialog', () => {
  const h = harness([10010, 10010]); h.show(1); h.escapable[0] = false; h.show(0);
  assert.equal(h.key('Escape').stopped, true); assert.deepEqual(h.closed, []);
  h.unmount();
});

test('re-enabling background Escape after busy work keeps the same-layer foreground order', () => {
  const h = harness([10010, 10010]); h.escapable[0] = false; h.show(0); h.show(1);
  h.escapable[0] = true; h.flush();
  assert.equal(h.document.activeElement, h.nodes[1]);
  h.key('Tab'); assert.equal(h.document.activeElement, h.nodes[1].controls[0]);
  h.key('Escape'); h.flush(); assert.deepEqual(h.closed, [1]);
  assert.equal(h.open[0], true); h.unmount();
});

test('detached ref is ignored before cleanup and focus restoration uses the captured node', () => {
  const h = harness(); h.show(1); h.nodes[1].controls[0].focus(); h.show(0);
  h.detach(0); h.open[0] = false; h.flush();
  assert.equal(h.document.activeElement, h.nodes[1].controls[0]);
  h.key('Escape'); h.flush(); assert.deepEqual(h.closed, [1]);
  h.unmount(); assert.deepEqual(h.listenerCounts(), { window: 0, document: 0 });
});

test('a stale disconnected registration is skipped even before its effect cleanup', () => {
  const h = harness(); h.show(1); h.show(0); h.detach(0);
  h.key('Escape'); assert.deepEqual(h.closed, [1]);
  h.unmount();
});

test('parent and child detached in one commit restore the page and remove all listeners', () => {
  const h = harness([10010, 10010]); h.show(0); h.nodes[0].controls[0].focus(); h.show(1);
  h.detach(0); h.detach(1); h.unmount();
  assert.equal(h.document.activeElement, h.page);
  assert.deepEqual(h.listenerCounts(), { window: 0, document: 0 });
  h.key('Escape'); assert.deepEqual(h.closed, []);
});

test('portaled menu ref can attach after registration and its current layer wins Escape', () => {
  const h = harness([10010, 10011]); h.show(0);
  const menu = h.refs[1].current; h.refs[1].current = null;
  h.open[1] = true;
  // Only register the real Escape hook here; no focus trap exists for the menu.
  h.active[1] = true; h.flush(); h.refs[1].current = menu;
  h.key('Escape'); h.flush(); assert.deepEqual(h.closed, [1]);
  assert.equal(h.open[0], true); h.unmount();
});
