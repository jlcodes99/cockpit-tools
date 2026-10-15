import { useState, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { ChevronDown, ChevronUp, Package, Gift, Zap, MoreHorizontal } from 'lucide-react';
import type { QuotaCategoryGroup, CodebuddyOfficialQuotaResource } from '../../types/codebuddy';

interface CodeBuddyQuotaCategoryListProps {
  groups: QuotaCategoryGroup[];
  formatNumber: (value: number) => string;
  formatDateTime: (timeMs: number | null) => string;
}

const CATEGORY_ICONS: Record<string, React.ReactNode> = {
  base: <Package size={14} />,
  activity: <Gift size={14} />,
  extra: <Zap size={14} />,
  other: <MoreHorizontal size={14} />,
};

const CATEGORY_COLORS: Record<string, string> = {
  base: '#3b82f6',
  activity: '#f59e0b',
  extra: '#8b5cf6',
  other: '#6b7280',
};

function getQuotaClass(remainPercent: number | null): string {
  if (remainPercent == null || !Number.isFinite(remainPercent)) return 'high';
  if (remainPercent <= 10) return 'critical';
  if (remainPercent <= 30) return 'low';
  if (remainPercent <= 60) return 'medium';
  return 'high';
}

export function CodeBuddyQuotaCategoryList({ groups, formatNumber, formatDateTime }: CodeBuddyQuotaCategoryListProps) {
  const { t } = useTranslation();
  const [expandedKeys, setExpandedKeys] = useState<Set<string>>(new Set());

  const toggleExpand = useCallback((key: string) => {
    setExpandedKeys((prev) => {
      const next = new Set(prev);
      if (next.has(key)) {
        next.delete(key);
      } else {
        next.add(key);
      }
      return next;
    });
  }, []);

  // 只显示 visible 的分组
  const visibleGroups = groups.filter((g) => g.visible);

  if (visibleGroups.length === 0) {
    return (
      <div className="quota-category-empty">
        {t('common.shared.quota.noData', '暂无配额数据')}
      </div>
    );
  }

  return (
    <div className="quota-category-list">
      {visibleGroups.map((group) => {
        const isExpanded = expandedKeys.has(group.key);
        const hasDetails = group.items.length > 1 || (group.items.length === 1 && group.items[0].packageName);

        return (
          <div key={group.key} className={`quota-category-item ${getQuotaClass(group.remainPercent)}`}>
            {/* 分组头部 - 始终显示 */}
            <div
              className="quota-category-header"
              onClick={() => hasDetails && toggleExpand(group.key)}
              style={{ cursor: hasDetails ? 'pointer' : 'default' }}
            >
              <div className="quota-category-info">
                <span className="quota-category-icon" style={{ color: CATEGORY_COLORS[group.key] }}>
                  {CATEGORY_ICONS[group.key]}
                </span>
                <span className="quota-category-label">{group.label}</span>
                {hasDetails && (
                  <span className="quota-category-count">({group.items.length})</span>
                )}
              </div>
              <div className="quota-category-stats">
                <span className="quota-category-value">
                  {group.unlimited
                    ? t('common.shared.quota.unlimited', '无限额度')
                    : `${formatNumber(group.used)} / ${formatNumber(group.total)}`}
                </span>
                {hasDetails && (
                  <span className="quota-category-expand-icon">
                    {isExpanded ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
                  </span>
                )}
              </div>
            </div>

            {/* 进度条 */}
            <div className="quota-category-progress">
              <div
                className={`quota-category-progress-bar ${getQuotaClass(group.remainPercent)}`}
                style={{ width: `${Math.min(100, group.usedPercent)}%` }}
              />
            </div>

            {/* 详情列表 - 展开时显示 */}
            {isExpanded && hasDetails && (
              <div className="quota-category-details">
                {group.items.map((item, idx) => (
                  <QuotaItemDetail
                    key={`${group.key}-${idx}`}
                    item={item}
                    formatNumber={formatNumber}
                    formatDateTime={formatDateTime}
                  />
                ))}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

interface QuotaItemDetailProps {
  item: CodebuddyOfficialQuotaResource;
  formatNumber: (value: number) => string;
  formatDateTime: (timeMs: number | null) => string;
}

function QuotaItemDetail({ item, formatNumber, formatDateTime }: QuotaItemDetailProps) {
  const { t } = useTranslation();
  const remainPercent = item.remainPercent ?? (item.total > 0 ? (item.remain / item.total) * 100 : null);

  // 时间显示逻辑
  let timeText = '';
  if (item.expireAt) {
    timeText = t('common.shared.quota.expiresAt', '到期时间：{{time}}', { time: formatDateTime(item.expireAt) });
  } else if (item.refreshAt) {
    timeText = t('common.shared.quota.resetAt', '重置：{{time}}', { time: formatDateTime(item.refreshAt) });
  }

  return (
    <div className={`quota-category-detail-item ${getQuotaClass(remainPercent)}`}>
      <div className="quota-detail-header">
        <span className="quota-detail-name" title={item.packageName || ''}>
          {item.packageName || t('common.shared.quota.noData', '暂无配额数据')}
        </span>
        <span className={`quota-detail-value ${getQuotaClass(remainPercent)}`}>
          {item.unlimited
            ? t('common.shared.quota.unlimited', '无限额度')
            : `${formatNumber(item.used)} / ${formatNumber(item.total)}`}
        </span>
      </div>
      {timeText && (
        <div className="quota-detail-meta">{timeText}</div>
      )}
    </div>
  );
}
