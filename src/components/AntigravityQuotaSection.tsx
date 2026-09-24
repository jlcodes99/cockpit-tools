import type { TFunction } from 'i18next';
import type { AgQuotaDisplayItem } from '../presentation/platformAccountPresentation';
import { formatResetTimeDisplay, getQuotaClass } from '../utils/account';

interface Props {
  items: AgQuotaDisplayItem[];
  isList?: boolean;
  t: TFunction;
}

export function AntigravityQuotaSection({ items, isList = false, t }: Props) {
  if (items.length === 0) {
    return (
      <div className="quota-empty" style={{ gridColumn: '1 / -1', textAlign: 'center' }}>
        {t('overview.noQuotaData')}
      </div>
    );
  }

  const renderBar = (key: string, label: string, item?: AgQuotaDisplayItem) => {
    const percentage = item?.percentage;
    const quotaClass = percentage == null ? '' : getQuotaClass(percentage);
    const resetLabel = item?.resetTime ? formatResetTimeDisplay(item.resetTime, t) : '';
    return (
      <div key={key} className={isList ? 'quota-item' : 'quota-compact-item'}>
        <div className={isList ? 'quota-header' : 'quota-compact-header'}>
          <span
            className={isList ? 'quota-name' : 'model-label'}
            title={label}
            style={{ minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}
          >
            {label}
          </span>
          <span
            className={`${isList ? 'quota-value' : 'model-pct'} ${quotaClass}`}
            title={percentage == null ? t('overview.noQuotaData') : undefined}
            style={{ flexShrink: 0, marginLeft: 4 }}
          >
            {percentage == null ? '—' : `${percentage}%`}
          </span>
        </div>
        <div className={isList ? 'quota-progress-track' : 'quota-compact-bar-track'}>
          {percentage != null && (
            <div
              className={`${isList ? 'quota-progress-bar' : 'quota-compact-bar'} ${quotaClass}`}
              style={{ width: `${percentage}%` }}
            />
          )}
        </div>
        {(isList || resetLabel) && (
          <div className={isList ? 'quota-footer' : undefined}>
            <span
              className={isList ? 'quota-reset' : 'quota-compact-reset'}
              title={resetLabel || undefined}
            >
              {resetLabel || '\u00A0'}
            </span>
          </div>
        )}
      </div>
    );
  };

  const bucketKeys = new Set(['claude:5h', 'claude:weekly', 'gemini:5h', 'gemini:weekly']);
  const hasBuckets = items.some((item) => bucketKeys.has(item.key));
  const hasAny5h = items.some((item) => item.key.endsWith(':5h'));
  return (
    <>
      {items.some((item) => item.stale) && (
        <div className="quota-empty" style={{ gridColumn: '1 / -1' }}>
          {t('common.shared.quota.cachedRefreshFailed')}
        </div>
      )}
      {hasBuckets && ['claude', 'gemini'].map((family) => (
        <div key={family} className="quota-column">
          <div className="quota-column-title">{family === 'claude' ? 'Claude' : 'Gemini'}</div>
          {hasAny5h && renderBar(`${family}:5h`, '5h', items.find((item) => item.key === `${family}:5h`))}
          {renderBar(
            `${family}:weekly`,
            'Weekly',
            items.find((item) => item.key === `${family}:weekly`),
          )}
        </div>
      ))}
      {items.filter((item) => !bucketKeys.has(item.key)).map((item) => (
        <div key={item.key} className="quota-column">
          {renderBar(item.key, item.label, item)}
        </div>
      ))}
    </>
  );
}
