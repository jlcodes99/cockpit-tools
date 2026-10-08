import assert from 'node:assert/strict';
import test from 'node:test';
import {
  getAntigravityRuntimeTarget,
  hasSavedAntigravityRuntimeTarget,
  resolveAntigravityStartupTarget,
} from './antigravityRuntimeTarget.ts';

test('saved client preferences survive unavailable installation discovery', () => {
  const previous = Object.getOwnPropertyDescriptor(globalThis, 'window');
  try {
    for (const saved of ['antigravity', 'antigravity_ide', null, 'unknown']) {
      Object.defineProperty(globalThis, 'window', { configurable: true, value: {
        localStorage: { getItem: () => saved },
      } });
      assert.equal(hasSavedAntigravityRuntimeTarget(), saved === 'antigravity' || saved === 'antigravity_ide');
      if (hasSavedAntigravityRuntimeTarget()) assert.equal(getAntigravityRuntimeTarget(), saved);
    }
    Object.defineProperty(globalThis, 'window', { configurable: true, value: {
      localStorage: { getItem: () => { throw new Error('unavailable'); } },
    } });
    assert.equal(hasSavedAntigravityRuntimeTarget(), false);
  } finally {
    if (previous) Object.defineProperty(globalThis, 'window', previous);
    else Reflect.deleteProperty(globalThis, 'window');
  }
});

test('explicit startup client distinguishes APP and IDE without changing legacy overview', () => {
  assert.equal(resolveAntigravityStartupTarget(' Antigravity '), 'antigravity');
  assert.equal(resolveAntigravityStartupTarget('Antigravity-IDE'), 'antigravity_ide');
  for (const value of ['overview', 'last', 'codex', null]) {
    assert.equal(resolveAntigravityStartupTarget(value), null);
  }
});
