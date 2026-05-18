/**
 * 账号分组管理弹窗
 * - 创建 / 重命名 / 删除分组
 * - 显示分组列表及账号数量
 */

import { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { X, FolderOpen, Plus, Pencil, Trash2, FolderPlus, AlertCircle } from 'lucide-react';
import {
  AccountGroup,
  getAccountGroups,
  createGroup,
  deleteGroup,
  renameGroup,
  addAccountsToGroup,
  moveAccountsBetweenGroups,
} from '../services/accountGroupService';
import { useEscClose } from '../hooks/useEscClose';
import './AccountGroupModal.css';

// ─── 分组管理弹窗 ──────────────────────────────────────────

interface AccountGroupModalProps {
  isOpen: boolean;
  onClose: () => void;
  onGroupsChanged: () => Promise<void> | void;
  /** 当前被勾选用于筛选的分组 ID 列表 */
  groupFilter?: string[];
  /** 切换某个分组的筛选状态 */
  onToggleGroupFilter?: (groupId: string) => void;
  /** 清空分组筛选 */
  onClearGroupFilter?: () => void;
}

export const AccountGroupModal = ({
  isOpen, onClose, onGroupsChanged,
  groupFilter = [], onToggleGroupFilter, onClearGroupFilter,
}: AccountGroupModalProps) => {
  const { t } = useTranslation();
  useEscClose(isOpen, onClose);
  const [groups, setGroups] = useState<AccountGroup[]>([]);
  const [newName, setNewName] = useState('');
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState('');
  const [deleteConfirmId, setDeleteConfirmId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    setGroups(await getAccountGroups());
  }, []);

  useEffect(() => {
    if (isOpen) {
      reload();
      setNewName('');
      setRenamingId(null);
      setDeleteConfirmId(null);
      setError(null);
    }
  }, [isOpen, reload]);

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
      await createGroup(name);
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
      await renameGroup(groupId, name);
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
      await deleteGroup(groupId);
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
    <div className="modal-overlay" onClick={onClose}>
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

          {/* 筛选提示 */}
          {groupFilter.length > 0 && onClearGroupFilter && (
            <div className="group-filter-hint">
              <span>{t('accounts.groups.filterHint', { count: groupFilter.length })}</span>
              <button type="button" className="group-filter-clear-btn" onClick={onClearGroupFilter}>
                {t('accounts.clearFilter', '清空筛选')}
              </button>
            </div>
          )}

          {/* 分组列表 */}
          {groups.length === 0 ? (
            <div className="group-modal-empty">
              <FolderPlus size={36} />
              <div>{t('accounts.groups.empty')}</div>
            </div>
          ) : (
            <div className="group-modal-list">
              {groups.map((group) => (
                <div key={group.id} className={`group-modal-item ${groupFilter.includes(group.id) ? 'group-filter-active' : ''}`}>
                  {/* 筛选复选框 */}
                  {onToggleGroupFilter && (
                    <input
                      type="checkbox"
                      className="group-filter-checkbox"
                      checked={groupFilter.includes(group.id)}
                      onChange={() => onToggleGroupFilter(group.id)}
                      title={t('accounts.groups.filterToggle', '勾选以筛选此分组')}
                    />
                  )}
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
}

export const AddToGroupModal = ({ isOpen, onClose, accountIds, sourceGroupId, onAdded }: AddToGroupModalProps) => {
  const { t } = useTranslation();
  useEscClose(isOpen, onClose);
  const [groups, setGroups] = useState<AccountGroup[]>([]);
  const [newName, setNewName] = useState('');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (isOpen) {
      (async () => setGroups(await getAccountGroups()))();
      setNewName('');
      setError(null);
    }
  }, [isOpen]);

  const handleSelect = async (groupId: string) => {
    setError(null);
    try {
      if (sourceGroupId) {
        await moveAccountsBetweenGroups(sourceGroupId, groupId, accountIds);
      } else {
        await addAccountsToGroup(groupId, accountIds);
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
      const group = await createGroup(name);
      if (sourceGroupId) {
        await moveAccountsBetweenGroups(sourceGroupId, group.id, accountIds);
      } else {
        await addAccountsToGroup(group.id, accountIds);
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
    <div className="modal-overlay" onClick={onClose}>
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
