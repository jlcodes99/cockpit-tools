import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';

test('a stuck recycle-bin read times out and releases its timer', async () => {
  const read = deferred<unknown>();
  let expire: (() => void) | undefined;
  let cleared = 0;
  const h = loadHookModule(new URL('./codexRecycleBinService.ts', import.meta.url), {
    '@tauri-apps/plugin-dialog': { save() {} },
    '@tauri-apps/api/core': { invoke: () => read.promise },
  }, {
    setTimeout(callback: () => void, delay: number) { assert.equal(delay, 15_000); expire = callback; return 1; },
    clearTimeout() { cleared += 1; },
  });
  const result = h.exports.listCodexRecycledAccounts();
  const rejected = assert.rejects(result, /CODEX_RECYCLE_BIN_READ_TIMEOUT/);
  expire!(); await rejected;
  assert.equal(cleared, 1);
  read.resolve([]);
});

test('destructive commands address recycle entries and preserve the clear snapshot', async () => {
  const calls: { command: string; args: any }[] = [];
  const h = loadHookModule(new URL('./codexRecycleBinService.ts', import.meta.url), {
    '@tauri-apps/plugin-dialog': { save() {} },
    '@tauri-apps/api/core': { async invoke(command: string, args: unknown) { calls.push({ command, args }); } },
  });
  await h.exports.restoreCodexRecycledAccount('entry-A');
  await h.exports.deleteCodexRecycledAccount('entry-B');
  await h.exports.emptyCodexRecycleBin(['entry-C', 'entry-D']);
  assert.equal(calls[0].command, 'restore_codex_recycled_account');
  assert.equal(calls[0].args.recycleId, 'entry-A');
  assert.equal(calls[1].args.recycleId, 'entry-B');
  assert.equal(calls[2].command, 'empty_codex_recycle_bin');
  assert.deepEqual(calls[2].args.recycleIds, ['entry-C', 'entry-D']);
});

test('cancelled export does not decrypt or write; empty selection does not open a dialog', async () => {
  let dialogs = 0;
  const h = loadHookModule(new URL('./codexRecycleBinService.ts', import.meta.url), {
    '@tauri-apps/plugin-dialog': { async save() { dialogs += 1; return null; } },
    '@tauri-apps/api/core': { invoke() { assert.fail('cancelled export must not invoke the backend'); } },
  });
  assert.equal(await h.exports.saveCodexRecycledAccounts([]), false);
  assert.equal(dialogs, 0);
  assert.equal(await h.exports.saveCodexRecycledAccounts(['A']), false);
  assert.equal(dialogs, 1);
});

test('export awaits backend persistence and freezes selection before opening the save dialog', async () => {
  const dialog = deferred<string>();
  const write = deferred<void>();
  const calls: { command: string; args: any }[] = [];
  const h = loadHookModule(new URL('./codexRecycleBinService.ts', import.meta.url), {
    '@tauri-apps/plugin-dialog': { save() { return dialog.promise; } },
    '@tauri-apps/api/core': { invoke(command: string, args: unknown) { calls.push({ command, args }); return write.promise; } },
  });
  const selection = ['A'];
  let completed = false;
  const result = h.exports.saveCodexRecycledAccounts(selection).then((saved: boolean) => { completed = true; return saved; });
  selection.push('B');
  dialog.resolve('/tmp/accounts.json');
  await settlePromises();
  assert.equal(calls[0].command, 'export_codex_recycled_accounts');
  assert.deepEqual(Array.from(calls[0].args.recycleIds), ['A']);
  assert.equal(calls[0].args.path, '/tmp/accounts.json');
  assert.equal(completed, false);
  write.resolve();
  assert.equal(await result, true);
});

test('export errors propagate instead of reporting a saved file', async () => {
  const h = loadHookModule(new URL('./codexRecycleBinService.ts', import.meta.url), {
    '@tauri-apps/plugin-dialog': { async save() { return '/tmp/accounts.json'; } },
    '@tauri-apps/api/core': { async invoke() { throw new Error('write failed'); } },
  });
  await assert.rejects(h.exports.saveCodexRecycledAccounts(['A']), /write failed/);
});
