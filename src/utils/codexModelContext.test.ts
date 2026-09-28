import assert from 'node:assert/strict';
import test from 'node:test';
import {
  deriveAutoCompactTokenLimit,
  deriveAutoCompactTokenLimitInput,
  resolveStoredCompactLimitInput,
  validateEffectiveModelContexts,
  validateModelContext,
} from './codexModelContext';

test('model context accepts complete and empty overrides', () => {
  assert.equal(validateModelContext({}), null);
  assert.equal(validateModelContext({ context_window: 516000, auto_compact_token_limit: 464400 }), null);
  assert.equal(validateModelContext({ context_window: 1000000, auto_compact_token_limit: 900000 }), null);
  assert.equal(validateModelContext({ context_window: 32768, auto_compact_token_limit: 30000 }), null);
});

test('auto compact limit is derived as 90% of the context window', () => {
  assert.equal(deriveAutoCompactTokenLimit(516000), 464400);
  assert.equal(deriveAutoCompactTokenLimit(1000000), 900000);
  assert.equal(deriveAutoCompactTokenLimit(272000), 244800);
  assert.equal(deriveAutoCompactTokenLimit(1048576), 943718);
  assert.ok(Number.isNaN(deriveAutoCompactTokenLimit(0)));
  assert.ok(Number.isNaN(deriveAutoCompactTokenLimit(-1)));
  assert.ok(Number.isNaN(deriveAutoCompactTokenLimit(1.5)));
  assert.equal(deriveAutoCompactTokenLimitInput(' 516000 '), '464400');
  assert.equal(deriveAutoCompactTokenLimitInput(''), '');
  assert.equal(deriveAutoCompactTokenLimitInput('0'), '');
});

test('stored config without a usable compact limit falls back to the 90% value', () => {
  // 缺少压缩阈值、等于上下文（永不压缩）、超过上下文或非法值都要归一。
  assert.equal(resolveStoredCompactLimitInput(1000000, undefined), '900000');
  assert.equal(resolveStoredCompactLimitInput(1000000, null), '900000');
  assert.equal(resolveStoredCompactLimitInput(1000000, 1000000), '900000');
  assert.equal(resolveStoredCompactLimitInput(1000000, 1200000), '900000');
  assert.equal(resolveStoredCompactLimitInput(516000, 0), '464400');
  // 合法的用户显式值保持不变。
  assert.equal(resolveStoredCompactLimitInput(516000, 460000), '460000');
  assert.equal(resolveStoredCompactLimitInput(1000000, 900000), '900000');
  // 上下文非法时保留原值，交给上层校验报错。
  assert.equal(resolveStoredCompactLimitInput(undefined, 460000), '460000');
  assert.equal(resolveStoredCompactLimitInput(undefined, undefined), '');
});

test('model context accepts independent overrides and rejects invalid values', () => {
  assert.equal(validateModelContext({ context_window: 100 }), null);
  assert.equal(validateModelContext({ auto_compact_token_limit: 50 }), null);
  for (const model of [
    { context_window: 0, auto_compact_token_limit: 1 },
    { context_window: 1.5, auto_compact_token_limit: 1 },
    { context_window: Infinity, auto_compact_token_limit: 1 },
    { context_window: Number.MAX_SAFE_INTEGER + 1, auto_compact_token_limit: 1 },
    { context_window: 100, auto_compact_token_limit: NaN },
    { context_window: 100, auto_compact_token_limit: -1 },
    { context_window: 100, auto_compact_token_limit: 100 },
    { context_window: 100, auto_compact_token_limit: 101 },
  ]) assert.ok(validateModelContext(model), JSON.stringify(model));
});

test('effective context applies global values to models without overrides', () => {
  const models = [{ model_id: 'gpt-6-sol', display_name: 'GPT-6 Sol' }];
  assert.equal(validateEffectiveModelContexts(models, 2_760_000, 2_750_000), null);
  assert.match(
    validateEffectiveModelContexts([{ ...models[0], context_window: 1_050_000 }], 2_760_000, 2_750_000) ?? '',
    /gpt-6-sol.*2750000.*1050000/,
  );
});

test('source metadata is used after per-model and instance values', () => {
  const model = { model_id: 'model-x', display_name: 'Model X' };
  const metadata = { 'model-x': { contextWindow: 1_050_000, autoCompactTokenLimit: 900_000 } };
  assert.equal(validateEffectiveModelContexts([model], undefined, undefined, metadata), null);
  assert.equal(validateEffectiveModelContexts([model], 2_760_000, 2_750_000, metadata), null);
  assert.match(
    validateEffectiveModelContexts([{ ...model, context_window: 1_050_000 }],
      2_760_000, 2_750_000, metadata) ?? '', /model-x.*2750000.*1050000/,
  );
});
