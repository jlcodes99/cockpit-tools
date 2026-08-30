import assert from "node:assert/strict";
import test from "node:test";

import {
  extractCodexQuotaErrorStatusCode,
  isBlockingCodexQuotaError,
  isVerboseCodexQuotaErrorMessage,
  summarizeCodexQuotaErrorMessage,
} from "./codexQuotaError.ts";

const htmlDump =
  'API 返回错误 403 Forbidden [body_len:6632] [request-id:] [body:<html><head><meta name="viewport" content="width=device-width"></head><body>blocked</body></html>]';

test("extracts status code from structured API error prefix", () => {
  assert.equal(extractCodexQuotaErrorStatusCode(htmlDump), "403");
});

test("detects verbose HTML dumps", () => {
  assert.equal(isVerboseCodexQuotaErrorMessage(htmlDump), true);
  assert.equal(isVerboseCodexQuotaErrorMessage("token expired"), false);
});

test("summarizes long HTML dumps without keeping markup", () => {
  const summary = summarizeCodexQuotaErrorMessage(htmlDump);
  assert.ok(summary.length < htmlDump.length);
  assert.equal(summary.toLowerCase().includes("<html"), false);
  assert.ok(summary.includes("403"));
});

test("does not classify refresh token reuse as a blocking quota error", () => {
  assert.equal(
    isBlockingCodexQuotaError({
      code: "refresh_token_reused",
      message:
        "Token 刷新失败: status=401 Unauthorized, error_code=refresh_token_reused",
      timestamp: 1,
    }),
    false,
  );
});
