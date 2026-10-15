import assert from 'node:assert/strict';
import test from 'node:test';
import { proxyPickerPosition } from './codexProxyPickerPosition';

test('short menu opens below even when there is less than 400px', () => {
  assert.deepEqual(proxyPickerPosition(500, 550, 800, 180), { top: 556, bottom: undefined, maxHeight: 238 });
});
test('upward menus anchor their bottom edge, not an assumed 400px top', () => {
  assert.deepEqual(proxyPickerPosition(500, 550, 600, 180), { top: undefined, bottom: 106, maxHeight: 400 });
});
test('large menu is constrained and filtering back to short content opens below', () => {
  assert.equal(proxyPickerPosition(500, 550, 800, 900).bottom, 306);
  assert.equal(proxyPickerPosition(500, 550, 800, 100).top, 556);
  assert.equal(proxyPickerPosition(100, 150, 300, 900).maxHeight, 138);
});
