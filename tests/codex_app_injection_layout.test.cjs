const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const test = require('node:test');

const source = fs.readFileSync(path.join(__dirname, '../src-tauri/src/modules/codex_app_injection_layout.js'), 'utf8');
const rect = (left, top, width, height) => ({ left, top, width, height, right: left + width, bottom: top + height });
const viewport = { width: 1200, height: 800 };
const footer = rect(100, 700, 1000, 48);
const permissions = rect(150, 710, 130, 28);
const badge = { width: 330, height: 24 };
const layout = vm.runInNewContext(source);

function avoids(placement, size, blocker) {
  return placement.left + size.width / 2 <= blocker.left - 8
    || placement.left - size.width / 2 >= blocker.right + 8;
}

test('quota badges use the real free gap before context, Fast, model and microphone controls', () => {
  const controls = [rect(820, 710, 28, 28), rect(852, 710, 30, 28), rect(886, 710, 140, 28), rect(1032, 710, 28, 28)];
  const placement = layout.findPlacement(footer, permissions, controls, badge, viewport);
  assert.ok(placement);
  assert.equal(placement.top, 724);
  assert.ok(placement.left - badge.width / 2 >= permissions.right + 8);
  for (const control of controls) assert.ok(avoids(placement, badge, control));
});

test('a recording waveform, timer or recognition error takes priority over the badges', () => {
  const occupied = [rect(300, 706, 700, 36), rect(1020, 710, 40, 28)];
  assert.equal(layout.findPlacement(footer, permissions, occupied, badge, viewport), null);
});

test('narrow windows hide the complete badge group and widening restores it', () => {
  const model = rect(500, 710, 180, 28);
  assert.equal(layout.findPlacement(rect(100, 700, 600, 48), permissions, [model], badge, viewport), null);
  assert.ok(layout.findPlacement(footer, permissions, [rect(900, 710, 180, 28)], badge, viewport));
});

test('does not bridge disjoint free gaps or cover native text between controls', () => {
  const blockers = [rect(460, 710, 130, 28), rect(780, 710, 300, 28)];
  assert.equal(layout.findPlacement(footer, permissions, blockers, badge, viewport), null);
  const compact = { width: 160, height: 24 };
  const placement = layout.findPlacement(footer, permissions, blockers, compact, viewport);
  assert.ok(placement);
  blockers.forEach((blocker) => assert.ok(avoids(placement, compact, blocker)));
});

test('ignores another row while respecting viewport edges and hidden composer geometry', () => {
  assert.ok(layout.findPlacement(footer, permissions, [rect(300, 660, 700, 24)], badge, viewport));
  assert.equal(layout.findPlacement(null, permissions, [], badge, viewport), null);
  assert.equal(layout.findPlacement(footer, permissions, [], badge, { width: 600, height: 800 }), null);
  assert.equal(layout.findPlacement(footer, permissions, [], badge, { width: 1200, height: 720 }), null);
});

function node(box, { control = true, text = '', hidden = false, own = false } = {}) {
  return {
    childNodes: text ? [{ nodeType: 3, textContent: text }] : [],
    style: {}, hidden, own,
    getClientRects() { return hidden ? [] : [box]; },
    getBoundingClientRect() { return box; },
    closest() { return own ? this : null; },
    matches() { return control; },
    hasAttribute(name) { return Boolean(this.attributes?.[name]); },
  };
}

function domHarness() {
  const observed = [];
  let disconnects = 0;
  const dom = vm.runInNewContext(source, {
    window: { innerWidth: 1200, innerHeight: 800, ResizeObserver: true },
    getComputedStyle: (element) => ({ display: element.hidden ? 'none' : 'block', visibility: 'visible' }),
    ResizeObserver: class {
      observe(element) { observed.push(element); }
      disconnect() { disconnects++; }
    },
  });
  const host = node(rect(0, 0, 330, 24));
  const permission = node(permissions);
  const toolbar = node(footer);
  const controls = [node(rect(850, 710, 30, 28)), node(rect(890, 710, 190, 28))];
  toolbar.querySelectorAll = () => controls;
  return { dom, host, permission, toolbar, controls, observed, disconnects: () => disconnects };
}

test('DOM integration measures full badges and reuses ResizeObserver without a render loop', () => {
  const { dom, host, permission, toolbar, observed, disconnects } = domHarness();
  const state = { scheduleRender() {} };
  assert.ok(dom.place(state, host, toolbar, permission));
  assert.equal(host.style.visibility, 'visible');
  assert.equal(host.style.display, 'flex');
  assert.equal(observed.length, 4);
  dom.place(state, host, toolbar, permission);
  assert.equal(disconnects(), 1);
  dom.disconnect(state);
  assert.equal(disconnects(), 2);
});

test('DOM integration gives non-button native error text space and recovers after it disappears', () => {
  const { dom, host, permission, toolbar, controls } = domHarness();
  const state = { scheduleRender() {} };
  controls.push(node(rect(300, 710, 500, 28), { control: false, text: 'native recording status' }));
  assert.equal(dom.place(state, host, toolbar, permission), null);
  assert.equal(host.style.display, 'none');
  controls.pop();
  assert.ok(dom.place(state, host, toolbar, permission));
  assert.equal(host.style.visibility, 'visible');
});

test('DOM integration excludes hidden controls and its own detached overlays', () => {
  const { dom, host, permission, toolbar, controls } = domHarness();
  controls.push(node(rect(300, 710, 500, 28), { hidden: true }));
  controls.push(node(rect(300, 710, 500, 28), { own: true }));
  assert.ok(dom.place({ scheduleRender() {} }, host, toolbar, permission));
});


test('native dictation waveform and live-text modes hide badges until the toolbar returns', () => {
  const { dom, host, permission, toolbar } = domHarness();
  const state = { scheduleRender() {} };
  for (const mode of ['waveform', 'text']) {
    toolbar.attributes = { 'data-dictation-view': mode };
    assert.equal(dom.place(state, host, toolbar, permission), null);
    assert.equal(host.style.display, 'none');
  }
  toolbar.attributes = { 'data-dictation-enabled': '' };
  assert.ok(dom.place(state, host, toolbar, permission));
  assert.equal(host.style.display, 'flex');
});
