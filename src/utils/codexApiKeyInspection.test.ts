import assert from 'node:assert/strict';
import test from 'node:test';
import {
  requestCodexApiKeyInspection,
  consumeCodexApiKeyInspectionRequest,
  subscribeCodexApiKeyInspectionRequests,
  type CodexApiKeyInspectionRequest,
} from './codexApiKeyInspection';

function withWindow(run: (window: EventTarget) => void) {
  const previous = Object.getOwnPropertyDescriptor(globalThis, 'window');
  const window = new EventTarget();
  Object.defineProperty(globalThis, 'window', { configurable: true, value: window });
  try { run(window); }
  finally {
    consumeCodexApiKeyInspectionRequest();
    if (previous) Object.defineProperty(globalThis, 'window', previous);
    else Reflect.deleteProperty(globalThis, 'window');
  }
}

test('inspection context persists until a new API service page subscribes, then consumes once', () => withWindow((window) => {
  const destinations: unknown[] = [];
  window.addEventListener('app-request-navigate', (event) => destinations.push((event as CustomEvent).detail));
  const request = { apiKeyId: 'client-key-id', requestKind: 'text' };
  requestCodexApiKeyInspection(request);
  request.apiKeyId = 'changed-after-dispatch';
  assert.deepEqual(destinations, ['codex-api-service']);
  const received: CodexApiKeyInspectionRequest[] = [];
  const unsubscribe = subscribeCodexApiKeyInspectionRequests((value) => received.push(value));
  assert.deepEqual(received, [{ apiKeyId: 'client-key-id', requestKind: 'text' }]);
  assert.equal(consumeCodexApiKeyInspectionRequest(), null);
  unsubscribe();
  const another = subscribeCodexApiKeyInspectionRequests((value) => received.push(value));
  assert.equal(received.length, 1);
  another();
}));

test('an already mounted page receives each request without timers and unsubscribe retains later pending requests', () => withWindow(() => {
  const received: CodexApiKeyInspectionRequest[] = [];
  const unsubscribe = subscribeCodexApiKeyInspectionRequests((value) => received.push(value));
  requestCodexApiKeyInspection({ apiKeyId: '__internal_profile__', requestKind: 'image_edit' });
  assert.deepEqual(received, [{ apiKeyId: '__internal_profile__', requestKind: 'image_edit' }]);
  assert.equal(consumeCodexApiKeyInspectionRequest(), null);
  unsubscribe();
  requestCodexApiKeyInspection({ apiKeyId: 'next-key', requestKind: 'image_generation' });
  assert.equal(received.length, 1);
  assert.deepEqual(consumeCodexApiKeyInspectionRequest(), { apiKeyId: 'next-key', requestKind: 'image_generation' });
  assert.equal(consumeCodexApiKeyInspectionRequest(), null);
}));

test('navigation-triggered subscription consumes context before the notification without duplicate delivery', () => withWindow((window) => {
  const received: CodexApiKeyInspectionRequest[] = [];
  let unsubscribe = () => {};
  window.addEventListener('app-request-navigate', () => {
    unsubscribe = subscribeCodexApiKeyInspectionRequests((value) => received.push(value));
  });
  requestCodexApiKeyInspection({ apiKeyId: 'mounted-during-navigation', requestKind: 'text' });
  assert.equal(received.length, 1);
  assert.equal(consumeCodexApiKeyInspectionRequest(), null);
  unsubscribe();
}));
