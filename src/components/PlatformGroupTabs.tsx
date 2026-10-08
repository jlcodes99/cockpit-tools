import { useTranslation } from 'react-i18next';
import { Plus } from 'lucide-react';
import type { AccountGroup } from '../services/platformGroupService';

export interface PlatformGroupTabsProps {
  groups: AccountGroup[];
  activeGroupId: string | null;
  onSelectGroup: (groupId: string | null) => void;
  onOpenManage: () => void;
  /** 账号列表，用于计算各分组实际账号匹配数量 */
  accounts?: Array<{ id: string }>;
  totalCount?: number;
}

export function PlatformGroupTabs({
  groups,
  activeGroupId,
  onSelectGroup,
  onOpenManage,
  accounts,
  totalCount,
}: PlatformGroupTabsProps) {
  const { t } = useTranslation();
  const allCount = totalCount !== undefined ? totalCount : (accounts ? accounts.length : 0);

  return (
    <div className="account-group-tabs-container">
      <div className="account-group-tabs" role="tablist">
        <button
          type="button"
          role="tab"
          aria-selected={activeGroupId === null}
          className={`account-group-tab ${activeGroupId === null ? 'active' : ''}`}
          onClick={() => onSelectGroup(null)}
        >
          <span className="account-group-tab-name">{t('accounts.filters.all', '全部')}</span>
          <span className="account-group-tab-badge">{allCount}</span>
        </button>

        {groups.map((group) => {
          const count = accounts
            ? accounts.filter((acc) => group.accountIds.includes(acc.id)).length
            : group.accountIds.length;
          const isActive = activeGroupId === group.id;
          return (
            <button
              key={group.id}
              type="button"
              role="tab"
              aria-selected={isActive}
              className={`account-group-tab ${isActive ? 'active' : ''}`}
              onClick={() => onSelectGroup(group.id)}
              title={group.name}
            >
              <span className="account-group-tab-name">{group.name}</span>
              <span className="account-group-tab-badge">{count}</span>
            </button>
          );
        })}

        <button
          type="button"
          className="account-group-tab-manage-btn"
          onClick={onOpenManage}
          title={t('accounts.groups.manageTitle', '分组管理')}
          aria-label={t('accounts.groups.manageTitle', '分组管理')}
        >
          <Plus size={14} />
        </button>
      </div>
    </div>
  );
}
