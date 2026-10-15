import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { proxyBatchCompletedSuccessfully } from './codexProxyBatch';

test('close only after all targets succeed without cancellation', () => {
  const success = [{ accountId: 'a', ok: true }, { accountId: 'b', ok: true }];
  assert.equal(proxyBatchCompletedSuccessfully(success, 2, false), true);
  assert.equal(proxyBatchCompletedSuccessfully(success, 2, true), false);
  assert.equal(proxyBatchCompletedSuccessfully(success.slice(0, 1), 2, false), false);
  assert.equal(proxyBatchCompletedSuccessfully([success[0], { accountId: 'b', ok: false }], 2, false), false);
  assert.equal(proxyBatchCompletedSuccessfully([], 0, false), false);
});

test('all batch dialog entries guard automatic closing with full success and cancellation', () => {
  for (const file of ['CodexProxyAssignDialog', 'CodexProxyBatchBindDialog', 'CodexProxyAccountDialog']) {
    const source = readFileSync(new URL(`../components/codex/${file}.tsx`, import.meta.url), 'utf8');
    assert.match(source, /completed = next/);
    assert.match(source, /mounted.current && proxyBatchCompletedSuccessfully\(completed, (targets|remaining).length, cancelled.current\)\) onClose\(\)/);
  }
});

test('single account closes only after save or unbind completes without error', () => {
  const source = readFileSync(new URL('../components/codex/CodexProxyAccountDialog.tsx', import.meta.url), 'utf8');
  assert.match(source, /previousBusy.current === 'save' \|\| previousBusy.current === 'unbind'/);
  assert.match(source, /!editor.busy && !editor.error && editor.account/);
  assert.match(source, /applied.current\([\s\S]*?closeAfterSave.current\(\)/);
});
