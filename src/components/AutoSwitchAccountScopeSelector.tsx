import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { createPortal } from 'react-dom';
import {
  MultiSelectFilterDropdown,
  type MultiSelectFilterOption,
} from './MultiSelectFilterDropdown';
import { normalizeAccountTag } from '../utils/accountFilters';
import './AutoSwitchAccountScopeSelector.css';

const UNGROUPED_ACCOUNT_GROUP_FILTER_KEY = '__ungrouped__';

export type AutoSwitchAccountScopeMode = 'all_accounts' | 'selected_accounts';

export interface AutoSwitchScopeAccount {
  id: string;
  label: string;
  searchableText?: string;
  tags?: string[];
  type?: string;
  badge?: ReactNode;
}

export interface AutoSwitchScopeGroup {
  id: string;
  name: string;
  accountIds: string[];
}

interface AutoSwitchAccountScopeSelectorProps {
  mode: AutoSwitchAccountScopeMode;
  onModeChange: (mode: AutoSwitchAccountScopeMode) => void;
  selectedAccountIds: string[];
  onSelectedAccountIdsChange: (ids: string[]) => void;
  accounts: AutoSwitchScopeAccount[];
  groups: AutoSwitchScopeGroup[];
  typeOptions?: MultiSelectFilterOption[];
  useDialog?: boolean;
}

function toggleMultiValue(values: string[], value: string) {
  return values.includes(value)
    ? values.filter((item) => item !== value)
    : [...values, value];
}

export function AutoSwitchAccountScopeSelector({
  mode,
  onModeChange,
  selectedAccountIds,
  onSelectedAccountIdsChange,
  accounts,
  groups,
  typeOptions,
  useDialog = false,
}: AutoSwitchAccountScopeSelectorProps) {
  const { t } = useTranslation();
  const selectAllRef = useRef<HTMLInputElement | null>(null);
  const [dialogOpen, setDialogOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState('');
  const [typeFilter, setTypeFilter] = useState<string[]>([]);
  const [tagFilter, setTagFilter] = useState<string[]>([]);
  const [groupFilter, setGroupFilter] = useState<string[]>([]);

  const accountById = useMemo(() => {
    const map = new Map<string, AutoSwitchScopeAccount>();
    accounts.forEach((account) => {
      map.set(account.id, account);
    });
    return map;
  }, [accounts]);

  const availableTagOptions = useMemo<MultiSelectFilterOption[]>(() => {
    const tagSet = new Set<string>();
    accounts.forEach((account) => {
      (account.tags || []).forEach((tag) => {
        const normalized = normalizeAccountTag(tag);
        if (normalized) {
          tagSet.add(normalized);
        }
      });
    });
    return Array.from(tagSet)
      .sort((left, right) => left.localeCompare(right))
      .map((tag) => ({ value: tag, label: tag }));
  }, [accounts]);

  const groupIdsByAccountId = useMemo(() => {
    const map = new Map<string, string[]>();
    groups.forEach((group) => {
      group.accountIds.forEach((accountId) => {
        const current = map.get(accountId);
        if (!current) {
          map.set(accountId, [group.id]);
          return;
        }
        current.push(group.id);
      });
    });
    return map;
  }, [groups]);

  const accountIdsInAnyGroup = useMemo(() => {
    const ids = new Set<string>();
    groups.forEach((group) => {
      group.accountIds.forEach((accountId) => ids.add(accountId));
    });
    return ids;
  }, [groups]);

  const groupOptions = useMemo<MultiSelectFilterOption[]>(() => {
    const options = groups
      .map((group) => {
        const count = group.accountIds.filter((id) => accountById.has(id)).length;
        return {
          value: group.id,
          label: `${group.name} (${count})`,
        };
      })
      .sort((left, right) => left.label.localeCompare(right.label));
    const ungroupedCount = accounts.reduce(
      (count, account) => (accountIdsInAnyGroup.has(account.id) ? count : count + 1),
      0,
    );
    return [
      ...options,
      {
        value: UNGROUPED_ACCOUNT_GROUP_FILTER_KEY,
        label: `${t('accounts.groups.ungrouped')} (${ungroupedCount})`,
      },
    ];
  }, [accountById, accountIdsInAnyGroup, accounts, groups, t]);

  const filteredAccounts = useMemo(() => {
    const query = searchQuery.trim().toLowerCase();
    const selectedTypes = new Set(typeFilter);
    const selectedTags = new Set(tagFilter.map(normalizeAccountTag));
    const selectedGroups = new Set(groupFilter);

    return accounts.filter((account) => {
      const searchText = `${account.label || ''} ${account.searchableText || ''}`.toLowerCase();
      if (query && !searchText.includes(query)) {
        return false;
      }
      if (selectedTypes.size > 0) {
        const typeValue = account.type || '';
        if (!selectedTypes.has(typeValue)) {
          return false;
        }
      }
      if (selectedTags.size > 0) {
        const accountTags = (account.tags || []).map(normalizeAccountTag).filter(Boolean);
        if (!accountTags.some((tag) => selectedTags.has(tag))) {
          return false;
        }
      }
      if (selectedGroups.size > 0) {
        const groupIds = groupIdsByAccountId.get(account.id) || [];
        const matchesGrouped = groupIds.some((groupId) => selectedGroups.has(groupId));
        const matchesUngrouped =
          groupIds.length === 0 && selectedGroups.has(UNGROUPED_ACCOUNT_GROUP_FILTER_KEY);
        if (!matchesGrouped && !matchesUngrouped) {
          return false;
        }
      }
      return true;
    });
  }, [accounts, groupFilter, groupIdsByAccountId, searchQuery, tagFilter, typeFilter]);

  const availableSelectedAccountIds = useMemo(
    () => selectedAccountIds.filter((accountId) => accountById.has(accountId)),
    [accountById, selectedAccountIds],
  );
  const selectedSet = useMemo(
    () => new Set(availableSelectedAccountIds),
    [availableSelectedAccountIds],
  );
  const visibleAccountIds = useMemo(
    () => filteredAccounts.map((account) => account.id),
    [filteredAccounts],
  );

  const selectedVisibleCount = useMemo(
    () =>
      visibleAccountIds.reduce(
        (count, accountId) => (selectedSet.has(accountId) ? count + 1 : count),
        0,
      ),
    [selectedSet, visibleAccountIds],
  );
  const allVisibleSelected = useMemo(
    () => visibleAccountIds.length > 0 && selectedVisibleCount === visibleAccountIds.length,
    [selectedVisibleCount, visibleAccountIds.length],
  );
  const partiallyVisibleSelected = useMemo(
    () => selectedVisibleCount > 0 && selectedVisibleCount < visibleAccountIds.length,
    [selectedVisibleCount, visibleAccountIds.length],
  );

  useEffect(() => {
    const validGroupIds = new Set(groups.map((group) => group.id));
    setGroupFilter((prev) =>
      prev.filter((value) => value === UNGROUPED_ACCOUNT_GROUP_FILTER_KEY || validGroupIds.has(value)),
    );
  }, [groups]);

  useEffect(() => {
    if (!typeOptions || typeOptions.length === 0) {
      if (typeFilter.length > 0) {
        setTypeFilter([]);
      }
      return;
    }
    const validTypeIds = new Set(typeOptions.map((option) => option.value));
    setTypeFilter((prev) => prev.filter((value) => validTypeIds.has(value)));
  }, [typeFilter.length, typeOptions]);

  useEffect(() => {
    const validTags = new Set(availableTagOptions.map((option) => option.value));
    setTagFilter((prev) => prev.filter((value) => validTags.has(value)));
  }, [availableTagOptions]);

  useEffect(() => {
    if (!selectAllRef.current) return;
    selectAllRef.current.indeterminate = partiallyVisibleSelected;
  }, [partiallyVisibleSelected]);

  useEffect(() => {
    if (mode !== 'selected_accounts') {
      setDialogOpen(false);
    }
  }, [mode]);

  useEffect(() => {
    if (!dialogOpen) return;
    const handleEsc = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setDialogOpen(false);
      }
    };
    document.addEventListener('keydown', handleEsc);
    return () => document.removeEventListener('keydown', handleEsc);
  }, [dialogOpen]);

  const handleToggleAllVisible = () => {
    if (visibleAccountIds.length === 0) {
      return;
    }
    const visibleSet = new Set(visibleAccountIds);
    if (allVisibleSelected) {
      onSelectedAccountIdsChange(
        availableSelectedAccountIds.filter((accountId) => !visibleSet.has(accountId)),
      );
      return;
    }
    const selected = new Set(availableSelectedAccountIds);
    const merged = [...availableSelectedAccountIds];
    visibleAccountIds.forEach((accountId) => {
      if (!selected.has(accountId)) {
        selected.add(accountId);
        merged.push(accountId);
      }
    });
    onSelectedAccountIdsChange(merged);
  };

  const accountScopeSummaryText =
    mode === 'all_accounts'
      ? t('settings.general.autoSwitchAccountScopeAllSummary', {
          total: accounts.length,
        })
      : t('settings.general.autoSwitchAccountScopeSummary', {
          selected: availableSelectedAccountIds.length,
          total: accounts.length,
        });

  const renderSelectionPanel = (panelMode: 'inline' | 'dialog') => (
    <div
      className={`settings-account-scope-panel ${
        panelMode === 'dialog' ? 'settings-account-scope-panel--dialog' : ''
      }`}
    >
      <div className="settings-account-scope-toolbar">
        <label className="settings-account-scope-select-all">
          <input
            ref={selectAllRef}
            type="checkbox"
            checked={allVisibleSelected}
            disabled={visibleAccountIds.length === 0}
            onChange={handleToggleAllVisible}
          />
          <span>{t('wakeup.verification.actions.selectAllAccounts')}</span>
        </label>
        <input
          type="text"
          className="settings-input settings-account-scope-search"
          placeholder={t('accounts.search')}
          value={searchQuery}
          onChange={(event) => setSearchQuery(event.target.value)}
        />
        <div className="settings-account-scope-filters">
          {typeOptions && typeOptions.length > 0 && (
            <MultiSelectFilterDropdown
              options={typeOptions}
              selectedValues={typeFilter}
              allLabel={t('wakeup.verification.filters.typeShort')}
              filterLabel={t('wakeup.verification.filters.typeShort')}
              clearLabel={t('accounts.clearFilter')}
              emptyLabel={t('common.none')}
              ariaLabel={t('wakeup.verification.filters.typeShort')}
              onToggleValue={(value) => setTypeFilter((prev) => toggleMultiValue(prev, value))}
              onClear={() => setTypeFilter([])}
            />
          )}
          <MultiSelectFilterDropdown
            options={availableTagOptions}
            selectedValues={tagFilter}
            allLabel={t('wakeup.verification.filters.tagsShort')}
            filterLabel={t('wakeup.verification.filters.tagsShort')}
            clearLabel={t('accounts.clearFilter')}
            emptyLabel={t('accounts.noAvailableTags')}
            ariaLabel={t('wakeup.verification.filters.tagsShort')}
            onToggleValue={(value) => setTagFilter((prev) => toggleMultiValue(prev, value))}
            onClear={() => setTagFilter([])}
          />
          <MultiSelectFilterDropdown
            options={groupOptions}
            selectedValues={groupFilter}
            allLabel={t('wakeup.verification.filters.groupsShort')}
            filterLabel={t('wakeup.verification.filters.groupsShort')}
            clearLabel={t('accounts.clearFilter')}
            emptyLabel={t('accounts.groups.noGroups')}
            ariaLabel={t('wakeup.verification.filters.groupsShort')}
            onToggleValue={(value) => setGroupFilter((prev) => toggleMultiValue(prev, value))}
            onClear={() => setGroupFilter([])}
          />
        </div>
        <span className="settings-account-scope-count">
          {selectedVisibleCount}/{visibleAccountIds.length}
          {visibleAccountIds.length !== accounts.length
            ? ` · ${availableSelectedAccountIds.length}/${accounts.length}`
            : ''}
        </span>
      </div>

      <div className="settings-account-scope-list">
        {accounts.length === 0 ? (
          <span className="settings-account-scope-empty">{t('common.shared.empty.title')}</span>
        ) : filteredAccounts.length === 0 ? (
          <span className="settings-account-scope-empty">{t('accounts.noMatch.title')}</span>
        ) : (
          filteredAccounts.map((account) => {
            const isSelected = selectedSet.has(account.id);
            return (
              <label
                key={account.id}
                className={`settings-account-scope-item ${isSelected ? 'selected' : ''}`}
              >
                <input
                  type="checkbox"
                  checked={isSelected}
                  onChange={() =>
                    onSelectedAccountIdsChange(
                      toggleMultiValue(availableSelectedAccountIds, account.id),
                    )
                  }
                />
                <span className="settings-account-scope-item-label" title={account.label}>
                  {account.label}
                </span>
                {account.badge}
              </label>
            );
          })
        )}
      </div>
    </div>
  );

  return (
    <div className={`settings-account-scope ${useDialog ? 'settings-account-scope--compact' : ''}`}>
      {useDialog ? (
        <div className="settings-account-scope-compact-row">
          <span className="settings-account-scope-compact-summary">{accountScopeSummaryText}</span>
          <button
            type="button"
            className="btn btn-secondary"
            onClick={() => setDialogOpen(true)}
          >
            {t('settings.general.autoSwitchAccountScopeChoose')}
          </button>
        </div>
      ) : (
        <>
          <div className="settings-account-scope-mode">
            <button
              type="button"
              className={`btn btn-secondary ${mode === 'all_accounts' ? 'is-active' : ''}`}
              onClick={() => onModeChange('all_accounts')}
            >
              {t('settings.general.autoSwitchAccountScopeAll')}
            </button>
            <button
              type="button"
              className={`btn btn-secondary ${mode === 'selected_accounts' ? 'is-active' : ''}`}
              onClick={() => onModeChange('selected_accounts')}
            >
              {t('settings.general.autoSwitchAccountScopeSelected')}
            </button>
          </div>

          {mode === 'selected_accounts' && renderSelectionPanel('inline')}
        </>
      )}

      {useDialog &&
        dialogOpen &&
        createPortal(
          <div
            className="settings-account-scope-dialog-overlay"
          >
            <div
              className="settings-account-scope-dialog"
              onClick={(event) => event.stopPropagation()}
            >
              <div className="settings-account-scope-dialog-header">
                <div className="settings-account-scope-dialog-title">
                  {t('settings.general.autoSwitchAccountScopeDialogTitle')}
                </div>
                <button
                  type="button"
                  className="btn btn-secondary"
                  onClick={() => setDialogOpen(false)}
                >
                  {t('common.close')}
                </button>
              </div>

              <div className="settings-account-scope-mode settings-account-scope-mode--dialog">
                <button
                  type="button"
                  className={`btn btn-secondary ${mode === 'all_accounts' ? 'is-active' : ''}`}
                  onClick={() => onModeChange('all_accounts')}
                >
                  {t('settings.general.autoSwitchAccountScopeAll')}
                </button>
                <button
                  type="button"
                  className={`btn btn-secondary ${mode === 'selected_accounts' ? 'is-active' : ''}`}
                  onClick={() => onModeChange('selected_accounts')}
                >
                  {t('settings.general.autoSwitchAccountScopeSelected')}
                </button>
              </div>

              {mode === 'selected_accounts' ? (
                renderSelectionPanel('dialog')
              ) : (
                <div className="settings-account-scope-dialog-all-accounts-hint">
                  {t('settings.general.autoSwitchAccountScopeAllHint', {
                    total: accounts.length,
                  })}
                </div>
              )}

              <div className="settings-account-scope-dialog-footer">
                <span className="settings-account-scope-dialog-summary">
                  {accountScopeSummaryText}
                </span>
                <button
                  type="button"
                  className="btn btn-primary"
                  onClick={() => setDialogOpen(false)}
                >
                  {t('common.confirm')}
                </button>
              </div>
            </div>
          </div>,
          document.body,
        )}
    </div>
  );
}
