import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import {
  CODEX_TOP_LAYOUT_TABS,
  createDefaultCodexTopLayout,
  readCodexTopLayoutPreference,
  writeCodexTopLayoutPreference,
  normalizeCodexTopLayout,
  moveCodexTopLayoutTab,
} from './codexTopLayoutPreferences';

test('default layout preserves all released tabs in the top row', () => {
  const layout = createDefaultCodexTopLayout();
  assert.deepEqual(layout.order, ['overview', 'providers', 'wakeup', 'instances', 'sessions', 'proxy', 'top-layout']);
  assert.ok(layout.order.slice(0, 5).every((tab) => layout.placement[tab] === 'top'));
  assert.equal(layout.placement.proxy, 'more');
  assert.equal(layout.placement['top-layout'], 'more');
});

test('saved layout normalizes stale tabs and survives invalid storage', () => {
  const original = Object.getOwnPropertyDescriptor(globalThis, 'localStorage');
  let stored = JSON.stringify({ order: ['sessions', 'sessions', 'obsolete'], placement: { sessions: 'more' } });
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: {
      getItem: () => stored,
      setItem: (_key: string, value: string) => { stored = value; },
    },
  });
  try {
    const layout = readCodexTopLayoutPreference();
    assert.equal(layout.order[0], 'sessions');
    assert.equal(layout.order.length, CODEX_TOP_LAYOUT_TABS.length);
    assert.equal(layout.placement.sessions, 'top');
    assert.equal(layout.placement.proxy, 'more');
    assert.equal(layout.placement['top-layout'], 'more');
    writeCodexTopLayoutPreference(layout);
    assert.deepEqual(JSON.parse(stored), { order: layout.order });
    assert.deepEqual(readCodexTopLayoutPreference(), layout);
    const reordered = {
      order: ['top-layout', 'proxy', ...layout.order.filter((tab) => tab !== 'proxy' && tab !== 'top-layout')],
      placement: { ...layout.placement, proxy: 'top', 'top-layout': 'top' },
    } as typeof layout;
    writeCodexTopLayoutPreference(reordered);
    assert.deepEqual(readCodexTopLayoutPreference(), normalizeCodexTopLayout(reordered));
    assert.equal(readCodexTopLayoutPreference().placement.instances, 'more');
    stored = '{invalid';
    assert.deepEqual(readCodexTopLayoutPreference(), createDefaultCodexTopLayout());
  } finally {
    if (original) Object.defineProperty(globalThis, 'localStorage', original);
    else Reflect.deleteProperty(globalThis, 'localStorage');
  }
});

test('dragging across the fifth position and arrow moves share the same placement rule', () => {
  const original = createDefaultCodexTopLayout();
  const movedUp = moveCodexTopLayoutTab(original, 5, 4);
  assert.deepEqual(movedUp.order.slice(4), ['proxy', 'sessions', 'top-layout']);
  assert.equal(movedUp.placement.proxy, 'top');
  assert.equal(movedUp.placement.sessions, 'more');
  assert.deepEqual(moveCodexTopLayoutTab(movedUp, 4, 5), original);
  const dragged = moveCodexTopLayoutTab(original, 6, 0);
  assert.equal(dragged.order[0], 'top-layout');
  assert.equal(dragged.placement['top-layout'], 'top');
  assert.deepEqual(dragged.order.filter(tab => dragged.placement[tab] === 'more'), ['sessions', 'proxy']);
  assert.deepEqual(original, createDefaultCodexTopLayout());
  for (const [from, to] of [[0, -1], [6, 7], [-1, 0], [0, NaN], [0.5, 1], [2, 2]]) {
    assert.deepEqual(moveCodexTopLayoutTab(original, from, to), original);
  }
});

test('normalization always derives exactly five top tabs regardless of legacy placement', () => {
  for (const placement of [undefined, {}, Object.fromEntries(CODEX_TOP_LAYOUT_TABS.map(tab => [tab, 'top'])),
    Object.fromEntries(CODEX_TOP_LAYOUT_TABS.map(tab => [tab, 'more']))]) {
    const layout = normalizeCodexTopLayout({ order: [...CODEX_TOP_LAYOUT_TABS].reverse(), placement });
    assert.deepEqual(layout.order.filter(tab => layout.placement[tab] === 'top'), layout.order.slice(0, 5));
    assert.deepEqual(layout.order.filter(tab => layout.placement[tab] === 'more'), layout.order.slice(5));
  }
  for (const value of [null, [], 42, { order: 'invalid' }]) {
    assert.deepEqual(normalizeCodexTopLayout(value), createDefaultCodexTopLayout());
  }
});

test('restoring More tools does not reintroduce removed risk detection', () => {
  for (const path of [
    '../pages/CodexAccountsOverviewPanel.tsx',
    '../pages/useCodexAccountsOverviewController.tsx',
    '../pages/useCodexAccountsRenderers.tsx',
    '../pages/CodexApiServiceView.tsx',
  ]) {
    const source = readFileSync(new URL(path, import.meta.url), 'utf8');
    assert.doesNotMatch(source, /CodexAccountTurnState|turnStateCheck|renderAccountTurnStatePill/);
  }
});
