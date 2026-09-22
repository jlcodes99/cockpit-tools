import test from "node:test";
import assert from "node:assert/strict";
import {
  resolveCodexLocalAccessRuntimeStatus,
  resolveCodexLocalAccessStatusBadgeTone,
} from "./codexLocalAccessStatus";

const collection = (enabled: boolean) => ({ enabled });
const state = (running: boolean, internalRequired: boolean) => ({
  running,
  internalRequired,
});

test("disabled public entry with internal demand is distinct from stopped", () => {
  const status = resolveCodexLocalAccessRuntimeStatus(
    collection(false),
    state(true, true),
  );
  assert.equal(status, "internal");
  assert.equal(resolveCodexLocalAccessStatusBadgeTone(status), "warning");
  assert.equal(
    resolveCodexLocalAccessRuntimeStatus(collection(false), state(false, false)),
    "disabled",
  );
});

test("enabled service reports ordinary running/stopped states", () => {
  assert.equal(
    resolveCodexLocalAccessRuntimeStatus(collection(true), state(true, false)),
    "running",
  );
  assert.equal(
    resolveCodexLocalAccessRuntimeStatus(collection(true), state(false, false)),
    "stopped",
  );
});
