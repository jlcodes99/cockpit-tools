/**
 * 账号分组管理弹窗
 * - 创建 / 重命名 / 删除分组
 * - 显示分组列表及账号数量
 */

import { useState, useEffect, useCallback, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { X, FolderOpen, Plus, Pencil, Trash2, FolderPlus, AlertCircle, GripVertical, ChevronUp, ChevronDown } from 'lucide-react';
import {
  AccountGroup,
  getPlatformGroups,
  createPlatformGroup,
  deletePlatformGroup,
  renamePlatformGroup,
  assignAccountsToPlatformGroup,
  reorderPlatformGroups,
  normalizePlatform,
} from '../services/platformGroupService';
import { invalidateCache as invalidateLegacyCache } from '../services/accountGroupService';
import { useEscClose } from '../hooks/useEscClose';
import './AccountGroupModal.css';

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
  /** 点击添加账号回调 */
  onAddAccounts?: (group: AccountGroup) => void;
}

export const AccountGroupModal = ({
  isOpen, onClose, onGroupsChanged, platform,
  onAddAccounts,
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

  const listRef = useRef<HTMLDivElement>(null);
  const dragSourceIndexRef = useRef<number | null>(null);
  const dragStartPosRef = useRef<{ x: number; y: number } | null>(null);
  const [draggingIndex, setDraggingIndex] = useState<number | null>(null);
  const [dragOverIndex, setDragOverIndex] = useState<number | null>(null);

  const reload = useCallback(async () => {
    setGroups(await getPlatformGroups(platformKey));
  }, [platformKey]);

  useEffect(() => {
    if (isOpen) {
      reload();
      setNewName('');
      setRenamingId(null);
      setDeleteConfirmId(null);
      dragSourceIndexRef.current = null;
      dragStartPosRef.current = null;
      setDraggingIndex(null);
      setDragOverIndex(null);
      setError(null);
    }
  }, [isOpen, reload]);

  const handleItemPointerDown = (e: React.PointerEvent, index: number) => {
    if (e.button !== 0) return;
    if (renamingId !== null) return;
    const target = e.target as HTMLElement;
    if (target.closest('button, input, textarea, .group-actions, .group-filter-checkbox')) {
      return;
    }

    try {
      e.currentTarget.setPointerCapture(e.pointerId);
    } catch {}
    dragSourceIndexRef.current = index;
    dragStartPosRef.current = { x: e.clientX, y: e.clientY };
  };

  const handleItemPointerMove = (e: React.PointerEvent) => {
    if (dragSourceIndexRef.current === null || !dragStartPosRef.current) return;
    const dist = Math.hypot(e.clientX - dragStartPosRef.current.x, e.clientY - dragStartPosRef.current.y);
    if (dist > 4) {
      if (draggingIndex === null) {
        setDraggingIndex(dragSourceIndexRef.current);
      }
      const idx = getGroupIndexAtPoint(e.clientX, e.clientY, listRef.current);
      if (idx !== null && idx !== dragOverIndex) {
        setDragOverIndex(idx);
      }
    }
  };

  const handleItemPointerUp = async (e: React.PointerEvent) => {
    try {
      e.currentTarget.releasePointerCapture(e.pointerId);
    } catch {}

    const sourceIdx = dragSourceIndexRef.current;
    const targetIdx = dragOverIndex;

    dragSourceIndexRef.current = null;
    dragStartPosRef.current = null;
    setDraggingIndex(null);
    setDragOverIndex(null);

    if (sourceIdx !== null && targetIdx !== null && sourceIdx !== targetIdx && targetIdx >= 0 && targetIdx < groups.length) {
      const nextGroups = [...groups];
      const [moved] = nextGroups.splice(sourceIdx, 1);
      nextGroups.splice(targetIdx, 0, moved);
      setGroups(nextGroups);
      try {
        await reorderPlatformGroups(platformKey, nextGroups.map((g) => g.id));
        if (platformKey === 'antigravity') {
          invalidateLegacyCache();
        }
        await onGroupsChanged();
      } catch (err) {
        console.error('Failed to reorder groups:', err);
        await reload();
      }
    }
  };

  const handleItemPointerCancel = (e: React.PointerEvent) => {
    try {
      e.currentTarget.releasePointerCapture(e.pointerId);
    } catch {}
    dragSourceIndexRef.current = null;
    dragStartPosRef.current = null;
    setDraggingIndex(null);
    setDragOverIndex(null);
  };

  const handleMove = async (index: number, direction: 'up' | 'down') => {
    const targetIndex = direction === 'up' ? index - 1 : index + 1;
    if (targetIndex < 0 || targetIndex >= groups.length) return;
    const nextGroups = [...groups];
    const temp = nextGroups[index];
    nextGroups[index] = nextGroups[targetIndex];
    nextGroups[targetIndex] = temp;
    setGroups(nextGroups);
    try {
      await reorderPlatformGroups(platformKey, nextGroups.map((g) => g.id));
      if (platformKey === 'antigravity') {
        invalidateLegacyCache();
      }
      await onGroupsChanged();
    } catch (err) {
      console.error('Failed to move group:', err);
      await reload();
    }
  };

  const handleCreate = async () => {
    const name = newName.trim();
    if (!name) return;
    setError(null);
    try {
      // 重名检查
      if (groups.some((g) => g.name === name)) {
        setError(t('accounts.groups.error.duplicate'));
        return;
      }
      await createPlatformGroup(platformKey, name);
      if (platformKey === 'antigravity') {
        invalidateLegacyCache();
      }
      setNewName('');
      await reload();
      await onGroupsChanged();
    } catch (err) {
      console.error('Failed to create group:', err);
      setError(t('accounts.groups.error.createFailed', {
        error: String(err),
      }));
    }
  };

  const handleRename = async (groupId: string) => {
    const name = renameValue.trim();
    if (!name) return;
    setError(null);
    try {
      // 重名检查（排除自己）
      if (groups.some((g) => g.id !== groupId && g.name === name)) {
        setError(t('accounts.groups.error.duplicate'));
        return;
      }
      await renamePlatformGroup(platformKey, groupId, name);
      if (platformKey === 'antigravity') {
        invalidateLegacyCache();
      }
      setRenamingId(null);
      await reload();
      await onGroupsChanged();
    } catch (err) {
      console.error('Failed to rename group:', err);
      setError(t('accounts.groups.error.renameFailed', {
        error: String(err),
      }));
    }
  };

  const handleDelete = async (groupId: string) => {
    setError(null);
    try {
      await deletePlatformGroup(platformKey, groupId);
      if (platformKey === 'antigravity') {
        invalidateLegacyCache();
      }
      setDeleteConfirmId(null);
      await reload();
      await onGroupsChanged();
    } catch (err) {
      console.error('Failed to delete group:', err);
      setError(t('accounts.groups.error.deleteFailed', {
        error: String(err),
      }));
    }
  };

  if (!isOpen) return null;

  return (
    <div className="modal-overlay">
      <div className="modal account-group-modal" onClick={(e) => e.stopPropagation()}>
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
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter') handleCreate(); }}
              placeholder={t('accounts.groups.newPlaceholder')}
              maxLength={30}
            />
            <button
              className="btn btn-primary"
              onClick={handleCreate}
              disabled={!newName.trim()}
            >
              <Plus size={14} />
              {t('accounts.groups.create')}
            </button>
          </div>

          {/* 错误提示 */}
          {error && (
            <div className="group-modal-error">
              <AlertCircle size={14} />
              <span>{error}</span>
            </div>
          )}

          {/* 分组列表 */}
          {groups.length === 0 ? (
            <div className="group-modal-empty">
              <FolderPlus size={36} />
              <div>{t('accounts.groups.empty')}</div>
            </div>
          ) : (
            <div className="group-modal-list" ref={listRef}>
              {groups.map((group, index) => (
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
                          value={renameValue}
                          onChange={(e) => setRenameValue(e.target.value)}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter') handleRename(group.id);
                            if (e.key === 'Escape') setRenamingId(null);
                          }}
                          onBlur={() => handleRename(group.id)}
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
                    </div>
                    <div className="group-actions">
                      {deleteConfirmId === group.id ? (
                        <>
                          <button
                            className="group-action-btn danger"
                            onClick={() => handleDelete(group.id)}
                            title={t('common.confirm')}
                          >
                            ✓
                          </button>
                          <button
                            className="group-action-btn"
                            onClick={() => setDeleteConfirmId(null)}
                            title={t('common.cancel')}
                          >
                            ✗
                          </button>
                        </>
                      ) : (
                        <>
                          {onAddAccounts && (
                            <button
                              type="button"
                              className="group-action-btn add-btn"
                              onClick={() => onAddAccounts(group)}
                              title={t('accounts.groups.addAccounts', '添加账号')}
                            >
                              <FolderPlus size={14} />
                              <span>{t('accounts.groups.addAccounts', '添加账号')}</span>
                            </button>
                          )}
                          <button
                            type="button"
                            className="group-action-btn"
                            disabled={index === 0}
                            onClick={() => handleMove(index, 'up')}
                            title={t('accounts.groups.moveUp', '上移')}
                          >
                            <ChevronUp size={14} />
                          </button>
                          <button
                            type="button"
                            className="group-action-btn"
                            disabled={index === groups.length - 1}
                            onClick={() => handleMove(index, 'down')}
                            title={t('accounts.groups.moveDown', '下移')}
                          >
                            <ChevronDown size={14} />
                          </button>
                          <button
                            className="group-action-btn"
                            onClick={() => {
                              setRenamingId(group.id);
                              setRenameValue(group.name);
                            }}
                            title={t('accounts.groups.rename')}
                          >
                            <Pencil size={14} />
                          </button>
                          <button
                            className="group-action-btn danger"
                            onClick={() => setDeleteConfirmId(group.id)}
                            title={t('common.delete')}
                          >
                            <Trash2 size={14} />
                          </button>
                        </>
                      )}
                    </div>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>

        <div className="modal-footer">
          <button className="btn btn-secondary" onClick={onClose}>
            {t('common.close')}
          </button>
        </div>
      </div>
    </div>
  );
};

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

  useEffect(() => {
    if (isOpen) {
      (async () => setGroups(await getPlatformGroups(platformKey)))();
      setNewName('');
      setError(null);
    }
  }, [isOpen, platformKey]);

  const handleSelect = async (groupId: string) => {
    setError(null);
    try {
      await assignAccountsToPlatformGroup(platformKey, groupId, accountIds);
      if (platformKey === 'antigravity') {
        invalidateLegacyCache();
      }
      await onAdded();
      onClose();
    } catch (err) {
      console.error('Failed to add accounts to group:', err);
      setError(t('accounts.groups.error.addFailed', {
        error: String(err),
      }));
    }
  };

  const handleCreateAndAdd = async () => {
    const name = newName.trim();
    if (!name) return;
    setError(null);
    try {
      const group = await createPlatformGroup(platformKey, name);
      await assignAccountsToPlatformGroup(platformKey, group.id, accountIds);
      if (platformKey === 'antigravity') {
        invalidateLegacyCache();
      }
      await onAdded();
      onClose();
    } catch (err) {
      console.error('Failed to create group and add accounts:', err);
      setError(t('accounts.groups.error.createAndAddFailed', {
        error: String(err),
      }));
    }
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
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter') handleCreateAndAdd(); }}
              placeholder={t('accounts.groups.createAndAdd')}
              maxLength={30}
            />
            <button
              className="btn btn-primary"
              onClick={handleCreateAndAdd}
              disabled={!newName.trim()}
            >
              <Plus size={14} />
            </button>
          </div>

          {groups.length > 0 && (
            <div className="add-to-group-list">
              {groups.filter((g) => g.id !== sourceGroupId).map((group) => (
                <div
                  key={group.id}
                  className="add-to-group-item"
                  onClick={() => handleSelect(group.id)}
                >
                  <FolderOpen size={16} className="group-icon" />
                  <span className="group-name">{group.name}</span>
                  <span className="group-count">
                    {group.accountIds.length}
                  </span>
                </div>
              ))}
            </div>
          )}

          {/* 错误提示 */}
          {error && (
            <div className="group-modal-error">
              <AlertCircle size={14} />
              <span>{error}</span>
            </div>
          )}
        </div>
      </div>
    </div>
  );
};

export default AccountGroupModal;
