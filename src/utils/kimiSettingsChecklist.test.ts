import assert from "node:assert/strict";
import test from "node:test";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import {
  CURRENT_ACCOUNT_REFRESH_PLATFORMS,
  buildDefaultCurrentAccountRefreshMinutesMap,
} from "./currentAccountRefresh.ts";

test("kimi is a current-account refresh platform", () => {
  assert.ok(CURRENT_ACCOUNT_REFRESH_PLATFORMS.includes("kimi"));
  const map = buildDefaultCurrentAccountRefreshMinutesMap();
  assert.equal(typeof map.kimi, "number");
  assert.ok(map.kimi > 0);
});

test("kimi launch-on-switch copy exists in all locale files", () => {
  const root = join(dirname(fileURLToPath(import.meta.url)), "..", "locales");
  const files = readdirSync(root).filter((file) => file.endsWith(".json"));
  assert.ok(files.length >= 18, `expected 18 locale files, got ${files.length}`);
  for (const file of files) {
    const locales = JSON.parse(readFileSync(join(root, file), "utf8")) as {
      quickSettings?: {
        kimi?: { launchOnSwitch?: string; launchOnSwitchDesc?: string };
        kimiRefreshInterval?: string;
      };
    };
    assert.ok(
      locales.quickSettings?.kimi?.launchOnSwitch,
      `${file} missing quickSettings.kimi.launchOnSwitch`,
    );
    assert.ok(
      locales.quickSettings?.kimi?.launchOnSwitchDesc,
      `${file} missing quickSettings.kimi.launchOnSwitchDesc`,
    );
    assert.ok(
      locales.quickSettings?.kimiRefreshInterval,
      `${file} missing quickSettings.kimiRefreshInterval`,
    );
  }
});
