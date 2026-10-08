import assert from 'node:assert/strict';
import test from 'node:test';
import { floatingCardTargetHeight, nextFloatingCardOpacity, normalizeFloatingCardOpacity } from './floatingCardAppearance';

test('old floating windows retain full size and background by default', () => {
  assert.equal(normalizeFloatingCardOpacity(undefined), 1);
  assert.equal(normalizeFloatingCardOpacity(Number.NaN), 1);
  assert.equal(floatingCardTargetHeight(false, 85, false), 290);
});
test('minimal windows shrink, retain space for close confirmation and cap long content', () => {
  assert.equal(floatingCardTargetHeight(true, 85, false), 110);
  assert.equal(floatingCardTargetHeight(true, 85, true), 240);
  assert.equal(floatingCardTargetHeight(true, 130.5, false), 131);
  assert.equal(floatingCardTargetHeight(true, 900, false), 520);
  assert.equal(floatingCardTargetHeight(false, Number.NaN, false), 290);
});
test('background cycling reaches transparent and returns to the existing appearance', () => {
  assert.equal(nextFloatingCardOpacity(1), .85);
  assert.equal(nextFloatingCardOpacity(.4), .25);
  assert.equal(nextFloatingCardOpacity(.25), 0);
  assert.equal(nextFloatingCardOpacity(0), 1);
  assert.equal(normalizeFloatingCardOpacity(-1), 0);
  assert.equal(normalizeFloatingCardOpacity(2), 1);
});
