import assert from 'node:assert/strict';
import test from 'node:test';
import { DEFAULT_WAKEUP_MODEL, selectProviderBatchTestModelId } from './codexTestModel';

test('Responses tests prefer the wakeup default over legacy and image models', () => {
  assert.equal(
    selectProviderBatchTestModelId('responses', ['gpt-image-2', 'gpt-5.4', 'gpt-5.5', DEFAULT_WAKEUP_MODEL]),
    DEFAULT_WAKEUP_MODEL,
  );
  assert.equal(
    selectProviderBatchTestModelId('responses', ['gpt-5.5', ` ${DEFAULT_WAKEUP_MODEL.toUpperCase()} `]),
    DEFAULT_WAKEUP_MODEL.toUpperCase(),
  );
});

test('providers without the wakeup model keep their supported text models', () => {
  assert.equal(selectProviderBatchTestModelId('responses', ['gpt-5.4', 'gpt-5.5']), 'gpt-5.5');
  assert.equal(selectProviderBatchTestModelId('responses', ['gpt-image-2', 'deepseek-chat']), 'deepseek-chat');
});

test('Chat Completions tests retain provider catalog order', () => {
  assert.equal(
    selectProviderBatchTestModelId('chat_completions', ['provider-default', DEFAULT_WAKEUP_MODEL]),
    'provider-default',
  );
});

test('empty provider catalogs leave model discovery to the backend', () => {
  assert.equal(selectProviderBatchTestModelId('responses', [' ', '']), null);
  assert.equal(selectProviderBatchTestModelId('responses'), null);
});


test('GPT-6.1 Sol is preferred over legacy candidates when wakeup model is absent', () => {
  assert.equal(selectProviderBatchTestModelId('responses', ['gpt-image-2', 'gpt-5.4', 'gpt-6.1-sol']), 'gpt-6.1-sol');
  assert.equal(selectProviderBatchTestModelId('responses', ['gpt-6.1-sol', DEFAULT_WAKEUP_MODEL]), DEFAULT_WAKEUP_MODEL);
});


test('current models outrank retired provider entries during automatic tests', () => {
  assert.equal(selectProviderBatchTestModelId('responses', ['gpt-4o', 'gpt-5.4', 'gpt-6-luna']), 'gpt-6-luna');
  assert.equal(selectProviderBatchTestModelId('responses', ['gpt-5.2', 'gpt-6-astra']), 'gpt-6-astra');
});
