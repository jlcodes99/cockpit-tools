import assert from "node:assert/strict";
import test from "node:test";
import {
  PLATFORM_LAYOUT_ICON_STORAGE_KEY,
  persistPlatformLayoutCustomIcons,
} from "./platformLayoutIconStorage.ts";

test("custom platform icons are serialized to the expected key", () => {
  const writes: Array<[string, string]> = [];
  const storage = {
    setItem(key: string, value: string) {
      writes.push([key, value]);
    },
  };
  const icons = [{ id: "one", dataUrl: "data:image/png;base64,abc" }];

  assert.equal(persistPlatformLayoutCustomIcons(storage, icons), true);
  assert.deepEqual(writes, [
    [PLATFORM_LAYOUT_ICON_STORAGE_KEY, JSON.stringify(icons)],
  ]);
});

test("quota errors do not escape custom icon persistence", () => {
  const storage = {
    setItem() {
      throw new DOMException("The quota has been exceeded.", "QuotaExceededError");
    },
  };

  assert.doesNotThrow(() => persistPlatformLayoutCustomIcons(storage, []));
  assert.equal(persistPlatformLayoutCustomIcons(storage, []), false);
});
