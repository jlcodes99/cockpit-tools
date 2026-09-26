import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';
import * as enginePrerequisite from '../utils/codexProxyEnginePrerequisite';

const compiled = ts.transpileModule(readFileSync(new URL('./codexProxyEngineService.ts', import.meta.url), 'utf8'), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
function harness(invoke: (command: string, args?: unknown) => Promise<unknown>) {
  const exports: Record<string, any> = {};
  const timers = new Map<number, () => void>();
  let timerId = 0;
  vm.runInNewContext(compiled, { exports, require: (name: string) => name.endsWith('codexProxyEnginePrerequisite') ? enginePrerequisite : ({ invoke }),
    setTimeout: (callback: () => void) => { timers.set(++timerId, callback); return timerId; },
    clearTimeout: (id: number) => timers.delete(id),
  });
  return { service: exports, timers };
}

test('status requests remain single-flight across timeout and resume after settling', async () => {
  let complete!: (value: unknown) => void;
  let calls = 0;
  const { service, timers } = harness(() => { calls += 1; return new Promise((resolve) => { complete = resolve; }); });
  const first = service.getCodexProxyEngineStatus();
  const second = service.getCodexProxyEngineStatus();
  assert.equal(calls, 1);
  const expired = assert.rejects(first, /ENGINE_INSTALL_TIMEOUT/);
  timers.values().next().value!();
  await expired;
  const third = service.getCodexProxyEngineStatus();
  assert.equal(calls, 1, 'a timeout must not spawn another unresolved backend call');
  complete({ phase: 'idle' });
  assert.equal((await second).phase, 'idle');
  assert.equal((await third).phase, 'idle');
  const next = service.getCodexProxyEngineStatus();
  assert.equal(calls, 2);
  complete({ phase: 'completed' });
  await next;
  await Promise.resolve();
  assert.equal(timers.size, 0);
});

test('failed status reads reject instead of claiming engine is missing', async () => {
  const { service } = harness(async () => { throw new Error('read failure'); });
  await assert.rejects(service.getCodexProxyEngineStatus(), /read failure/);
  await assert.rejects(service.getCodexProxyEngineStatus(), /read failure/);
});

test('installer sends only the selected archive or cancellation job identifier', async () => {
  const calls: { command: string; args: unknown }[] = [];
  const { service } = harness(async (command, args) => { calls.push({ command, args }); return {}; });
  await service.installCodexProxyEngine(null);
  await service.installCodexProxyEngine('/fixture/official.tar.gz');
  await service.cancelCodexProxyEngineInstall('job-1');
  assert.equal(JSON.stringify(calls), JSON.stringify([
    { command: 'codex_proxy_engine_install', args: { archivePath: null } },
    { command: 'codex_proxy_engine_install', args: { archivePath: '/fixture/official.tar.gz' } },
    { command: 'codex_proxy_engine_cancel', args: { jobId: 'job-1' } },
  ]));
});

test('errors use a safe allowlist and never expose arbitrary credential/path text', () => {
  const { service } = harness(async () => ({}));
  assert.equal(service.engineInstallErrorKey('ENGINE_INSTALL_CHECKSUM'), 'codex.proxy.engine.invalidArchive');
  assert.equal(service.engineInstallErrorKey('ENGINE_INSTALL_VERIFY'), 'codex.proxy.engine.verificationFailed');
  assert.equal(service.engineInstallErrorKey('ENGINE_INSTALL_START_TIMEOUT'), 'codex.proxy.engine.startTimeout');
  assert.equal(service.engineInstallErrorKey('ENGINE_INSTALL_START_FAILED'), 'codex.proxy.engine.startFailed');
  assert.equal(service.engineInstallErrorKey('ENGINE_INSTALL_VERSION'), 'codex.proxy.engine.versionMismatch');
  assert.equal(service.engineInstallErrorKey(new Error('ENGINE_INSTALL_TIMEOUT')), 'codex.proxy.engine.timeout');
  assert.equal(service.engineInstallErrorKey('https://user:secret@host /private/archive.zip'), 'codex.proxy.engine.failed');
  assert.equal(service.engineInstallErrorKey('ENGINE_INSTALL_DOWNLOAD: secret'), 'codex.proxy.engine.failed');
});

test('only active installation phases keep polling', () => {
  const { service } = harness(async () => ({}));
  for (const phase of ['downloading', 'importing', 'verifying', 'extracting', 'checking', 'installing']) {
    assert.equal(service.engineInstallActive({ phase }), true);
  }
  for (const phase of ['idle', 'completed', 'cancelled', 'failed']) {
    assert.equal(service.engineInstallActive({ phase }), false);
  }
  assert.equal(service.engineInstallActive(null), false);
});
