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

const STORE_LAUNCH_ERROR = [
  "CODEX_MANAGED_STORE_LAUNCH_UNSAFE:",
  "direct_error=拒绝访问。 (os error 5); ",
  "powershell_error=PowerShell 启动 Codex 失败: status=exit code: 1, stderr=Start-Process : Access is denied。; ",
  "launch_path=C:\\Program Files\\WindowsApps\\OpenAI.Codex_26.820.7780.0_x64__2p2nqsd0c76g0\\app\\ChatGPT.exe; ",
  "launch_path_exists=true; ",
  "registered_path=C:\\Program Files\\WindowsApps\\OpenAI.Codex_26.908.4834.0_x64__2p2nqsd0c76g0\\app\\ChatGPT.exe; ",
  "path_matches_registered=false; ",
  "codex_home=C:\\Users\\me\\.antigravity_cockpit\\instances\\codex\\abc",
].join("");

test("parses the Codex Store launch-blocked error as a repairable case", () => {
  // API 服务等入口会在错误前面再包一层说明，这里按真实形态验证。
  const wrapped = `Codex API Service 已激活，但客户端启动失败: ${STORE_LAUNCH_ERROR}`;
  const parsed = parseWindowsOperationError(wrapped, {
    operation: "start_sidecar",
    platform: "Win32",
  });

  assert.equal(parsed?.code, "codex_store_launch_blocked");
  assert.equal(parsed?.operation, "start_sidecar");
  assert.equal(parsed?.retryable, true);
  assert.equal(parsed?.canElevate, false);
  assert.equal(
    parsed?.target,
    "C:\\Program Files\\WindowsApps\\OpenAI.Codex_26.820.7780.0_x64__2p2nqsd0c76g0\\app\\ChatGPT.exe",
  );
  assert.deepEqual(
    parsed?.diagnostics.map((item) => item.label),
    [
      "launch_path",
      "launch_path_exists",
      "registered_path",
      "path_matches_registered",
    ],
  );
  assert.equal(
    parsed?.diagnostics.find((item) => item.label === "path_matches_registered")
      ?.value,
    "false",
  );
});

test("does not parse the Store launch error outside Windows", () => {
  assert.equal(
    parseWindowsOperationError(STORE_LAUNCH_ERROR, { platform: "MacIntel" }),
    null,
  );
});

test("keeps the Store launch code distinct from a plain access denied error", () => {
  const parsed = parseWindowsOperationError("启动 Codex 失败: 拒绝访问。 (os error 5)", {
    operation: "launch_app",
    platform: "Windows",
  });
  assert.equal(parsed?.code, "access_denied");
  assert.deepEqual(parsed?.diagnostics, []);
});

test("reports Store identity confirmation failures without suggesting elevation", () => {
  const parsed = parseWindowsOperationError(
    "CODEX_MANAGED_STORE_LAUNCH_UNSAFE:direct_error=not attempted for registered Store packages; powershell_error=GetPackageFamilyName pid=32820: win32=15700; launch_path=C:\\WindowsApps\\Codex\\app\\ChatGPT.exe; launch_stage=confirmation; codex_home=C:\\isolated",
    { platform: "Win32", operation: "launch_app" },
  );
  assert.equal(parsed?.code, "codex_store_launch_blocked");
  assert.equal(parsed?.retryable, true);
  assert.equal(parsed?.canElevate, false);
  assert.equal(parsed?.manualActionAvailable, false);
  assert.ok(parsed?.diagnostics.some((item) => item.label === "launch_stage" && item.value === "confirmation"));
});
