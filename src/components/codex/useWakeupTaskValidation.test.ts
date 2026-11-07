import assert from 'node:assert/strict';
import test from 'node:test';
import { loadHookModule } from '../../../tests/helpers/reactHookHarness';

function harness() {
  const frames = new Map<number, () => void>();
  const operations: any[] = [];
  let sequence = 0;
  const control = { focus: (options: unknown) => operations.push(['focus', options]) };
  const group = { querySelector: () => control };
  const h = loadHookModule(new URL('./useWakeupTaskValidation.tsx', import.meta.url), {
    '../../utils/reducedMotion': { scrollElementIntoView: (element: unknown) => operations.push(['scroll', element]) },
  }, { window: {
    requestAnimationFrame: (callback: () => void) => { const id = ++sequence; frames.set(id, callback); return id; },
    cancelAnimationFrame: (id: number) => frames.delete(id),
  } });
  let draft = {};
  let open = true;
  let validation = h.render(() => h.exports.useWakeupTaskValidation(draft, open));
  validation.dialog.current = { querySelector: (selector: string) => { operations.push(selector); return group; } };
  return { h, operations, frames, get validation() { return validation; },
    flush() { validation = h.flush(); },
    nextDraft() { draft = {}; validation = h.flush(); },
    close() { open = false; validation = h.flush(); },
    tick() { const callbacks = [...frames.values()]; frames.clear(); callbacks.forEach((callback) => callback()); },
  };
}

test('field failures display beside their field and scroll then focus the matching control', () => {
  const h = harness();
  h.validation.report('accounts', 'Choose accounts'); h.flush(); h.tick();
  assert.equal(h.validation.message('name'), null);
  assert.equal(h.validation.message('accounts').props.role, 'alert');
  assert.equal(h.validation.message('accounts').props.children, 'Choose accounts');
  assert.equal(h.operations[0], '[data-wakeup-field="accounts"]');
  assert.equal(h.operations[1][0], 'scroll');
  assert.equal(h.operations[2][0], 'focus');
  assert.equal(h.operations[2][1].preventScroll, true);
  h.h.unmount();
});

test('changing a field clears stale errors and prevents late focus after closing', () => {
  const h = harness();
  h.validation.report('name', 'Required'); h.flush();
  h.nextDraft();
  assert.equal(h.validation.invalid('name'), false);
  assert.equal(h.frames.size, 0);
  h.validation.report('model', 'Choose model'); h.flush();
  h.close(); h.tick();
  assert.equal(h.validation.message('model'), null);
  assert.deepEqual(h.operations, []);
  h.h.unmount();
});

test('resubmission clears the previous field error and cancels its pending focus', () => {
  const h = harness();
  h.validation.report('name', 'Required'); h.flush();
  h.validation.clear(); h.flush(); h.tick();
  assert.equal(h.validation.message('name'), null);
  assert.deepEqual(h.operations, []);
  h.h.unmount();
});
