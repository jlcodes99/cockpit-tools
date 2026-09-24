import { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { FolderPlus, LogOut } from 'lucide-react';
import { PlatformGroupTabs } from './PlatformGroupTabs';
import { AccountGroupModal, AddToGroupModal } from './AccountGroupModal';
import type { UsePlatformAccountGroupsReturn } from '../hooks/usePlatformAccountGroups';

export interface AccountSelectionToolbarProps {
  selectedCount: number;
  allSelected: boolean;
  disabled?: boolean;
  onToggleSelectAll: () => void;
  onClearSelection: () => void;
  actions?: ReactNode;
  children?: ReactNode;

  /** 通用平台分组 Hook 返回对象，传入后自动在中间渲染分组 Tabs，并在操作栏追加加入/移出分组按钮及自动挂载弹窗 */
  grouping?: UsePlatformAccountGroupsReturn;
  /** 用于分组 Tabs 统计各组账号数量 */
  accounts?: Array<{ id: string }>;
  /** 当前选中的账号 ID 列表，用于加入/移出分组 */
  selectedIds?: string[];
}

export function AccountSelectionToolbar({
  selectedCount,
  allSelected,
  disabled = false,
  onToggleSelectAll,
  onClearSelection,
  actions,
  children,
  grouping,
  accounts,
  selectedIds,
}: AccountSelectionToolbarProps) {
  const { t } = useTranslation();
  const hasSelection = selectedCount > 0;

  const middleContent = children ?? (grouping ? (
    <PlatformGroupTabs
      groups={grouping.groups}
      activeGroupId={grouping.activeGroupId}
      onSelectGroup={grouping.setActiveGroupId}
      onOpenManage={() => grouping.setShowManageModal(true)}
      accounts={accounts || []}
    />
  ) : null);

  const resolvedActions = (
    <>
      {grouping && (
        <>
          <button
            type="button"
            className="btn btn-secondary icon-only"
            onClick={() => grouping.setShowAddToGroupModal(true)}
            title={t('accounts.groups.addToGroup', '加入分组')}
            aria-label={t('accounts.groups.addToGroup', '加入分组')}
          >
            <FolderPlus size={14} />
          </button>
          {grouping.activeGroup && (
            <button
              type="button"
              className="btn btn-secondary icon-only"
              onClick={() => void grouping.handleRemoveFromGroup(selectedIds || [])}
              title={t('accounts.groups.removeFromGroup', '从分组移出')}
              aria-label={t('accounts.groups.removeFromGroup', '从分组移出')}
            >
              <LogOut size={14} />
            </button>
          )}
        </>
      )}
      {actions}
    </>
  );

  return (
    <>
      <div className={`codex-overview-selection-bar account-selection-toolbar ${middleContent ? 'has-middle' : ''}`}>
        <div className="codex-overview-selection-left">
          <label className="codex-overview-select-all">
            <input
              type="checkbox"
              checked={allSelected}
              disabled={disabled}
              onChange={onToggleSelectAll}
            />
            <span>{t('common.selectAll', '全选')}</span>
          </label>
          {hasSelection && (
            <>
              <span className="codex-overview-selected-count">
                {t('claude.selection.selected', '已选 {{count}}', { count: selectedCount })}
              </span>
              <button
                type="button"
                className="codex-overview-clear-selection-btn"
                onClick={onClearSelection}
              >
                {t('messages.clearSelection', '取消选择')}
              </button>
            </>
          )}
        </div>
        {middleContent && (
          <div className="account-selection-toolbar-middle">
            {middleContent}
          </div>
        )}
        {hasSelection && (
          <div className="codex-overview-selection-actions">{resolvedActions}</div>
        )}
      </div>

      {grouping && (
        <>
          <AccountGroupModal
            isOpen={grouping.showManageModal}
            onClose={() => grouping.setShowManageModal(false)}
            platform={grouping.platform}
            onGroupsChanged={grouping.reloadGroups}
          />

          <AddToGroupModal
            isOpen={grouping.showAddToGroupModal}
            onClose={() => grouping.setShowAddToGroupModal(false)}
            accountIds={selectedIds || []}
            platform={grouping.platform}
            onAdded={() => {
              void grouping.reloadGroups();
              onClearSelection();
            }}
          />
        </>
      )}
    </>
  );
}
