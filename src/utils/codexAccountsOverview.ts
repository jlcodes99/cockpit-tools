import type { CodexAccount } from "../types/codex";

export type CodexTierCountKey =
  | "all"
  | "VALID"
  | "FREE"
  | "PLUS"
  | "PRO"
  | "TEAM"
  | "ENTERPRISE"
  | "PENDING"
  | "ERROR";

export type CodexTierCounts = Record<CodexTierCountKey, number>;

export function buildEmptyCodexTierCounts(total = 0): CodexTierCounts {
  return {
    all: total,
    VALID: 0,
    FREE: 0,
    PLUS: 0,
    PRO: 0,
    TEAM: 0,
    ENTERPRISE: 0,
    PENDING: 0,
    ERROR: 0,
  };
}

export function buildCodexTierCounts(
  accounts: readonly CodexAccount[],
  isAbnormalAccount: (account: CodexAccount) => boolean,
  resolvePlanKey: (account: CodexAccount) => string,
): CodexTierCounts {
  const counts = buildEmptyCodexTierCounts(accounts.length);

  accounts.forEach((account) => {
    const abnormal = isAbnormalAccount(account);
    if (!abnormal) {
      counts.VALID += 1;
    }

    const tier = resolvePlanKey(account);
    if (Object.prototype.hasOwnProperty.call(counts, tier)) {
      counts[tier as CodexTierCountKey] += 1;
    }

    if (abnormal) {
      counts.ERROR += 1;
    }
  });

  return counts;
}

export function collectNormalizedCodexAccountTags(
  accounts: readonly CodexAccount[],
  normalizeTag: (tag: string) => string,
): string[] {
  const tagSet = new Set<string>();

  accounts.forEach((account) => {
    (account.tags || []).forEach((tag) => {
      const normalized = normalizeTag(tag);
      if (normalized) {
        tagSet.add(normalized);
      }
    });
  });

  return Array.from(tagSet).sort((a, b) => a.localeCompare(b));
}
