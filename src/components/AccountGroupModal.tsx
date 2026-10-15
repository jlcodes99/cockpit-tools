/**
 * 账号分组管理弹窗
 * - 创建 / 重命名 / 删除分组
 * - 显示分组列表及账号数量
 */

import { useState, useEffect, useCallback, useRef, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { X, FolderOpen, Plus, Pencil, Trash2, FolderPlus, AlertCircle, GripVertical, ChevronUp, ChevronDown, Search } from 'lucide-react';
import {
  AccountGroup,
  getPlatformGroups,
  createPlatformGroup,
  deletePlatformGroup,
  renamePlatformGroup,
  assignAccountsToPlatformGroup,
  setPlatformGroupAccounts,
  reorderPlatformGroups,
  normalizePlatform,
} from '../services/platformGroupService';
import { invalidateCache as invalidateLegacyCache } from '../services/accountGroupService';
import { listAccounts } from '../services/accountService';
import { getAntigravityTierBadge } from '../utils/account';
import { SingleSelectDropdown } from './SingleSelectDropdown';
import {
  setCodexGroupQuotaAutoRefreshMinutes,
  resolveCodexGroupQuotaAutoRefreshMinutes,
  normalizeCodexGroupQuotaAutoRefreshMinutes,
  type CodexGroupQuotaAutoRefreshMinutes,
} from '../services/codexAccountGroupService';
import { useEscClose } from '../hooks/useEscClose';
import './AccountGroupModal.css';
import './GroupAccountPickerModal.css';

function getGroupIndexAtPoint(clientX: number, clientY: number, container: HTMLElement | null): number | null {
  const element = document.elementFromPoint(clientX, clientY);
  if (element) {
    const itemElement = element.closest('[data-group-index]');
    if (itemElement) {
      const idx = Number(itemElement.getAttribute('data-group-index'));
      if (!isNaN(idx)) return idx;
    }
  }
  if (container) {
    const items = container.querySelectorAll<HTMLElement>('[data-group-index]');
    for (const item of items) {
      const rect = item.getBoundingClientRect();
      if (clientY >= rect.top && clientY <= rect.bottom) {
        const idx = Number(item.getAttribute('data-group-index'));
        if (!isNaN(idx)) return idx;
      }
    }
    if (items.length > 0) {
      const firstRect = items[0].getBoundingClientRect();
      if (clientY < firstRect.top) return 0;
      const lastRect = items[items.length - 1].getBoundingClientRect();
      if (clientY > lastRect.bottom) return items.length - 1;
    }
  }
  return null;
}

// ─── 分组管理弹窗 ──────────────────────────────────────────

interface AccountGroupModalProps {
  isOpen: boolean;
  onClose: () => void;
  onGroupsChanged: () => Promise<void> | void;
  /** 平台标识（默认 antigravity） */
  platform?: string;
  /** 当前被勾选用于筛选的分组 ID 列表 */
  groupFilter?: string[];
  /** 切换某个分组的筛选状态 */
  onToggleGroupFilter?: (groupId: string) => void;
  /** 清空分组筛选 */
  onClearGroupFilter?: () => void;
  /** 点击添加账号回调（可选，优先由外部处理；若未提供则使用内置通用账号选择器） */
  onAddAccounts?: (group: AccountGroup) => void;
  /** 可选账号列表（用于通用添加账号弹窗） */
  accounts?: Array<{ id: string; [key: string]: any }>;
}

export const AccountGroupModal = ({
  isOpen, onClose, onGroupsChanged, platform,
  onAddAccounts, accounts,
}: AccountGroupModalProps) => {
  const { t } = useTranslation();
  useEscClose(isOpen, onClose);
  const platformKey = normalizePlatform(platform || 'antigravity');
  const [groups, setGroups] = useState<AccountGroup[]>([]);
  const [newName, setNewName] = useState('');
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState('');
  const [deleteConfirmId, setDeleteConfirmId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pickerTargetGroup, setPickerTargetGroup] = useState<AccountGroup | null>(null);
  const [loadedAccounts, setLoadedAccounts] = useState<Array<{ id: string; [key: string]: any }>>([]);
  const [fieldError, setFieldError] = useState<{ id: string; message: string } | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadFailed, setLoadFailed] = useState(false);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const errorRef = useRef<HTMLDivElement>(null);
  const modalRef = useRef<HTMLDivElement>(null);
  const [quotaCustomModeId, setQuotaCustomModeId] = useState<string | null>(null);
  const [quotaCustomDraft, setQuotaCustomDraft] = useState('5');
  const loadGeneration = useRef(0);

  const listRef = useRef<HTMLDivElement>(null);
  const dragSourceIndexRef = useRef<number | null>(null);
  const dragStartPosRef = useRef<{ x: number; y: number } | null>(null);
  const [draggingIndex, setDraggingIndex] = useState<number | null>(null);
  const [dragOverIndex, setDragOverIndex] = useState<number | null>(null);

  const clearErrors = () => {
    setError(null);
    setFieldError(null);
  };

  const reload = useCallback(async () => {
    const generation = loadGeneration.current;
    const nextGroups = await getPlatformGroups(platformKey);
    if (generation === loadGeneration.current) setGroups(nextGroups);
  }, [platformKey]);

  const loadGroups = useCallback(async () => {
    const generation = ++loadGeneration.current;
    setLoading(true);
    setError(null);
    setFieldError(null);
    try {
      await reload();
      if (generation === loadGeneration.current) setLoadFailed(false);
    } catch (err) {
      if (generation === loadGeneration.current) {
        setLoadFailed(true);
        setError(`${t('common.failed')}: ${String(err)}`);
      }
    } finally {
      if (generation === loadGeneration.current) setLoading(false);
    }
  }, [reload, t]);

  useEffect(() => {
    if (accounts && accounts.length > 0) {
      setLoadedAccounts(accounts);
    } else if (platformKey === 'antigravity' && isOpen) {
      listAccounts().then((accs) => setLoadedAccounts(accs)).catch(console.error);
    } else {
      setLoadedAccounts(accounts || []);
    }
  }, [accounts, platformKey, isOpen]);

  useEffect(() => {
    if (isOpen) {
      setGroups([]);
      void loadGroups();
      setNewName('');
      setRenamingId(null);
      setDeleteConfirmId(null);
      setPickerTargetGroup(null);
      setQuotaCustomModeId(null);
      dragSourceIndexRef.current = null;
      dragStartPosRef.current = null;
      setDraggingIndex(null);
      setDragOverIndex(null);
    }
    return () => { loadGeneration.current += 1; };
  }, [isOpen, loadGroups]);

  useEffect(() => {
    const target = fieldError
      ? Array.from(modalRef.current?.querySelectorAll<HTMLElement>('[data-error-field]') ?? [])
        .find((element) => element.dataset.errorField === fieldError.id)
      : errorRef.current;
    if (error || fieldError) {
      target?.scrollIntoView({ block: 'nearest' });
      target?.focus({ preventScroll: true });
    }
  }, [error, fieldError]);

  const runMutation = async (operation: () => Promise<void>, errorKey: string) => {
    if (busyRef.current || loading || loadFailed) return;
    busyRef.current = true;
    setBusy(true);
    clearErrors();
    try {
      await operation();
      await onGroupsChanged();
    } catch (err) {
      setError(errorKey === 'common.failed'
        ? `${t(errorKey)}: ${String(err)}`
        : t(errorKey, { error: String(err) }));
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  };

  const saveOrder = async (nextGroups: AccountGroup[]) => {
    await runMutation(async () => {
      setGroups(await reorderPlatformGroups(platformKey, nextGroups.map((group) => group.id)));
    }, 'common.failed');
  };

  const handleItemPointerDown = (e: React.PointerEvent, index: number) => {
    if (e.button !== 0 || busyRef.current || loading || loadFailed || renamingId !== null) return;
    const target = e.target as HTMLElement;
    if (target.closest('button, input, textarea, .group-actions, .group-modal-item-meta')) return;
    try { e.currentTarget.setPointerCapture(e.pointerId); } catch { /* Capture is optional. */ }
    dragSourceIndexRef.current = index;
    dragStartPosRef.current = { x: e.clientX, y: e.clientY };
  };

  const handleItemPointerMove = (e: React.PointerEvent) => {
    if (dragSourceIndexRef.current === null || !dragStartPosRef.current) return;
    const dist = Math.hypot(e.clientX - dragStartPosRef.current.x, e.clientY - dragStartPosRef.current.y);
    if (dist > 4) {
      if (draggingIndex === null) setDraggingIndex(dragSourceIndexRef.current);
      const idx = getGroupIndexAtPoint(e.clientX, e.clientY, listRef.current);
      if (idx !== null && idx !== dragOverIndex) setDragOverIndex(idx);
    }
  };

  const handleItemPointerCancel = (e: React.PointerEvent) => {
    try { e.currentTarget.releasePointerCapture(e.pointerId); } catch { /* Capture may already be released. */ }
    dragSourceIndexRef.current = null;
    dragStartPosRef.current = null;
    setDraggingIndex(null);
    setDragOverIndex(null);
  };

  const handleItemPointerUp = async (e: React.PointerEvent) => {
    const sourceIdx = dragSourceIndexRef.current;
    const targetIdx = dragOverIndex;
    handleItemPointerCancel(e);
    if (sourceIdx !== null && targetIdx !== null && sourceIdx !== targetIdx && targetIdx >= 0 && targetIdx < groups.length) {
      const nextGroups = [...groups];
      const [moved] = nextGroups.splice(sourceIdx, 1);
      nextGroups.splice(targetIdx, 0, moved);
      await saveOrder(nextGroups);
    }
  };

  const handleMove = async (index: number, direction: 'up' | 'down') => {
    const targetIndex = direction === 'up' ? index - 1 : index + 1;
    if (targetIndex < 0 || targetIndex >= groups.length) return;
    const nextGroups = [...groups];
    [nextGroups[index], nextGroups[targetIndex]] = [nextGroups[targetIndex], nextGroups[index]];
    await saveOrder(nextGroups);
  };

  const handleCreate = async () => {
    const name = newName.trim();
    if (!name || busyRef.current || loading || loadFailed) return;
    clearErrors();
    if (groups.some((group) => group.name === name)) {
      setFieldError({ id: 'new', message: t('accounts.groups.error.duplicate') });
      return;
    }
    await runMutation(async () => {
      await createPlatformGroup(platformKey, name);
      setNewName('');
      await reload();
    }, 'accounts.groups.error.createFailed');
  };

  const handleRename = async (groupId: string) => {
    const name = renameValue.trim();
    if (!name || busyRef.current || loading || loadFailed) return;
    clearErrors();
    if (groups.some((group) => group.id !== groupId && group.name === name)) {
      setFieldError({ id: groupId, message: t('accounts.groups.error.duplicate') });
      return;
    }
    await runMutation(async () => {
      await renamePlatformGroup(platformKey, groupId, name);
      setRenamingId(null);
      await reload();
    }, 'accounts.groups.error.renameFailed');
  };

  const handleDelete = async (groupId: string) => {
    await runMutation(async () => {
      await deletePlatformGroup(platformKey, groupId);
      setDeleteConfirmId(null);
      await reload();
    }, 'accounts.groups.error.deleteFailed');
  };

  const applyQuotaMinutes = async (group: AccountGroup, minutes: CodexGroupQuotaAutoRefreshMinutes) => {
    await runMutation(async () => {
      const updated = await setCodexGroupQuotaAutoRefreshMinutes(group.id, minutes);
      if (!updated) throw new Error(t('accounts.groups.error.notFound'));
      setGroups((previous) => previous.map((item) => item.id === group.id ? updated : item));
      setQuotaCustomModeId(null);
    }, 'accounts.groups.error.quotaRefreshFailed');
  };

  const handleQuotaSelectChange = (group: AccountGroup, value: string) => {
    clearErrors();
    if (value === 'custom') {
      const current = resolveCodexGroupQuotaAutoRefreshMinutes(group as AccountGroup & { quotaAutoRefreshMinutes?: unknown });
      setQuotaCustomDraft(typeof current === 'number' && current > 0 ? String(current) : '5');
      setQuotaCustomModeId(group.id);
      return;
    }
    void applyQuotaMinutes(group, value === 'inherit' ? null : Number(value));
  };

  const handleQuotaCustomApply = (group: AccountGroup) => {
    const normalized = normalizeCodexGroupQuotaAutoRefreshMinutes(quotaCustomDraft);
    void applyQuotaMinutes(group, normalized === null || normalized === -1 ? 5 : normalized);
  };

  const quotaSelectOptions = [
    { value: 'inherit', label: t('accounts.groups.quotaRefreshInherit') },
    { value: '-1', label: t('settings.general.autoRefreshDisabled') },
    ...[2, 5, 10, 15].map((minutes) => ({ value: String(minutes), label: `${minutes} ${t('settings.general.minutes')}` })),
    { value: 'custom', label: t('settings.general.autoRefreshCustom') },
  ];

  if (!isOpen) return null;

  return (
    <div className="modal-overlay">
      <div ref={modalRef} className="modal account-group-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h2>
            <FolderOpen size={18} />
            {t('accounts.groups.manageTitle')}
          </h2>
          <button className="modal-close" onClick={onClose}>
            <X size={18} />
          </button>
        </div>

        <div className="modal-body">
          {/* 创建分组 */}
          <div className="group-create-row">
            <input
              type="text"
              data-error-field="new"
              aria-invalid={fieldError?.id === 'new'}
              disabled={busy || loading || loadFailed}
              value={newName}
              onChange={(e) => { setNewName(e.target.value); clearErrors(); }}
              onKeyDown={(e) => { if (e.key === 'Enter') handleCreate(); }}
              placeholder={t('accounts.groups.newPlaceholder')}
              maxLength={30}
            />
            <button
              className="btn btn-primary"
              onClick={handleCreate}
              disabled={busy || loading || loadFailed || !newName.trim()}
            >
              <Plus size={14} />
              {t('accounts.groups.create')}
            </button>
          </div>

          {fieldError?.id === 'new' && <div className="group-field-error" role="alert">{fieldError.message}</div>}

          {/* 错误提示 */}
          {error && (
            <div className="group-modal-error" ref={errorRef} tabIndex={-1} role="alert">
              <AlertCircle size={14} />
              <span>{error}</span>
              {loadFailed && <button className="btn btn-secondary" disabled={loading} onClick={() => void loadGroups()}>{t('common.retry')}</button>}
            </div>
          )}

          {/* 分组列表 */}
          {loading ? <div role="status">{t('common.loading')}</div> : !loadFailed && groups.length === 0 ? (
            <div className="group-modal-empty">
              <FolderPlus size={36} />
              <div>{t('accounts.groups.empty')}</div>
            </div>
          ) : (
            <div className="group-modal-list" ref={listRef}>
              {groups.map((group, index) => {
                const quotaMinutes = resolveCodexGroupQuotaAutoRefreshMinutes(group as AccountGroup & { quotaAutoRefreshMinutes?: unknown });
                const quotaValue = quotaMinutes === null ? 'inherit' : String(quotaMinutes);
                const quotaOptions = quotaSelectOptions.some((option) => option.value === quotaValue)
                  ? quotaSelectOptions
                  : [{ value: quotaValue, label: `${quotaMinutes} ${t('settings.general.minutes')}` }, ...quotaSelectOptions];
                return (
                <div
                  key={group.id}
                  data-group-index={index}
                  className={`group-modal-item ${draggingIndex === index ? 'is-dragging' : ''} ${dragOverIndex === index && draggingIndex !== null && draggingIndex !== index ? 'drop-target' : ''}`}
                  onPointerDown={(e) => handleItemPointerDown(e, index)}
                  onPointerMove={handleItemPointerMove}
                  onPointerUp={handleItemPointerUp}
                  onPointerCancel={handleItemPointerCancel}
                >
                  <div className="group-modal-item-main">
                    {/* 拖动手柄 */}
                    <div
                      className="group-drag-handle"
                      title={t('accounts.groups.dragToSort', '按住拖动排序')}
                    >
                      <GripVertical size={14} />
                    </div>

                    <FolderOpen size={18} className="group-icon" />
                    <div className="group-info">
                      {renamingId === group.id ? (
                        <input
                          className="group-rename-input"
                          data-error-field={group.id}
                          aria-invalid={fieldError?.id === group.id}
                          disabled={busy || loading || loadFailed}
                          value={renameValue}
                          onChange={(e) => { setRenameValue(e.target.value); clearErrors(); }}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter') handleRename(group.id);
                            if (e.key === 'Escape') { e.stopPropagation(); clearErrors(); setRenamingId(null); }
                          }}
                          autoFocus
                          maxLength={30}
                        />
                      ) : (
                        <>
                          <span className="group-name">{group.name}</span>
                          <span className="group-count">
                            {t('accounts.groups.accountCount', {
                              count: group.accountIds.length,
                            })}
                          </span>
                        </>
                      )}
                      {fieldError?.id === group.id && <span className="group-field-error" role="alert">{fieldError.message}</span>}
                    </div>
                    <div className="group-actions">
                      {renamingId === group.id ? (
                        <><button className="group-action-btn" disabled={busy} onClick={() => void handleRename(group.id)} title={t('common.confirm')}>✓</button><button className="group-action-btn" disabled={busy} onClick={() => { clearErrors(); setRenamingId(null); }} title={t('common.cancel')}>✗</button></>
                      ) : deleteConfirmId === group.id ? (
                        <>
                          <button
                            className="group-action-btn danger"
                            disabled={busy || loading || loadFailed}
                            onClick={() => handleDelete(group.id)}
                            title={t('common.confirm')}
                          >
                            ✓
                          </button>
                          <button
                            className="group-action-btn"
                            disabled={busy || loading || loadFailed}
                            onClick={() => { clearErrors(); setDeleteConfirmId(null); }}
                            title={t('common.cancel')}
                          >
                            ✗
                          </button>
                        </>
                      ) : (
                        <>
                          <button
                            type="button"
                            className="group-action-btn add-btn"
                            disabled={busy || loading || loadFailed}
                            onClick={() => {
                              if (onAddAccounts) {
                                onAddAccounts(group);
                              } else {
                                setPickerTargetGroup(group);
                              }
                            }}
                            title={t('accounts.groups.addAccounts', '添加账号')}
                          >
                            <FolderPlus size={14} />
                            <span>{t('accounts.groups.addAccounts', '添加账号')}</span>
                          </button>
                          <button
                            type="button"
                            className="group-action-btn"
                            disabled={busy || loading || loadFailed || index === 0}
                            onClick={() => handleMove(index, 'up')}
                            title={t('accounts.groups.moveUp', '上移')}
                          >
                            <ChevronUp size={14} />
                          </button>
                          <button
                            type="button"
                            className="group-action-btn"
                            disabled={busy || loading || loadFailed || index === groups.length - 1}
                            onClick={() => handleMove(index, 'down')}
                            title={t('accounts.groups.moveDown', '下移')}
                          >
                            <ChevronDown size={14} />
                          </button>
                          <button
                            className="group-action-btn"
                            disabled={busy || loading || loadFailed}
                            onClick={() => {
                              clearErrors();
                              setRenamingId(group.id);
                              setRenameValue(group.name);
                            }}
                            title={t('accounts.groups.rename')}
                          >
                            <Pencil size={14} />
                          </button>
                          <button
                            className="group-action-btn danger"
                            disabled={busy || loading || loadFailed}
                            onClick={() => { clearErrors(); setDeleteConfirmId(group.id); }}
                            title={t('common.delete')}
                          >
                            <Trash2 size={14} />
                          </button>
                        </>
                      )}
                    </div>
                  </div>
                  {platformKey === 'codex' && (
                    <div className="group-modal-item-meta">
                      <span className="group-quota-meta-label" title={t('accounts.groups.quotaRefreshPolicyHint')}>{t('accounts.groups.quotaRefresh')}</span>
                      {quotaCustomModeId === group.id ? (
                        <div className="group-quota-custom-input">
                          <input type="number" min={1} max={999} className="group-quota-custom-field"
                            value={quotaCustomDraft} disabled={busy || loading || loadFailed}
                            aria-label={t('accounts.groups.quotaRefresh')}
                            onChange={(event) => { clearErrors(); setQuotaCustomDraft(event.target.value.replace(/[^\d]/g, '')); }}
                            onKeyDown={(event) => {
                              if (event.key === 'Enter') { event.preventDefault(); handleQuotaCustomApply(group); }
                              if (event.key === 'Escape') { event.stopPropagation(); clearErrors(); setQuotaCustomModeId(null); }
                            }} autoFocus />
                          <span className="group-quota-unit">{t('settings.general.minutes')}</span>
                          <button className="group-action-btn" disabled={busy} title={t('common.confirm')} onClick={() => handleQuotaCustomApply(group)}>✓</button>
                          <button className="group-action-btn" disabled={busy} title={t('common.cancel')} onClick={() => { clearErrors(); setQuotaCustomModeId(null); }}>✗</button>
                        </div>
                      ) : <SingleSelectDropdown className="group-quota-dropdown" menuClassName="group-quota-dropdown-menu"
                        value={quotaValue} options={quotaOptions} disabled={busy || loading || loadFailed || deleteConfirmId === group.id}
                        ariaLabel={t('accounts.groups.quotaRefresh')} menuWidth={168} menuMaxHeight={260}
                        onChange={(value) => handleQuotaSelectChange(group, value)} />}
                    </div>
                  )}
                </div>
                );
              })}
            </div>
          )}
        </div>

        <div className="modal-footer">
          <button className="btn btn-secondary" onClick={onClose}>
            {t('common.close')}
          </button>
        </div>
      </div>

      <UniversalGroupAccountPickerModal
        isOpen={!!pickerTargetGroup}
        targetGroup={pickerTargetGroup}
        accounts={loadedAccounts}
        accountGroups={groups}
        platform={platformKey}
        onClose={() => setPickerTargetGroup(null)}
        onConfirm={async ({ accountIds }) => {
          if (!pickerTargetGroup) return;
          await setPlatformGroupAccounts(
            platformKey,
            pickerTargetGroup.id,
            accountIds
          );
          if (platformKey === 'antigravity') {
            invalidateLegacyCache();
          }
          await reload();
          await onGroupsChanged();
        }}
      />
    </div>
  );
};

// ─── 通用添加账号到分组弹窗 ──────────────────────────────────

export interface UniversalGroupAccountPickerModalProps {
  isOpen: boolean;
  targetGroup: AccountGroup | null;
  accounts: Array<{ id: string; [key: string]: any }>;
  accountGroups: AccountGroup[];
  platform?: string;
  onClose: () => void;
  onConfirm: (payload: { accountIds: string[] }) => Promise<void> | void;
}

export function UniversalGroupAccountPickerModal({
  isOpen,
  targetGroup,
  accounts,
  accountGroups,
  onClose,
  onConfirm,
}: UniversalGroupAccountPickerModalProps) {
  const { t } = useTranslation();
  useEscClose(isOpen, onClose);
  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const selectAllCheckboxRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (!isOpen || !targetGroup) return;
    setQuery('');
    const validIds = new Set(accounts.map((a) => a.id));
    setSelected(new Set((targetGroup.accountIds || []).filter((id) => validIds.has(id))));
    setError('');
  }, [isOpen, targetGroup, accounts]);

  const groupsByAccountId = useMemo(() => {
    const result = new Map<string, AccountGroup[]>();
    for (const group of accountGroups) {
      for (const accountId of group.accountIds) {
        const list = result.get(accountId) || [];
        list.push(group);
        result.set(accountId, list);
      }
    }
    return result;
  }, [accountGroups]);

  const visibleAccounts = useMemo(() => {
    if (!targetGroup) return [];
    const queryText = query.trim().toLowerCase();
    let next = [...accounts].sort((a, b) => {
      const aName = (a.email || a.name || a.id || '').toLowerCase();
      const bName = (b.email || b.name || b.id || '').toLowerCase();
      return aName.localeCompare(bName);
    });

    if (!queryText) return next;

    return next.filter((account) => {
      const email = (account.email || '').toLowerCase();
      const name = (account.name || account.displayName || account.username || '').toLowerCase();
      const groupNames = (groupsByAccountId.get(account.id) || [])
        .map((g) => g.name.toLowerCase())
        .join(' ');
      return (
        email.includes(queryText) ||
        name.includes(queryText) ||
        account.id.toLowerCase().includes(queryText) ||
        groupNames.includes(queryText)
      );
    });
  }, [accounts, groupsByAccountId, query, targetGroup]);

  const selectedVisibleCount = useMemo(
    () =>
      visibleAccounts.reduce(
        (count, account) => count + (selected.has(account.id) ? 1 : 0),
        0
      ),
    [selected, visibleAccounts]
  );

  const allVisibleSelected =
    visibleAccounts.length > 0 && selectedVisibleCount === visibleAccounts.length;

  useEffect(() => {
    if (!selectAllCheckboxRef.current) return;
    selectAllCheckboxRef.current.indeterminate =
      selectedVisibleCount > 0 && !allVisibleSelected;
  }, [allVisibleSelected, selectedVisibleCount]);

  const toggleSelectAllVisible = () => {
    if (saving || visibleAccounts.length === 0) return;
    setSelected((prev) => {
      const next = new Set(prev);
      if (allVisibleSelected) {
        for (const account of visibleAccounts) {
          next.delete(account.id);
        }
      } else {
        for (const account of visibleAccounts) {
          next.add(account.id);
        }
      }
      return next;
    });
  };

  const toggleSelect = (accountId: string) => {
    if (saving) return;
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(accountId)) {
        next.delete(accountId);
      } else {
        next.add(accountId);
      }
      return next;
    });
  };

  const handleConfirm = async () => {
    if (!targetGroup || saving) return;
    setSaving(true);
    setError('');
    try {
      await onConfirm({
        accountIds: Array.from(selected),
      });
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setSaving(false);
    }
  };

  if (!isOpen || !targetGroup) return null;

  return (
    <div className="modal-overlay" style={{ zIndex: 10050 }}>
      <div className="modal group-account-picker-modal" onClick={(event) => event.stopPropagation()}>
        <div className="modal-header">
          <h2 className="group-account-picker-title">
            <FolderPlus size={18} />
            <span>{t('accounts.groups.addAccounts', '添加账号')}</span>
            <span className="group-account-picker-target">{targetGroup.name}</span>
          </h2>
          <button
            className="modal-close"
            onClick={onClose}
            aria-label={t('common.close', '关闭')}
          >
            <X size={18} />
          </button>
        </div>

        <div className="modal-body group-account-picker-body">
          <div className="group-account-toolbar">
            <div className="group-account-search">
              <Search size={16} className="group-account-search-icon" />
              <input
                type="text"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder={t('accounts.search', '搜索账号...')}
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
            <div className="group-account-main">
              <span className="group-account-email" style={{ fontWeight: 600, fontSize: '12px', color: 'var(--text-secondary)' }}>
                {t('common.selectAll', '全选')} ({selectedVisibleCount}/{visibleAccounts.length})
              </span>
            </div>
          </div>

          <div className="group-account-list">
            {visibleAccounts.length === 0 ? (
              <div className="group-account-empty">{t('accounts.groups.accountPickerEmpty', '没有符合条件的账号')}</div>
            ) : (
              visibleAccounts.map((account) => {
                const currentGroups = groupsByAccountId.get(account.id) || [];
                const isChecked = selected.has(account.id);
                const isUngrouped = currentGroups.length === 0;

                const email = account.email || account.account_name || account.account_id || '';
                const name = account.name || account.displayName || account.username || '';
                let displayName = email;
                if (email && name && email !== name) {
                  displayName = `${email} (${name})`;
                } else if (!displayName) {
                  displayName = name || account.id || '';
                }

                let planLabel = '';
                let planClass = '';
                if (account.quota) {
                  const badge = getAntigravityTierBadge(account.quota);
                  if (badge.tier !== 'UNKNOWN') {
                    planLabel = badge.label;
                    planClass = badge.className;
                  }
                } else if (account.plan_type) {
                  planLabel = String(account.plan_type).toUpperCase();
                  planClass = 'plan-badge-default';
                } else if (account.subscription_tier) {
                  planLabel = String(account.subscription_tier).toUpperCase();
                  planClass = 'plan-badge-default';
                } else if (account.plan) {
                  planLabel = String(account.plan).toUpperCase();
                  planClass = 'plan-badge-default';
                }

                return (
                  <label
                    key={account.id}
                    className={`group-account-item${isChecked ? ' is-current' : ''}`}
                  >
                    <input
                      type="checkbox"
                      checked={isChecked}
                      disabled={saving}
                      onChange={() => toggleSelect(account.id)}
                    />
                    <div className="group-account-main">
                      <span className="group-account-email" title={displayName}>
                        {displayName}
                      </span>
                      <div className="group-account-meta">
                        {planLabel && (
                          <span className={`tier-badge ${planClass} group-account-tier-badge`}>
                            {planLabel}
                          </span>
                        )}
                        {isUngrouped ? (
                          <span className="group-account-badge is-ungrouped">
                            {t('accounts.groups.ungrouped', '未分组')}
                          </span>
                        ) : (
                          currentGroups.map((g) => (
                            <span
                              key={g.id}
                              className={`group-account-badge${g.id === targetGroup.id ? ' is-current-target' : ''}`}
                            >
                              {g.name}
                            </span>
                          ))
                        )}
                      </div>
                    </div>
                  </label>
                );
              })
            )}
          </div>

          {error && <div className="group-account-error">{error}</div>}
        </div>

        <div className="modal-footer group-account-picker-footer">
          <button className="btn btn-secondary" onClick={onClose} disabled={saving}>
            {t('common.cancel', '取消')}
          </button>
          <button
            className="btn btn-primary"
            onClick={handleConfirm}
            disabled={saving}
          >
            {saving
              ? t('common.saving', '保存中...')
              : `${t('common.save', '保存')} (${selected.size})`}
          </button>
        </div>
      </div>
    </div>
  );
}

// ─── 添加到分组弹窗 ──────────────────────────────────────────

interface AddToGroupModalProps {
  isOpen: boolean;
  onClose: () => void;
  accountIds: string[];
  sourceGroupId?: string;
  onAdded: () => Promise<void> | void;
  platform?: string;
}

export const AddToGroupModal = ({ isOpen, onClose, accountIds, sourceGroupId, onAdded, platform }: AddToGroupModalProps) => {
  const { t } = useTranslation();
  useEscClose(isOpen, onClose);
  const platformKey = normalizePlatform(platform || 'antigravity');
  const [groups, setGroups] = useState<AccountGroup[]>([]);
  const [newName, setNewName] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [fieldError, setFieldError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadFailed, setLoadFailed] = useState(false);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const errorRef = useRef<HTMLDivElement>(null);
  const nameRef = useRef<HTMLInputElement>(null);
  const loadGeneration = useRef(0);
  // If assignment fails after creation, retry the same group instead of creating another.
  const createdGroupRef = useRef<AccountGroup | null>(null);

  const clearErrors = () => { setError(null); setFieldError(null); };
  const loadGroups = useCallback(async () => {
    const generation = ++loadGeneration.current;
    setLoading(true);
    setError(null);
    setFieldError(null);
    try {
      const loaded = await getPlatformGroups(platformKey);
      if (generation === loadGeneration.current) {
        setGroups(loaded);
        setLoadFailed(false);
      }
    } catch (err) {
      if (generation === loadGeneration.current) {
        setLoadFailed(true);
        setError(`${t('common.failed')}: ${String(err)}`);
      }
    } finally {
      if (generation === loadGeneration.current) setLoading(false);
    }
  }, [platformKey, t]);

  useEffect(() => {
    if (isOpen) {
      setGroups([]);
      setNewName('');
      createdGroupRef.current = null;
      void loadGroups();
    }
    return () => { loadGeneration.current += 1; };
  }, [isOpen, loadGroups]);

  useEffect(() => {
    const target = fieldError ? nameRef.current : errorRef.current;
    if (fieldError || error) {
      target?.scrollIntoView({ block: 'nearest' });
      target?.focus({ preventScroll: true });
    }
  }, [fieldError, error]);

  const runAssignment = async (operation: () => Promise<string>, errorKey: string) => {
    if (busyRef.current || loading || loadFailed) return;
    busyRef.current = true;
    setBusy(true);
    clearErrors();
    try {
      const groupId = await operation();
      const updated = await assignAccountsToPlatformGroup(platformKey, groupId, accountIds);
      if (!updated) throw new Error(t('accounts.groups.error.notFound'));
      await onAdded();
      onClose();
    } catch (err) {
      setError(t(errorKey, { error: String(err) }));
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  };

  const handleSelect = async (groupId: string) => {
    await runAssignment(async () => groupId, 'accounts.groups.error.addFailed');
  };

  const handleCreateAndAdd = async () => {
    const name = newName.trim();
    if (!name || busyRef.current || loading || loadFailed) return;
    clearErrors();
    const created = createdGroupRef.current;
    if (groups.some((group) => group.name === name && group.id !== created?.id)) {
      setFieldError(t('accounts.groups.error.duplicate'));
      return;
    }
    await runAssignment(async () => {
      if (created?.name === name) return created.id;
      const group = await createPlatformGroup(platformKey, name);
      createdGroupRef.current = group;
      setGroups((previous) => [...previous, group]);
      return group.id;
    }, 'accounts.groups.error.createAndAddFailed');
  };

  if (!isOpen) return null;

  return (
    <div className="modal-overlay">
      <div className="modal add-to-group-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h2>
            <FolderPlus size={18} />
            {sourceGroupId ? t('accounts.groups.moveToGroup') : t('accounts.groups.addToGroup')}
          </h2>
          <button className="modal-close" onClick={onClose}>
            <X size={18} />
          </button>
        </div>

        <div className="modal-body">
          <div className="group-create-row">
            <input
              type="text"
              ref={nameRef}
              aria-invalid={Boolean(fieldError)}
              disabled={busy || loading || loadFailed}
              value={newName}
              onChange={(e) => { setNewName(e.target.value); clearErrors(); }}
              onKeyDown={(e) => { if (e.key === 'Enter') handleCreateAndAdd(); }}
              placeholder={t('accounts.groups.createAndAdd')}
              maxLength={30}
            />
            <button
              className="btn btn-primary"
              onClick={handleCreateAndAdd}
              disabled={busy || loading || loadFailed || !newName.trim()}
            >
              <Plus size={14} />
            </button>
          </div>

          {fieldError && <div className="group-field-error" role="alert">{fieldError}</div>}
          {error && (
            <div className="group-modal-error" ref={errorRef} role="alert" tabIndex={-1}>
              <AlertCircle size={14} />
              <span>{error}</span>
              {loadFailed && <button className="btn btn-secondary" disabled={loading} onClick={() => void loadGroups()}>{t('common.retry')}</button>}
            </div>
          )}
          {loading && <div role="status">{t('common.loading')}</div>}

          {groups.length > 0 && (
            <div className="add-to-group-list">
              {groups.filter((g) => g.id !== sourceGroupId).map((group) => (
                <button
                  type="button"
                  disabled={busy || loading || loadFailed}
                  key={group.id}
                  className="add-to-group-item"
                  onClick={() => handleSelect(group.id)}
                >
                  <FolderOpen size={16} className="group-icon" />
                  <span className="group-name">{group.name}</span>
                  <span className="group-count">
                    {group.accountIds.length}
                  </span>
                </button>
              ))}
            </div>
          )}

        </div>
      </div>
    </div>
  );
};

export default AccountGroupModal;
