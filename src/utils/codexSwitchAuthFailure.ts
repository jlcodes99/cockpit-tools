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

export function getCodexJwtExpiration(token: string): number | null {
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

/**
 * 额度/API 请求已经得到远端鉴权拒绝时，本地 JWT exp 不能再作为“API 可用”的依据。
 * 仅匹配 API 请求错误，避免把 refresh_token 自身失效误判成 access_token 已失效。
 */
export function isCodexApiServiceAuthRejected(
  account?: CodexAccount | null,
): boolean {
  const quotaError = account?.quota_error;
  if (!quotaError) return false;
  const raw = String(quotaError.message || "").trim();
  const lower = raw.toLowerCase();
  const code = String(quotaError.code || "").trim().toLowerCase();
  const refreshFailure =
    lower.includes("刷新 token") ||
    lower.includes("token 刷新") ||
    lower.includes("refresh_token") ||
    lower.includes("refresh token");
  const apiRequestUnauthorized =
    /api 返回错误\s+(401|403)\b/i.test(raw) ||
    /(?:api|quota|usage)[^\n]{0,80}\b(401|403)\s+(?:unauthorized|forbidden)\b/i.test(
      raw,
    );
  if (apiRequestUnauthorized) return true;
  if (
    lower.includes("token_invalidated") ||
    lower.includes("invalid_token") ||
    lower.includes("your authentication token has been invalidated")
  ) {
    return !refreshFailure;
  }
  return (
    !refreshFailure &&
    (code === "token_invalidated" || code === "invalid_token")
  );
}

/** 与后端 API 服务的 5 分钟安全窗口保持一致。 */
export function isCodexApiOnlyAccessTokenUsable(
  account?: CodexAccount | null,
  nowSeconds = Math.floor(Date.now() / 1000),
): boolean {
  if (isCodexServerRevokedReauth(account)) return false;
  if (isCodexApiServiceAuthRejected(account)) return false;
  const token = account?.tokens?.access_token?.trim() || "";
  if (!token) return false;
  if (token.startsWith("at-")) return true;
  const expiresAt = getCodexJwtExpiration(token);
  return (
    expiresAt !== null &&
    expiresAt >= nowSeconds + ACCESS_TOKEN_SAFETY_WINDOW_SECONDS
  );
}

export function isCodexRefreshTokenReauthReason(reason?: string | null): boolean {
  const normalized = reason?.trim().toLowerCase() || "";
  if (!normalized) return false;
  // refresh_token_reused 是一次刷新竞争结果，不代表当前账号或 access_token
  // 已被服务端撤销；历史错误也不应重新触发账号级授权状态。
  if (isCodexRefreshTokenReusedReason(normalized)) return false;
  return (
    normalized.includes("refresh_token_expired") ||
    normalized.includes("refresh_token_invalidated") ||
    normalized.includes("invalid_grant") ||
    normalized.includes("invalid refresh token") ||
    ((normalized.includes("status=401") ||
      normalized.includes("401 unauthorized")) &&
      (normalized.includes("refresh") || normalized.includes("刷新")))
  );
}

/** 历史 refresh_token_reused 状态不再参与账号展示、切换或可用性判断。 */
export function isCodexRefreshTokenReusedReason(
  reason?: string | null,
): boolean {
  const normalized = reason?.trim().toLowerCase() || "";
  return (
    normalized.includes("refresh_token_reused") ||
    normalized.includes("refresh token has been reused") ||
    normalized.includes("refresh_token 已被其它客户端或实例使用过")
  );
}

export function isCodexRefreshTokenReusedAccount(
  account?: CodexAccount | null,
): boolean {
  if (!account) return false;
  return (
    isCodexRefreshTokenReusedReason(account.reauth_reason) ||
    isCodexRefreshTokenReusedReason(account.quota_error?.code) ||
    isCodexRefreshTokenReusedReason(account.quota_error?.message)
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

/**
 * 服务端明确撤销 Codex 授权时，账号进入最高优先级的终止状态。
 * 这类状态表示 refresh_token 无法恢复，不能再与“客户端需授权但 API 仍可用”并列展示。
 */
export function isCodexServerRevokedReauth(
  account?: CodexAccount | null,
): boolean {
  if (!account) return false;
  const reason = String(account.reauth_reason || "").trim().toLowerCase();
  const quotaCode = String(account.quota_error?.code || "")
    .trim()
    .toLowerCase();
  return (
    reason.includes("refresh_token_invalidated") ||
    reason.includes("token_invalidated") ||
    reason.includes("authentication token has been invalidated") ||
    reason.includes("服务端撤销") ||
    quotaCode === "refresh_token_invalidated" ||
    quotaCode === "token_invalidated"
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
