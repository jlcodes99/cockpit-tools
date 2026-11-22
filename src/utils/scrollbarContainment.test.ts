import test from 'node:test';
import assert from 'node:assert/strict';
import { containsWithScrollbar, registerScrollbarOwner } from './scrollbarContainment';

function node(parent: Node | null = null): Node {
  const result = {
    parentNode: parent,
    contains(target: Node | null) {
      for (let current = target; current; current = current.parentNode) {
        if (current === result as unknown as Node) return true;
      }
      return false;
    },
  };
  return result as unknown as Node;
}

test('scrollbar gestures belong to their dropdown, including nested controls', () => {
  const menu = node();
  const viewport = node(menu);
  const externalSlot = node();
  const handle = node(node(externalSlot));
  registerScrollbarOwner(externalSlot, viewport);
  assert.equal(containsWithScrollbar(menu, handle), true);
  assert.equal(containsWithScrollbar(viewport, handle), true);
  assert.equal(containsWithScrollbar(menu, viewport), true);
});

test('outside clicks and controls belonging to other menus still close a dropdown', () => {
  const menu = node();
  const otherViewport = node();
  const otherSlot = node();
  registerScrollbarOwner(otherSlot, otherViewport);
  assert.equal(containsWithScrollbar(menu, node(otherSlot)), false);
  assert.equal(containsWithScrollbar(menu, node()), false);
  assert.equal(containsWithScrollbar(null, menu), false);
  assert.equal(containsWithScrollbar(menu, null), false);
});
