import test from "node:test";
import assert from "node:assert/strict";
import { clearRecoveredCodexGatewayError, isCodexGatewayKeyRecoveryError } from "./codexGatewayKeyRecovery";

const diagnostic = "网关无法解析入站 API Key，已保留原配置；请恢复被引用的账号后重试";

test("recovery and a recovered listener clear only the stale key diagnostic", () => {
  for (const state of [
    { running: false, recovering: true, lastError: null },
    { running: true, recovering: false, lastError: null },
  ]) {
    assert.equal(clearRecoveredCodexGatewayError(diagnostic, state), "");
    assert.equal(clearRecoveredCodexGatewayError("Error: " + diagnostic, state), "");
    assert.equal(clearRecoveredCodexGatewayError("HTTP 401", state), "HTTP 401");
  }
});

test("unresolved, stopped and unrelated failures remain visible", () => {
  assert.equal(clearRecoveredCodexGatewayError(diagnostic, {
    running: true, recovering: false, lastError: diagnostic,
  }), diagnostic);
  assert.equal(clearRecoveredCodexGatewayError(diagnostic, {
    running: false, recovering: false, lastError: null,
  }), diagnostic);
  assert.equal(isCodexGatewayKeyRecoveryError("port occupied"), false);
  assert.equal(isCodexGatewayKeyRecoveryError(undefined), false);
});
