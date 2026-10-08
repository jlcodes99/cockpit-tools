import assert from 'node:assert/strict';
import test from 'node:test';
import { createElement } from 'react';
import { renderToString } from 'react-dom/server';
import { useModalScrollLock } from '../hooks/useModalScrollLock.ts';
import { acquireModalScrollLock } from './modalScrollLock.ts';

interface InlineProperty {
  value: string;
  priority: string;
}

// Model overflow shorthand expansion as CSSStyleDeclaration does, so restoring
// mixed-priority longhands cannot accidentally pass using unrelated map entries.
class FakeStyle {
  private properties = new Map<string, InlineProperty>();
  writes = 0;

  getPropertyValue(property: string): string {
    if (property !== 'overflow') return this.properties.get(property)?.value ?? '';
    const x = this.properties.get('overflow-x');
    const y = this.properties.get('overflow-y');
    if (!x || !y || x.priority !== y.priority) return '';
    return x.value === y.value ? x.value : `${x.value} ${y.value}`;
  }

  getPropertyPriority(property: string): string {
    if (property !== 'overflow') return this.properties.get(property)?.priority ?? '';
    const x = this.properties.get('overflow-x');
    const y = this.properties.get('overflow-y');
    return x && y && x.priority === y.priority ? x.priority : '';
  }

  setProperty(property: string, value: string, priority = ''): void {
    this.writes += 1;
    if (!value) {
      this.removeProperty(property);
    } else if (property === 'overflow') {
      const [x, y = x] = value.split(' ');
      this.properties.set('overflow-x', { value: x, priority });
      this.properties.set('overflow-y', { value: y, priority });
    } else {
      this.properties.set(property, { value, priority });
    }
  }

  removeProperty(property: string): string {
    const previousValue = this.getPropertyValue(property);
    if (property === 'overflow') {
      this.properties.delete('overflow-x');
      this.properties.delete('overflow-y');
    } else {
      this.properties.delete(property);
    }
    return previousValue;
  }
}

function element(scrollTop = 0, scrollLeft = 0) {
  return { style: new FakeStyle(), scrollTop, scrollLeft };
}

type FakeElement = ReturnType<typeof element>;

function fixture(options: { empty?: boolean; withoutWrapper?: boolean } = {}) {
  const root = options.empty ? null : element(137, 4);
  const body = options.empty ? null : element(289, 5);
  const wrappers = options.empty || options.withoutWrapper ? [] : [element(421, 6)];
  const doc = {
    documentElement: root,
    body,
    querySelectorAll(selector: string) {
      assert.equal(selector, '.main-wrapper');
      return wrappers;
    },
  } as unknown as Document;
  return { doc, root, body, wrappers };
}

function assertLocked(target: FakeElement) {
  for (const property of ['overflow-x', 'overflow-y']) {
    assert.equal(target.style.getPropertyValue(property), 'hidden');
    assert.equal(target.style.getPropertyPriority(property), 'important');
  }
}

function assertUnstyled(target: FakeElement) {
  for (const property of ['overflow', 'overflow-x', 'overflow-y']) {
    assert.equal(target.style.getPropertyValue(property), '');
    assert.equal(target.style.getPropertyPriority(property), '');
  }
}

test('locks only background containers without changing any scroll position', () => {
  const { doc, root, body, wrappers } = fixture();
  const modalBody = element(83, 7);
  modalBody.style.setProperty('overflow-y', 'auto');
  const containers = [root!, body!, ...wrappers];
  const positions = containers.map(({ scrollTop, scrollLeft }) => [scrollTop, scrollLeft]);

  const release = acquireModalScrollLock(doc);
  containers.forEach(assertLocked);
  assert.equal(modalBody.style.getPropertyValue('overflow-y'), 'auto');
  assert.equal(modalBody.style.writes, 1);
  assert.deepEqual(containers.map(({ scrollTop, scrollLeft }) => [scrollTop, scrollLeft]), positions);

  release();
  containers.forEach(assertUnstyled);
  assert.deepEqual(containers.map(({ scrollTop, scrollLeft }) => [scrollTop, scrollLeft]), positions);
});

test('restores shorthand, longhand values and mixed priorities without reverting other edits', () => {
  const { doc, root, body, wrappers } = fixture();
  root!.style.setProperty('overflow', 'auto scroll', 'important');
  body!.style.setProperty('overflow-x', 'clip');
  body!.style.setProperty('overflow-y', 'auto', 'important');
  wrappers[0].style.setProperty('overflow-y', 'scroll');
  const containers = [root!, body!, ...wrappers];
  const snapshot = () => containers.map(({ style }) => (
    ['overflow', 'overflow-x', 'overflow-y'].map((property) => [
      style.getPropertyValue(property), style.getPropertyPriority(property),
    ])
  ));
  const before = snapshot();

  const release = acquireModalScrollLock(doc);
  containers.forEach(assertLocked);
  body!.style.setProperty('color', 'red');
  release();

  assert.deepEqual(snapshot(), before);
  assert.equal(body!.style.getPropertyValue('color'), 'red');
});

for (const releaseOrder of [[0, 1, 2], [2, 1, 0], [1, 0, 2]]) {
  test(`nested locks stay active until final release in order ${releaseOrder.join(',')}`, () => {
    const { doc, root, body, wrappers } = fixture();
    const containers = [root!, body!, ...wrappers];
    const releases = [acquireModalScrollLock(doc), acquireModalScrollLock(doc), acquireModalScrollLock(doc)];

    for (const index of releaseOrder.slice(0, -1)) {
      releases[index]();
      releases[index]();
      containers.forEach(assertLocked);
    }
    releases[releaseOrder[2]]();
    containers.forEach(assertUnstyled);
  });
}

test('a stale cleanup cannot release a newly acquired lock', () => {
  const { doc, root } = fixture();
  const releaseFirst = acquireModalScrollLock(doc);
  releaseFirst();
  const releaseNext = acquireModalScrollLock(doc);
  releaseFirst();
  assertLocked(root!);
  releaseNext();
  assertUnstyled(root!);
});

test('an existing inline scroll lock is preserved after all modals close', () => {
  const { doc, body } = fixture();
  body!.style.setProperty('overflow', 'hidden', 'important');
  const release = acquireModalScrollLock(doc);
  release();
  assertLocked(body!);
});

test('independent documents are restored independently', () => {
  const first = fixture();
  const second = fixture();
  const releaseFirst = acquireModalScrollLock(first.doc);
  const releaseSecond = acquireModalScrollLock(second.doc);
  releaseFirst();
  assertUnstyled(first.root!);
  assertLocked(second.root!);
  releaseSecond();
  assertUnstyled(second.root!);
});

test('works without a main wrapper or without attached document elements', () => {
  const withoutWrapper = fixture({ withoutWrapper: true });
  const release = acquireModalScrollLock(withoutWrapper.doc);
  assertLocked(withoutWrapper.root!);
  assertLocked(withoutWrapper.body!);
  release();
  assertUnstyled(withoutWrapper.root!);
  assertUnstyled(withoutWrapper.body!);

  const empty = fixture({ empty: true });
  const releaseEmpty = acquireModalScrollLock(empty.doc);
  releaseEmpty();
  releaseEmpty();
});

test('a wrapper mounted between nested acquisitions is locked and restored', () => {
  const { doc, root, wrappers } = fixture({ withoutWrapper: true });
  const releaseFirst = acquireModalScrollLock(doc);
  const wrapper = element(550);
  wrapper.style.setProperty('overflow-y', 'auto');
  wrappers.push(wrapper);
  const releaseSecond = acquireModalScrollLock(doc);
  releaseFirst();
  assertLocked(root!);
  assertLocked(wrapper);
  releaseSecond();
  assertUnstyled(root!);
  assert.equal(wrapper.style.getPropertyValue('overflow-y'), 'auto');
  assert.equal(wrapper.scrollTop, 550);
});

test('deduplicates containers matching more than one background selector', () => {
  const { doc, root, wrappers } = fixture();
  wrappers.push(root!);
  const release = acquireModalScrollLock(doc);
  assert.equal(root!.style.writes, 2);
  release();
  assertUnstyled(root!);
});

test('the hook is safe during server rendering without a document', () => {
  assert.equal(typeof document, 'undefined');
  function Probe({ open }: { open: boolean }) {
    useModalScrollLock(open);
    return createElement('span', null, 'modal');
  }
  assert.equal(renderToString(createElement(Probe, { open: true })), '<span>modal</span>');
  assert.equal(renderToString(createElement(Probe, { open: false })), '<span>modal</span>');
});
