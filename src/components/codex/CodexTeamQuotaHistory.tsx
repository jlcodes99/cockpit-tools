import { Clock3 } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { UnifiedQuotaMetric } from '../../presentation/platformAccountPresentation';
import type { CodexAccount } from '../../types/codex';
import { CodexQuotaMiniRows } from './CodexQuotaMiniRows';

function formatRelativeReset(resetAtMs: number, now: number, locale: string): string {
  const minutes = Math.max(1, Math.ceil((resetAtMs - now) / 60_000));
  const [value, unit]: [number, Intl.RelativeTimeFormatUnit] =
    minutes >= 1440
      ? [Math.ceil(minutes / 1440), 'day']
      : minutes >= 60
        ? [Math.ceil(minutes / 60), 'hour']
        : [minutes, 'minute'];
  return new Intl.RelativeTimeFormat(locale, { numeric: 'always' }).format(value, unit);
}

export function CodexTeamQuotaHistory({ account }: { account: CodexAccount }) {
  const { t, i18n } = useTranslation();
  const [now, setNow] = useState(() => Date.now());
  const history = account.team_quota_history;
  const visible = account.plan_type === 'self_serve_business_usage_based';

  useEffect(() => {
    if (!visible) return;
    const timer = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => window.clearInterval(timer);
  }, [visible]);

  if (!visible) return null;
  const valid =
    history &&
    history.user_id === account.user_id &&
    history.account_id === account.account_id;
  const items: UnifiedQuotaMetric[] = valid
    ? ([
        [
          'primary',
          t('codex.instances.quota.hourly'),
          history.hourly_reset_time,
          history.hourly_percentage,
        ],
        [
          'secondary',
          t('codex.instances.quota.weekly'),
          history.weekly_reset_time,
          history.weekly_percentage,
        ],
      ] as const).map(([key, label, reset, remaining]) => {
        const percentage = Math.max(0, Math.min(100, remaining ?? 0));
        const resetAtMs = reset ? reset * 1000 : null;
        return {
          key,
          label,
          percentage,
          resetAt: reset,
          quotaClass: percentage > 50 ? 'high' : percentage > 20 ? 'medium' : 'low',
          showProgress: remaining != null,
          valueText: remaining == null ? t('codex.teamHistory.unknown') : `${percentage}%`,
          hintText: t('codex.teamHistory.hint'),
          resetText:
            resetAtMs == null
              ? t('codex.teamHistory.unknown')
              : resetAtMs <= now
                ? t('codex.teamHistory.elapsed')
                : t('codex.teamHistory.resetIn', {
                    duration: formatRelativeReset(resetAtMs, now, i18n.resolvedLanguage ?? i18n.language),
                  }),
        };
      })
    : [];

  return (
    <div className="codex-team-quota-history">
      <div className="codex-quota-mini-reset">{t('codex.teamHistory.title')}</div>
      {!valid ? (
        <div className="codex-quota-mini-reset">{t('codex.teamHistory.unknown')}</div>
      ) : (
        <>
          {items.map((item) => (
            <div key={item.key} className="codex-team-history-window">
              <CodexQuotaMiniRows items={[{ ...item, resetText: undefined }]} t={t} />
              <div
                className={`codex-subscription-footer codex-team-history-reset ${
                  item.resetAt && Number(item.resetAt) * 1000 <= now ? 'active' : 'pending'
                }`}
              >
                <div className="codex-subscription-footer-main">
                  <Clock3 size={14} aria-hidden="true" />
                  <strong>{item.resetText}</strong>
                </div>
                {item.resetAt && (
                  <time
                    className="codex-subscription-footer-date"
                    dateTime={new Date(Number(item.resetAt) * 1000).toISOString()}
                  >
                    {new Date(Number(item.resetAt) * 1000).toLocaleString(undefined, {
                      month: '2-digit',
                      day: '2-digit',
                      hour: '2-digit',
                      minute: '2-digit',
                      hour12: false,
                    })}
                  </time>
                )}
              </div>
            </div>
          ))}
          <div className="codex-quota-mini-reset">
            {t('codex.teamHistory.observed', {
              time: new Date(history.observed_at * 1000).toLocaleString(),
            })}
          </div>
        </>
      )}
    </div>
  );
}
