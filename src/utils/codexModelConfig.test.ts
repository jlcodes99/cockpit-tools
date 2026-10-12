import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { codexModelConfigErrorKey, withSupportedReasoningEfforts } from './codexModelConfig.ts';
import type { CodexExperimentalModelDefinition, CodexReasoningEffort } from '../types/codex.ts';

test('changing supported reasoning efforts clears only unsupported explicit defaults', () => {
  const model: CodexExperimentalModelDefinition = { model_id: 'custom', display_name: 'Custom',
    reasoning_efforts: ['low', 'high'], default_reasoning_effort: 'high', context_window: 1000 };
  assert.equal(withSupportedReasoningEfforts(model, ['low']).default_reasoning_effort, undefined);
  assert.equal(withSupportedReasoningEfforts(model, ['high']).default_reasoning_effort, 'high');
  const inherited = withSupportedReasoningEfforts(model, undefined, ['low']);
  assert.equal(inherited.reasoning_efforts, undefined);
  assert.equal(inherited.default_reasoning_effort, undefined);
  assert.equal(inherited.context_window, 1000);
  assert.equal(model.default_reasoning_effort, 'high');
});

test('supported reasoning efforts use increasing strength regardless of selection order', () => {
  const model: CodexExperimentalModelDefinition = { model_id: 'gpt-6-astra', display_name: 'Astra',
    default_reasoning_effort: 'high' };
  assert.deepEqual(withSupportedReasoningEfforts(model, ['high', 'medium']).reasoning_efforts, ['medium', 'high']);
  const selected: CodexReasoningEffort[] = ['ultra', 'max', 'xhigh', 'high', 'medium', 'low'];
  const result = withSupportedReasoningEfforts(model, selected);
  assert.deepEqual(result.reasoning_efforts, ['low', 'medium', 'high', 'xhigh', 'max', 'ultra']);
  assert.equal(result.default_reasoning_effort, 'high');
  assert.deepEqual(selected, ['ultra', 'max', 'xhigh', 'high', 'medium', 'low']);
  assert.equal(withSupportedReasoningEfforts(model, []).reasoning_efforts, undefined);
});

test('import errors are localized without exposing unknown raw diagnostic data', () => {
  assert.equal(codexModelConfigErrorKey('Error: MODEL_CONFIG_STATE_CHANGED'), 'codex.modelConfig.errors.stateChanged');
  assert.equal(codexModelConfigErrorKey('EXPERIMENTAL_MODEL_CATALOG_DEFAULT_REASONING_INVALID'), 'codex.experimentalModels.defaultReasoningInvalid');
  assert.equal(codexModelConfigErrorKey('private file /secret/password', 'read'), 'codex.modelConfig.errors.read');
  assert.equal(codexModelConfigErrorKey('MODEL_CONFIG_RECOVERY_CONFLICT'), 'codex.modelConfig.errors.recovery');
});

test('field-error translations cover missing required fields and remain localized', () => {
  const key = codexModelConfigErrorKey('MODEL_CONFIG_FIELDS_UNSUPPORTED');
  assert.equal(key, 'codex.modelConfig.errors.fields');
  const locales = ['ar', 'cs', 'de', 'en-US', 'en', 'es', 'fr', 'id', 'it', 'ja', 'ko', 'pl', 'pt-br', 'ru', 'tr', 'vi', 'zh-CN', 'zh-tw'];
  const messages = Object.fromEntries(locales.map((locale) => {
    const document = JSON.parse(readFileSync(new URL(`../locales/${locale}.json`, import.meta.url), 'utf8'));
    return [locale, document.codex.modelConfig.errors.fields as string];
  }));
  assert.match(messages['zh-CN'], /字段缺失/);
  assert.match(messages['en-US'], /missing, invalid, or unsupported fields/);
  assert.equal(messages.en, messages['en-US']);
  for (const locale of locales.filter((locale) => locale !== 'en' && locale !== 'en-US')) {
    assert.notEqual(messages[locale], messages.en, locale);
  }
});
