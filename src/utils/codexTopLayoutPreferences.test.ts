import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { loadHookModule } from '../../tests/helpers/reactHookHarness';
import {
  CODEX_TOP_LAYOUT_TABS,
  createDefaultCodexTopLayout,
  normalizeCodexTopLayout,
  moveCodexTopLayoutTab,
} from './codexTopLayoutPreferences';

test('default layout preserves all released tabs in the top row', () => {
  const layout = createDefaultCodexTopLayout();
  assert.deepEqual(layout.order, ['overview', 'providers', 'wakeup', 'instances', 'sessions', 'proxy', 'top-layout', 'quota-history']);
  assert.ok(layout.order.slice(0, 5).every((tab) => layout.placement[tab] === 'top'));
  assert.equal(layout.placement.proxy, 'more');
  assert.equal(layout.placement['top-layout'], 'more');
  assert.equal(layout.placement['quota-history'], 'more');
});

function layoutStore(initial: string | null) {
  let value = initial;
  let fail = false;
  const { exports } = loadHookModule(new URL('./codexTopLayoutPreferences.ts', import.meta.url), {
    './durableUiSettings': {
      readDurableUiSetting: () => value,
      async saveDurableUiSetting(_key: string, next: string) {
        if (fail) throw Error('quota');
        value = next;
      },
    },
  });
  return { exports, get value() { return value; }, set value(next) { value = next; }, set fail(next: boolean) { fail = next; } };
}

test('strict storage access propagates failures so callers can offer retry', async () => {
  const store = layoutStore(null);
  const layout = moveCodexTopLayoutTab(createDefaultCodexTopLayout(), 5, 0);
  store.fail = true;
  await assert.rejects(store.exports.writeCodexTopLayoutPreference(layout), /quota/);
  assert.equal(store.value, null);
  store.fail = false;
  await store.exports.writeCodexTopLayoutPreference(layout);
  assert.deepEqual(Array.from(store.exports.loadCodexTopLayoutPreference().order), layout.order);
  store.value = '{corrupt';
  assert.throws(store.exports.loadCodexTopLayoutPreference);
  assert.equal(store.value, '{corrupt');
});

test('saved layout normalizes stale tabs and survives invalid storage', async () => {
  const store = layoutStore(JSON.stringify({ order: ['sessions', 'sessions', 'obsolete'], placement: { sessions: 'more' } }));
  const layout = store.exports.readCodexTopLayoutPreference();
  assert.equal(layout.order[0], 'sessions');
  assert.equal(layout.order.length, CODEX_TOP_LAYOUT_TABS.length);
  assert.equal(layout.placement.sessions, 'top');
  assert.equal(layout.placement.proxy, 'more');
  await store.exports.writeCodexTopLayoutPreference(layout);
  assert.deepEqual(JSON.parse(store.value!), { order: Array.from(layout.order) });
  const reordered = {
    order: ['top-layout', 'proxy', ...layout.order.filter((tab: string) => tab !== 'proxy' && tab !== 'top-layout')],
    placement: { ...layout.placement, proxy: 'top', 'top-layout': 'top' },
  } as typeof layout;
  await store.exports.writeCodexTopLayoutPreference(reordered);
  assert.deepEqual(Array.from(store.exports.readCodexTopLayoutPreference().order), normalizeCodexTopLayout(reordered).order);
  assert.equal(store.exports.readCodexTopLayoutPreference().placement.instances, 'more');
  store.value = '{invalid';
  assert.deepEqual(Array.from(store.exports.readCodexTopLayoutPreference().order), createDefaultCodexTopLayout().order);
});

test('dragging across the fifth position and arrow moves share the same placement rule', () => {
  const original = createDefaultCodexTopLayout();
  const movedUp = moveCodexTopLayoutTab(original, 5, 4);
  assert.deepEqual(movedUp.order.slice(4), ['proxy', 'sessions', 'top-layout', 'quota-history']);
  assert.equal(movedUp.placement.proxy, 'top');
  assert.equal(movedUp.placement.sessions, 'more');
  assert.deepEqual(moveCodexTopLayoutTab(movedUp, 4, 5), original);
  const dragged = moveCodexTopLayoutTab(original, 6, 0);
  assert.equal(dragged.order[0], 'top-layout');
  assert.equal(dragged.placement['top-layout'], 'top');
  assert.deepEqual(dragged.order.filter(tab => dragged.placement[tab] === 'more'), ['sessions', 'proxy', 'quota-history']);
  assert.deepEqual(original, createDefaultCodexTopLayout());
  for (const [from, to] of [[0, -1], [7, 8], [-1, 0], [0, NaN], [0.5, 1], [2, 2]]) {
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
