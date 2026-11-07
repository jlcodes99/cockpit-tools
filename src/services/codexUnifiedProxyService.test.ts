import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import * as enginePrerequisite from '../utils/codexProxyEnginePrerequisite';

const compiled = ts.transpileModule(readFileSync(new URL('./codexUnifiedProxyService.ts', import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;

function harness(invoke: (command: string, args?: unknown) => Promise<unknown>) {
  const exports: Record<string, any> = {};
  const calls: Array<{ command: string; args?: unknown }> = [];
  vm.runInNewContext(compiled, {
    exports,
    require: (name: string) => name.endsWith('codexProxyEnginePrerequisite') ? enginePrerequisite : ({ invoke: (command: string, args?: unknown) => { calls.push({ command, args }); return invoke(command, args); } }),
  });
  /** Arguments are built inside the VM realm, so compare a plain copy instead of its prototype. */
  const call = (index: number) => JSON.parse(JSON.stringify(calls[index] ?? null));
  return { service: exports, calls, call };
}

test('backend codes map to copy keys and raw messages never leak', () => {
  const { service } = harness(async () => ({}));
  assert.equal(service.unifiedProxyErrorKey('UNIFIED_PROXY_BUSY'), 'codex.proxy.unified.errorBusy');
  assert.equal(service.unifiedProxyErrorKey('UNIFIED_PROXY_TIMEOUT'), 'codex.proxy.unified.errorTimeout');
  assert.equal(service.unifiedProxyErrorKey(new Error('CATALOG_TIMEOUT')), 'codex.proxy.unified.errorTimeout');
  assert.equal(service.unifiedProxyErrorKey('UNIFIED_PROXY_STALE'), 'codex.proxy.unified.errorStale');
  assert.equal(service.unifiedProxyErrorKey('CATALOG_NOT_FOUND'), 'codex.proxy.unified.errorStale');
  assert.equal(service.unifiedProxyErrorKey('CATALOG_CHANGED'), 'codex.proxy.unified.errorStale');
  assert.equal(service.unifiedProxyErrorKey('UNIFIED_PROXY_STORAGE'), 'codex.proxy.unified.errorRead');
  assert.equal(service.unifiedProxyErrorKey('failed to write /Users/someone/config.json'), 'codex.proxy.unified.errorFailed');
});

test('selection arguments stay camelCase and drop an empty group context', async () => {
  const { service, call } = harness(async () => ({ mode: 'off' }));
  await service.getCodexUnifiedProxy();
  await service.applyCodexUnifiedProxy('source', 'node', { choose: 'Alpha' });
  await service.previewCodexUnifiedProxy('source', 'node', {}, '');
  await service.disableCodexUnifiedProxy();
  assert.deepEqual(call(0), { command: 'codex_unified_proxy_get' });
  assert.deepEqual(call(1), { command: 'codex_unified_proxy_apply', args: { sourceId: 'source', itemId: 'node', selections: { choose: 'Alpha' } } });
  assert.deepEqual(call(2), { command: 'codex_unified_proxy_preview', args: { sourceId: 'source', itemId: 'node', selections: {} } });
  assert.deepEqual(call(3), { command: 'codex_unified_proxy_disable' });
});

test('a group context is forwarded when the user picked a group', async () => {
  const { service, call } = harness(async () => ({ mode: 'off' }));
  await service.previewCodexUnifiedProxy('source', 'group', {}, 'group');
  assert.deepEqual(call(0), { command: 'codex_unified_proxy_preview', args: { sourceId: 'source', itemId: 'group', selections: {}, groupId: 'group' } });
});
