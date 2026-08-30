import test from "node:test";
import assert from "node:assert/strict";
import { resolveCodexHealthIssueDisplayName } from "./codexAccountDisplayName.ts";

test("prefers a manually assigned API Key account name over the generated email", () => {
  assert.equal(
    resolveCodexHealthIssueDisplayName(
      "Production Key",
      "api-key-d5008680",
      "api-key-d5008680",
      "codex-account-id",
    ),
    "Production Key",
  );
});

test("falls back to the account email when no custom name exists", () => {
  assert.equal(
    resolveCodexHealthIssueDisplayName(
      "",
      "api-key-d5008680",
      "health@example.com",
      "codex-account-id",
    ),
    "api-key-d5008680",
  );
});

test("falls back to health email and then local account ID", () => {
  assert.equal(
    resolveCodexHealthIssueDisplayName(undefined, undefined, "health@example.com", "account-id"),
    "health@example.com",
  );
  assert.equal(
    resolveCodexHealthIssueDisplayName(undefined, undefined, undefined, "account-id"),
    "account-id",
  );
});
