import type { CodexAccount } from '../types/codex';

/** A profile response must not overwrite concurrent quota, token or user edits. */
export function mergeCodexProfileResult(
  current: CodexAccount, original: CodexAccount, refreshed: CodexAccount,
): CodexAccount {
  if (current.token_generation !== original.token_generation ||
      current.tokens.access_token !== original.tokens.access_token ||
      current.account_id !== original.account_id) return current;
  let result = current;
  for (const field of ['account_name', 'account_structure', 'account_id'] as const) {
    const value = refreshed[field];
    if (current[field] === original[field] && value != null && value !== current[field]) {
      if (result === current) result = { ...current };
      result[field] = value;
    }
  }
  return result;
}
