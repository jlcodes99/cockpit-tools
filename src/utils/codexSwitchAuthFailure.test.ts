import assert from "node:assert/strict";
import test from "node:test";
import type { CodexAccount } from "../types/codex.ts";
import {
  isCodexApiOnlyAccessTokenUsable,
  isCodexClientReauthNoticeOnly,
  isCodexIdTokenReauthReason,
  isCodexRefreshTokenNoticeOnly,
  normalizeCodexSwitchError,
  parseCodexSwitchAuthFailure,
} from "./codexSwitchAuthFailure.ts";

function jwt(exp: number): string {
  const encode = (value: Record<string, unknown>) =>
    Buffer.from(JSON.stringify(value)).toString("base64url");
  return `${encode({ alg: "none" })}.${encode({ exp })}.signature`;
}

function accountWithAccessToken(accessToken: string): CodexAccount {
  return {
    id: "codex-test",
    email: "test@example.com",
    tokens: {
      id_token: "",
      access_token: accessToken,
      refresh_token: "",
    },
    created_at: 1,
    last_used: 1,
  };
}

test("parses structured Codex switch authorization failures", () => {
  const payload = {
    accountId: "codex-test",
    reasonCode: "refresh_token_reused",
    apiOnlyAvailable: true,
    accessTokenExpiresAt: 4_102_444_800,
    message: "refresh token needs reauthorization",
  };
  const raw = `CODEX_SWITCH_AUTH_REQUIRED:${JSON.stringify(payload)}`;

  assert.deepEqual(parseCodexSwitchAuthFailure(raw), payload);
  const normalized = normalizeCodexSwitchError(raw);
  assert.equal(normalized.message, payload.message);
  assert.deepEqual(normalized.authFailure, payload);
});

test("API-only availability follows the backend five-minute safety window", () => {
  const now = 2_000_000_000;
  assert.equal(
    isCodexApiOnlyAccessTokenUsable(accountWithAccessToken(jwt(now + 301)), now),
    true,
  );
  assert.equal(
    isCodexApiOnlyAccessTokenUsable(accountWithAccessToken(jwt(now + 299)), now),
    false,
  );
  assert.equal(
    isCodexApiOnlyAccessTokenUsable(accountWithAccessToken("at-opaque-token"), now),
    true,
  );
});

test("refresh token reuse stays a non-blocking notice while access token is usable", () => {
  const now = 2_000_000_000;
  const account = accountWithAccessToken(jwt(now + 3600));
  account.requires_reauth = true;
  account.reauth_reason =
    "Token 刷新失败: error_code=refresh_token_reused";
  account.quota_error = {
    code: "refresh_token_reused",
    message:
      "Token 刷新失败: status=401 Unauthorized, error_code=refresh_token_reused",
    timestamp: 1,
  };
  assert.equal(isCodexRefreshTokenNoticeOnly(account, now), true);

  account.tokens.access_token = jwt(now + 120);
  assert.equal(isCodexRefreshTokenNoticeOnly(account, now), false);
});

test("other refresh-token authorization failures use the same API-only state", () => {
  const now = 2_000_000_000;
  const account = accountWithAccessToken(jwt(now + 3600));
  account.requires_reauth = true;

  account.reauth_reason = "Token 刷新失败: error_code=invalid_grant";
  assert.equal(isCodexRefreshTokenNoticeOnly(account, now), true);

  account.reauth_reason =
    "Token 刷新失败: status=401 Unauthorized, body_len=231";
  assert.equal(isCodexRefreshTokenNoticeOnly(account, now), true);

  account.reauth_reason = "id_token 已过期且刷新响应未返回新 id_token";
  assert.equal(isCodexRefreshTokenNoticeOnly(account, now), false);
});

test("recognizes id_token failures that require client reauthorization", () => {
  assert.equal(
    isCodexIdTokenReauthReason(
      "Codex 客户端登录凭据中的 id_token 已过期、无效或即将过期，自动刷新后仍未获得新的有效 id_token。",
    ),
    true,
  );
  assert.equal(
    isCodexIdTokenReauthReason("access_token 刷新失败"),
    false,
  );

  const now = 2_000_000_000;
  const account = accountWithAccessToken(jwt(now + 3600));
  account.requires_reauth = true;
  account.reauth_reason =
    "Codex 客户端登录凭据中的 id_token 已过期，自动刷新后仍未获得新的有效 id_token。";
  assert.equal(isCodexClientReauthNoticeOnly(account, now), true);
  account.tokens.access_token = jwt(now + 120);
  assert.equal(isCodexClientReauthNoticeOnly(account, now), false);
});
