import assert from "node:assert/strict";
import test from "node:test";
import {
  parseTimeToMinutes,
  formatMinutesToTime,
  ensureAccountSchedules,
  getWorkbuddyAutoCheckinNextDelayMs,
  type WorkbuddyAutoCheckinConfig,
} from "./workbuddyAutoCheckinService.ts";

test("parseTimeToMinutes correctly converts HH:mm to minutes", () => {
  assert.equal(parseTimeToMinutes("06:00"), 360);
  assert.equal(parseTimeToMinutes("12:30"), 750);
  assert.equal(parseTimeToMinutes("00:00"), 0);
  assert.equal(parseTimeToMinutes("23:59"), 1439);
});

test("formatMinutesToTime correctly converts minutes to HH:mm", () => {
  assert.equal(formatMinutesToTime(360), "06:00");
  assert.equal(formatMinutesToTime(750), "12:30");
  assert.equal(formatMinutesToTime(0), "00:00");
  assert.equal(formatMinutesToTime(1439), "23:59");
});

test("ensureAccountSchedules allocates scheduled minutes within defined time range", () => {
  const config: WorkbuddyAutoCheckinConfig = {
    enabled: true,
    startTime: "06:00",
    endTime: "12:00",
  };

  const accounts = [
    { id: "acc_1", email: "user1@example.com" },
    { id: "acc_2", email: "user2@example.com" },
  ];

  const updatedConfig = ensureAccountSchedules(config, accounts);
  assert.ok(updatedConfig.accountSchedules);
  assert.equal(Object.keys(updatedConfig.accountSchedules).length, 2);

  const sch1 = updatedConfig.accountSchedules["acc_1"];
  assert.ok(sch1);
  assert.ok(sch1.scheduledMinute >= 360 && sch1.scheduledMinute <= 720);

  const sch2 = updatedConfig.accountSchedules["acc_2"];
  assert.ok(sch2);
  assert.ok(sch2.scheduledMinute >= 360 && sch2.scheduledMinute <= 720);
});

test("getWorkbuddyAutoCheckinNextDelayMs returns idle delay when disabled", () => {
  const delay = getWorkbuddyAutoCheckinNextDelayMs();
  assert.ok(delay > 0);
});
