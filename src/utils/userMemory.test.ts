import assert from "node:assert/strict";
import test from "node:test";
import { deferred, loadHookModule, settlePromises } from "../../tests/helpers/reactHookHarness";

import {
  isUserMemoryDismissed,
  mergeIdLists,
  mergeIdListsPreferExisting,
  USER_MEMORY_FLAGS,
} from "./userMemory.ts";

test("treats either risk-notice localStorage generation as dismissed", () => {
  const store = new Map<string, string>();
  const original = globalThis.localStorage;
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => {
        store.set(key, value);
      },
      removeItem: (key: string) => {
        store.delete(key);
      },
    },
  });
  try {
    assert.equal(isUserMemoryDismissed(USER_MEMORY_FLAGS.riskNotice), false);
    store.set("agtools.codex.local_access.risk_notice.dismissed.v1", "1");
    assert.equal(isUserMemoryDismissed(USER_MEMORY_FLAGS.riskNotice), true);
  } finally {
    Object.defineProperty(globalThis, "localStorage", {
      configurable: true,
      value: original,
    });
  }
});

function memoryHarness(diskOrder: string[]) {
  const listId = 'codex.accounts.custom_sort';
  const items = new Map<string, string>();
  let savedOrder = [...diskOrder];
  let fail = false;
  let blocker: Promise<void> | undefined;
  const memory = loadHookModule(new URL('./userMemory.ts', import.meta.url), {
    '@tauri-apps/api/core': { invoke: async (command: string, args?: { items: string[] }) => {
      if (command === 'load_user_memory') return { dismissed: {}, lists: { [listId]: savedOrder } };
      if (command === 'save_user_memory_list') {
        await blocker;
        if (fail) throw Error('disk unavailable');
        savedOrder = [...args!.items];
      }
    } },
  }, { localStorage: { getItem: (k: string) => items.get(k) ?? null, setItem: (k: string, v: string) => items.set(k, v) } }).exports;
  return { memory, listId, items, get savedOrder() { return savedOrder; },
    failWrites(value = true) { fail = value; }, blockWrites(promise: Promise<void>) { blocker = promise; } };
}

test('restored order waits for disk and notifies subscribers after replacing cached order', async () => {
  const h = memoryHarness(['a', 'b']);
  await h.memory.hydrateUserMemory();
  const notifications: string[][] = [];
  h.memory.subscribeUserMemory(() => notifications.push([...h.memory.readUserMemoryList(h.listId)]));
  const blocked = deferred<void>(); h.blockWrites(blocked.promise);
  const restore = h.memory.restoreUserMemoryList(h.listId, [' b ', 'a', 'b']);
  await settlePromises();
  assert.deepEqual(notifications, []);
  assert.deepEqual([...h.memory.readUserMemoryList(h.listId)], ['a', 'b']);
  blocked.resolve(); await restore;
  assert.deepEqual(h.savedOrder, ['b', 'a']);
  assert.deepEqual(notifications, [['b', 'a']]);
});

test('disk failure preserves current cache and empty restore does not erase existing memory', async () => {
  const h = memoryHarness(['a', 'b']);
  await h.memory.hydrateUserMemory(); h.failWrites();
  await assert.rejects(h.memory.restoreUserMemoryList(h.listId, ['b', 'a']), /disk unavailable/);
  assert.deepEqual([...h.memory.readUserMemoryList(h.listId)], ['a', 'b']);
  await h.memory.restoreUserMemoryList(h.listId, []);
  assert.deepEqual([...h.memory.readUserMemoryList(h.listId)], ['a', 'b']);
  h.failWrites(false);
  await h.memory.restoreUserMemoryList(h.listId, ['b', 'a']);
  assert.deepEqual(h.savedOrder, ['b', 'a']);
});

test('an older in-flight save cannot overwrite restored order', async () => {
  const h = memoryHarness(['a', 'b']);
  await h.memory.hydrateUserMemory();
  const blocked = deferred<void>(); h.blockWrites(blocked.promise);
  h.memory.persistUserMemoryList(h.listId, ['a', 'b', 'c']);
  await settlePromises();
  // Later writes can finish immediately; the older write is still blocked.
  h.blockWrites(Promise.resolve());
  let restored = false;
  const restore = h.memory.restoreUserMemoryList(h.listId, ['c', 'b', 'a']);
  void restore.then(() => { restored = true; });
  await settlePromises();
  assert.equal(restored, false);
  blocked.resolve(); await restore;
  assert.deepEqual(h.savedOrder, ['c', 'b', 'a']);
});

test("keeps disk custom-sort order and only appends unseen ids", () => {
  assert.deepEqual(mergeIdLists(["c", "a", "b"], ["a", "b", "c", "d"]), [
    "c",
    "a",
    "b",
    "d",
  ]);
  const diskOrder = ["c", "a", "b"];
  assert.equal(
    mergeIdListsPreferExisting(diskOrder, ["c", "a", "b"]),
    diskOrder,
  );
  assert.deepEqual(
    mergeIdListsPreferExisting(["c", "a", "b"], ["a", "b", "c", "d"]),
    ["c", "a", "b", "d"],
  );
  assert.deepEqual(mergeIdListsPreferExisting([], ["c", "a"]), ["c", "a"]);
});
