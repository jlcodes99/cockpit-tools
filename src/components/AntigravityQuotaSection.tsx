import type { TFunction } from 'i18next';
import type { AgQuotaDisplayItem } from '../presentation/platformAccountPresentation';
import { formatResetTimeDisplay, getQuotaClass } from '../utils/account';

interface Props {
  items: AgQuotaDisplayItem[];
  modelItems?: AgQuotaDisplayItem[];
  isList?: boolean;
  isNeedsReauth?: boolean;
  t: TFunction;
}

const PAID_BUCKET_WINDOWS: ReadonlyArray<{ key: string; label: string }> = [
  { key: 'gemini:5h', label: 'Gemini · 5h' },
  { key: 'gemini:weekly', label: 'Gemini · Weekly' },
  { key: 'claude:5h', label: 'Claude · 5h' },
  { key: 'claude:weekly', label: 'Claude · Weekly' },
];

const FREE_BUCKET_WINDOWS: ReadonlyArray<{ key: string; label: string }> = [
  { key: 'gemini:weekly', label: 'Gemini · Weekly' },
  { key: 'claude:weekly', label: 'Claude · Weekly' },
];

const BUCKET_KEYS = new Set(PAID_BUCKET_WINDOWS.map((window) => window.key));

function compactResetText(fullReset: string): string {
  const idx = fullReset.indexOf(' (');
  return idx > 0 ? fullReset.slice(0, idx) : fullReset;
}

export function AntigravityQuotaSection({
  items,
  modelItems = [],
  isList = false,
  isNeedsReauth = false,
  t,
}: Props) {
  if (items.length === 0 && modelItems.length === 0) {
    return (
      <div className="quota-empty" style={{ gridColumn: '1 / -1', textAlign: 'center' }}>
        {t('overview.noQuotaData')}
      </div>
    );
  }

  const renderCell = (
    key: string,
    label: string,
    item?: AgQuotaDisplayItem,
    fullWidth = false,
  ) => {
    const percentage = item?.percentage;
    const quotaClass = percentage == null ? 'neutral' : getQuotaClass(percentage);
    const fullReset = item?.resetTime ? formatResetTimeDisplay(item.resetTime, t) : '';
    const shortReset = fullReset ? compactResetText(fullReset) : '';

    return (
      <div
        key={key}
        className={`ag-quota-cell ${quotaClass}${fullWidth ? ' ag-quota-cell--full' : ''}`}
      >
        <div className="ag-quota-cell-head">
          <span className="ag-quota-cell-label" title={label}>
            {label}
          </span>
          <span
            className={`ag-quota-cell-pct ${quotaClass}`}
            title={percentage == null ? t('overview.noQuotaData') : undefined}
          >
            {percentage == null ? '—' : `${percentage}%`}
          </span>
        </div>
        <div className="ag-quota-bar-track" aria-hidden="true">
          {percentage != null && (
            <div
              className={`ag-quota-bar-fill ${quotaClass}`}
              style={{ width: `${Math.max(0, Math.min(100, percentage))}%` }}
            />
          )}
        </div>
        {shortReset ? (
          <div className="ag-quota-cell-reset" title={fullReset}>
            {shortReset}
          </div>
        ) : null}
      </div>
    );
  };

  const hasBuckets = items.some((item) => BUCKET_KEYS.has(item.key));
  const hasAny5h = items.some((item) => item.key.endsWith(':5h'));
  const targetWindows = hasAny5h ? PAID_BUCKET_WINDOWS : FREE_BUCKET_WINDOWS;
  const extraItems = items.filter((item) => !BUCKET_KEYS.has(item.key));

  return (
    <>
      {items.some((item) => item.stale) && (
        <div
          className={`quota-empty ${isNeedsReauth ? 'quota-reauth-warning' : ''}`}
          style={{
            gridColumn: '1 / -1',
            ...(isNeedsReauth ? { color: 'var(--color-warning, #f59e0b)', fontWeight: 500 } : {}),
          }}
        >
          {isNeedsReauth
            ? t('common.shared.quota.cachedNeedsReauth', '⚠️ 账号需完成网页验证以继续使用（以下为上次成功数据）')
            : t('common.shared.quota.cachedRefreshFailed')}
        </div>
      )}
      {items.length > 0 && (
        <div className={`ag-quota-grid${isList ? ' ag-quota-grid--list' : ''}`}>
          {hasBuckets &&
            targetWindows.map((window) =>
              renderCell(
                window.key,
                window.label,
                items.find((item) => item.key === window.key),
              ),
            )}
          {extraItems.map((item, idx) =>
            renderCell(
              item.key,
              item.label,
              item,
              extraItems.length % 2 === 1 && idx === extraItems.length - 1,
            ),
          )}
        </div>
      )}
      {modelItems.length > 0 && (
        <details className="antigravity-model-quota-details">
          <summary>{t('modals.quota.title')}</summary>
          <div className="ag-quota-grid ag-quota-grid--list">
            {modelItems.map((item) => renderCell(item.key, item.label, item))}
          </div>
        </details>
      )}
    </>
  );
}
