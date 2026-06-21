import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  buildCodexBatchImportApiServiceAccountIds,
  findNextCodexBatchImportTaskId,
  getCodexBatchImportProgressTone,
  getCodexBatchImportProgressPercent,
  mergeCodexBatchImportDefaultSelection,
  recoverCodexBatchImportStartedTaskFromPreview,
  type CodexBatchImportQueueTaskLike,
} from "../src/utils/codexBatchImportQueue";

describe("codex batch import queue helpers", () => {
  it("does not start a queued task while another task is running", () => {
    const tasks: CodexBatchImportQueueTaskLike[] = [
      { id: "first", status: "running" },
      { id: "second", status: "queued" },
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
