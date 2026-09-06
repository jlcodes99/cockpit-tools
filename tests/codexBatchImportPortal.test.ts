import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";

describe("codex batch import portal rendering", () => {
  it("renders the modal overlay through document.body so it opens outside hidden pages", () => {
    const source = readFileSync(
      `${process.cwd()}/src/pages/CodexAccountsView.tsx`,
      "utf8",
    );

    const overlayIndex = source.indexOf(
      'className="modal-overlay codex-batch-import-overlay"',
    );
    const createPortalIndex = source.lastIndexOf("createPortal(", overlayIndex);
    const documentBodyIndex = source.indexOf("document.body", overlayIndex);

    assert.notEqual(overlayIndex, -1, "batch import overlay should exist");
    assert.ok(
      createPortalIndex !== -1 &&
        documentBodyIndex !== -1 &&
        createPortalIndex < overlayIndex &&
        overlayIndex < documentBodyIndex,
      "batch import overlay should be inside a createPortal call targeting document.body",
    );
  });

  it("keeps background jobs only in the global bottom-right task stack", () => {
    const pageSource = readFileSync(
      `${process.cwd()}/src/pages/CodexAccountsPage.tsx`,
      "utf8",
    );
    const appSource = readFileSync(`${process.cwd()}/src/App.tsx`, "utf8");
    const globalTaskSource = readFileSync(
      `${process.cwd()}/src/components/CodexBatchImportGlobalTask.tsx`,
      "utf8",
    );

    assert.ok(
      appSource.includes("<CodexBatchImportGlobalTask"),
      "the global batch import task stack should stay mounted by App",
    );
    assert.equal(
      pageSource.includes("codex-batch-import-floating-panel"),
      false,
      "the duplicate top-right task panel should be removed",
    );
    assert.equal(
      pageSource.includes('className="codex-batch-import-task-list"'),
      false,
      "the duplicate in-page task list should be removed",
    );
    assert.ok(
      globalTaskSource.includes("visible.slice(0, 3)"),
      "the global task stack should show at most three collapsed jobs",
    );
    assert.ok(
      globalTaskSource.includes("codex.batchImport.viewAllTasks"),
      "the global task stack should expose the full queue",
    );
    assert.ok(
      globalTaskSource.includes("codex.batchImport.scanCompleteReview"),
      "a completed background scan should ask the user to review its result",
    );
  });

  it("consumes each reopen request once so background progress cannot reopen the dialog", () => {
    const source = readFileSync(
      `${process.cwd()}/src/pages/useCodexAccountsBaseController.tsx`,
      "utf8",
    );

    assert.ok(source.includes("handledBatchImportReopenNonceRef"));
    assert.ok(
      source.includes(
        "batchImportReopenNonce === handledBatchImportReopenNonceRef.current",
      ),
    );
    assert.ok(
      source.includes("publishBatchImportTask({"),
      "the Accounts controller should publish the active background task",
    );
    assert.ok(source.includes("consumeBatchImportReopen();"));
    assert.ok(source.includes("setBatchImportOpen(true);"));
  });
});
