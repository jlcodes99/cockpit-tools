import type { Account } from '../types/account'
import { getSubscriptionTier } from './account'

export type AccountFilterType =
  | 'PRO'
  | 'ULTRA'
  | 'FREE'
  | 'UNKNOWN'
  | 'VERIFICATION_REQUIRED'
  | 'TOS_VIOLATION'

interface TierCounts {
  all: number
  PRO: number
  ULTRA: number
  FREE: number
  UNKNOWN: number
  VERIFICATION_REQUIRED: number
  TOS_VIOLATION: number
}

type Translate = (key: string, options?: Record<string, unknown>) => string

export function normalizeAccountTag(tag: string): string {
  return tag.trim().toLowerCase()
}

export function collectAvailableAccountTags(accounts: Account[]): string[] {
  const values = new Set<string>()
  for (const account of accounts) {
    for (const tag of account.tags || []) {
      const normalized = normalizeAccountTag(tag)
      if (normalized) {
        values.add(normalized)
      }
    }
  }
  return Array.from(values).sort((left, right) => left.localeCompare(right))
}

export function isAccountVerificationRequired(
  account: Account,
  verificationStatusMap: Record<string, string>
): boolean {
  const verificationStatus = account.disabled_reason || verificationStatusMap[account.id]
  if (verificationStatus === 'verification_required') return true
  const qErr = account.quota_error
  if (qErr?.reason === 'VALIDATION_REQUIRED' || Boolean(qErr?.validation_url)) return true
  if (qErr?.message && (qErr.message.includes('Verify your account') || qErr.message.includes('VALIDATION_REQUIRED'))) {
    return true
  }
  return false
}

export function accountMatchesTypeFilters(
  account: Account,
  selectedTypes: Set<AccountFilterType>,
  verificationStatusMap: Record<string, string>
): boolean {
  if (selectedTypes.size === 0) return true

  const verificationStatus = account.disabled_reason || verificationStatusMap[account.id]
  const tier = getSubscriptionTier(account.quota) as AccountFilterType

  if (
    selectedTypes.has('VERIFICATION_REQUIRED') &&
    isAccountVerificationRequired(account, verificationStatusMap)
  ) {
    return true
  }

  if (selectedTypes.has('TOS_VIOLATION') && verificationStatus === 'tos_violation') {
    return true
  }

  return selectedTypes.has(tier)
}

export function accountMatchesTagFilters(
  account: Account,
  selectedTags: Set<string>
): boolean {
  if (selectedTags.size === 0) return true
  const tags = (account.tags || []).map(normalizeAccountTag)
  return tags.some((tag) => selectedTags.has(tag))
}

export function buildAccountTierCounts(
  accounts: Account[],
  verificationStatusMap: Record<string, string>
): TierCounts {
  const counts: TierCounts = {
    all: accounts.length,
    PRO: 0,
    ULTRA: 0,
    FREE: 0,
    UNKNOWN: 0,
    VERIFICATION_REQUIRED: 0,
    TOS_VIOLATION: 0,
  }

  for (const account of accounts) {
    const tier = getSubscriptionTier(account.quota)
    if (tier === 'PRO') counts.PRO++
    else if (tier === 'ULTRA') counts.ULTRA++
    else if (tier === 'FREE') counts.FREE++
    else counts.UNKNOWN++

    const verificationStatus = account.disabled_reason || verificationStatusMap[account.id]
    if (isAccountVerificationRequired(account, verificationStatusMap)) counts.VERIFICATION_REQUIRED++
    else if (verificationStatus === 'tos_violation') counts.TOS_VIOLATION++
  }

  return counts
}

export function buildAccountTierFilterOptions(
  t: Translate,
  tierCounts: TierCounts
): Array<{ value: AccountFilterType; label: string }> {
  return [
    { value: 'PRO', label: `PRO (${tierCounts.PRO})` },
    { value: 'ULTRA', label: `ULTRA (${tierCounts.ULTRA})` },
    { value: 'FREE', label: `FREE (${tierCounts.FREE})` },
    {
      value: 'VERIFICATION_REQUIRED',
      label: `${t('wakeup.errorUi.verificationRequiredTitle')} (${tierCounts.VERIFICATION_REQUIRED})`,
    },
    {
      value: 'TOS_VIOLATION',
      label: `${t('wakeup.errorUi.tosViolationTitle')} (${tierCounts.TOS_VIOLATION})`,
    },
    { value: 'UNKNOWN', label: `UNKNOWN (${tierCounts.UNKNOWN})` },
  ]
}
