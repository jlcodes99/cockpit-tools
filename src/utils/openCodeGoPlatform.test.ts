import assert from 'node:assert/strict';
import test from 'node:test';
import type { CodexModelProvider } from '../services/codexModelProviderService.ts';
import {
  classifyOpenCodeGoUsageError,
  findOpenCodeGoProvider,
  isPlaintextOpenCodeGoApiKey,
  maskOpenCodeGoApiKey,
  openCodeGoConnectionCapacity,
  orderOpenCodeGoConnections,
  sanitizeOpenCodeGoProviderForTransfer,
  stripOpenCodeGoKeyMaterial,
} from './openCodeGoPlatform.ts';

function provider(
  baseUrl: string,
  apiKeys: CodexModelProvider['apiKeys'] = [],
): CodexModelProvider {
  return {
    id: 'provider-1',
    name: 'OpenCode Go',
    baseUrl,
    supportsWebsockets: false,
    apiKeys,
    createdAt: 1,
    updatedAt: 1,
  };
}

test('findOpenCodeGoProvider selects only the official Go endpoint', () => {
  const expected = provider('https://opencode.ai/zen/go/v1/');
  assert.equal(
    findOpenCodeGoProvider([
      provider('https://example.com/zen/go/v1'),
      expected,
    ]),
    expected,
  );
});

test('OpenCode Go connection policy rejects a fifth key and reports four occupied slots', () => {
  assert.deepEqual(openCodeGoConnectionCapacity(0), {
    count: 0,
    remaining: 4,
    canAdd: true,
  });
  assert.deepEqual(openCodeGoConnectionCapacity(4), {
    count: 4,
    remaining: 0,
    canAdd: false,
  });
  assert.throws(
    () => openCodeGoConnectionCapacity(5),
    /OPENCODE_GO_CONNECTION_LIMIT_EXCEEDED/,
  );
});

test('OpenCode Go connections have stable creation order with an id tie-breaker', () => {
  const connections = [
    { id: 'ocg_z', createdAt: 10 },
    { id: 'ocg_b', createdAt: 20 },
    { id: 'ocg_a', createdAt: 20 },
  ];

  assert.deepEqual(
    orderOpenCodeGoConnections([connections[1], connections[0], connections[2]]).map(
      ({ id }) => id,
    ),
    ['ocg_z', 'ocg_a', 'ocg_b'],
  );
  assert.deepEqual(connections.map(({ id }) => id), ['ocg_z', 'ocg_b', 'ocg_a']);
});

test('maskOpenCodeGoApiKey never returns the complete credential', () => {
  assert.equal(maskOpenCodeGoApiKey('sk-example-secret'), 'sk-e****cret');
  assert.equal(maskOpenCodeGoApiKey('short'), 'sh****');
});

test('classifyOpenCodeGoUsageError maps provider failures without exposing details', () => {
  assert.equal(
    classifyOpenCodeGoUsageError('PROVIDER_USAGE_HTTP_401: rejected'),
    'authentication',
  );
  assert.equal(
    classifyOpenCodeGoUsageError('PROVIDER_USAGE_NETWORK_FAILED: timeout'),
    'network',
  );
});

const secret = 'ocg-live-credential-9876';

function goProviderWithKey(apiKey: string): CodexModelProvider {
  return provider('https://opencode.ai/zen/go/v1/', [
    {
      id: 'key-1',
      name: 'Primary',
      apiKey,
      createdAt: 1,
      updatedAt: 1,
    },
  ]);
}

test('sanitizeOpenCodeGoProviderForTransfer redacts only OpenCode Go keys', () => {
  const sanitized = sanitizeOpenCodeGoProviderForTransfer(
    goProviderWithKey(secret),
  );
  const [redacted] = sanitized.apiKeys;
  assert.equal(redacted.id, 'key-1');
  assert.equal(redacted.name, 'Primary');
  assert.match(redacted.apiKey, /^redacted:ocg-/);
  assert.ok(!redacted.apiKey.includes(secret));

  const untouched = provider('https://example.com/v1');
  assert.equal(sanitizeOpenCodeGoProviderForTransfer(untouched), untouched);
});

test('isPlaintextOpenCodeGoApiKey rejects redaction markers and empties', () => {
  assert.equal(isPlaintextOpenCodeGoApiKey(secret), true);
  assert.equal(
    isPlaintextOpenCodeGoApiKey(`redacted:${maskOpenCodeGoApiKey(secret)}`),
    false,
  );
  assert.equal(isPlaintextOpenCodeGoApiKey(''), false);
  assert.equal(isPlaintextOpenCodeGoApiKey('   '), false);
  assert.equal(isPlaintextOpenCodeGoApiKey(undefined), false);
  assert.equal(isPlaintextOpenCodeGoApiKey(null), false);
  assert.equal(isPlaintextOpenCodeGoApiKey(42), false);
});

test('stripOpenCodeGoKeyMaterial keeps only non-secret import fields', () => {
  const stripped = stripOpenCodeGoKeyMaterial({
    id: 'key-9',
    name: 'Carried Name',
    createdAt: 5,
    updatedAt: 6,
    apiKey: secret,
  });
  assert.deepEqual(stripped, { id: 'key-9', apiKey: 'redacted:' });
});
