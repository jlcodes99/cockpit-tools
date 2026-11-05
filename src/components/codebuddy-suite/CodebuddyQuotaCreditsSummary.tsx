import { useMemo, useState } from 'react';
import {
  ChevronDown,
  ChevronRight,
  Gift,
  MoreHorizontal,
  Package,
  Zap,
  type LucideIcon,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { QuotaCategoryGroup } from '../../types/codebuddy-suite';

const CATEGORY_ICONS: Record<string, LucideIcon> = {
  base: Package,
  activity: Gift,
  extra: Zap,
  other: MoreHorizontal,
};

/**
 * CodeBuddy CN 账号卡片的新版额度概览。
 *
 * 与 WorkBuddy 积分卡片保持一致的呈现：总剩余额度、按资源包分组的
 * 用量/剩余进度条，以及可展开的额度明细。复用 `workbuddy-credit-*` 样式，
 * 明细沿用配额查询组件已有的 `.quota-category-details` 滚动容器，
 * 避免资源包较多时把账号卡片撑得过长。
 */
export function CodebuddyQuotaCreditsSummary({
  groups,
}: {
  groups: QuotaCategoryGroup[];
}) {
  const { t, i18n } = useTranslation();
  const [detailsOpen, setDetailsOpen] = useState(false);
  const locale = i18n.resolvedLanguage ?? i18n.language;
  const { number, date } = useMemo(
    () => ({
      number: new Intl.NumberFormat(locale),
      date: new Intl.DateTimeFormat(locale, {
        dateStyle: 'medium',
        timeStyle: 'short',
      }),
    }),
    [locale],
  );

  const visibleGroups = groups.filter((group) => group.visible);
  if (visibleGroups.length === 0) return null;

  const left = visibleGroups.reduce(
    (sum, group) => sum + Math.max(0, group.remain),
    0,
  );
  const items = visibleGroups.flatMap((group) =>
    group.items.map((item) => ({ group, item })),
  );

  return (
    <div className="workbuddy-credits-summary workbuddy-credits-compact">
      <div className="workbuddy-credit-balance">
        <div className="workbuddy-credit-balance-value">
          <span>{t('workbuddy.credits.total')}</span>
          <strong>{number.format(left)}</strong>
        </div>
        <button
          type="button"
          className="btn btn-sm workbuddy-credit-link"
          aria-expanded={detailsOpen}
          onClick={() => setDetailsOpen((open) => !open)}
        >
          {t('workbuddy.credits.details')}
          {detailsOpen ? (
            <ChevronDown size={13} aria-hidden="true" />
          ) : (
            <ChevronRight size={13} aria-hidden="true" />
          )}
        </button>
      </div>

      {visibleGroups.map((group) => {
        const Icon = CATEGORY_ICONS[group.key] ?? MoreHorizontal;
        const remainPercent =
          group.remainPercent ?? Math.max(0, 100 - group.usedPercent);
        return (
          <div
            key={group.key}
            className="workbuddy-credit-entry"
            data-kind={group.key}
          >
            <div className="workbuddy-credit-line workbuddy-credit-group">
              <strong>
                <Icon size={14} aria-hidden="true" />
                {group.label}
              </strong>
              <span>{number.format(Math.max(0, group.remain))}</span>
            </div>
            <div className="workbuddy-credit-line workbuddy-credit-muted">
              <span>
                {t('workbuddy.credits.usedOf', {
                  used: number.format(group.used),
                  total: number.format(group.total),
                })}
              </span>
              <span>
                {t('workbuddy.credits.remainingPercent', {
                  percent: number.format(Math.floor(remainPercent)),
                })}
              </span>
            </div>
            <div className="workbuddy-credit-track" aria-hidden="true">
              <span
                style={{
                  width: `${Math.min(100, Math.max(0, group.usedPercent))}%`,
                }}
              />
            </div>
          </div>
        );
      })}

      {detailsOpen && (
        <div className="quota-category-details">
          {items.length === 0 ? (
            <div className="quota-category-empty">
              {t('common.shared.quota.noData', '暂无配额数据')}
            </div>
          ) : (
            items.map(({ group, item }, index) => (
              <div
                key={`${group.key}-${index}`}
                className="quota-category-detail-item"
              >
                <div className="quota-detail-header">
                  <span
                    className="quota-detail-name"
                    title={item.packageName ?? undefined}
                  >
                    {item.packageName || group.label}
                  </span>
                  <span className="quota-detail-value">
                    {item.unlimited
                      ? t('common.shared.quota.unlimited', '无限额度')
                      : `${number.format(item.remain)} / ${number.format(item.total)}`}
                  </span>
                </div>
                {item.expireAt != null ? (
                  <div className="quota-detail-meta">
                    {t('common.shared.quota.expiresAt', '到期时间：{{time}}', {
                      time: date.format(new Date(item.expireAt)),
                    })}
                  </div>
                ) : item.refreshAt != null ? (
                  <div className="quota-detail-meta">
                    {t('common.shared.quota.resetAt', '重置：{{time}}', {
                      time: date.format(new Date(item.refreshAt)),
                    })}
                  </div>
                ) : null}
              </div>
            ))
          )}
        </div>
      )}
    </div>
  );
}
