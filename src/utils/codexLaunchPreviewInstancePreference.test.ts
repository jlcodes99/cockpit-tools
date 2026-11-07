import assert from "node:assert/strict";
import test from "node:test";

import {
  persistCodexLaunchPreviewLastInstanceId,
  readCodexLaunchPreviewLastInstanceId,
  resolveCodexLaunchPreviewLastInstanceId,
} from "./codexLaunchPreviewInstancePreference.ts";

class MemoryStorage {
  private readonly values = new Map<string, string>();

  getItem(key: string): string | null {
    return this.values.get(key) ?? null;
  }

  setItem(key: string, value: string): void {
    this.values.set(key, value);
  }
}

const AVAILABLE = ["__default__", "6666", "work"];

test("resolveCodexLaunchPreviewLastInstanceId keeps a still-available saved instance", () => {
  assert.equal(
    resolveCodexLaunchPreviewLastInstanceId("6666", AVAILABLE, "__default__"),
    "6666",
  );
});

test("resolveCodexLaunchPreviewLastInstanceId falls back when the saved instance is gone", () => {
  assert.equal(
    resolveCodexLaunchPreviewLastInstanceId("gone", AVAILABLE, "__default__"),
    "__default__",
  );
});

test("read and persist remember the last instance per card", () => {
  const storage = new MemoryStorage();

  persistCodexLaunchPreviewLastInstanceId("account-a", "6666", storage);
  persistCodexLaunchPreviewLastInstanceId("account-b", "work", storage);

  assert.equal(
    readCodexLaunchPreviewLastInstanceId("account-a", AVAILABLE, "__default__", storage),
    "6666",
  );
  assert.equal(
    readCodexLaunchPreviewLastInstanceId("account-b", AVAILABLE, "__default__", storage),
    "work",
  );
  assert.equal(
    readCodexLaunchPreviewLastInstanceId("account-c", AVAILABLE, "__default__", storage),
    "__default__",
  );
});

test("readCodexLaunchPreviewLastInstanceId ignores a deleted instance for that card", () => {
  const storage = new MemoryStorage();
  persistCodexLaunchPreviewLastInstanceId("account-a", "gone", storage);

  assert.equal(
    readCodexLaunchPreviewLastInstanceId("account-a", AVAILABLE, "__default__", storage),
    "__default__",
  );
});
