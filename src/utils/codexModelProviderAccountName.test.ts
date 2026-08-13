import test from "node:test";
import assert from "node:assert/strict";
import {
  resolveCodexModelProviderAccountName,
  shouldSyncCodexModelProviderAccountName,
} from "./codexModelProviderAccountName.ts";

test("API Key name is preferred over the provider name", () => {
  assert.equal(
    resolveCodexModelProviderAccountName("Provider", "  Team A  "),
    "Team A",
  );
});

test("provider name is the fallback for an unnamed API Key", () => {
  assert.equal(resolveCodexModelProviderAccountName("Provider", "  "), "Provider");
  assert.equal(resolveCodexModelProviderAccountName("  Provider  "), "Provider");
});

test("renaming a key updates legacy provider/key names but preserves manual names", () => {
  assert.equal(
    shouldSyncCodexModelProviderAccountName("Provider", "Provider", "Old key"),
    true,
  );
  assert.equal(
    shouldSyncCodexModelProviderAccountName("Old key", "Provider", "Old key"),
    true,
  );
  assert.equal(
    shouldSyncCodexModelProviderAccountName("My account label", "Provider", "Old key"),
    false,
  );
});
