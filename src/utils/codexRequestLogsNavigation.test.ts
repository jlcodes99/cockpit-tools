import assert from 'node:assert/strict';
import test from 'node:test';
import { parseCodexRequestLogsNavigation, requestCodexRequestLogsNavigation, takeCodexRequestLogsNavigation } from './codexRequestLogsNavigation';

test('request-log routes preserve an exact session and distinguish unfiltered navigation', () => {
  assert.deepEqual(parseCodexRequestLogsNavigation('codex-api-service?view=logs&sessionId=chat-123'), { sessionId: 'chat-123' });
  assert.deepEqual(parseCodexRequestLogsNavigation('codex-api-service?view=logs'), { sessionId: '' });
  assert.equal(parseCodexRequestLogsNavigation('codex-api-service'), null);
  assert.equal(parseCodexRequestLogsNavigation('dashboard?view=logs'), null);
});

test('lazy page mounting consumes the latest navigation once, including clearing a former session', () => {
  const previous = Object.getOwnPropertyDescriptor(globalThis, 'window');
  Object.defineProperty(globalThis, 'window', { configurable: true, value: new EventTarget() });
  try {
    requestCodexRequestLogsNavigation('codex-api-service?view=logs&sessionId=old');
    requestCodexRequestLogsNavigation('codex-api-service?view=logs');
    assert.deepEqual(takeCodexRequestLogsNavigation(), { sessionId: '' });
    assert.equal(takeCodexRequestLogsNavigation(), null);
  } finally {
    if (previous) Object.defineProperty(globalThis, 'window', previous);
    else Reflect.deleteProperty(globalThis, 'window');
  }
});
