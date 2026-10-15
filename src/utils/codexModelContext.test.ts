import assert from 'node:assert/strict';
import test from 'node:test';
import {
  deriveAutoCompactTokenLimit,
  deriveAutoCompactTokenLimitInput,
  resolveStoredCompactLimitInput,
  validateModelContext,
} from './codexModelContext';

test('model context follows metadata unless both overrides are explicitly configured', () => {
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

test('model context rejects missing, non-integer, unsafe and out-of-range values', () => {
  for (const model of [
    { context_window: 100 }, { auto_compact_token_limit: 50 },
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
