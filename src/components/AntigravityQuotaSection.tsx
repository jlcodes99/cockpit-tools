import type { TFunction } from 'i18next';
import type { AgQuotaDisplayItem } from '../presentation/platformAccountPresentation';
import { formatResetTimeDisplay, getQuotaClass } from '../utils/account';

interface Props {
  items: AgQuotaDisplayItem[];
  isList?: boolean;
  isNeedsReauth?: boolean;
  t: TFunction;
}

const PAID_BUCKET_WINDOWS: ReadonlyArray<{ key: string; label: string }> = [
  { key: 'claude:5h', label: 'Claude (5h)' },
  { key: 'claude:weekly', label: 'Claude (Weekly)' },
  { key: 'gemini:5h', label: 'Gemini (5h)' },
  { key: 'gemini:weekly', label: 'Gemini (Weekly)' },
  { key: 'argon:weekly', label: 'Gemini 4 Argon (Weekly)' },
];

const FREE_BUCKET_WINDOWS: ReadonlyArray<{ key: string; label: string }> = [
  { key: 'claude:weekly', label: 'Claude (Weekly)' },
  { key: 'gemini:weekly', label: 'Gemini (Weekly)' },
  { key: 'argon:weekly', label: 'Gemini 4 Argon (Weekly)' },
];

const BUCKET_KEYS = new Set(PAID_BUCKET_WINDOWS.map((window) => window.key));

export function AntigravityQuotaSection({ items, isNeedsReauth = false, t }: Props) {
  if (items.length === 0) {
    return (
      <div className="quota-empty" style={{ gridColumn: '1 / -1', textAlign: 'center' }}>
        {t('overview.noQuotaData')}
      </div>
    );
  }

  const renderBar = (key: string, label: string, item?: AgQuotaDisplayItem) => {
    const percentage = item?.percentage;
    const quotaClass = percentage == null ? 'neutral' : getQuotaClass(percentage);
    const resetLabel = item?.resetTime ? formatResetTimeDisplay(item.resetTime, t) : '';
    return (
      <div key={key} className="codex-quota-mini">
        <div className="codex-quota-mini-head">
          <span className="codex-quota-mini-label" title={label}>
            {label}
          </span>
        </div>
        <div className="codex-quota-mini-meter">
          <div className="codex-quota-mini-track" aria-hidden="true">
            {percentage != null && (
              <div
                className={`codex-quota-mini-bar ${quotaClass}`}
                style={{ width: `${Math.max(0, Math.min(100, percentage))}%` }}
              />
            )}
          </div>
          <span
            className={`codex-quota-mini-pct ${quotaClass}`}
            title={percentage == null ? t('overview.noQuotaData') : undefined}
          >
            {percentage == null ? '—' : `${percentage}%`}
          </span>
        </div>
        {resetLabel ? (
          <div className="codex-quota-mini-reset" title={resetLabel}>
            {resetLabel}
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
      <div className="codex-quota-mini-list">
        {hasBuckets &&
          targetWindows.map((window) =>
            renderBar(
              window.key,
              window.label,
              items.find((item) => item.key === window.key),
            ),
          )}
        {extraItems.map((item) => renderBar(item.key, item.label, item))}
      </div>
    </>
  );
}
