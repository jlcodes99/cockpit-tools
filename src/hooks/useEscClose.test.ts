import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';

interface KeyEvent {
  key: string;
  defaultPrevented: boolean;
  stopped: boolean;
  preventDefault(): void;
  stopImmediatePropagation(): void;
}

/** Window capture listeners run before bubble listeners, each in registration order. */
function windowEvents() {
  type Listener = { callback: (event: KeyEvent) => void; capture: boolean };
  const listeners: Listener[] = [];
  let added = 0;
  let removed = 0;
  return {
    addEventListener(type: string, callback: Listener['callback'], capture = false) {
      assert.equal(type, 'keydown');
      listeners.push({ callback, capture }); added++;
    },
    removeEventListener(type: string, callback: Listener['callback'], capture = false) {
      assert.equal(type, 'keydown');
      const index = listeners.findIndex((entry) => entry.callback === callback && entry.capture === capture);
      if (index !== -1) { listeners.splice(index, 1); removed++; }
    },
    dispatch(key = 'Escape') {
      const event: KeyEvent = { key, defaultPrevented: false, stopped: false,
        preventDefault() { this.defaultPrevented = true; },
        stopImmediatePropagation() { this.stopped = true; } };
      for (const capture of [true, false]) {
        for (const listener of [...listeners]) {
          if (event.stopped) return event;
          if (listener.capture === capture && listeners.includes(listener)) listener.callback(event);
        }
      }
      return event;
    },
    get count() { return listeners.length; },
    get added() { return added; },
    get removed() { return removed; },
  };
}

function setup() {
  const window = windowEvents();
  const hooks = loadHookModule(new URL('./useEscClose.ts', import.meta.url), {}, { window });
  return { window, hooks };
}

test('Escape closes the engine prerequisite first and preserves the account draft', () => {
  const { window, hooks } = setup();
  const closed: string[] = [];
  let accountOpen = true;
  let engineOpen = false;
  let accountDraft: string | null = 'selected proxy group';
  hooks.render(() => {
    hooks.exports.useEscCloseTopmost(accountOpen, () => {
      closed.push('account'); accountOpen = false; accountDraft = null;
    });
    hooks.exports.useEscCloseTopmost(engineOpen, () => { closed.push('engine'); engineOpen = false; });
  });
  engineOpen = true; hooks.flush();

  const event = window.dispatch();
  hooks.flush();
  assert.deepEqual(closed, ['engine']);
  assert.equal(accountDraft, 'selected proxy group');
  assert.equal(accountOpen, true);
  assert.equal(event.defaultPrevented, true);
  assert.equal(event.stopped, true);
  assert.equal(window.added, 1);

  window.dispatch(); hooks.flush();
  assert.deepEqual(closed, ['engine', 'account']);
  assert.equal(window.count, 0);
  assert.equal(window.removed, 1);
  hooks.unmount();
});

test('callback updates do not reorder the stack and the latest callback is used', () => {
  const { window, hooks } = setup();
  const closed: string[] = [];
  let version = 1;
  let topOpen = true;
  hooks.render(() => {
    const current = version;
    hooks.exports.useEscCloseTopmost(true, () => closed.push(`account-${current}`));
    hooks.exports.useEscCloseTopmost(topOpen, () => { closed.push(`engine-${current}`); topOpen = false; });
  });
  version = 2; hooks.flush();
  window.dispatch(); hooks.flush();
  window.dispatch();
  assert.deepEqual(closed, ['engine-2', 'account-2']);
  assert.equal(window.added, 1);
  hooks.unmount();
  assert.equal(window.count, 0);
});

test('removing a non-top entry keeps the remaining dialogs in order', () => {
  const { window, hooks } = setup();
  const closed: number[] = [];
  const open = [true, true, true];
  hooks.render(() => {
    for (let index = 0; index < open.length; index++) {
      hooks.exports.useEscCloseTopmost(open[index], () => { closed.push(index); open[index] = false; });
    }
  });
  open[1] = false; hooks.flush();
  assert.equal(window.count, 1);
  window.dispatch(); hooks.flush();
  window.dispatch(); hooks.flush();
  assert.deepEqual(closed, [2, 0]);
  assert.equal(window.count, 0);
  hooks.unmount();
});

test('unmount cleanup works from bottom to top and removes the last listener', () => {
  const { window, hooks } = setup();
  let closed = 0;
  hooks.render(() => {
    hooks.exports.useEscCloseTopmost(true, () => closed++);
    hooks.exports.useEscCloseTopmost(true, () => closed++);
    hooks.exports.useEscCloseTopmost(true, () => closed++);
  });
  assert.equal(window.count, 1);
  hooks.unmount();
  assert.equal(window.count, 0);
  assert.equal(window.removed, 1);
  window.dispatch();
  assert.equal(closed, 0);
});

test('ordinary Escape handling resumes after the topmost dialog closes', () => {
  const { window, hooks } = setup();
  const closed: string[] = [];
  let topOpen = false;
  hooks.render(() => {
    // Registered before the capture listener, just as an existing account dialog is.
    hooks.exports.useEscClose(true, () => closed.push('ordinary'));
    hooks.exports.useEscCloseTopmost(topOpen, () => { closed.push('top'); topOpen = false; });
  });
  window.dispatch();
  assert.deepEqual(closed, ['ordinary']);
  topOpen = true; hooks.flush();
  const otherKey = window.dispatch('Enter');
  assert.equal(otherKey.defaultPrevented, false);
  assert.deepEqual(closed, ['ordinary']);
  window.dispatch(); hooks.flush();
  assert.deepEqual(closed, ['ordinary', 'top']);
  window.dispatch();
  assert.deepEqual(closed, ['ordinary', 'top', 'ordinary']);
  hooks.unmount();
  assert.equal(window.count, 0);
});
