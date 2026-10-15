import assert from 'node:assert/strict';
import test from 'node:test';
import { updateModelPricingDraft, type ModelPricingDraft } from './codexModelPricingDraft';

const draft = (): ModelPricingDraft => ({
  modelId: 'gpt-5.5', longContextThresholdTokens: '272000',
  inputUsdPerMillion: '5', cachedInputUsdPerMillion: '0.5', outputUsdPerMillion: '30',
  standardLongInputUsdPerMillion: '10', standardLongCachedInputUsdPerMillion: '1',
  standardLongOutputUsdPerMillion: '45', standardLongPriceOverride: false,
  priorityInputUsdPerMillion: '10', priorityCachedInputUsdPerMillion: '1',
  priorityOutputUsdPerMillion: '60', hasPreset: true, custom: false,
});

test('changing base rates keeps unedited long rates derived, not marked custom', () => {
  const changed = updateModelPricingDraft(draft(), 'inputUsdPerMillion', '8');
  assert.equal(changed.standardLongInputUsdPerMillion, '16');
  assert.equal(changed.standardLongPriceOverride, false);
  const cacheFallback = updateModelPricingDraft(changed, 'cachedInputUsdPerMillion', '');
  assert.equal(cacheFallback.standardLongCachedInputUsdPerMillion, '16');
  assert.equal(draft().standardLongInputUsdPerMillion, '10');
});

test('explicit zero long prices survive subsequent base-price edits', () => {
  const custom = updateModelPricingDraft(draft(), 'standardLongInputUsdPerMillion', '0');
  const changed = updateModelPricingDraft(custom, 'inputUsdPerMillion', '8');
  assert.equal(changed.standardLongPriceOverride, true);
  assert.equal(changed.standardLongInputUsdPerMillion, '0');
  assert.equal(changed.standardLongOutputUsdPerMillion, '45');
});

test('clearing all long-price fields restores derivation on the next base edit', () => {
  let cleared = draft();
  for (const key of ['standardLongInputUsdPerMillion', 'standardLongCachedInputUsdPerMillion',
    'standardLongOutputUsdPerMillion'] as const) cleared = updateModelPricingDraft(cleared, key, '');
  assert.equal(cleared.standardLongPriceOverride, false);
  assert.equal(updateModelPricingDraft(cleared, 'outputUsdPerMillion', '20')
    .standardLongOutputUsdPerMillion, '30');
});

test('invalid base input stays empty in derived displays for validation', () => {
  assert.equal(updateModelPricingDraft(draft(), 'inputUsdPerMillion', '-1')
    .standardLongInputUsdPerMillion, '');
  const withoutLong = { ...draft(), longContextThresholdTokens: '',
    standardLongInputUsdPerMillion: '', standardLongCachedInputUsdPerMillion: '',
    standardLongOutputUsdPerMillion: '' };
  assert.equal(updateModelPricingDraft(withoutLong, 'inputUsdPerMillion', '8')
    .standardLongInputUsdPerMillion, '');
});
