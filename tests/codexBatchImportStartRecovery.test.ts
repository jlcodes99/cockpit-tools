import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { recoverCodexBatchImportStartFromPreview } from "../src/utils/codexBatchImportQueue.ts";

describe("codex batch import start recovery", () => {
  const readyPreview = {
    sessionId: "session-fast",
    status: "ready",
    checkQuota: false,
    total: 3,
    items: [
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
    ],
  };

  it("recovers a terminal preview that completed before the start response", () => {
    const recovery = recoverCodexBatchImportStartFromPreview(
      "session-fast",
      "session-fast",
      readyPreview,
      ["manual"],
    );

    assert.deepEqual(recovery, {
      preview: readyPreview,
      selectedIds: ["manual", "ready-default", "existing-default"],
    });
  });

  it("recovers a cancelled terminal preview", () => {
    const recovery = recoverCodexBatchImportStartFromPreview(
      "session-fast",
      "session-fast",
      { ...readyPreview, status: "cancelled" },
      [],
    );

    assert.equal(recovery?.preview.status, "cancelled");
    assert.deepEqual(recovery?.selectedIds, [
      "ready-default",
      "existing-default",
    ]);
  });

  it("does not clear the busy state from a running or stale session preview", () => {
    assert.equal(
      recoverCodexBatchImportStartFromPreview(
        "session-fast",
        "session-fast",
        { ...readyPreview, status: "running" },
        [],
      ),
      null,
    );
    assert.equal(
      recoverCodexBatchImportStartFromPreview(
        "replacement-session",
        "session-fast",
        readyPreview,
        [],
      ),
      null,
    );
    assert.equal(
      recoverCodexBatchImportStartFromPreview(
        "session-fast",
        "session-fast",
        { ...readyPreview, sessionId: "unexpected-session" },
        [],
      ),
      null,
    );
  });
});
