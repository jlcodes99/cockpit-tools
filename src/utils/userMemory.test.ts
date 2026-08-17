import assert from "node:assert/strict";
import test from "node:test";

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
