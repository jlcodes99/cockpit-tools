import assert from 'node:assert/strict';
import test from 'node:test';
import type { TFunction } from 'i18next';
import { pelicanError } from './pelicanUtils.ts';

const translations: Record<string, string> = {
  'pelican.errorTimeout': '请求超时',
  'pelican.errorStream': '响应失败或未完整结束',
  'pelican.errorLimit': '响应过大',
  'pelican.errorPreviewLimit': '预览过大',
  'pelican.error.accountUnavailable': '账号不可用',
  'pelican.cancelled': '已取消',
  'pelican.error.requestFailed': '请求失败',
  'pelican.noHtml': '没有 HTML',
};
const t = ((key: string) => translations[key] ?? key) as TFunction;

test('historical bare Pelican error codes remain localized', () => {
  for (const [code, key] of [
    ['PELICAN_TIMEOUT', 'pelican.errorTimeout'],
    ['PELICAN_STREAM_INCOMPLETE', 'pelican.errorStream'],
    ['PELICAN_RESPONSE_TOO_LARGE', 'pelican.errorLimit'],
    ['PELICAN_PREVIEW_LIMIT', 'pelican.errorPreviewLimit'],
    ['PELICAN_UNSUPPORTED_ACCOUNT', 'pelican.error.accountUnavailable'],
    ['PELICAN_CANCELLED', 'pelican.cancelled'],
  ]) {
    assert.equal(pelicanError(code, t), translations[key]);
  }
});

test('stream errors retain upstream code, message and incomplete reason', () => {
  for (const detail of [
    'code=insufficient_quota; message=You exceeded your current quota',
    'reason=max_output_tokens',
    'connection reset by peer',
  ]) {
    assert.equal(pelicanError(`PELICAN_STREAM_INCOMPLETE: ${detail}`, t),
      `响应失败或未完整结束: ${detail}`);
  }
});

test('multiline upstream details survive summary localization', () => {
  const detail = 'response.failed\ncode=server_error\nmessage=Upstream unavailable\nRetry later';
  assert.equal(pelicanError(`PELICAN_STREAM_INCOMPLETE: ${detail}`, t),
    `响应失败或未完整结束: ${detail}`);
  assert.equal(pelicanError('PELICAN_TIMEOUT: Read timed out', t), '请求超时: Read timed out');
});

test('nonlocal messages mentioning an internal code are left untouched', () => {
  for (const message of [
    'Upstream rejected text PELICAN_STREAM_INCOMPLETE: details',
    'PELICAN_STREAM_INCOMPLETE_OTHER: another code',
    'PELICAN_TIMEOUT was found in request input',
    '{"message":"PELICAN_STREAM_INCOMPLETE"}',
  ]) {
    assert.equal(pelicanError(message, t), message);
  }
});

test('Error objects use their message to recognize local codes', () => {
  assert.equal(pelicanError(new Error('PELICAN_STREAM_INCOMPLETE: reason=content_filter'), t),
    '响应失败或未完整结束: reason=content_filter');
  assert.equal(pelicanError(new Error('Connection refused'), t), 'Connection refused');
});

test('legacy localized error keys and their multiline details remain supported', () => {
  assert.equal(pelicanError('pelican.error.requestFailed', t), '请求失败');
  assert.equal(pelicanError('pelican.error.requestFailed: HTTP 429\nrate_limit_exceeded', t),
    '请求失败: HTTP 429\nrate_limit_exceeded');
  assert.equal(pelicanError('pelican.noHtml', t), '没有 HTML');
});

test('a code with an empty detail does not show a dangling separator', () => {
  assert.equal(pelicanError('PELICAN_STREAM_INCOMPLETE:   ', t), '响应失败或未完整结束');
});
