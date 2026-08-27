// Executable audit coverage for the autonomous OpenCode Go platform.
// Run with: node --test tests/openCodeGoPlatformMatrix.test.ts
import assert from 'node:assert/strict';
import test from 'node:test';

import {
  createOpenCodeGoConnectionSlots,
} from '../src/utils/openCodeGoConnections.ts';
import {
  classifyOpenCodeGoUsageError,
  findOpenCodeGoProvider,
  maskOpenCodeGoApiKey,
} from '../src/utils/openCodeGoPlatform.ts';
import type { OpenCodeGoConnection } from '../src/types/openCodeGo.ts';

function connection(id: string): OpenCodeGoConnection {
  return {
    id,
    name: `Connection ${id}`,
    keyHint: `${id.slice(0, 2)}****`,
    createdAt: 1,
    updatedAt: 1,
  };
}

test('exact-four invariant: the autonomous page always exposes four connection slots', () => {
  const slots = createOpenCodeGoConnectionSlots([
    connection('one'),
    connection('two'),
  ]);
  assert.equal(slots.length, 4);
  assert.deepEqual(slots.map((slot) => slot.connection?.id ?? null), [
    'one',
    'two',
    null,
    null,
  ]);
  assert.throws(
    () => createOpenCodeGoConnectionSlots([
      connection('1'),
      connection('2'),
      connection('3'),
      connection('4'),
      connection('5'),
    ]),
    /OPENCODE_GO_CONNECTION_LIMIT_EXCEEDED/,
  );
});

test('Codex isolation: legacy provider lookup ignores lookalikes and unrelated endpoints', () => {
  const official = {
    id: 'p-official',
    name: 'OpenCode Go',
    baseUrl: 'https://opencode.ai/zen/go/v1/',
    supportsWebsockets: false,
    apiKeys: [],
    createdAt: 1,
    updatedAt: 1,
  };
  const lookalike = { ...official, id: 'p-lookalike', baseUrl: 'https://evil.example/zen/go/v1' };
  const codexLike = { ...official, id: 'p-codex', baseUrl: 'https://chatgpt.com/backend-api' };

  assert.equal(findOpenCodeGoProvider([lookalike]), null);
  assert.equal(findOpenCodeGoProvider([codexLike]), null);
  assert.equal(findOpenCodeGoProvider([lookalike, official]), official);
  assert.equal(findOpenCodeGoProvider([]), null);
});

test('masked keys never leak the complete credential', () => {
  const secret = 'sk-opencode-go-super-secret-value';
  const masked = maskOpenCodeGoApiKey(secret);
  assert.ok(masked.length < secret.length);
  assert.ok(!masked.includes('super-secret'));
  assert.equal(masked, maskOpenCodeGoApiKey(`  ${secret}  `));
  assert.equal(maskOpenCodeGoApiKey('   '), '');
});

test('provider failures map to stable kinds without returning raw error details', () => {
  assert.equal(classifyOpenCodeGoUsageError('HTTP_401: rejected'), 'authentication');
  assert.equal(classifyOpenCodeGoUsageError('HTTP_403: forbidden'), 'authentication');
  assert.equal(classifyOpenCodeGoUsageError('HTTP_429: slow down'), 'rate_limit');
  assert.equal(classifyOpenCodeGoUsageError('PROVIDER_USAGE_NETWORK_FAILED'), 'network');
  assert.equal(classifyOpenCodeGoUsageError(new TypeError('boom')), 'unavailable');
});
