import assert from 'node:assert/strict';
import test from 'node:test';
import type { CodexModelProvider } from '../services/codexModelProviderService.ts';
import {
  classifyOpenCodeGoUsageError,
  findOpenCodeGoProvider,
  maskOpenCodeGoApiKey,
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
