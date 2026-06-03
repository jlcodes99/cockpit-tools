import type { CodexQuotaErrorInfo } from "../types/codex";

const BLOCKING_STATUS_CODES = new Set(["401", "403", "429"]);
const BLOCKING_ERROR_CODES = new Set([
  "invalid_grant",
  "invalid_token",
  "refresh_token_expired",
  "refresh_token_invalidated",
  "refresh_token_reused",
  "token_invalidated",
  "usage_limit_reached",
  "insufficient_quota",
  "rate_limit_exceeded",
]);

export function isBlockingCodexQuotaError(
  quotaError?: CodexQuotaErrorInfo | null,
): boolean {
  const rawMessage = quotaError?.message?.trim();
  if (!rawMessage) return false;

  const lower = rawMessage.toLowerCase();
  const statusCode =
    rawMessage.match(/API 返回错误\s+(\d{3})/i)?.[1] ||
    rawMessage.match(/status[=: ]+(\d{3})/i)?.[1] ||
    "";
  const errorCode = (
    quotaError?.code ||
    rawMessage.match(/\[error_code:([^\]]+)\]/)?.[1] ||
    rawMessage.match(/error_code[=:]\s*([^,\]\s]+)/i)?.[1] ||
    ""
  )
    .trim()
    .toLowerCase();

  if (BLOCKING_STATUS_CODES.has(statusCode)) return true;
  if (errorCode && BLOCKING_ERROR_CODES.has(errorCode)) return true;

  return (
    lower.includes("401 unauthorized") ||
    lower.includes("403 forbidden") ||
    lower.includes("429 too many requests") ||
    lower.includes("invalid_grant") ||
    lower.includes("invalid_token") ||
    lower.includes("refresh_token_reused") ||
    lower.includes("refresh_token_expired") ||
    lower.includes("refresh_token_invalidated") ||
    lower.includes("token_invalidated") ||
    lower.includes("usage_limit_reached") ||
    lower.includes("insufficient_quota") ||
    lower.includes("rate_limit_exceeded") ||
    lower.includes("quota exceeded") ||
    lower.includes("your authentication token has been invalidated") ||
    lower.includes("refresh_token 已被其它客户端或实例使用过") ||
    lower.includes("token 已过期且无 refresh_token") ||
    lower.includes("缺少 refresh_token") ||
    lower.includes("token 已过期且刷新失败") ||
    lower.includes("刷新 token 失败")
  );
}
