import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';
import { reorderCodexCardIds } from '../utils/codexCardOrder';

class FakeElement {
  dataset: Record<string, string> = {};
  classes = new Set<string>();
  attributes: Record<string, string> = {};
  childNodes: any[] = [];
  captured = new Set<number>();
  styles = new Map<string, string>();
  rect = { left: 0, top: 0, width: 100, height: 100 };
  visualScale = 1;
  animations: Array<{
    keyframes: Array<Record<string, string>>;
    options: { duration: number; easing: string };
    onfinish: (() => void) | null;
    oncancel: (() => void) | null;
    cancelled: boolean;
    cancel: () => void;
  }> = [];
  style = {
    setProperty: (key: string, value: string) => this.styles.set(key, value),
    removeProperty: (key: string) => this.styles.delete(key),
  };

  constructor(public tagName = 'div', public parent: FakeElement | null = null) {
    parent?.childNodes.push(this);
  }

  closest(selector: string): FakeElement | null {
    for (let node: FakeElement | null = this; node; node = node.parent) {
      if (selector === '[data-codex-sort-id]' && node.dataset.codexSortId) return node;
      if (selector === '.codex-accounts-grid' && node.classes.has('codex-accounts-grid')) return node;
      if (selector.includes('button, a, input')) {
        if (['button', 'a', 'input', 'textarea', 'select', 'label', 'summary'].includes(node.tagName)
          || ['button', 'checkbox', 'switch', 'menuitem'].includes(node.attributes.role)
          || (node.attributes.contenteditable != null && node.attributes.contenteditable !== 'false')
          || node.classes.has('account-email') || 'noCardDrag' in node.dataset) return node;
      }
    }
    return null;
  }

  contains(candidate: FakeElement): boolean {
    for (let node: FakeElement | null = candidate; node; node = node.parent) {
      if (node === this) return true;
    }
    return false;
  }

  getBoundingClientRect() {
    const prefix = this.dataset.codexDragging ? 'drag' : 'shift';
    const scale = this.dataset.codexDragging ? this.visualScale : 1;
    const width = this.rect.width * scale;
    const height = this.rect.height * scale;
    const left = this.rect.left + Number.parseFloat(this.styles.get(`--codex-card-${prefix}-x`) || '0') - (width - this.rect.width) / 2;
    const top = this.rect.top + Number.parseFloat(this.styles.get(`--codex-card-${prefix}-y`) || '0') - (height - this.rect.height) / 2;
    return { left, top, width, height, right: left + width, bottom: top + height };
  }

  isConnected = true;

  animate(keyframes: Array<Record<string, string>>, options: { duration: number; easing: string }) {
    const animation = { keyframes, options, onfinish: null as (() => void) | null,
      oncancel: null as (() => void) | null, cancelled: false,
      cancel() { this.cancelled = true; this.oncancel?.(); } };
    this.animations.push(animation);
    return animation;
  }

  get lastAnimation() { return this.animations[this.animations.length - 1]; }

  querySelectorAll(selector: string): FakeElement[] {
    return this.childNodes.flatMap((child) => child instanceof FakeElement
      ? [...(selector === '[data-codex-sort-id]' && child.dataset.codexSortId ? [child] : []), ...child.querySelectorAll(selector)]
      : []);
  }

  removeAttribute(name: string) {
    if (name.startsWith('data-')) {
      delete this.dataset[name.slice(5).replace(/-([a-z])/g, (_, letter: string) => letter.toUpperCase())];
    } else delete this.attributes[name];
  }

  setPointerCapture(id: number) { this.captured.add(id); }
  hasPointerCapture(id: number) { return this.captured.has(id); }
  releasePointerCapture(id: number) { this.captured.delete(id); }
}

function event(extra: Record<string, unknown> = {}) {
  return {
    pointerId: 7, button: 0, buttons: 1, isPrimary: true, pointerType: 'mouse',
    clientX: 10, clientY: 10, defaultPrevented: false, propagationStopped: false,
    preventDefault() { this.defaultPrevented = true; },
    stopPropagation() { this.propagationStopped = true; },
    ...extra,
  };
}

function harness() {
  const listeners = new Map<string, Set<(value: any) => void>>();
  const frames = new Map<number, () => void>();
  const timers = new Map<number, () => void>();
  let sequence = 0;
  let hit: FakeElement | null = null;
  let selectionCollapsed = true;
  let reducedMotion = false;
  let osReducedMotion = false;
  const grid = new FakeElement();
  grid.classes.add('codex-accounts-grid');
  const cards = Object.fromEntries(['a', 'b', 'c'].map((id, index) => {
    const card = new FakeElement('div', grid);
    card.dataset.codexSortId = id;
    card.rect.left = index * 120;
    return [id, card];
  }));
  const apiCard = new FakeElement('div', grid);
  apiCard.rect.left = -120;
  const commits: string[][] = [];
  let onCommit = (_ids: string[]) => {};
  let options = {
    orderedIds: ['a', 'b', 'c'], enabled: true, viewKey: 'grid:all:page1',
    onReorder(ids: string[]) { commits.push(Array.from(ids)); onCommit(Array.from(ids)); },
  };
  const window = {
    matchMedia: () => ({ matches: osReducedMotion }),
    addEventListener(type: string, callback: (value: any) => void) {
      const callbacks = listeners.get(type) ?? new Set();
      callbacks.add(callback); listeners.set(type, callbacks);
    },
    removeEventListener(type: string, callback: (value: any) => void) {
      listeners.get(type)?.delete(callback);
      if (!listeners.get(type)?.size) listeners.delete(type);
    },
    requestAnimationFrame(callback: () => void) { frames.set(++sequence, callback); return sequence; },
    cancelAnimationFrame(id: number) { frames.delete(id); },
    setTimeout(callback: () => void) { timers.set(++sequence, callback); return sequence; },
    clearTimeout(id: number) { timers.delete(id); },
  };
  const document = {
    elementFromPoint: () => hit,
    getSelection: () => ({ isCollapsed: selectionCollapsed }),
    createRange() {
      let selected: any;
      return { selectNodeContents(node: any) { selected = node; }, getClientRects: () => selected?.rects ?? [] };
    },
  };
  const hook = loadHookModule(new URL('./useCodexCardReorder.ts', import.meta.url), {
    '../utils/codexCardOrder': { reorderCodexCardIds },
    '../utils/reducedMotion': { isReducedMotionEnabled: () => reducedMotion },
  }, { window, document, Element: FakeElement, Node: { TEXT_NODE: 3 } });
  const render = () => hook.exports.useCodexCardReorder(options);
  hook.render(render);
  const emit = (type: string, extra: Record<string, unknown> = {}) => {
    const value = event(extra);
    for (const callback of [...(listeners.get(type) ?? [])]) callback(value);
    return value;
  };
  const flushFrames = () => {
    const callbacks = [...frames.values()]; frames.clear(); callbacks.forEach((callback) => callback());
  };
  return {
    hook, grid, cards, apiCard, commits, listeners, frames, timers, emit, flushFrames,
    props: (id: string, visible = ['a', 'b', 'c']) => hook.flush().cardProps(id, visible),
    start(id = 'a', target = cards[id], extra: Record<string, unknown> = {}, visible = ['a', 'b', 'c']) {
      const value = event({ currentTarget: cards[id], target,
        clientX: cards[id].rect.left + 10, clientY: cards[id].rect.top + 10, ...extra });
      hook.flush().cardProps(id, visible).onPointerDown(value);
      return value;
    },
    move(target: FakeElement | null = cards.c, extra: Record<string, unknown> = {}) {
      hit = target;
      const value = emit('pointermove', { clientX: (target?.rect.left ?? -240) + 30,
        clientY: (target?.rect.top ?? 0) + 10, ...extra });
      flushFrames(); return value;
    },
    drop(target: FakeElement | null = cards.c, extra: Record<string, unknown> = {}) {
      hit = target; return emit('pointerup', { clientX: (target?.rect.left ?? -240) + 30,
        clientY: (target?.rect.top ?? 0) + 10, ...extra });
    },
    configure(next: Partial<typeof options>) { options = { ...options, ...next }; hook.render(render); },
    selection(collapsed: boolean) { selectionCollapsed = collapsed; },
    motionPreference(app: boolean, os = false) { reducedMotion = app; osReducedMotion = os; },
    afterCommit(callback: (ids: string[]) => void) { onCommit = callback; },
    flushTimers() { const callbacks = [...timers.values()]; timers.clear(); callbacks.forEach((callback) => callback()); },
    assertClean() {
      assert.equal(grid.dataset.codexSorting, undefined);
      for (const card of Object.values(cards)) {
        assert.equal(card.dataset.codexDragging, undefined);
        assert.equal(card.dataset.codexShifting, undefined);
        assert.equal(card.captured.size, 0);
        assert.equal(card.styles.size, 0);
      }
      assert.equal(frames.size, 0);
    },
  };
}

test('a click or sub-threshold pointer movement keeps native behavior and never saves order', () => {
  const h = harness();
  const down = h.start();
  const move = h.move(h.cards.c, { clientX: 15 });
  const up = h.drop();
  assert.equal(down.defaultPrevented, false);
  assert.equal(move.defaultPrevented, false);
  assert.equal(up.defaultPrevented, false);
  assert.equal(h.emit('click').defaultPrevented, false);
  assert.deepEqual(h.commits, []); h.assertClean(); h.hook.unmount();
});

test('nested buttons, inputs, editable names and semantic controls do not start card dragging', () => {
  for (const kind of ['button', 'input', 'a', 'label', 'role', 'editable', 'name', 'opt-out']) {
    const h = harness();
    const target = new FakeElement(['role', 'editable', 'name', 'opt-out'].includes(kind) ? 'span' : kind, h.cards.a);
    if (kind === 'role') target.attributes.role = 'button';
    if (kind === 'editable') target.attributes.contenteditable = 'true';
    if (kind === 'name') target.classes.add('account-email');
    if (kind === 'opt-out') target.dataset.noCardDrag = '';
    const child = new FakeElement('svg', target);
    h.start('a', child); h.move(); h.drop();
    assert.deepEqual(h.commits, [], kind); h.assertClean(); h.hook.unmount();
  }
});

test('text glyphs and an existing text selection remain selectable instead of becoming a drag', () => {
  const h = harness();
  const text = new FakeElement('span', h.cards.a);
  text.childNodes.push({ nodeType: 3, textContent: 'Account name', rects: [{ left: 5, right: 15, top: 5, bottom: 15 }] });
  h.start('a', text); h.move(); h.drop();
  assert.deepEqual(h.commits, []);
  h.selection(false); h.start(); h.move(); h.drop();
  assert.deepEqual(h.commits, []); h.assertClean(); h.hook.unmount();
});

test('blank space beside text can drag, while touch, secondary buttons and non-primary pointers cannot', () => {
  const h = harness();
  const text = new FakeElement('span', h.cards.a);
  text.childNodes.push({ nodeType: 3, textContent: 'Account name', rects: [{ left: 50, right: 100, top: 5, bottom: 15 }] });
  for (const extra of [{ pointerType: 'touch' }, { button: 2 }, { isPrimary: false }]) {
    h.start('a', text, extra); h.move(); h.drop();
  }
  assert.deepEqual(h.commits, []);
  h.start('a', text); h.move(); h.drop();
  assert.deepEqual(h.commits, [['b', 'c', 'a']]); h.hook.unmount();
});

test('dragging moves intermediate cards into the vacancy before release and only then saves once', () => {
  const h = harness(); h.start();
  assert.equal(h.move().defaultPrevented, true);
  assert.equal(h.cards.a.dataset.codexDragging, 'true');
  assert.equal(h.grid.dataset.codexSorting, 'true');
  assert.equal(h.cards.b.dataset.codexShifting, 'true');
  assert.equal(h.cards.b.styles.get('--codex-card-shift-x'), '-120px');
  assert.equal(h.cards.c.styles.get('--codex-card-shift-x'), '-120px');
  assert.equal(h.cards.a.styles.get('--codex-card-drag-x'), '260px');
  assert.equal(h.cards.a.captured.has(7), true);
  assert.deepEqual(h.commits, []);
  h.drop(); h.drop();
  assert.deepEqual(h.commits, [['b', 'c', 'a']]); h.assertClean();
  const click = h.emit('click');
  assert.equal(click.defaultPrevented, true); assert.equal(click.propagationStopped, true);
  h.flushTimers(); assert.equal(h.emit('click').defaultPrevented, false); h.hook.unmount();
});

test('backward dragging fills the original vacancy and dropping uses the actual release slot', () => {
  const h = harness(); h.start('c'); h.move(h.cards.a);
  assert.equal(h.cards.a.styles.get('--codex-card-shift-x'), '120px');
  assert.equal(h.cards.b.styles.get('--codex-card-shift-x'), '120px');
  h.drop(h.cards.b);
  assert.deepEqual(h.commits, [['a', 'c', 'b']]); h.assertClean(); h.hook.unmount();
});

test('the fixed API service card, empty space and another grid cannot receive an account drop', () => {
  const h = harness();
  const otherGrid = new FakeElement(); otherGrid.classes.add('codex-accounts-grid');
  const otherCard = new FakeElement('div', otherGrid); otherCard.dataset.codexSortId = 'c';
  for (const target of [h.apiCard, null, otherCard, h.cards.a]) {
    h.start(); h.move(target); h.drop(target); h.assertClean();
  }
  assert.deepEqual(h.commits, []); h.hook.unmount();
});

test('a scoped page or tag group cannot drop onto an account outside its visible IDs', () => {
  const h = harness();
  h.start('a', h.cards.a, {}, ['a', 'b']); h.move(h.cards.c); h.drop(h.cards.c);
  assert.deepEqual(h.commits, []);
  h.start('a', h.cards.a, {}, ['a', 'b']); h.move(h.cards.b); h.drop(h.cards.b);
  assert.deepEqual(h.commits, [['b', 'a', 'c']]); h.hook.unmount();
});

test('Escape, blur, resize, pointer cancellation, lost capture and released buttons cancel without saving', () => {
  for (const reason of ['keydown', 'blur', 'resize', 'pointercancel', 'lostpointercapture', 'buttons']) {
    const h = harness(); h.start(); h.move();
    if (reason === 'buttons') h.move(h.cards.c, { buttons: 0 });
    else h.emit(reason, { key: 'Escape' });
    h.drop();
    assert.deepEqual(h.commits, [], reason); h.assertClean(); h.hook.unmount();
  }
});

test('a vacated destination stays stable even when the pointer now hits the empty grid', () => {
  const h = harness(); h.start(); h.move();
  for (let frame = 0; frame < 5; frame += 1) {
    h.move(h.grid, { clientX: 270 });
    assert.equal(h.cards.b.styles.get('--codex-card-shift-x'), '-120px');
    assert.equal(h.cards.c.styles.get('--codex-card-shift-x'), '-120px');
    assert.deepEqual(h.commits, []);
  }
  h.drop(h.grid, { clientX: 270 });
  assert.deepEqual(h.commits, [['b', 'c', 'a']]); h.assertClean(); h.hook.unmount();
});

test('moving back to the original slot restores the preview without saving a new order', () => {
  const h = harness(); h.start(); h.move();
  h.move(h.cards.b, { clientX: 30 });
  assert.equal(h.cards.b.styles.get('--codex-card-shift-x'), '0px');
  assert.equal(h.cards.c.styles.get('--codex-card-shift-x'), '0px');
  h.drop(h.cards.b, { clientX: 30 });
  assert.deepEqual(h.commits, []); h.assertClean(); h.hook.unmount();
});

test('cross-row previews shift both axes and keep the fixed API card untouched', () => {
  const h = harness(); h.cards.c.rect = { left: 0, top: 160, width: 100, height: 100 };
  h.start(); h.move();
  assert.equal(h.cards.b.styles.get('--codex-card-shift-x'), '-120px');
  assert.equal(h.cards.c.styles.get('--codex-card-shift-x'), '120px');
  assert.equal(h.cards.c.styles.get('--codex-card-shift-y'), '-160px');
  assert.equal(h.apiCard.styles.size, 0);
  h.drop(); assert.deepEqual(h.commits, [['b', 'c', 'a']]); h.hook.unmount();
});

test('gaps retain the preview but releasing outside a card slot cancels instead of saving', () => {
  const h = harness(); h.start(); h.move();
  h.move(h.grid, { clientX: 110 });
  assert.equal(h.cards.b.styles.get('--codex-card-shift-x'), '-120px');
  h.drop(h.grid, { clientX: 110 });
  assert.deepEqual(h.commits, []); h.assertClean(); h.hook.unmount();
});

test('scrolling keeps the held card under the pointer and translates slot hit testing with the grid', () => {
  const h = harness(); h.start(); h.move();
  h.grid.rect.top = -40;
  h.emit('scroll'); h.flushFrames();
  assert.equal(h.cards.a.styles.get('--codex-card-drag-y'), '40px');
  h.drop(h.grid, { clientX: 270, clientY: 10 });
  assert.deepEqual(h.commits, [['b', 'c', 'a']]); h.assertClean(); h.hook.unmount();
});

test('events from another pointer cannot move, cancel or release an active drag', () => {
  const h = harness(); h.start(); h.move();
  h.emit('pointercancel', { pointerId: 8 }); h.emit('lostpointercapture', { pointerId: 8 });
  h.move(h.cards.b, { pointerId: 8 }); h.drop(h.cards.b, { pointerId: 8 });
  assert.equal(h.cards.a.dataset.codexDragging, 'true');
  assert.deepEqual(h.commits, []);
  h.drop(h.cards.c); assert.deepEqual(h.commits, [['b', 'c', 'a']]); h.hook.unmount();
});

test('changing view, account order or enabled state cancels the current drag', () => {
  for (const next of [{ viewKey: 'grid:filter:page2' }, { orderedIds: ['a', 'new', 'b', 'c'] }, { enabled: false }]) {
    const h = harness(); h.start(); h.move(); h.configure(next); h.drop();
    assert.deepEqual(h.commits, []); h.assertClean(); h.hook.unmount();
  }
});

test('disabled sorting or a single visible card exposes no usable drag operation', () => {
  const h = harness();
  h.configure({ enabled: false }); assert.equal(h.props('a')['data-codex-sort-id'], undefined);
  h.start(); h.move(); h.drop(); assert.deepEqual(h.commits, []);
  h.configure({ enabled: true }); h.start('a', h.cards.a, {}, ['a']); h.move(); h.drop();
  assert.deepEqual(h.commits, []); h.hook.unmount();
});

test('unmount cancels pending animation, releases capture and removes global listeners without saving', () => {
  const h = harness(); h.start(); h.move();
  h.emit('pointermove', { clientX: 40 });
  assert.equal(h.frames.size, 1);
  h.hook.unmount(); h.flushFrames();
  h.assertClean(); assert.equal(h.listeners.size, 0); assert.equal(h.timers.size, 0);
  assert.deepEqual(h.commits, []);
});

test('a drop remains protected from a synthetic click after the saved order causes an immediate rerender', () => {
  const h = harness();
  h.afterCommit((ids) => h.configure({ orderedIds: ids }));
  h.start(); h.move(); h.drop();
  assert.deepEqual(h.commits, [['b', 'c', 'a']]);
  const click = h.emit('click');
  assert.equal(click.defaultPrevented, true);
  assert.equal(click.propagationStopped, true);
  h.flushTimers(); assert.equal(h.emit('click').defaultPrevented, false); h.hook.unmount();
});

test('drop saves immediately and animates from the held position into the committed layout', () => {
  const h = harness();
  h.afterCommit((ids) => {
    ids.forEach((id, index) => { h.cards[id].rect.left = index * 120; });
    h.configure({ orderedIds: ids });
  });
  h.start(); h.move(); h.cards.a.visualScale = 1.015;
  // Simulate another card still halfway through its preview transition.
  h.cards.b.styles.set('--codex-card-shift-x', '-50px');
  h.drop();
  assert.deepEqual(h.commits, [['b', 'c', 'a']]);
  const landing = h.cards.a.lastAnimation;
  assert.ok(landing, 'the dragged card should glide to its saved position');
  assert.equal(landing.keyframes[0].translate, '20px 0px');
  assert.equal(landing.keyframes[0].scale, '1.015 1.015');
  assert.equal(landing.keyframes[1].translate, '0px 0px');
  assert.equal(landing.keyframes[1].scale, '1 1');
  assert.equal(landing.options.duration, 260);
  assert.equal(h.cards.b.lastAnimation.keyframes[0].translate, '70px 0px');
  assert.equal(h.apiCard.animations.length, 0);
  for (const card of Object.values(h.cards)) {
    card.lastAnimation?.onfinish?.();
    assert.equal(card.dataset.codexSettling, undefined);
  }
  h.assertClean(); h.hook.unmount();
});

test('Escape and an invalid drop animate cards home without saving a different order', () => {
  for (const action of ['escape', 'outside']) {
    const h = harness(); h.start(); h.move();
    if (action === 'escape') h.emit('keydown', { key: 'Escape' });
    else h.drop(null);
    assert.deepEqual(h.commits, []);
    assert.ok(h.cards.a.animations.length > 0);
    assert.equal(h.cards.a.lastAnimation.keyframes[1].translate, '0px 0px');
    h.hook.unmount();
    for (const card of Object.values(h.cards)) {
      assert.equal(card.dataset.codexSettling, undefined);
      assert.ok(card.animations.every((animation) => animation.cancelled));
    }
    h.assertClean();
  }
});

test('reduced motion skips landing animations for either application or system preference', () => {
  for (const [app, os] of [[true, false], [false, true]]) {
    const h = harness(); h.motionPreference(app, os);
    h.afterCommit((ids) => {
      ids.forEach((id, index) => { h.cards[id].rect.left = index * 120; });
      h.configure({ orderedIds: ids });
    });
    h.start(); h.move(); h.drop();
    assert.deepEqual(h.commits, [['b', 'c', 'a']]);
    assert.ok(Object.values(h.cards).every((card) => card.animations.length === 0));
    h.assertClean(); h.hook.unmount();
  }
});

test('starting another drag cancels the previous landing animation before taking new measurements', () => {
  const h = harness(); h.start(); h.move(); h.emit('keydown', { key: 'Escape' });
  const landing = h.cards.a.lastAnimation;
  assert.ok(landing);
  h.start('b');
  assert.equal(landing.cancelled, true);
  assert.ok(Object.values(h.cards).every((card) => card.dataset.codexSettling === undefined));
  h.move(h.cards.c); h.drop(h.cards.c);
  assert.deepEqual(h.commits, [['a', 'c', 'b']]); h.hook.unmount();
});

test('changing the view or resizing cancels landing animations and removes temporary state', () => {
  for (const action of ['view', 'resize']) {
    const h = harness(); h.start(); h.move(); h.emit('keydown', { key: 'Escape' });
    const landing = h.cards.a.lastAnimation;
    assert.ok(landing);
    if (action === 'view') h.configure({ viewKey: 'different-page' });
    else h.emit('resize');
    assert.equal(landing.cancelled, true);
    assert.ok(Object.values(h.cards).every((card) => card.dataset.codexSettling === undefined));
    h.assertClean(); h.hook.unmount();
  }
});
