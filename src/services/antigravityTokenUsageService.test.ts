import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  currentTzOffsetMinutes,
  normalizeAntigravityUsageRange,
} from './antigravityTokenUsageService';
import { isMainWindowNavigablePage } from '../types/navigation';
import { resolvePlatformIdFromPage } from '../utils/accountSyncEvents';

test('normalizeAntigravityUsageRange preserves valid presets and falls back to 30d', () => {
  assert.equal(normalizeAntigravityUsageRange('today'), 'today');
  assert.equal(normalizeAntigravityUsageRange('7d'), '7d');
  assert.equal(normalizeAntigravityUsageRange('30d'), '30d');
  assert.equal(normalizeAntigravityUsageRange('month'), 'month');
  assert.equal(normalizeAntigravityUsageRange('all'), 'all');
  assert.equal(normalizeAntigravityUsageRange('invalid'), '30d');
  assert.equal(normalizeAntigravityUsageRange(null), '30d');
  assert.equal(normalizeAntigravityUsageRange(undefined), '30d');
});

test('antigravity-usage is navigable and preserves floating card platform target', () => {
  assert.equal(isMainWindowNavigablePage('antigravity-usage'), true);
  assert.equal(resolvePlatformIdFromPage('antigravity-usage'), null);
  assert.equal(Number.isInteger(currentTzOffsetMinutes()), true);
});
