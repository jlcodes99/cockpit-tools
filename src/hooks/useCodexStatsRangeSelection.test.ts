import assert from "node:assert/strict";
import test from "node:test";
import { loadHookModule } from "../../tests/helpers/reactHookHarness";

test("calendar ranges follow successive midnights and resume while custom dates stay fixed", () => {
  const midnight = (day: number) => new Date(2026, 5, day).getTime();
  let now = midnight(7) - 2_000;
  class Clock extends Date {
    constructor(value: number = now) { super(value); }
    static now() { return now; }
  }
  let sequence = 0;
  const timers = new Map<number, { callback: () => void; due: number }>();
  const window = Object.assign(new EventTarget(), {
    setTimeout(callback: () => void, delay: number) {
      timers.set(++sequence, { callback, due: now + delay });
      return sequence;
    },
    clearTimeout(id: number) { timers.delete(id); },
  });
  const document = Object.assign(new EventTarget(), { visibilityState: "visible" });
  const h = loadHookModule(new URL("./useCodexStatsRangeSelection.ts", import.meta.url), {},
    { Date: Clock, window, document });
  let preference = "daily";
  let active = true;
  function advance(time: number) {
    now = time;
    for (const [id, timer] of [...timers]) {
      if (timer.due <= time) {
        timers.delete(id);
        timer.callback();
      }
    }
    return h.flush();
  }
  const read = () => h.exports.useCodexStatsRangeSelection(preference, active);
  assert.equal(h.render(read).range.startInput, "2026-06-06");
  assert.equal(advance(midnight(7)).range.startInput, "2026-06-07");
  assert.equal(advance(midnight(8)).range.startInput, "2026-06-08");
  now = midnight(9) + 1_000;
  document.dispatchEvent(new Event("visibilitychange"));
  assert.equal(h.flush().range.startInput, "2026-06-09");
  assert.equal(timers.size, 1);

  preference = '{"key":"custom","startInput":"2026-06-02","endInput":"2026-06-03"}';
  assert.equal(h.flush().range.startInput, "2026-06-02");
  assert.equal(timers.size, 0);
  assert.equal(advance(midnight(11)).range.endInput, "2026-06-03");
  preference = "daily";
  assert.equal(h.flush().range.startInput, "2026-06-11");
  active = false;
  h.flush();
  assert.equal(timers.size, 0);
  now = midnight(13);
  active = true;
  assert.equal(h.flush().range.startInput, "2026-06-13");
  h.unmount();
  assert.equal(timers.size, 0);
});
