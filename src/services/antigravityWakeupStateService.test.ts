import assert from 'node:assert/strict';
import test from 'node:test';
import { normalizeWakeupTransportMode } from './antigravityWakeupStateService';

test('normalizeWakeupTransportMode maps clientless and falls back to language_server', () => {
  assert.equal(normalizeWakeupTransportMode('clientless'), 'clientless');
  assert.equal(normalizeWakeupTransportMode('language_server'), 'language_server');
  assert.equal(normalizeWakeupTransportMode(null), 'language_server');
  assert.equal(normalizeWakeupTransportMode(undefined), 'language_server');
  assert.equal(normalizeWakeupTransportMode('unknown'), 'language_server');
});
