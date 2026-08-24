import test from "node:test";
import assert from "node:assert/strict";
import {
  parseWindowsOperationError,
  redactWindowsOperationError,
} from "./windowsOperationError.ts";

test("parses structured Windows operation errors", () => {
  const error = `WINDOWS_OPERATION_ERROR:${JSON.stringify({
    code: "access_denied",
    operation: "stop_process",
    summary: "无法关闭实例进程",
    originalReason: "taskkill: Access is denied (os error 5)",
    pids: [123, 123, 456],
    retryable: true,
    canElevate: true,
    manualActionAvailable: true,
  })}`;
  const parsed = parseWindowsOperationError(error, { platform: "Win32" });
  assert.equal(parsed?.code, "access_denied");
  assert.deepEqual(parsed?.pids, [123, 456]);
  assert.equal(parsed?.canElevate, true);
});

test("recognizes raw os error 5 and extracts the WindowsApps path", () => {
  const parsed = parseWindowsOperationError(
    "启动失败 (C:\\Program Files\\WindowsApps\\OpenAI.Codex\\Codex.exe): 拒绝访问。 (os error 5)",
    { platform: "Windows", operation: "launch_app" },
  );
  assert.equal(parsed?.code, "access_denied");
  assert.equal(parsed?.target, "C:\\Program Files\\WindowsApps\\OpenAI.Codex\\Codex.exe");
});

test("does not treat ordinary business errors as Windows operation errors", () => {
  assert.equal(
    parseWindowsOperationError("refresh_token_reused", { platform: "Win32" }),
    null,
  );
  assert.equal(
    parseWindowsOperationError("Access is denied", { platform: "MacIntel" }),
    null,
  );
});

test("redacts credentials from copied diagnostics", () => {
  const value = redactWindowsOperationError(
    "access_token=secret refresh_token=rt.abcdefghijklmnopqrstuvwxyz123456 api_key=sk-secret",
  );
  assert.equal(value.includes("secret"), false);
  assert.equal(value.includes("abcdefghijklmnopqrstuvwxyz"), false);
});
