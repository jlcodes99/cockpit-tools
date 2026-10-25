const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const test = require('node:test');

const source = fs.readFileSync(path.join(__dirname, '../src-tauri/src/modules/codex_app_injection_usage.js'), 'utf8');
const metrics = vm.runInNewContext(source);
const base = Date.parse('2026-10-20T00:00:00Z');
const event = (seconds, output, input = 100, cached = 80, extra = {}) => ({
  t: new Date(base + seconds * 1000).toISOString(), turn_output_tokens: output,
  input_tokens: input, cached_input_tokens: cached, ...extra,
});
const read = (events, seconds = 60, turnId = null) => metrics.snapshot(events, base + seconds * 1000, turnId);

test('cache hits use input-token weighting, not the mean of call percentages', () => {
  const value = read([event(0, 100, 100, 100), event(10, 600, 900, 0)]);
  assert.equal(value.cache, 10);
  assert.equal(value.speed, 50);
});

test('missing cache counters stay unknown while explicit zero stays zero', () => {
  for (const missing of [undefined, null, '', '0', NaN, Infinity]) {
    assert.equal(read([event(0, 100, 100, 80, { cached_input_tokens: missing })]).cache, null);
  }
  assert.equal(read([event(0, 100, 100, 0)]).cache, 0);
  assert.equal(read([event(0, 100, 100, 100), event(10, 600, 900, null)]).cache, 100);
  assert.equal(read([event(0, 100, null, 0)]).cache, null);
});

test('speed uses only the three latest valid interval rates', () => {
  const value = read([event(0, 0), event(1, 1000), event(2, 1010), event(3, 1030), event(4, 1060)]);
  assert.equal(value.speed, 20);
});

test('counter rollback starts a new segment for both cache and speed', () => {
  const previous = [event(0, 100, 100, 100), event(10, 600, 100, 100)];
  const value = read([...previous, event(20, 10, 100, 20)]);
  assert.equal(value.speed, null);
  assert.equal(value.cache, 20);
  assert.equal(read([...previous, event(20, 10), event(22, 30)]).speed, 10);
});

test('turn IDs split metrics even when cumulative counters do not decrease', () => {
  const events = [event(0, 10, 100, 100, { turn_id: 'a' }),
    event(1, 50, 100, 100, { turn_id: 'a' }), event(2, 100, 100, 30, { turn_id: 'b' })];
  assert.equal(read(events).speed, null);
  assert.equal(read(events).cache, 30);
  assert.equal(read(events, 60, 'not-yet-written'), null);
  assert.equal(read(events, 60, 'b').cache, 30);
});

test('missing output samples cannot be coerced to zero or bridged', () => {
  const events = [event(0, 100), event(1, null), event(2, 300)];
  assert.equal(read(events).speed, null);
  assert.equal(read([event(0, null), event(1, 100)]).speed, null);
  assert.equal(read([event(0, 50), event(1, NaN), event(2, 10)]).speed, null);
});

test('sampling accepts 0.5–120 seconds and rejects too short, long, reversed or invalid times', () => {
  assert.equal(read([event(0, 0), event(0.5, 10)]).speed, 20);
  assert.equal(read([event(0, 0), event(120, 120)]).speed, 1);
  for (const seconds of [0, 0.499, 120.001, -1]) {
    assert.equal(read([event(0, 0), event(seconds, 120)]).speed, null);
  }
  assert.equal(read([event(0, 0), event(1, 120, 100, 80, { t: 'invalid' })]).speed, null);
  assert.equal(read([event(0, 100), event(1, 100)]).speed, null);
});

test('staleness is based on the newest event and missing data returns null', () => {
  assert.equal(read([event(0, 100)], 120).stale, false);
  assert.equal(read([event(0, 100)], 120).observedAt, base);
  assert.equal(read([event(0, 100, 100, 80, { t: 'invalid' })]).observedAt, null);
  assert.equal(read([event(0, 100)], 120.001).stale, true);
  assert.equal(read([event(0, 100), event(121, 200)], 122).stale, false);
  assert.equal(read([]), null);
  assert.equal(read(null), null);
  assert.equal(read([null, undefined]), null);
});

test('percentages clamp oversized cache values and reject negative or nonnumeric counters', () => {
  assert.equal(read([event(0, 100, 100, 200)]).cache, 100);
  for (const invalid of [-1, NaN, Infinity, null, '10']) {
    assert.equal(metrics.percent(invalid, 100), null);
    assert.equal(metrics.percent(100, invalid), null);
    assert.equal(read([event(0, 100, 100, invalid)]).cache, null);
  }
  assert.equal(metrics.percent(0, 100), 0);
  assert.equal(metrics.percent(100, 0), null);
  assert.equal(metrics.percent(200, 100), 100);
});

function domHarness({ composerId, placement = 'thread', sidebarId, cards = [] } = {}) {
  const attributes = { 'data-composer-placement': placement };
  const root = {
    getAttribute: (name) => attributes[name] ?? null,
    querySelector: () => composerId == null ? null : { getAttribute: () => composerId },
  };
  const card = { closest: () => root };
  const dom = vm.runInNewContext(source, {
    document: {
      querySelector: () => sidebarId == null ? null : { getAttribute: () => sidebarId },
      querySelectorAll: () => cards,
    },
    getComputedStyle: (element) => element.style,
  });
  return { dom, card };
}
const firstId = '01a157ce-ea9c-7683-940e-db6cb1f236db';
const secondId = 'aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee';

test('composer identity wins over sidebar and accepts only UUIDs with optional local prefix', () => {
  const { dom, card } = domHarness({ composerId: `local:${firstId}`, sidebarId: secondId });
  assert.equal(dom.threadId(card), firstId);
  const fallback = domHarness({ sidebarId: secondId });
  assert.equal(fallback.dom.threadId(fallback.card), secondId);
  const uppercase = domHarness({ composerId: firstId.toUpperCase() });
  assert.equal(uppercase.dom.threadId(uppercase.card), firstId);
  assert.equal(dom.threadId(null), '');
});

test('new chat, home composer and invalid identifiers cannot inherit the previous sidebar thread', () => {
  for (const composerId of [`local:client-new-thread:${firstId}`, 'prototype', 'constructor', '__proto__', '../file', 'not-a-uuid', '', '  ']) {
    const { dom, card } = domHarness({ composerId, sidebarId: secondId });
    assert.equal(dom.threadId(card), '');
  }
  const { dom, card } = domHarness({ placement: 'home', sidebarId: secondId });
  assert.equal(dom.threadId(card), '');
});

function surface({ hidden = false, visibility = 'visible', own = false, editors = [], width = 400 } = {}) {
  return {
    style: { display: hidden ? 'none' : 'block', visibility },
    getClientRects: () => hidden ? [] : [{}],
    getBoundingClientRect: () => ({ width, height: 100 }),
    closest: () => own ? {} : null,
    querySelectorAll: (selector) => selector.includes('reasoning') ? [] : editors,
  };
}

test('visible composer selection ignores hidden surfaces, empty shells and Cockpit nodes', () => {
  const editor = surface();
  const selected = surface({ editors: [editor] });
  const { dom } = domHarness({ cards: [surface({ hidden: true, editors: [editor] }), surface(),
    surface({ own: true, editors: [editor] }), surface({ editors: [surface({ hidden: true })] }), selected] });
  assert.equal(dom.visibleCard(), selected);
});

test('no actual visible editor means no composer even with a visible card shell', () => {
  const { dom } = domHarness({ cards: [surface(), surface({ editors: [surface({ visibility: 'hidden' })] }),
    surface({ width: 0, editors: [surface()] })] });
  assert.equal(dom.visibleCard(), null);
});
