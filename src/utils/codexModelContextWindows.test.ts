import assert from "node:assert/strict";
import test from "node:test";

import {
  contextWindowDraftsFromRecord,
  lookupContextWindowDraft,
  parseContextWindowDrafts,
} from "./codexModelContextWindows.ts";

test("keeps positive integer drafts and drops empty values", () => {
  const parsed = parseContextWindowDrafts(
    {
      "gpt-5.4": "272000",
      "custom-flash": " 900000 ",
      "keep-default": "",
    },
    ["gpt-5.4", "custom-flash", "keep-default"],
  );

  assert.deepEqual(parsed, {
    ok: true,
    windows: {
      "gpt-5.4": 272000,
      "custom-flash": 900000,
    },
  });
});

test("rejects zero or non-integer drafts", () => {
  assert.equal(
    parseContextWindowDrafts({ "custom-flash": "0" }, ["custom-flash"]).ok,
    false,
  );
  assert.equal(
    parseContextWindowDrafts({ "custom-flash": "128000.5" }, ["custom-flash"])
      .ok,
    false,
  );
});

test("round-trips stored windows back to drafts by catalog order", () => {
  assert.deepEqual(
    contextWindowDraftsFromRecord(
      { "Custom-Flash": 900000, ignored: 1 },
      ["custom-flash", "gpt-5.4"],
    ),
    { "custom-flash": "900000" },
  );
});

test("looks up drafts by client or upstream model", () => {
  assert.equal(
    lookupContextWindowDraft(
      { "gpt-5.6-sol": "900000" },
      "custom-flash",
      "gpt-5.6-sol",
    ),
    "900000",
  );
});
