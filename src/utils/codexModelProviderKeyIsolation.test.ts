import assert from 'node:assert/strict';
import test from 'node:test';

import {
  resolveCodexModelProviderKeyModels,
  snapshotLegacyCodexModelProviderKeys,
  type CodexModelProvider,
} from '../services/codexModelProviderService.ts';
import { buildCodexModelProviderAccountSnapshot } from './codexModelProviderAccountSync.ts';

const provider: CodexModelProvider = {
  id: 'shared-upstream',
  name: 'sub2api',
  baseUrl: 'https://sub2api.example/v1',
  modelCatalog: ['legacy-shared'],
  modelContextWindows: { 'legacy-shared': 128_000 },
  supportsWebsockets: false,
  apiKeys: [
    { id: 'oai-key', name: 'kfjie', apiKey: 'oai-secret',
      modelCatalog: ['gpt-6-sol'],
      modelContextWindows: { 'gpt-6-sol': 1_000_000 },
      modelAutoCompactTokenLimits: { 'gpt-6-sol': 900_000 },
      compactionMode: 'remote', createdAt: 1, updatedAt: 1 },
    { id: 'third-party-key', name: 'kfjie-3', apiKey: 'third-party-secret',
      modelCatalog: ['deepseek-v4', 'kimi-k3'],
      modelContextWindows: { 'deepseek-v4': 128_000, 'kimi-k3': 256_000 },
      modelAutoCompactTokenLimits: { 'deepseek-v4': 110_000, 'kimi-k3': 220_000 },
      compactionMode: 'local', createdAt: 2, updatedAt: 2 },
  ],
  createdAt: 1,
  updatedAt: 2,
};

test('same endpoint keeps each key model catalog, context and compaction settings separate', () => {
  const oai = resolveCodexModelProviderKeyModels(provider, provider.apiKeys[0]);
  const thirdParty = resolveCodexModelProviderKeyModels(provider, provider.apiKeys[1]);
  assert.deepEqual(oai.modelCatalog, ['gpt-6-sol']);
  assert.deepEqual(thirdParty.modelCatalog, ['deepseek-v4', 'kimi-k3']);
  assert.equal(oai.modelContextWindows['gpt-6-sol'], 1_000_000);
  assert.equal(thirdParty.modelContextWindows['deepseek-v4'], 128_000);
  assert.equal(oai.modelAutoCompactTokenLimits['gpt-6-sol'], 900_000);
  assert.equal(thirdParty.modelAutoCompactTokenLimits['kimi-k3'], 220_000);
  assert.equal(oai.compactionMode, 'remote');
  assert.equal(thirdParty.compactionMode, 'local');
});

test('account projections select the credential, not the shared base URL', () => {
  const oai = buildCodexModelProviderAccountSnapshot(provider, 'kfjie', 'oai-secret');
  const thirdParty = buildCodexModelProviderAccountSnapshot(provider, 'kfjie-3', 'third-party-secret');
  assert.deepEqual(oai.apiModelCatalog, ['gpt-6-sol']);
  assert.deepEqual(thirdParty.apiModelCatalog, ['deepseek-v4', 'kimi-k3']);
  assert.deepEqual(oai.apiModelContextWindows, { 'gpt-6-sol': 1_000_000 });
  assert.deepEqual(thirdParty.apiModelContextWindows, { 'deepseek-v4': 128_000, 'kimi-k3': 256_000 });
  assert.throws(
    () => buildCodexModelProviderAccountSnapshot(provider, 'unknown', 'unregistered-secret'),
    /API_KEY_NOT_FOUND/,
  );
});

test('the same model id can have a different window under each key', () => {
  const sharedModel = structuredClone(provider);
  sharedModel.apiKeys[0].modelCatalog = ['model-x'];
  sharedModel.apiKeys[0].modelContextWindows = { 'model-x': 200_000 };
  sharedModel.apiKeys[1].modelCatalog = ['model-x'];
  sharedModel.apiKeys[1].modelContextWindows = { 'model-x': 1_000_000 };
  const first = buildCodexModelProviderAccountSnapshot(sharedModel, 'kfjie', 'oai-secret');
  const second = buildCodexModelProviderAccountSnapshot(sharedModel, 'kfjie-3', 'third-party-secret');
  assert.equal(first.apiModelContextWindows?.['model-x'], 200_000);
  assert.equal(second.apiModelContextWindows?.['model-x'], 1_000_000);
});

test('legacy keys inherit provider defaults until edited, including an explicit empty catalog', () => {
  assert.deepEqual(resolveCodexModelProviderKeyModels(provider, undefined).modelCatalog, ['legacy-shared']);
  assert.deepEqual(resolveCodexModelProviderKeyModels(provider, {
    ...provider.apiKeys[0], modelCatalog: [],
  }).modelCatalog, []);
});

test('first-load migration gives legacy keys independent copies of the provider defaults', () => {
  const legacy = structuredClone(provider);
  for (const key of legacy.apiKeys) {
    delete key.modelCatalog;
    delete key.modelContextWindows;
  }
  assert.equal(snapshotLegacyCodexModelProviderKeys(legacy), true);
  legacy.apiKeys[0].modelCatalog!.push('gpt-5.6-sol');
  legacy.apiKeys[0].modelContextWindows!['gpt-5.6-sol'] = 516_000;
  assert.deepEqual(legacy.apiKeys[1].modelCatalog, provider.modelCatalog);
  assert.deepEqual(legacy.apiKeys[1].modelContextWindows, provider.modelContextWindows);
  assert.equal(snapshotLegacyCodexModelProviderKeys(legacy), false);
});
