import assert from "node:assert/strict";
import test from "node:test";

import {
  formatSessionUsageCostUsd,
  formatSessionUsageTokensShort,
  hasTrustedSessionUsageCache,
  resolveSessionUsageSummaryStatus,
} from "./codexSessionUsageFormat.ts";

test("uses 万/亿 for Chinese and Japanese", () => {
  assert.equal(formatSessionUsageTokensShort(0, "zh-CN"), "0");
  assert.equal(formatSessionUsageTokensShort(9999, "zh-CN"), "9,999");
  assert.equal(formatSessionUsageTokensShort(12_345, "zh-CN"), "1.2 万");
  assert.equal(formatSessionUsageTokensShort(123_456_789, "zh-CN"), "1.23 亿");
  assert.equal(formatSessionUsageTokensShort(12_345, "ja"), "1.2 万");
  assert.equal(formatSessionUsageTokensShort(12_345, "zh-TW"), "1.2 萬");
});

test("uses K/M/B for English", () => {
  assert.equal(formatSessionUsageTokensShort(999, "en"), "999");
  assert.equal(formatSessionUsageTokensShort(1_250, "en"), "1.3K");
  assert.equal(formatSessionUsageTokensShort(1_250_000, "en"), "1.25M");
  assert.equal(formatSessionUsageTokensShort(2_000_000_000, "en"), "2.00B");
});

test("formats session usage cost with four decimals", () => {
  assert.equal(formatSessionUsageCostUsd(0), "$0.0000");
  assert.equal(formatSessionUsageCostUsd(1.52), "$1.5200");
});

test("treats lastSyncedAt as the only trusted cache signal", () => {
  assert.equal(hasTrustedSessionUsageCache(null), false);
  assert.equal(hasTrustedSessionUsageCache({}), false);
  assert.equal(hasTrustedSessionUsageCache({ lastSyncedAt: 0 }), false);
  assert.equal(hasTrustedSessionUsageCache({ lastSyncedAt: 1_800_000_000 }), true);
});

test("shows scanning before the first sync and updating only with cache", () => {
  assert.equal(resolveSessionUsageSummaryStatus("scanning", false), "scanning");
  assert.equal(resolveSessionUsageSummaryStatus("updating", true), "updating");
  assert.equal(resolveSessionUsageSummaryStatus("ready", true), null);
  assert.equal(resolveSessionUsageSummaryStatus("failed", false), null);
  assert.equal(resolveSessionUsageSummaryStatus("scanning", true), null);
});
