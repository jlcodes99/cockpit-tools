import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import vm from 'node:vm';
import ts from 'typescript';

const compiled = ts.transpileModule(
  readFileSync(new URL('./scheduledBackupService.ts', import.meta.url), 'utf8'),
  { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } },
).outputText;

function harness(invoke: (command: string) => Promise<unknown>) {
  const exports: Record<string, any> = {};
  const timers = new Map<number, () => void>();
  let timerId = 0;
  vm.runInNewContext(compiled, {
    exports, require: () => ({ invoke }),
    setTimeout: (callback: () => void) => { timers.set(++timerId, callback); return timerId; },
    clearTimeout: (id: number) => timers.delete(id),
  });
  exports.timers = timers;
  return exports;
}

test('backup scan failures preserve loaded settings and other successful data', async () => {
  const settings = { directory_path: '/backups', retention_days: 7 };
  const usage = { total_file_count: 2 };
  const service = harness(async (command) => {
    if (command === 'get_auto_backup_settings') return settings;
    if (command === 'get_backup_usage') return usage;
    throw new Error('backup folder is unreadable');
  });
  const result = await service.getAutoBackupState();
  assert.equal(result.settings.value, settings);
  assert.equal(result.usage.value, usage);
  assert.equal(result.files.status, 'rejected');
  assert.match(result.files.reason.message, /unreadable/);
});

test('a stalled scan times out, retains settings and stays single-flight on retry', async () => {
  const settings = { directory_path: '/backups', retention_days: 7 };
  let scanCount = 0;
  const service = harness(async (command) => {
    if (command === 'get_auto_backup_settings') return settings;
    if (command === 'get_backup_usage') return { total_file_count: 0 };
    scanCount++;
    return new Promise(() => {});
  });
  const pending = service.getAutoBackupState();
  await new Promise((resolve) => setImmediate(resolve));
  for (const timer of service.timers.values()) timer();
  const result = await pending;
  assert.equal(result.settings.value, settings);
  assert.match(result.files.reason.message, /backup_load_timeout/);
  const retry = service.getAutoBackupState();
  await new Promise((resolve) => setImmediate(resolve));
  for (const timer of service.timers.values()) timer();
  await retry;
  assert.equal(scanCount, 1);
});

test('unavailable data root remains an explicit error without inventing empty backups', async () => {
  const service = harness(async () => { throw new Error('data root unavailable'); });
  const result = await service.getAutoBackupState();
  for (const state of Object.values(result) as PromiseRejectedResult[]) {
    assert.equal(state.status, 'rejected');
    assert.match(state.reason.message, /data root unavailable/);
  }
});

test('changing directory starts a fresh scan instead of reusing the old pending directory', async () => {
  let directory = '/old-backups';
  let scans = 0;
  let finishOldScan!: (files: unknown[]) => void;
  const service = harness(async (command) => {
    if (command === 'change_backup_directory') { directory = '/new-backups'; return {}; }
    if (command === 'get_auto_backup_settings') return { directory_path: directory };
    if (command === 'get_backup_usage') return { total_file_count: 0 };
    scans++;
    if (scans === 1) return new Promise((resolve) => { finishOldScan = resolve; });
    return [{ file_name: 'new-file.json' }];
  });
  const oldLoad = service.getAutoBackupState();
  await new Promise((resolve) => setImmediate(resolve));
  await service.changeBackupDirectory('/new-backups', false, 'switch');
  const latest = await service.getAutoBackupState();
  assert.equal(latest.settings.value.directory_path, '/new-backups');
  assert.equal(latest.files.value[0].file_name, 'new-file.json');
  assert.equal(scans, 2);
  finishOldScan([{ file_name: 'old-file.json' }]);
  await oldLoad;
});
