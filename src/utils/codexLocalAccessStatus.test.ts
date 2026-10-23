import test from "node:test";
import assert from "node:assert/strict";
import { resolveCodexLocalAccessRuntimeStatus } from "./codexLocalAccessStatus";

test("a disabled public entry stays disabled while a stale process snapshot says running", () => {
  assert.equal(
    resolveCodexLocalAccessRuntimeStatus({ enabled: false }, { running: true }),
    "disabled",
  );
  assert.equal(resolveCodexLocalAccessRuntimeStatus(null, { running: true }), "disabled");
});

test("an enabled entry reports stopped until its runtime confirms running", () => {
  assert.equal(resolveCodexLocalAccessRuntimeStatus({ enabled: true }, null), "stopped");
  assert.equal(
    resolveCodexLocalAccessRuntimeStatus({ enabled: true }, { running: false }),
    "stopped",
  );
  assert.equal(
    resolveCodexLocalAccessRuntimeStatus({ enabled: true }, { running: true }),
    "running",
  );
});
