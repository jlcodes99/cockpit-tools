import { useEffect, useMemo, useRef, useState } from 'react'
import { FolderPlus, Pencil, Search, X } from 'lucide-react'
import { useTranslation } from 'react-i18next'
import type { Account } from '../types/account'
import type { AccountGroup } from '../services/accountGroupService'
import { getAntigravityTierBadge } from '../utils/account'
import {
  accountMatchesTagFilters,
  accountMatchesTypeFilters,
  buildAccountTierCounts,
  buildAccountTierFilterOptions,
  collectAvailableAccountTags,
  normalizeAccountTag,
  type AccountFilterType,
} from '../utils/accountFilters'
import { MultiSelectFilterDropdown } from './MultiSelectFilterDropdown'
import { AccountTagFilterDropdown } from './AccountTagFilterDropdown'
import { useEscClose } from '../hooks/useEscClose'
import './GroupAccountPickerModal.css'

interface GroupAccountPickerModalProps {
  isOpen: boolean
  targetGroup: AccountGroup | null
  accounts: Account[]
  accountGroups: AccountGroup[]
  verificationStatusMap: Record<string, string>
  getVerificationBadge: (account: Account) => { label: string; className: string } | null
  maskAccountText: (value?: string | null) => string
  onClose: () => void
  onConfirm: (payload: { name: string; accountIds: string[] }) => Promise<void> | void
  mode?: 'edit' | 'addAccounts'
}

export function GroupAccountPickerModal({
  isOpen,
  targetGroup,
  accounts,
  accountGroups,
  verificationStatusMap,
  getVerificationBadge,
  maskAccountText,
  onClose,
  onConfirm,
  mode = 'edit',
}: GroupAccountPickerModalProps) {
  const { t } = useTranslation()
  useEscClose(isOpen, onClose)
  const isQuickAddMode = mode === 'addAccounts'
  const [query, setQuery] = useState('')
  const [groupName, setGroupName] = useState('')
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [filterTypes, setFilterTypes] = useState<AccountFilterType[]>([])
  const [tagFilter, setTagFilter] = useState<string[]>([])
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState('')
  const selectAllCheckboxRef = useRef<HTMLInputElement | null>(null)

  useEffect(() => {
    if (!isOpen) return
    setQuery('')
    setGroupName(targetGroup?.name ?? '')
    setSelected(isQuickAddMode ? new Set() : new Set(targetGroup?.accountIds ?? []))
    setFilterTypes([])
    setTagFilter([])
    setError('')
  }, [isOpen, isQuickAddMode, targetGroup])

  const groupByAccountId = useMemo(() => {
    const result = new Map<string, AccountGroup>()
    for (const group of accountGroups) {
      for (const accountId of group.accountIds) {
        if (!result.has(accountId)) {
          result.set(accountId, group)
        }
      }
    }
    return result
  }, [accountGroups])

  const availableTags = useMemo(() => collectAvailableAccountTags(accounts), [accounts])

  const tierCounts = useMemo(
    () => buildAccountTierCounts(accounts, verificationStatusMap),
    [accounts, verificationStatusMap]
  )

  const tierFilterOptions = useMemo(
    () => buildAccountTierFilterOptions(t, tierCounts),
    [t, tierCounts]
  )

  const visibleAccounts = useMemo(() => {
    const normalized = query.trim().toLowerCase()
    const selectedTypes = new Set(filterTypes)
    const selectedTags = new Set(tagFilter.map(normalizeAccountTag))
    let next = [...accounts].sort((a, b) => a.email.localeCompare(b.email))

    if (isQuickAddMode && targetGroup) {
      const existingIds = new Set(targetGroup.accountIds)
      next = next.filter((account) => !existingIds.has(account.id))
    }

    if (selectedTypes.size > 0) {
      next = next.filter((account) =>
        accountMatchesTypeFilters(account, selectedTypes, verificationStatusMap)
      )
    }

    if (selectedTags.size > 0) {
      next = next.filter((account) => accountMatchesTagFilters(account, selectedTags))
    }

    if (!normalized) return next

    return next.filter((account) => {
      const currentGroupName = groupByAccountId.get(account.id)?.name?.toLowerCase() || ''
      return account.email.toLowerCase().includes(normalized) || currentGroupName.includes(normalized)
    })
  }, [accounts, filterTypes, groupByAccountId, isQuickAddMode, query, tagFilter, targetGroup, verificationStatusMap])

  const selectedVisibleCount = useMemo(
    () =>
      visibleAccounts.reduce(
        (count, account) => count + (selected.has(account.id) ? 1 : 0),
        0
      ),
    [selected, visibleAccounts]
  )

  const allVisibleSelected =
    visibleAccounts.length > 0 &&
    selectedVisibleCount === visibleAccounts.length

  const hasSelectionChanges = useMemo(() => {
    if (!targetGroup) return false
    if (isQuickAddMode) return selected.size > 0
    if (selected.size !== targetGroup.accountIds.length) return true
    return targetGroup.accountIds.some((accountId) => !selected.has(accountId))
  }, [isQuickAddMode, selected, targetGroup])

  const trimmedGroupName = groupName.trim()

  const hasNameChanges = useMemo(() => {
    if (!targetGroup) return false
    if (isQuickAddMode) return false
    return trimmedGroupName !== targetGroup.name
  }, [isQuickAddMode, targetGroup, trimmedGroupName])

  const hasChanges = hasNameChanges || hasSelectionChanges

  useEffect(() => {
    if (!selectAllCheckboxRef.current) return
    selectAllCheckboxRef.current.indeterminate =
      selectedVisibleCount > 0 && !allVisibleSelected
  }, [allVisibleSelected, selectedVisibleCount])

  const toggleSelectAllVisible = () => {
    if (saving || visibleAccounts.length === 0) return

    setSelected((prev) => {
      const next = new Set(prev)
      if (allVisibleSelected) {
        for (const account of visibleAccounts) {
          next.delete(account.id)
        }
      } else {
        for (const account of visibleAccounts) {
          next.add(account.id)
        }
      }
      return next
    })
  }

  const toggleSelect = (accountId: string, disabled: boolean) => {
    if (disabled) return
    setSelected((prev) => {
      const next = new Set(prev)
      if (next.has(accountId)) {
        next.delete(accountId)
      } else {
        next.add(accountId)
      }
      return next
    })
  }

  const handleConfirm = async () => {
    if (!targetGroup || !hasChanges || saving) return
    if (!isQuickAddMode && !trimmedGroupName) {
      setError(t('platformLayout.groupNameRequired'))
      return
    }
    setSaving(true)
    setError('')
    try {
      const nextAccountIds = isQuickAddMode
        ? [...targetGroup.accountIds, ...Array.from(selected)]
        : Array.from(selected)
      await onConfirm({
        name: isQuickAddMode ? targetGroup.name : trimmedGroupName,
        accountIds: nextAccountIds,
      })
      onClose()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setSaving(false)
    }
  }

  if (!isOpen || !targetGroup) return null

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal group-account-picker-modal" onClick={(event) => event.stopPropagation()}>
        <div className="modal-header">
          <h2 className="group-account-picker-title">
            {isQuickAddMode ? <FolderPlus size={18} /> : <Pencil size={18} />}
            <span>{isQuickAddMode ? t('accounts.groups.addAccounts') : t('accounts.groups.editTitle')}</span>
            {isQuickAddMode && (
              <span className="group-account-picker-target">{targetGroup.name}</span>
            )}
          </h2>
          <button
            className="modal-close"
            onClick={onClose}
            aria-label={t('common.close')}
          >
            <X size={18} />
          </button>
        </div>

        <div className="modal-body group-account-picker-body">
          {!isQuickAddMode && (
            <div className="group-account-name-field">
              <label htmlFor="group-account-name">{t('platformLayout.groupName')}</label>
              <input
                id="group-account-name"
                type="text"
                value={groupName}
                onChange={(event) => setGroupName(event.target.value)}
                placeholder={t('accounts.groups.newPlaceholder')}
                maxLength={30}
              />
            </div>
          )}

          <div className="group-account-toolbar">
            <div className="group-account-search">
              <Search size={16} className="group-account-search-icon" />
              <input
                type="text"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder={t('accounts.search')}
              />
            </div>

            <div className="group-account-picker-filters">
              <MultiSelectFilterDropdown
                options={tierFilterOptions}
                selectedValues={filterTypes}
                allLabel={t('accounts.filter.all', { count: tierCounts.all })}
                filterLabel={t('accounts.filterLabel', '筛选')}
                clearLabel={t('accounts.clearFilter', '清空筛选')}
                emptyLabel={t('common.none', '暂无')}
                ariaLabel={t('accounts.filterLabel', '筛选')}
                onToggleValue={(value) =>
                  setFilterTypes((prev) =>
                    prev.includes(value as AccountFilterType)
                      ? prev.filter((item) => item !== value)
                      : [...prev, value as AccountFilterType]
                  )
                }
                onClear={() => setFilterTypes([])}
              />
              <AccountTagFilterDropdown
                availableTags={availableTags}
                selectedTags={tagFilter}
                onToggleTag={(value) =>
                  setTagFilter((prev) =>
                    prev.includes(value) ? prev.filter((item) => item !== value) : [...prev, value]
                  )
                }
                onClear={() => setTagFilter([])}
              />
            </div>
          </div>

          <div className="group-account-item group-account-item-header">
            <input
              ref={selectAllCheckboxRef}
              type="checkbox"
              checked={allVisibleSelected}
              onChange={toggleSelectAllVisible}
              disabled={saving || visibleAccounts.length === 0}
            />
            <div className="group-account-main" />
          </div>

          <div className="group-account-list">
            {visibleAccounts.length === 0 ? (
              <div className="group-account-empty">{t('accounts.groups.accountPickerEmpty')}</div>
            ) : (
              visibleAccounts.map((account) => {
                const currentGroup = groupByAccountId.get(account.id) || null
                const isUngrouped = !currentGroup
                const isChecked = selected.has(account.id)
                const tierBadge = getAntigravityTierBadge(account.quota)
                const verificationBadge = getVerificationBadge(account)

                return (
                  <label
                    key={account.id}
                    className={`group-account-item${isChecked ? ' is-current' : ''}`}
                  >
                    <input
                      type="checkbox"
                      checked={isChecked}
                      disabled={saving}
                      onChange={() => toggleSelect(account.id, saving)}
                    />
                    <div className="group-account-main">
                      <span className="group-account-email" title={maskAccountText(account.email) || ''}>
                        {maskAccountText(account.email)}
                      </span>
                      <div className="group-account-meta">
                        <span className={`tier-badge ${tierBadge.className} group-account-tier-badge`}>
                          {tierBadge.label}
                        </span>
                        {verificationBadge && (
                          <span
                            className={`verification-status-pill ${verificationBadge.className}`}
                            title={verificationBadge.label}
                          >
                            {verificationBadge.label}
                          </span>
                        )}
                        <span className={`group-account-badge${isUngrouped ? ' is-ungrouped' : ''}`}>
                          {isUngrouped ? t('accounts.groups.ungrouped') : currentGroup.name}
                        </span>
                      </div>
                    </div>
                  </label>
                )
              })
            )}
          </div>

          {error && <div className="group-account-error">{error}</div>}
        </div>

        <div className="modal-footer group-account-picker-footer">
          <button className="btn btn-secondary" onClick={onClose} disabled={saving}>
            {t('common.cancel')}
          </button>
          <button
            className="btn btn-primary"
            onClick={handleConfirm}
            disabled={!hasChanges || saving || (!isQuickAddMode && !trimmedGroupName)}
          >
            {saving
              ? t('common.saving')
              : isQuickAddMode
                ? `${t('accounts.groups.addAccounts')} (${selected.size})`
                : `${t('common.save')} (${selected.size})`}
          </button>
        </div>
      </div>
    </div>
  )
}

export default GroupAccountPickerModal
