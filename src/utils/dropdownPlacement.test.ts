import assert from 'node:assert/strict';
import test from 'node:test';
import { dropdownLayer, dropdownPlacement } from './dropdownPlacement';

test('menus inherit the owning dialog layer rather than disappearing beneath it', () => {
  assert.equal(dropdownLayer(['auto', '12040', 'auto']), 12041);
  assert.equal(dropdownLayer(['12500', '12040']), 12501);
  assert.equal(dropdownLayer(['auto', '0']), 11000);
});

test('a trigger near the bottom of a 600px dialog opens above within the viewport', () => {
  const p = dropdownPlacement({ left: 500, top: 510, bottom: 556, width: 300 }, { width: 900, height: 600 }, 'down', undefined, 280, 12041);
  assert.equal(p.bottom, 100);
  assert.equal(p.maxHeight, 280);
  assert.equal(p.top, undefined);
});

test('requested upwards placement near the top falls back below', () => {
  const p = dropdownPlacement({ left: 40, top: 20, bottom: 66, width: 200 }, { width: 900, height: 600 }, 'up', undefined, 280, 11000);
  assert.equal(p.top, 76);
  assert.equal(p.bottom, undefined);
});

test('small windows constrain width and height rather than forcing a 160px overflow', () => {
  const p = dropdownPlacement({ left: -20, top: 80, bottom: 126, width: 300 }, { width: 220, height: 180 }, 'down', 500, 280, 11000);
  assert.equal(p.width, 196);
  assert.equal(p.left, 12);
  assert.equal(p.maxHeight, 58);
  assert.equal(p.bottom, 110);
});
