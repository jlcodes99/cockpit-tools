import assert from "node:assert/strict";
import test from "node:test";
import { resolveCodexImportSyncApiServicePreference } from "./codexImportPreferences.ts";

test("new installations automatically add imported accounts to API service", () => {
  assert.equal(resolveCodexImportSyncApiServicePreference(null), true);
});

test("an explicit API service import preference is preserved", () => {
  assert.equal(resolveCodexImportSyncApiServicePreference("true"), true);
  assert.equal(resolveCodexImportSyncApiServicePreference("false"), false);
});

test("invalid stored preferences do not silently enable synchronization", () => {
  assert.equal(resolveCodexImportSyncApiServicePreference("invalid"), false);
});
