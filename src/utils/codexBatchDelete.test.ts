import assert from "node:assert/strict";
import test from "node:test";

import { resolveCodexBatchDeleteRefreshOptions } from "./codexBatchDelete.ts";

const account = (id: string) => ({ id });

test("allows an empty refresh after deleting every cached account", () => {
  assert.deepEqual(
    resolveCodexBatchDeleteRefreshOptions({
      accounts: [account("account-a")],
      currentAccount: account("account-a"),
      removeIds: new Set(["account-a"]),
    }),
    {
      allowEmptyAccounts: true,
      allowEmptyCurrent: true,
    },
  );
});

test("keeps empty-response protection when some cached accounts remain", () => {
  assert.deepEqual(
    resolveCodexBatchDeleteRefreshOptions({
      accounts: [account("account-a"), account("account-b")],
      currentAccount: account("account-b"),
      removeIds: new Set(["account-a"]),
    }),
    {
      allowEmptyAccounts: false,
      allowEmptyCurrent: false,
    },
  );
});

test("uses the captured deletion ids independently of later mutable state", () => {
  const capturedRemoveIds = new Set(["account-a"]);
  const mutableRemoveIds = new Set(capturedRemoveIds);
  mutableRemoveIds.clear();

  assert.equal(
    resolveCodexBatchDeleteRefreshOptions({
      accounts: [account("account-a")],
      currentAccount: null,
      removeIds: capturedRemoveIds,
    }).allowEmptyAccounts,
    true,
  );
});
