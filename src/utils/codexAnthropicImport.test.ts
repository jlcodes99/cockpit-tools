import assert from 'node:assert/strict';
import test from 'node:test';
import { parseCodexAnthropicImport } from './codexAnthropicImport.ts';

test('imports Claude Code settings with API key precedence and explicit gateway URLs', () => {
  const result = parseCodexAnthropicImport(JSON.stringify({ env: {
    ANTHROPIC_BASE_URL: 'https://example.invalid/anthropic', ANTHROPIC_AUTH_TOKEN: 'token',
    ANTHROPIC_API_KEY: 'key', ANTHROPIC_MODEL: 'claude-sonnet-4-6',
    ANTHROPIC_DEFAULT_SONNET_MODEL: 'claude-sonnet-4-6',
  } }));
  assert.deepEqual(result, { baseUrl: 'https://example.invalid/anthropic', apiKey: 'key', authScheme: 'x-api-key', models: ['claude-sonnet-4-6'] });
  assert.equal(parseCodexAnthropicImport('{"ANTHROPIC_BASE_URL":"http://localhost:9000","ANTHROPIC_AUTH_TOKEN":"token"}')?.authScheme, 'bearer');
});

test('imports Desktop Gateway settings and retains only configured model names', () => {
  assert.deepEqual(parseCodexAnthropicImport(JSON.stringify({ inferenceGatewayBaseUrl: 'https://example.invalid',
    inferenceGatewayApiKey: 'fixture', inferenceGatewayAuthScheme: 'bearer',
    inferenceModels: [{ name: 'claude-opus-4-6', supports1m: true }, 'claude-sonnet-4-6'] })), {
    baseUrl: 'https://example.invalid', apiKey: 'fixture', authScheme: 'bearer', models: ['claude-opus-4-6', 'claude-sonnet-4-6'],
  });
});

test('rejects incomplete settings and credential-bearing or non-HTTP URLs', () => {
  for (const raw of ['[]', 'bad json', '{"env":{}}',
    '{"ANTHROPIC_BASE_URL":"file:///tmp/a","ANTHROPIC_API_KEY":"fixture"}',
    '{"ANTHROPIC_BASE_URL":"https://user:pass@example.invalid","ANTHROPIC_API_KEY":"fixture"}',
  ]) assert.equal(parseCodexAnthropicImport(raw), null);
});
