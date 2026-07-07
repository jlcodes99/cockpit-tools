import type { Account } from '../types/account';
import type { ClaudeAccount } from '../types/claude';
import type { CodebuddyAccount } from '../types/codebuddy';
import { getCodebuddyExtraCreditSummary, getCodebuddyOfficialQuotaModel, getCodebuddyResourceSummary } from '../types/codebuddy';
import type { CodexAccount } from '../types/codex';
import type { CursorAccount } from '../types/cursor';
import { getCursorUsage } from '../types/cursor';
import type { GeminiAccount } from '../types/gemini';
import { getGeminiTierQuotaSummary } from '../types/gemini';
import type { GitHubCopilotAccount } from '../types/githubCopilot';
import type { KiroAccount } from '../types/kiro';
import { getKiroCreditsSummary, isKiroAccountBanned } from '../types/kiro';
import type { PlatformId } from '../types/platform';
import type { QoderAccount } from '../types/qoder';
import { getQoderSubscriptionInfo } from '../types/qoder';
import type { TraeAccount } from '../types/trae';
import { getTraeUsage } from '../types/trae';
import type { WindsurfAccount } from '../types/windsurf';
import { getWindsurfCreditsSummary } from '../types/windsurf';
import type { WorkbuddyAccount } from '../types/workbuddy';
import { getWorkbuddyOfficialQuotaModel } from '../types/workbuddy';
import type { ZedAccount } from '../types/zed';
import { getZedUsage } from '../types/zed';

export const GHCP_CURRENT_ACCOUNT_ID_KEY = 'agtools.github_copilot.current_account_id';
export const WINDSURF_CURRENT_ACCOUNT_ID_KEY = 'agtools.windsurf.current_account_id';
export const KIRO_CURRENT_ACCOUNT_ID_KEY = 'agtools.kiro.current_account_id';
export const CURSOR_CURRENT_ACCOUNT_ID_KEY = 'agtools.cursor.current_account_id';
export const GEMINI_CURRENT_ACCOUNT_ID_KEY = 'agtools.gemini.current_account_id';
export const CODEBUDDY_CURRENT_ACCOUNT_ID_KEY = 'agtools.codebuddy.current_account_id';
export const CODEBUDDY_CN_CURRENT_ACCOUNT_ID_KEY = 'agtools.codebuddycn.current_account_id';
export const QODER_CURRENT_ACCOUNT_ID_KEY = 'agtools.qoder.current_account_id';
export const TRAE_CURRENT_ACCOUNT_ID_KEY = 'agtools.trae.current_account_id';
export const TRAE_SOLO_CURRENT_ACCOUNT_ID_KEY = 'agtools.trae_solo.current_account_id';
export const TRAE_CN_CURRENT_ACCOUNT_ID_KEY = 'agtools.trae_cn.current_account_id';
export const TRAE_SOLO_CN_CURRENT_ACCOUNT_ID_KEY = 'agtools.trae_solo_cn.current_account_id';
export const WORKBUDDY_CURRENT_ACCOUNT_ID_KEY = 'agtools.workbuddy.current_account_id';
export const ZED_CURRENT_ACCOUNT_ID_KEY = 'agtools.zed.current_account_id';

type TimestampedAccount = {
  id: string;
  last_used?: number | null;
  created_at?: number | null;
};

export type StoredCurrentPlatformId =
  | 'github-copilot'
  | 'windsurf'
  | 'kiro'
  | 'cursor'
  | 'gemini'
  | 'codebuddy'
  | 'codebuddy_cn'
  | 'qoder'
  | 'trae'
  | 'trae_solo'
  | 'trae_cn'
  | 'trae_solo_cn'
  | 'workbuddy'
  | 'zed';

const CURRENT_ACCOUNT_STORAGE_KEYS: Record<StoredCurrentPlatformId, string> = {
  'github-copilot': GHCP_CURRENT_ACCOUNT_ID_KEY,
  windsurf: WINDSURF_CURRENT_ACCOUNT_ID_KEY,
  kiro: KIRO_CURRENT_ACCOUNT_ID_KEY,
  cursor: CURSOR_CURRENT_ACCOUNT_ID_KEY,
  gemini: GEMINI_CURRENT_ACCOUNT_ID_KEY,
  codebuddy: CODEBUDDY_CURRENT_ACCOUNT_ID_KEY,
  codebuddy_cn: CODEBUDDY_CN_CURRENT_ACCOUNT_ID_KEY,
  qoder: QODER_CURRENT_ACCOUNT_ID_KEY,
  trae: TRAE_CURRENT_ACCOUNT_ID_KEY,
  trae_solo: TRAE_SOLO_CURRENT_ACCOUNT_ID_KEY,
  trae_cn: TRAE_CN_CURRENT_ACCOUNT_ID_KEY,
  trae_solo_cn: TRAE_SOLO_CN_CURRENT_ACCOUNT_ID_KEY,
  workbuddy: WORKBUDDY_CURRENT_ACCOUNT_ID_KEY,
  zed: ZED_CURRENT_ACCOUNT_ID_KEY,
};

function toFiniteNumber(value: number | null | undefined): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

export function readStoredCurrentAccountId(platformId: StoredCurrentPlatformId): string | null {
  const storageKey = CURRENT_ACCOUNT_STORAGE_KEYS[platformId];
  try {
    return localStorage.getItem(storageKey);
  } catch {
    return null;
  }
}

export function writeStoredCurrentAccountId(
  platformId: StoredCurrentPlatformId,
  accountId: string | null,
) {
  const storageKey = CURRENT_ACCOUNT_STORAGE_KEYS[platformId];
  try {
    if (accountId) {
      localStorage.setItem(storageKey, accountId);
    } else {
      localStorage.removeItem(storageKey);
    }
  } catch {
    // ignore storage write failures
  }
}

export function resolveCurrentOrMostRecentAccount<T extends TimestampedAccount>(
  accounts: T[],
  currentId: string | null | undefined,
): T | null {
  if (accounts.length === 0) return null;
  if (currentId) {
    const current = accounts.find((account) => account.id === currentId);
    if (current) return current;
  }
  return accounts.reduce((prev, curr) => {
    const prevScore = prev.last_used || prev.created_at || 0;
    const currScore = curr.last_used || curr.created_at || 0;
    return currScore > prevScore ? curr : prev;
  });
}

export function getRecommendedAntigravityAccount(
  accounts: Account[],
  currentId: string | null | undefined,
): Account | null {
  if (accounts.length <= 1) return null;
  const others = accounts.filter((account) => {
    if (account.id === currentId) return false;
    if (account.disabled) return false;
    if (account.quota?.is_forbidden) return false;
    if (!account.quota?.models || account.quota.models.length === 0) return false;
    return true;
  });
  if (others.length === 0) return null;

  const getScore = (account: Account) => {
    if (!account.quota?.models) return -1;
    const total = account.quota.models.reduce((sum, model) => sum + model.percentage, 0);
    return total / account.quota.models.length;
  };

  return others.reduce((best, candidate) => (
    getScore(candidate) > getScore(best) ? candidate : best
  ));
}

export function getRecommendedCodexAccount(
  accounts: CodexAccount[],
  currentId: string | null | undefined,
): CodexAccount | null {
  if (accounts.length <= 1) return null;
  const others = accounts.filter((account) => account.id !== currentId && Boolean(account.quota));
  if (others.length === 0) return null;

  const getScore = (account: CodexAccount) => {
    if (!account.quota) return -1;
    return (account.quota.hourly_percentage + account.quota.weekly_percentage) / 2;
  };

  return others.reduce((best, candidate) => (
    getScore(candidate) > getScore(best) ? candidate : best
  ));
}

export function getRecommendedClaudeAccount(
  accounts: ClaudeAccount[],
  currentId: string | null | undefined,
): ClaudeAccount | null {
  if (accounts.length <= 1) return null;
  const others = accounts.filter((account) => account.id !== currentId);
  if (others.length === 0) return null;

  const getScore = (account: ClaudeAccount) => {
    const usedValues = [
      account.quota?.five_hour_percentage,
      account.quota?.seven_day_percentage,
    ].filter((value): value is number => typeof value === 'number' && Number.isFinite(value));
    const remaining = usedValues.length > 0 ? 100 - Math.max(...usedValues) : -1;
    return {
      remaining,
      freshness: account.last_used || account.created_at || 0,
    };
  };

  return others.reduce((best, candidate) => {
    const bestScore = getScore(best);
    const candidateScore = getScore(candidate);
    if (candidateScore.remaining !== bestScore.remaining) {
      return candidateScore.remaining > bestScore.remaining ? candidate : best;
    }
    return candidateScore.freshness > bestScore.freshness ? candidate : best;
  });
}

export function getRecommendedGitHubCopilotAccount(
  accounts: GitHubCopilotAccount[],
  currentId: string | null | undefined,
): GitHubCopilotAccount | null {
  if (accounts.length <= 1) return null;
  const others = accounts.filter((account) => account.id !== currentId);
  if (others.length === 0) return null;

  const getScore = (account: GitHubCopilotAccount) => {
    const scores = [account.quota?.hourly_percentage, account.quota?.weekly_percentage].filter(
      (value): value is number => typeof value === 'number',
    );
    if (scores.length === 0) return 101;
    return scores.reduce((sum, value) => sum + value, 0) / scores.length;
  };

  return others.reduce((best, candidate) => (
    getScore(candidate) < getScore(best) ? candidate : best
  ));
}

export function getRecommendedWindsurfAccount(
  accounts: WindsurfAccount[],
  currentId: string | null | undefined,
): WindsurfAccount | null {
  if (accounts.length <= 1) return null;
  const others = accounts.filter((account) => account.id !== currentId);
  if (others.length === 0) return null;

  const getScore = (account: WindsurfAccount) => {
    const credits = getWindsurfCreditsSummary(account);
    const promptLeft = toFiniteNumber(credits.promptCreditsLeft);
    const addOnLeft = toFiniteNumber(credits.addOnCredits);

    if (promptLeft != null) {
      return promptLeft * 1000 + (addOnLeft ?? 0);
    }

    const quotaValues = [account.quota?.hourly_percentage, account.quota?.weekly_percentage].filter(
      (value): value is number => typeof value === 'number',
    );
    if (quotaValues.length > 0) {
      const avgUsed = quotaValues.reduce((sum, value) => sum + value, 0) / quotaValues.length;
      return 100 - avgUsed;
    }

    return (account.last_used || account.created_at || 0) / 1e9;
  };

  return others.reduce((best, candidate) => (
    getScore(candidate) > getScore(best) ? candidate : best
  ));
}

export function getRecommendedKiroAccount(
  accounts: KiroAccount[],
  currentId: string | null | undefined,
): KiroAccount | null {
  if (accounts.length <= 1) return null;
  const others = accounts.filter(
    (account) => account.id !== currentId && !isKiroAccountBanned(account),
  );
  if (others.length === 0) return null;

  const getScore = (account: KiroAccount) => {
    const credits = getKiroCreditsSummary(account);
    const promptLeft = toFiniteNumber(credits.promptCreditsLeft);
    const addOnLeft = toFiniteNumber(credits.addOnCredits);

    if (promptLeft != null) {
      return promptLeft * 1000 + (addOnLeft ?? 0);
    }

    const quotaValues = [account.quota?.hourly_percentage, account.quota?.weekly_percentage].filter(
      (value): value is number => typeof value === 'number',
    );
    if (quotaValues.length > 0) {
      const avgUsed = quotaValues.reduce((sum, value) => sum + value, 0) / quotaValues.length;
      return 100 - avgUsed;
    }

    return (account.last_used || account.created_at || 0) / 1e9;
  };

  return others.reduce((best, candidate) => (
    getScore(candidate) > getScore(best) ? candidate : best
  ));
}

export function getRecommendedCursorAccount(
  accounts: CursorAccount[],
  currentId: string | null | undefined,
): CursorAccount | null {
  if (accounts.length <= 1) return null;
  const others = accounts.filter((account) => account.id !== currentId);
  if (others.length === 0) return null;

  const getScore = (account: CursorAccount) => {
    const usage = getCursorUsage(account);
    const planLimit = toFiniteNumber(usage.planLimitCents);
    const planUsedRaw = toFiniteNumber(usage.planUsedCents);
    const hasPlanBudget = planLimit != null && planLimit > 0;
    const planUsed = planUsedRaw != null ? Math.max(planUsedRaw, 0) : null;
    const remainingBudget = hasPlanBudget
      ? Math.max((planLimit ?? 0) - (planUsed ?? 0), 0)
      : -1;

    const totalUsedPercent = toFiniteNumber(
      usage.totalPercentUsed ??
        (hasPlanBudget && planUsed != null && planLimit != null && planLimit > 0
          ? (planUsed / planLimit) * 100
          : null),
    );
    const usedPercentList = [
      totalUsedPercent,
      toFiniteNumber(usage.autoPercentUsed),
      toFiniteNumber(usage.apiPercentUsed),
    ].filter((value): value is number => value != null);
    const avgUsedPercent = usedPercentList.length > 0
      ? usedPercentList.reduce((sum, value) => sum + value, 0) / usedPercentList.length
      : 101;

    return {
      hasPlanBudget,
      remainingBudget,
      avgUsedPercent,
      freshness: account.last_used || account.created_at || 0,
    };
  };

  return others.reduce((best, candidate) => {
    const bestScore = getScore(best);
    const candidateScore = getScore(candidate);

    if (bestScore.hasPlanBudget !== candidateScore.hasPlanBudget) {
      return candidateScore.hasPlanBudget ? candidate : best;
    }

    if (bestScore.remainingBudget !== candidateScore.remainingBudget) {
      return candidateScore.remainingBudget > bestScore.remainingBudget
        ? candidate
        : best;
    }

    if (bestScore.avgUsedPercent !== candidateScore.avgUsedPercent) {
      return candidateScore.avgUsedPercent < bestScore.avgUsedPercent
        ? candidate
        : best;
    }

    return candidateScore.freshness > bestScore.freshness ? candidate : best;
  });
}

export function getRecommendedGeminiAccount(
  accounts: GeminiAccount[],
  currentId: string | null | undefined,
): GeminiAccount | null {
  if (accounts.length <= 1) return null;
  const others = accounts.filter((account) => account.id !== currentId);
  if (others.length === 0) return null;

  const getScore = (account: GeminiAccount) => {
    const tiers = getGeminiTierQuotaSummary(account);
    const remainingValues = [
      tiers.gemini5h.remainingPercent,
      tiers.geminiWeekly.remainingPercent,
      tiers.claude5h.remainingPercent,
      tiers.claudeWeekly.remainingPercent,
    ].filter(
      (value): value is number => typeof value === 'number' && Number.isFinite(value),
    );
    const totalUsed = remainingValues.length > 0
      ? 100 - Math.min(...remainingValues)
      : null;
    return {
      remainingPercent: totalUsed == null ? -1 : 100 - totalUsed,
      freshness: account.last_used || account.created_at || 0,
    };
  };

  return others.reduce((best, candidate) => {
    const bestScore = getScore(best);
    const candidateScore = getScore(candidate);
    if (candidateScore.remainingPercent !== bestScore.remainingPercent) {
      return candidateScore.remainingPercent > bestScore.remainingPercent ? candidate : best;
    }
    return candidateScore.freshness > bestScore.freshness ? candidate : best;
  });
}

export function getRecommendedCodebuddyAccount(
  accounts: CodebuddyAccount[],
  currentId: string | null | undefined,
): CodebuddyAccount | null {
  if (accounts.length <= 1) return null;
  const others = accounts.filter((account) => account.id !== currentId);
  if (others.length === 0) return null;

  const getScore = (account: CodebuddyAccount) => {
    const resource = getCodebuddyResourceSummary(account);
    const extra = getCodebuddyExtraCreditSummary(account);
    const remain = resource?.remainPercent ?? (extra.remainPercent ?? -1);
    return {
      remainPercent: remain,
      freshness: account.last_used || account.created_at || 0,
    };
  };

  return others.reduce((best, candidate) => {
    const bestScore = getScore(best);
    const candidateScore = getScore(candidate);
    if (candidateScore.remainPercent !== bestScore.remainPercent) {
      return candidateScore.remainPercent > bestScore.remainPercent ? candidate : best;
    }
    return candidateScore.freshness > bestScore.freshness ? candidate : best;
  });
}

export function getRecommendedCodebuddyCnAccount(
  accounts: CodebuddyAccount[],
  currentId: string | null | undefined,
): CodebuddyAccount | null {
  if (accounts.length <= 1) return null;
  const others = accounts.filter((account) => account.id !== currentId);
  if (others.length === 0) return null;

  const getScore = (account: CodebuddyAccount) => {
    const model = getCodebuddyOfficialQuotaModel(account);
    const baseResources = model.resources.filter(
      (resource) => resource.total > 0 || resource.remain > 0,
    );

    let avgRemainPercent = -1;
    if (baseResources.length > 0) {
      const totalRemainPercent = baseResources.reduce((sum, resource) => {
        const percent = resource.remainPercent ?? (
          resource.total > 0 ? Math.max(0, (resource.remain / resource.total) * 100) : 0
        );
        return sum + percent;
      }, 0);
      avgRemainPercent = totalRemainPercent / baseResources.length;
    }

    return {
      remaining: avgRemainPercent,
      freshness: account.last_used || account.created_at || 0,
    };
  };

  return others.reduce((best, candidate) => {
    const bestScore = getScore(best);
    const candidateScore = getScore(candidate);
    if (candidateScore.remaining !== bestScore.remaining) {
      return candidateScore.remaining > bestScore.remaining ? candidate : best;
    }
    return candidateScore.freshness > bestScore.freshness ? candidate : best;
  });
}

export function getRecommendedQoderAccount(
  accounts: QoderAccount[],
  currentId: string | null | undefined,
): QoderAccount | null {
  if (accounts.length <= 1) return null;
  const others = accounts.filter((account) => account.id !== currentId);
  if (others.length === 0) return null;

  const getScore = (account: QoderAccount) => {
    const subscription = getQoderSubscriptionInfo(account);
    const usedPercent = subscription.totalUsagePercentage ?? subscription.userQuota.percentage ?? 101;
    return {
      remaining: 100 - usedPercent,
      freshness: account.last_used || account.created_at || 0,
    };
  };

  return others.reduce((best, candidate) => {
    const bestScore = getScore(best);
    const candidateScore = getScore(candidate);
    if (candidateScore.remaining !== bestScore.remaining) {
      return candidateScore.remaining > bestScore.remaining ? candidate : best;
    }
    return candidateScore.freshness > bestScore.freshness ? candidate : best;
  });
}

export function getRecommendedTraeAccount(
  accounts: TraeAccount[],
  currentId: string | null | undefined,
): TraeAccount | null {
  if (accounts.length <= 1) return null;
  const others = accounts.filter((account) => account.id !== currentId);
  if (others.length === 0) return null;

  const getScore = (account: TraeAccount) => {
    const usage = getTraeUsage(account);
    const usedPercent = usage.usedPercent ?? 101;
    return {
      remaining: 100 - usedPercent,
      freshness: account.last_used || account.created_at || 0,
    };
  };

  return others.reduce((best, candidate) => {
    const bestScore = getScore(best);
    const candidateScore = getScore(candidate);
    if (candidateScore.remaining !== bestScore.remaining) {
      return candidateScore.remaining > bestScore.remaining ? candidate : best;
    }
    return candidateScore.freshness > bestScore.freshness ? candidate : best;
  });
}

export function getRecommendedWorkbuddyAccount(
  accounts: WorkbuddyAccount[],
  currentId: string | null | undefined,
): WorkbuddyAccount | null {
  if (accounts.length <= 1) return null;
  const others = accounts.filter((account) => account.id !== currentId);
  if (others.length === 0) return null;

  const getScore = (account: WorkbuddyAccount) => {
    const model = getWorkbuddyOfficialQuotaModel(account);
    const baseResources = model.resources.filter(
      (resource) => resource.total > 0 || resource.remain > 0,
    );

    let avgRemainPercent = -1;
    if (baseResources.length > 0) {
      const totalRemainPercent = baseResources.reduce((sum, resource) => {
        const percent = resource.remainPercent ?? (
          resource.total > 0 ? Math.max(0, (resource.remain / resource.total) * 100) : 0
        );
        return sum + percent;
      }, 0);
      avgRemainPercent = totalRemainPercent / baseResources.length;
    }

    return {
      remaining: avgRemainPercent,
      freshness: account.last_used || account.created_at || 0,
    };
  };

  return others.reduce((best, candidate) => {
    const bestScore = getScore(best);
    const candidateScore = getScore(candidate);
    if (candidateScore.remaining !== bestScore.remaining) {
      return candidateScore.remaining > bestScore.remaining ? candidate : best;
    }
    return candidateScore.freshness > bestScore.freshness ? candidate : best;
  });
}

export function getZedRecommendationScore(account: ZedAccount): {
  remainingPercent: number;
  freshness: number;
} {
  const usage = getZedUsage(account);
  const remainingValues: number[] = [];

  if (
    usage.remainingCompletions != null &&
    usage.totalCompletions != null &&
    usage.totalCompletions > 0
  ) {
    remainingValues.push((usage.remainingCompletions / usage.totalCompletions) * 100);
  }

  if (
    usage.remainingChat != null &&
    usage.totalChat != null &&
    usage.totalChat > 0
  ) {
    remainingValues.push((usage.remainingChat / usage.totalChat) * 100);
  }

  return {
    remainingPercent:
      remainingValues.length > 0
        ? remainingValues.reduce((sum, value) => sum + value, 0) / remainingValues.length
        : -1,
    freshness: account.last_used || account.created_at || 0,
  };
}

export function getRecommendedZedAccount(
  accounts: ZedAccount[],
  currentId: string | null | undefined,
): ZedAccount | null {
  if (accounts.length <= 1) return null;
  const others = accounts.filter((account) => account.id !== currentId);
  if (others.length === 0) return null;

  return others.reduce((best, candidate) => {
    const bestScore = getZedRecommendationScore(best);
    const candidateScore = getZedRecommendationScore(candidate);
    if (candidateScore.remainingPercent !== bestScore.remainingPercent) {
      return candidateScore.remainingPercent > bestScore.remainingPercent ? candidate : best;
    }
    return candidateScore.freshness > bestScore.freshness ? candidate : best;
  });
}

export function isStoredCurrentPlatformId(platformId: PlatformId): platformId is StoredCurrentPlatformId {
  return platformId in CURRENT_ACCOUNT_STORAGE_KEYS;
}
