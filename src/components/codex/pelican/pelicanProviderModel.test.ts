import assert from 'node:assert/strict';
import test from 'node:test';
import type { CodexModelProvider } from '../../../services/codexModelProviderService';
import { pelicanProviderTarget, validatePelicanProviderTargets } from './pelicanProviderModel';

const provider: CodexModelProvider = { id: 'p', name: 'Relay', baseUrl: 'https://example.test/v1',
  supportsWebsockets: false, createdAt: 1, updatedAt: 1,
  modelCatalog: ['gpt-image-2', 'custom-model', 'gpt-6-astra'],
  apiKeys: ['one', 'two'].map((id) => ({ id, name: id, apiKey: `secret-${id}`, createdAt: 1, updatedAt: 1 })) };

test('provider entry carries the selected key and Pelican model without credentials', () => {
  const target = pelicanProviderTarget(provider, 'two');
  assert.deepEqual(target, { providerId: 'p', apiKeyId: 'two', model: 'gpt-6-astra' });
  assert.equal(JSON.stringify(target).includes('secret-'), false);
});
test('non-Codex providers use their own text model and custom input remains valid', () => {
  assert.equal(pelicanProviderTarget({ ...provider, modelCatalog: ['gpt-image-2', 'deepseek-chat'] })?.model, 'deepseek-chat');
  assert.equal(validatePelicanProviderTargets([provider], [{ providerId: 'p', apiKeyId: 'one', model: 'custom-other' }]), true);
});
test('empty models and removed credentials cannot start a provider test', () => {
  assert.equal(pelicanProviderTarget({ ...provider, apiKeys: [] }), null);
  for (const target of [{ providerId: 'p', apiKeyId: 'gone', model: 'm' }, { providerId: 'p', apiKeyId: 'one', model: ' ' }]) {
    assert.equal(validatePelicanProviderTargets([provider], [target]), false);
  }
});


test('provider tests select the current key model catalog instead of provider defaults', () => {
  const isolated = { ...provider, apiKeys: provider.apiKeys.map((key) => ({
    ...key, modelCatalog: key.id === 'one' ? ['model-a'] : ['model-b'],
  })) };
  assert.equal(pelicanProviderTarget(isolated, 'one')?.model, 'model-a');
  assert.equal(pelicanProviderTarget(isolated, 'two')?.model, 'model-b');
});
