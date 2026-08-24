import assert from "node:assert/strict";
import test from "node:test";
import { conciseCodexCredentialFailure } from "./codexCredentialProgress.ts";

test("extracts the actionable OAuth failure shared by switch and instance progress", () => {
  assert.equal(
    conciseCodexCredentialFailure(
      "Codex 授权已失效。原始错误: Token 刷新失败: status=401 Unauthorized, error_code=refresh_token_reused, body_len=231",
    ),
    "Token 刷新失败: status=401 Unauthorized, error_code=refresh_token_reused, body_len=231",
  );
});
