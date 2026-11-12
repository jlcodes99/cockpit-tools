import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  buildCodexBatchImportApiServiceAccountIds,
  findNextCodexBatchImportTaskId,
  getCodexBatchImportProgressTone,
  getCodexBatchImportProgressPercent,
  matchesCodexBatchImportQuota,
  mergeCodexBatchImportDefaultSelection,
  recoverCodexBatchImportStartedTaskFromPreview,
  type CodexBatchImportQueueTaskLike,
} from "../src/utils/codexBatchImportQueue.ts";

describe("codex batch import queue helpers", () => {
  it("filters by the lowest confirmed quota and keeps accounts with unknown quotas", () => {
    const accounts = [
      { id: "zero", quotaWindows: [{ percentage: 80 }, { percentage: 0 }] },
      { id: "low", quotaWindows: [{ percentage: 2 }] },
      { id: "boundary", quotaWindows: [{ percentage: 75 }, { percentage: 20 }] },
      { id: "unknown" },
    ];
    const idsAtMinimum = (minimum: number) => accounts
      .filter((item) => matchesCodexBatchImportQuota(item, minimum))
      .map((item) => item.id);
    assert.deepEqual(idsAtMinimum(0), ["zero", "low", "boundary", "unknown"]);
    assert.deepEqual(idsAtMinimum(1), ["low", "boundary", "unknown"]);
    assert.deepEqual(idsAtMinimum(20), ["boundary", "unknown"]);
  });

  it("does not start a queued task while another task is running", () => {
    const tasks: CodexBatchImportQueueTaskLike[] = [
      { id: "first", status: "running" },
      { id: "second", status: "queued" },
    ];

    assert.equal(findNextCodexBatchImportTaskId(tasks), null);
  });

  it("does not start another task while accounts are being written", () => {
    const tasks: CodexBatchImportQueueTaskLike[] = [
      { id: "importing", status: "importing" },
      { id: "queued", status: "queued" },
    ];

    assert.equal(findNextCodexBatchImportTaskId(tasks), null);
  });

  it("starts the first queued task when no task is active", () => {
    const tasks: CodexBatchImportQueueTaskLike[] = [
      { id: "done", status: "ready" },
      { id: "second", status: "queued" },
      { id: "third", status: "queued" },
    ];

    assert.equal(findNextCodexBatchImportTaskId(tasks), "second");
  });

  it("keeps manual selection while adding newly default-selected import items", () => {
    const selected = mergeCodexBatchImportDefaultSelection(["manual"], [
      {
        itemId: "ready-default",
        defaultSelected: true,
        selectable: true,
        status: "ready",
      },
      {
        itemId: "existing-default",
        defaultSelected: true,
        selectable: true,
        status: "existing",
      },
      {
        itemId: "invalid-default",
        defaultSelected: true,
        selectable: false,
        status: "invalid",
      },
    ]);

    assert.deepEqual(selected.sort(), [
      "existing-default",
      "manual",
      "ready-default",
    ]);
  });

  it("calculates stable progress from running progress or completed preview", () => {
    assert.equal(
      getCodexBatchImportProgressPercent({
        id: "running",
        status: "running",
        progress: { current: 2, total: 5 },
      }),
      40,
    );
    assert.equal(
      getCodexBatchImportProgressPercent({
        id: "ready",
        status: "ready",
        preview: { items: [{ itemId: "a" }, { itemId: "b" }], total: 2 },
      }),
      100,
    );
    assert.equal(
      getCodexBatchImportProgressPercent({
        id: "queued",
        status: "queued",
      }),
      0,
    );
  });

  it("uses a success tone for ready and imported completed progress", () => {
    assert.equal(
      getCodexBatchImportProgressTone({
        id: "ready",
        status: "ready",
        preview: { items: [{ itemId: "a" }], total: 1 },
      }),
      "success",
    );
    assert.equal(
      getCodexBatchImportProgressTone({
        id: "imported",
        status: "imported",
        preview: { items: [{ itemId: "a" }], total: 1 },
      }),
      "success",
    );
    assert.equal(
      getCodexBatchImportProgressTone({
        id: "running",
        status: "running",
        progress: { current: 1, total: 2 },
      }),
      "active",
    );
  });

  it("merges selected preview accounts and imported accounts into API service ids", () => {
    const accountIds = buildCodexBatchImportApiServiceAccountIds(
      ["existing-api", "selected-existing"],
      ["ready-new", "selected-existing", "invalid"],
      [
        {
          itemId: "ready-new",
          accountId: null,
        },
        {
          itemId: "selected-existing",
          accountId: "selected-existing",
        },
        {
          itemId: "invalid",
          accountId: null,
        },
      ],
      [{ id: "imported-ready" }],
    );

    assert.deepEqual(accountIds, [
      "existing-api",
      "selected-existing",
      "imported-ready",
    ]);
  });

  it("recovers a fast completed preview after the start response sets session id", () => {
    const task = recoverCodexBatchImportStartedTaskFromPreview(
      {
        id: "task",
        status: "running",
        sessionId: null,
        selectedIds: [],
      },
      "session-fast",
      {
        sessionId: "session-fast",
        status: "ready",
        checkQuota: false,
        total: 1,
        items: [
          {
            itemId: "ready-account",
            defaultSelected: true,
            selectable: true,
            status: "ready",
          },
        ],
      },
    );

    assert.equal(task.sessionId, "session-fast");
    assert.equal(task.status, "ready");
    assert.deepEqual(task.selectedIds, ["ready-account"]);
  });
});
