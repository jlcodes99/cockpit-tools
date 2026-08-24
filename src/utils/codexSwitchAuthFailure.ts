import type { CodexAccount } from "../types/codex";

const SWITCH_AUTH_REQUIRED_PREFIX = "CODEX_SWITCH_AUTH_REQUIRED:";
const ACCESS_TOKEN_SAFETY_WINDOW_SECONDS = 5 * 60;

export interface CodexSwitchAuthFailure {
  accountId: string;
  reasonCode: string;
  apiOnlyAvailable: boolean;
  accessTokenExpiresAt: number | null;
  message: string;
}

export class CodexSwitchAccountError extends Error {
  readonly authFailure: CodexSwitchAuthFailure | null;

  constructor(message: string, authFailure: CodexSwitchAuthFailure | null) {
    super(message);
    this.name = "CodexSwitchAccountError";
    this.authFailure = authFailure;
  }
}

function normalizeOptionalTimestamp(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

export function parseCodexSwitchAuthFailure(
  error: unknown,
): CodexSwitchAuthFailure | null {
  const raw = String(error ?? "");
  const markerIndex = raw.indexOf(SWITCH_AUTH_REQUIRED_PREFIX);
  if (markerIndex < 0) return null;

  const payloadText = raw
    .slice(markerIndex + SWITCH_AUTH_REQUIRED_PREFIX.length)
    .trim();
  try {
    const payload = JSON.parse(payloadText) as Record<string, unknown>;
    const accountId =
      typeof payload.accountId === "string" ? payload.accountId.trim() : "";
    const message =
      typeof payload.message === "string" ? payload.message.trim() : "";
    if (!accountId || !message) return null;
    return {
      accountId,
      reasonCode:
        typeof payload.reasonCode === "string" && payload.reasonCode.trim()
          ? payload.reasonCode.trim()
          : "authorization_required",
      apiOnlyAvailable: payload.apiOnlyAvailable === true,
      accessTokenExpiresAt: normalizeOptionalTimestamp(
        payload.accessTokenExpiresAt,
      ),
      message,
    };
  } catch {
    return null;
  }
}

export function normalizeCodexSwitchError(error: unknown): CodexSwitchAccountError {
  const authFailure = parseCodexSwitchAuthFailure(error);
  const message = authFailure?.message || String(error).replace(/^Error:\s*/, "");
  return new CodexSwitchAccountError(message, authFailure);
}

function decodeJwtExpiration(token: string): number | null {
  const parts = token.trim().split(".");
  if (parts.length !== 3) return null;
  try {
    const normalized = parts[1].replace(/-/g, "+").replace(/_/g, "/");
    const padded = normalized.padEnd(Math.ceil(normalized.length / 4) * 4, "=");
    const payload = JSON.parse(atob(padded)) as { exp?: unknown };
    return typeof payload.exp === "number" && Number.isFinite(payload.exp)
      ? payload.exp
      : null;
  } catch {
    return null;
  }
}

/** 与后端 API 服务的 5 分钟安全窗口保持一致。 */
export function isCodexApiOnlyAccessTokenUsable(
  account?: CodexAccount | null,
  nowSeconds = Math.floor(Date.now() / 1000),
): boolean {
  const token = account?.tokens?.access_token?.trim() || "";
  if (!token) return false;
  if (token.startsWith("at-")) return true;
  const expiresAt = decodeJwtExpiration(token);
  return (
    expiresAt !== null &&
    expiresAt >= nowSeconds + ACCESS_TOKEN_SAFETY_WINDOW_SECONDS
  );
}

export function isCodexRefreshTokenReauthReason(reason?: string | null): boolean {
  const normalized = reason?.trim().toLowerCase() || "";
  if (!normalized) return false;
  return (
    normalized.includes("refresh_token_reused") ||
    normalized.includes("refresh_token_expired") ||
    normalized.includes("refresh_token_invalidated") ||
    normalized.includes("invalid_grant") ||
    normalized.includes("invalid refresh token") ||
    normalized.includes("refresh_token 已被其它客户端或实例使用过") ||
    normalized.includes("refresh token has been reused") ||
    ((normalized.includes("status=401") ||
      normalized.includes("401 unauthorized")) &&
      (normalized.includes("refresh") || normalized.includes("刷新")))
  );
}

export function isCodexIdTokenReauthReason(reason?: string | null): boolean {
  const normalized = reason?.trim().toLowerCase() || "";
  if (!normalized.includes("id_token")) return false;
  return (
    normalized.includes("已过期") ||
    normalized.includes("无效") ||
    normalized.includes("即将过期") ||
    normalized.includes("未获得新的有效") ||
    normalized.includes("expired") ||
    normalized.includes("invalid") ||
    normalized.includes("missing")
  );
}

export function isCodexRefreshTokenNoticeOnly(
  account?: CodexAccount | null,
  nowSeconds = Math.floor(Date.now() / 1000),
): boolean {
  return Boolean(
    account?.requires_reauth &&
      isCodexRefreshTokenReauthReason(account.reauth_reason) &&
      isCodexApiOnlyAccessTokenUsable(account, nowSeconds),
  );
}

export function isCodexClientReauthNoticeOnly(
  account?: CodexAccount | null,
  nowSeconds = Math.floor(Date.now() / 1000),
): boolean {
  return Boolean(
    account?.requires_reauth &&
      (isCodexRefreshTokenReauthReason(account.reauth_reason) ||
        isCodexIdTokenReauthReason(account.reauth_reason)) &&
      isCodexApiOnlyAccessTokenUsable(account, nowSeconds),
  );
}
