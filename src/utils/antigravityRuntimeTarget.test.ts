import assert from 'node:assert/strict';
import test from 'node:test';
import {
  antigravityAppPathTarget,
  getAntigravityRuntimeTarget,
  hasSavedAntigravityRuntimeTarget,
  normalizeAntigravityRuntimeTarget,
  resolveAntigravityStartupTarget,
} from './antigravityRuntimeTarget.ts';

test('desktop and IDE recovery use separate path targets and missing context defaults to IDE', () => {
  assert.equal(antigravityAppPathTarget(normalizeAntigravityRuntimeTarget('antigravity')), 'antigravity_legacy');
  assert.equal(antigravityAppPathTarget(normalizeAntigravityRuntimeTarget('antigravity_ide')), 'antigravity');
  assert.equal(antigravityAppPathTarget(normalizeAntigravityRuntimeTarget(undefined)), 'antigravity');
});

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
