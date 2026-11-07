import assert from 'node:assert/strict';
import test from 'node:test';
import { formatKiroImportError } from './kiroImportErrors.ts';

test('batch import identifies the bad row with translated credential guidance', () => {
  const result = formatKiroImportError('KIRO_IMPORT_ITEM:3:KIRO_IMPORT_MISSING_CREDENTIALS', (key, options) =>
    options ? `${options.index}: ${options.error}` : key);
  assert.equal(result, '3: kiro.import.invalidCredentials');
});

test('import bounds and refresh contention use existing localized error surfaces', () => {
  const t = (key: string, options?: Record<string, unknown>) => `${key}:${JSON.stringify(options ?? {})}`;
  assert.equal(formatKiroImportError('KIRO_IMPORT_TOO_MANY', t), 'common.shared.import.countLimit:{"count":1000}');
  assert.equal(formatKiroImportError('KIRO_REFRESH_BUSY', t), 'common.dataQueryBusy:{}');
  assert.equal(formatKiroImportError('HTTP 401', t), 'HTTP 401');
});
