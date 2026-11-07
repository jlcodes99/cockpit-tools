import assert from 'node:assert/strict';
import { test } from 'node:test';
import { isValidProviderApiKeyUrl } from '../src/utils/codexProviderApiKeyUrl';
import { applyExplicitApiBaseUrlToExternalImportItems as apply } from '../src/utils/externalImportRouting';
test('management links reject credentials and non-HTTP protocols', () => {
  for (const input of ['', 'https://example.invalid/keys', 'http://localhost/keys']) assert.ok(isValidProviderApiKeyUrl(input));
  for (const input of ['sk-synthetic', 'file:///tmp/key', 'https://user:password@example.invalid/keys']) assert.equal(isValidProviderApiKeyUrl(input), false);
});
test('external imports never derive an endpoint or overwrite an explicit provider', () => {
  const item = { auth_mode: 'apikey', OPENAI_API_KEY: 'synthetic' };
  assert.deepEqual(apply([item], { importUrl: 'https://download.invalid/api/cockpit-tools/import/fixture' }), [item]);
  const explicit = { ...item, base_url: 'https://provider.invalid/v1' };
  assert.deepEqual(apply([explicit], { apiBaseUrl: 'https://different.invalid/v1' }), [explicit]);
  assert.equal((apply([item], { apiBaseUrl: 'https://provider.invalid/v1/' })[0] as any).base_url, 'https://provider.invalid/v1');
  assert.throws(() => apply([item], { apiBaseUrl: 'https://user:password@example.invalid' }));
});

test('the host Cockpit Api bundle classification survives without deriving credential endpoints', () => {
  const item = { auth_mode: 'apikey', OPENAI_API_KEY: 'synthetic' };
  const request = { importUrl: 'https://download.invalid/api/cockpit-tools/import/fixture', apiBaseUrl: 'https://provider.invalid/v1' };
  const result = apply([item], request)[0] as Record<string, string>;
  assert.equal(result.api_provider_id, 'cockpit_api');
  assert.equal(result.base_url, 'https://provider.invalid/v1');
  assert.equal(result.plan_type, 'Cockpit Api');
  assert.deepEqual(apply([item], { importUrl: request.importUrl }), [item]);
  assert.equal((apply([item], { apiBaseUrl: request.apiBaseUrl })[0] as any).api_provider_id, undefined);
});
