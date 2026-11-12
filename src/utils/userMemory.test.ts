import assert from 'node:assert/strict';
import test from 'node:test';
import { deferred, loadHookModule, settlePromises } from '../../tests/helpers/reactHookHarness';
import type { UserMemorySnapshot } from './userMemory';

const listId = 'codex.accounts.custom_sort';
const legacyKey = 'agtools.codex.accounts.custom_sort_order.v1';

test('a sort edit made during hydration wins and never writes back to WebView', async () => {
  const load = deferred<UserMemorySnapshot>();
  const legacy = new Map([[legacyKey, '["old","order"]']]);
  const writes: string[][] = [];
  let loads = 0;
  const { exports: memory } = loadHookModule(new URL('./userMemory.ts', import.meta.url), {
    '@tauri-apps/api/core': { invoke: async (command: string, args: { legacy?: UserMemorySnapshot; items?: string[] }) => {
      if (command === 'load_user_memory') {
        loads++;
        assert.equal(JSON.stringify(args.legacy?.lists[listId]), '["old","order"]');
        return load.promise;
      }
      assert.equal(command, 'save_user_memory_list');
      writes.push(Array.from(args.items ?? []));
    } },
  }, { localStorage: {
    getItem: (key: string) => legacy.get(key) ?? null,
    setItem: (key: string, value: string) => legacy.set(key, value),
  } });

  const pending = memory.hydrateUserMemory();
  memory.persistUserMemoryList(listId, ['order', 'old']);
  assert.equal(JSON.stringify(memory.readUserMemoryList(listId)), '["order","old"]');
  assert.equal(writes.length, 0);
  load.resolve({ dismissed: {}, lists: { [listId]: [] } });
  await pending;
  await settlePromises();
  assert.equal(loads, 1);
  assert.deepEqual(writes, [['order', 'old']]);
  assert.equal(JSON.stringify(memory.readUserMemoryList(listId)), '["order","old"]');
  assert.equal(legacy.get(legacyKey), '["old","order"]');
});

test('failed native reads preserve legacy data for retry and an existing empty list stays empty', async () => {
  let attempts = 0;
  const { exports: memory } = loadHookModule(new URL('./userMemory.ts', import.meta.url), {
    '@tauri-apps/api/core': { invoke: async (command: string) => {
      assert.equal(command, 'load_user_memory');
      if (++attempts === 1) throw new Error('unreadable');
      return { dismissed: {}, lists: { [listId]: [] } };
    } },
  }, { localStorage: { getItem: (key: string) => key === legacyKey ? '["old"]' : null } });

  await assert.rejects(memory.hydrateUserMemory(), /unreadable/);
  assert.equal(JSON.stringify(memory.readUserMemoryList(listId)), '["old"]');
  await memory.hydrateUserMemory();
  assert.equal(JSON.stringify(memory.readUserMemoryList(listId)), '[]');
});
