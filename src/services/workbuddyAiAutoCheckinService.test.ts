import assert from "node:assert/strict";
import test from "node:test";
import {
  isValidTimeString,
  parseTimeToMinutes,
  formatMinutesToTime,
  normalizeWorkbuddyAiAutoCheckinCycleResult,
  DEFAULT_WORKBUDDY_AI_AUTO_CHECKIN_CONFIG,
  hasCachedWorkbuddyAiAutoCheckinConfig,
  getWorkbuddyAiAutoCheckinConfig,
  saveWorkbuddyAiAutoCheckinConfigAsync,
} from "./workbuddyAiAutoCheckinService.ts";

test("workbuddyAi isValidTimeString accepts HH:mm in 00:00..23:59 and rejects malformed values", () => {
  assert.equal(isValidTimeString("00:00"), true);
  assert.equal(isValidTimeString("06:30"), true);
  assert.equal(isValidTimeString("23:59"), true);
  assert.equal(isValidTimeString(""), false);
  assert.equal(isValidTimeString("6:00"), false);
  assert.equal(isValidTimeString("24:00"), false);
  assert.equal(isValidTimeString("12:60"), false);
  assert.equal(isValidTimeString("ab:cd"), false);
});

test("workbuddyAi parseTimeToMinutes converts HH:mm to minutes", () => {
  assert.equal(parseTimeToMinutes("06:00"), 360);
  assert.equal(parseTimeToMinutes("12:30"), 750);
  assert.equal(parseTimeToMinutes("00:00"), 0);
  assert.equal(parseTimeToMinutes("23:59"), 1439);
});

test("workbuddyAi formatMinutesToTime converts minutes to HH:mm", () => {
  assert.equal(formatMinutesToTime(360), "06:00");
  assert.equal(formatMinutesToTime(750), "12:30");
  assert.equal(formatMinutesToTime(0), "00:00");
  assert.equal(formatMinutesToTime(1439), "23:59");
});

test("workbuddyAi normalizeWorkbuddyAiAutoCheckinCycleResult preserves explicit statuses", () => {
  assert.equal(normalizeWorkbuddyAiAutoCheckinCycleResult("no_accounts"), "no_accounts");
  assert.equal(normalizeWorkbuddyAiAutoCheckinCycleResult("already_running"), "already_running");
  assert.equal(normalizeWorkbuddyAiAutoCheckinCycleResult("platform_disabled"), "platform_disabled");
  assert.equal(normalizeWorkbuddyAiAutoCheckinCycleResult("retry_scheduled"), "retry_scheduled");
  assert.equal(normalizeWorkbuddyAiAutoCheckinCycleResult("partial_failed"), "partial_failed");
  assert.equal(normalizeWorkbuddyAiAutoCheckinCycleResult("failed"), "failed");
  assert.equal(normalizeWorkbuddyAiAutoCheckinCycleResult("waiting"), "waiting");
  assert.equal(normalizeWorkbuddyAiAutoCheckinCycleResult("disabled"), "disabled");
  assert.equal(normalizeWorkbuddyAiAutoCheckinCycleResult("completed"), "completed");
});

test("workbuddyAi config cache reports unhydrated before fetch and hydrated after save", async () => {
  assert.equal(hasCachedWorkbuddyAiAutoCheckinConfig(), false);
  assert.deepEqual(getWorkbuddyAiAutoCheckinConfig(), DEFAULT_WORKBUDDY_AI_AUTO_CHECKIN_CONFIG);

  const customConfig = {
    enabled: true,
    startTime: "08:15",
    endTime: "17:45",
  };
  await saveWorkbuddyAiAutoCheckinConfigAsync(customConfig);
  assert.equal(hasCachedWorkbuddyAiAutoCheckinConfig(), true);
  assert.deepEqual(getWorkbuddyAiAutoCheckinConfig(), customConfig);
});
