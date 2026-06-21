import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { describe, it } from "node:test";

describe("codex batch import portal rendering", () => {
  it("renders the modal overlay through document.body so it opens outside hidden pages", () => {
    const source = readFileSync(
      `${process.cwd()}/src/pages/CodexAccountsPage.tsx`,
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
});
