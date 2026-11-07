import assert from 'node:assert/strict';
import test from 'node:test';
import type { CodexAccount } from '../types/codex';
import { discoverImageModels, eligibleImageAccountIds, isImageGenerationEligibleAccount, resolveImageAccountModel, selectedImageAccountModels, configuredImageAccountIds, imageModelsForAccount, isImageModelInFamily } from './codexImageAccounts';
const account = (id: string, props: Partial<CodexAccount> = {}): CodexAccount => ({ id, email: id, auth_mode: 'oauth', plan_type: 'plus', ...props } as CodexAccount);
const image2 = account('image2', { auth_mode: 'apikey', api_model_catalog: ['gpt-image-2'] });
const image25 = account('image25', { auth_mode: 'apikey', api_model_catalog: ['gpt-image-2.5'] });
test('API image candidates match the chosen bare family or an exact custom ID', () => {
  assert.equal(isImageGenerationEligibleAccount(image2, 'gpt-image-2'), true);
  assert.equal(isImageGenerationEligibleAccount(image2, 'gpt-image-2.5'), false);
  assert.equal(isImageGenerationEligibleAccount(image25, 'gpt-image-2'), false);
  assert.equal(isImageGenerationEligibleAccount(image25, 'gpt-image-2.5'), true);
  assert.equal(isImageGenerationEligibleAccount(image2), false);
  const high = account('high', { auth_mode: 'apikey', api_model_catalog: ['gpt-image-2.5-high'] });
  assert.equal(isImageGenerationEligibleAccount(high, 'gpt-image-2.5'), true);
  assert.equal(isImageGenerationEligibleAccount(high, 'gpt-image-2.5-high'), true);
  assert.equal(isImageGenerationEligibleAccount(high, 'gpt-image-2.50'), false);
  assert.equal(isImageGenerationEligibleAccount(high, 'gpt-image-2.5-high-extra'), false);
});
test('OAuth only supports bare official image models and preserves Free exclusions', () => {
  assert.equal(isImageGenerationEligibleAccount(account('plus'), 'gpt-image-2.5'), true);
  assert.equal(isImageGenerationEligibleAccount(account('plus'), 'gpt-image-2.5-high'), false);
  assert.equal(isImageGenerationEligibleAccount(account('plus'), 'grok-imagine-image'), false);
  assert.equal(isImageGenerationEligibleAccount(account('free', { plan_type: 'free' }), 'gpt-image-2'), false);
  assert.equal(isImageGenerationEligibleAccount(account('plus')), false);
});
test('changing the complete ID removes mismatched, free and deleted accounts', () => {
  const accounts = [image2, image25, account('plus'), account('free', { plan_type: 'free' })];
  assert.deepEqual(eligibleImageAccountIds(['image2', 'image25', 'plus', 'free', 'deleted', 'plus'], accounts, 'gpt-image-2'), ['image2', 'plus']);
  assert.deepEqual(eligibleImageAccountIds(['image2', 'image25', 'plus'], accounts, 'gpt-image-2.5'), ['image25', 'plus']);
});
test('complete IDs trim whitespace but do not prefix- or case-match', () => {
  assert.equal(isImageGenerationEligibleAccount(account('spaces', { auth_mode: 'apikey', api_model_catalog: [' gpt-image-2 '] }), ' gpt-image-2 '), true);
  assert.equal(isImageGenerationEligibleAccount(account('case', { auth_mode: 'apikey', api_model_catalog: ['GPT-IMAGE-2'] }), 'gpt-image-2'), false);
});
test('actual variants require a valid explicit map, and stale selections never fall back to bare', () => {
  const api = account('api', { auth_mode: 'apikey', api_model_catalog: ['gpt-image-2.5', 'gpt-image-2.5-high'] });
  assert.equal(resolveImageAccountModel(api, 'gpt-image-2.5', { api: 'gpt-image-2.5-high' }), 'gpt-image-2.5-high');
  assert.deepEqual(configuredImageAccountIds(['api'], [api], 'gpt-image-2.5', { api: 'removed-model' }), []);
  assert.equal(resolveImageAccountModel(api, 'gpt-image-2.5'), 'gpt-image-2.5');
  assert.equal(resolveImageAccountModel(api, 'gpt-image-2.5', { api: 'gpt-image-2.5-high' }, ['gpt-image-2.5-high']), null);
  assert.deepEqual(selectedImageAccountModels(['api'], [api], 'gpt-image-2.5', { api: 'gpt-image-2.5-high' }), { api: 'gpt-image-2.5-high' });
  const suffixOnly = account('suffix', { auth_mode: 'apikey', api_model_catalog: ['gpt-image-2.5-high'] });
  assert.equal(resolveImageAccountModel(suffixOnly, 'gpt-image-2.5'), null);
});

test('bare model families never cross versions and deleted actuals are filtered without hiding the family', () => {
  for (const model of ['gpt-image-2.5', 'gpt-image-2.5-high', 'gpt-image-2.50', 'gpt-image-2-']) assert.equal(isImageModelInFamily(model, 'gpt-image-2'), false, model);
  assert.equal(isImageModelInFamily('gpt-image-2-high', 'gpt-image-2'), true);
  assert.equal(isImageModelInFamily('gpt-image-2.5-high', 'gpt-image-2.5'), true);
  assert.equal(isImageModelInFamily('grok-imagine-image-pro', 'grok-imagine-image'), false);
  const api = account('api', { auth_mode: 'apikey', api_model_catalog: ['gpt-image-2.5', 'gpt-image-2.5-high', 'gpt-image-2.5-low'] });
  assert.deepEqual(imageModelsForAccount(api, 'gpt-image-2.5', ['gpt-image-2.5', 'gpt-image-2.5-high']), ['gpt-image-2.5-low']);
  assert.equal(isImageGenerationEligibleAccount(api, 'gpt-image-2.5', ['gpt-image-2.5']), true);
  assert.equal(isImageGenerationEligibleAccount(account('oauth'), 'gpt-image-2.5', ['gpt-image-2.5']), false);
});

test('discovery includes cached image variants and Grok image models while excluding videos and invalid IDs', () => {
  const api = account('api', { auth_mode: 'apikey', api_model_catalog: ['gpt-image-2', 'gpt-image-2.5-high', 'grok-imagine-image', 'grok-imagine-image-pro', 'grok-imagine-image-video', 'grok-imagine-video-1.5', 'gpt-6', 'gpt-image-bad id', 'gpt-image-'+ 'a'.repeat(201)], api_image_generation_account_models: { old: 'gpt-image-legacy' } });
  assert.deepEqual(discoverImageModels([api, image2]), ['gpt-image-2', 'gpt-image-2.5-high', 'grok-imagine-image', 'grok-imagine-image-pro', 'gpt-image-legacy']);
});
