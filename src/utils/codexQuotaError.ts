import type { CodexAccount, CodexQuotaErrorInfo } from "../types/codex";
import { isCodexClientReauthNoticeOnly } from "./codexSwitchAuthFailure";

const BLOCKING_STATUS_CODES = new Set(["401", "403", "429"]);
const BLOCKING_ERROR_CODES = new Set([
  "invalid_grant",
  "invalid_token",
  "refresh_token_expired",
  "refresh_token_invalidated",
  "token_invalidated",
  "usage_limit_reached",
  "insufficient_quota",
  "rate_limit_exceeded",
]);

/** Card summary stays short so HTML/error dumps never blow up the layout. */
const CARD_ERROR_SUMMARY_MAX_CHARS = 96;

export function extractCodexQuotaErrorStatusCode(message: string): string {
  const raw = message.trim();
  return (
    raw.match(/API 返回错误\s+(\d{3})/i)?.[1] ||
    raw.match(/status[=: ]+(\d{3})/i)?.[1] ||
    raw.match(/\b(401|403|429)\s+Forbidden\b/i)?.[1] ||
    raw.match(/\b(401|403|429)\b/)?.[1] ||
    ""
  );
}

export function extractCodexQuotaErrorCode(
  message: string,
  explicitCode?: string | null,
): string {
  return (
    explicitCode ||
    message.match(/\[error_code:([^\]]+)\]/)?.[1] ||
    message.match(/error_code[=:]\s*([^,\]\s]+)/i)?.[1] ||
    ""
  )
    .trim()
    .toLowerCase();
}

export function isVerboseCodexQuotaErrorMessage(message: string): boolean {
  const raw = message.trim();
  if (!raw) return false;
  if (raw.length > CARD_ERROR_SUMMARY_MAX_CHARS) return true;
  if (/<\/?[a-z][\s\S]*>/i.test(raw)) return true;
  if (/body_len\s*[:=]\s*\d{3,}/i.test(raw)) return true;
  if (/\[body\s*[:=]/i.test(raw)) return true;
  if (/<!doctype\s+html/i.test(raw) || /<html[\s>]/i.test(raw)) return true;
  return false;
}

/** Strip markup/noise and keep a single-line preview for card surfaces. */
export function summarizeCodexQuotaErrorMessage(
  message: string,
  options?: { maxChars?: number },
): string {
  const maxChars = options?.maxChars ?? CARD_ERROR_SUMMARY_MAX_CHARS;
  let text = message.replace(/\r\n/g, "\n").trim();
  if (!text) return "";

  // Prefer the structured prefix before a huge body dump.
  const bodyDumpIndex = text.search(
    /\s*\[body(?:_len)?\s*[:=]|\s*<!doctype\s+html|\s*<html[\s>]/i,
  );
  if (bodyDumpIndex > 0) {
    text = text.slice(0, bodyDumpIndex).trim();
  }

  text = text
    .replace(/<\/?[^>]+>/g, " ")
    .replace(/&nbsp;/gi, " ")
    .replace(/&amp;/gi, "&")
    .replace(/&lt;/gi, "<")
    .replace(/&gt;/gi, ">")
    .replace(/&quot;/gi, '"')
    .replace(/&#39;/g, "'")
    .replace(/\s+/g, " ")
    .trim();

  if (text.length <= maxChars) return text;
  return `${text.slice(0, Math.max(0, maxChars - 1)).trimEnd()}…`;
}

export function isBlockingCodexQuotaError(
  quotaError?: CodexQuotaErrorInfo | null,
): boolean {
  const rawMessage = quotaError?.message?.trim();
  if (!rawMessage) return false;

  const lower = rawMessage.toLowerCase();
  const statusCode = extractCodexQuotaErrorStatusCode(rawMessage);
  const errorCode = extractCodexQuotaErrorCode(rawMessage, quotaError?.code);

  // refresh_token_reused 只属于刷新竞争/轮换结果，不能因其携带 401 状态码
  // 被额度路由当成账号不可用。
  if (
    errorCode === "refresh_token_reused" ||
    lower.includes("refresh_token_reused") ||
    lower.includes("refresh token has been reused") ||
    lower.includes("refresh_token 已被其它客户端或实例使用过")
  ) {
    return false;
  }

  if (BLOCKING_STATUS_CODES.has(statusCode)) return true;
  if (errorCode && BLOCKING_ERROR_CODES.has(errorCode)) return true;

  return (
    lower.includes("401 unauthorized") ||
    lower.includes("403 forbidden") ||
    lower.includes("429 too many requests") ||
    lower.includes("invalid_grant") ||
    lower.includes("invalid_token") ||
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

export function isBlockingCodexAccountQuotaError(
  account: CodexAccount,
): boolean {
  return (
    !isCodexClientReauthNoticeOnly(account) &&
    isBlockingCodexQuotaError(account.quota_error)
  );
}
