import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { formatCodexResetTime, type CodexAccount } from '../../../types/codex';
import { pelicanAccountSummary } from './pelicanSetupModel';
import { CODEX_PLAN_BADGE_STYLE_CHANGED_EVENT, getCodexPlanBadgeStyle, withCodexPlanBadgeStyle } from '../../../utils/codexPreferences';

export function PelicanAccountSummary({ account }: { account: CodexAccount }) {
  const { t } = useTranslation();
  const [planStyle, setPlanStyle] = useState(getCodexPlanBadgeStyle);
  useEffect(() => {
    const sync = () => setPlanStyle(getCodexPlanBadgeStyle());
    window.addEventListener(CODEX_PLAN_BADGE_STYLE_CHANGED_EVENT, sync);
    return () => window.removeEventListener(CODEX_PLAN_BADGE_STYLE_CHANGED_EVENT, sync);
  }, []);
  const summary = pelicanAccountSummary(account);
  return <span className="pelican-account-summary">
    <span className={`tier-badge ${withCodexPlanBadgeStyle(summary.planClass, planStyle)}`} title={summary.plan ?? undefined}>{summary.plan ?? '—'}</span>
    <span className="pelican-account-quotas">
      {!summary.windows.length && <span className="pelican-muted">{t('common.shared.quota.noData')}</span>}
      {summary.windows.map((window) => {
        const weeks = /^(\d+) Week$/.exec(window.label);
        const label = window.label === 'Weekly' ? t('codex.instances.quota.weekly')
          : weeks ? t('codex.quota.windowWeeks', { count: Number(weeks[1]) }) : window.label;
        const value = window.percentage == null ? '—' : `${window.percentage}%`;
        const hint = window.percentage == null ? t('common.shared.quota.noData') : t('common.shared.quota.leftPercent', { value: window.percentage });
        const reset = formatCodexResetTime(window.resetTime, t);
        return <span className={`pelican-quota-badge ${window.quotaClass}`} key={window.id}
          title={[hint, reset].filter(Boolean).join(' · ')} aria-label={`${label} · ${hint}${reset ? ` · ${reset}` : ''}`}>
          <span>{label}</span><strong>{value}</strong>
        </span>;
      })}
    </span>
  </span>;
}

export function PelicanPlanBadge({ account }: { account: CodexAccount }) {
  const [planStyle, setPlanStyle] = useState(getCodexPlanBadgeStyle);
  useEffect(() => {
    const sync = () => setPlanStyle(getCodexPlanBadgeStyle());
    window.addEventListener(CODEX_PLAN_BADGE_STYLE_CHANGED_EVENT, sync);
    return () => window.removeEventListener(CODEX_PLAN_BADGE_STYLE_CHANGED_EVENT, sync);
  }, []);
  const summary = pelicanAccountSummary(account);
  return <span className={`tier-badge ${withCodexPlanBadgeStyle(summary.planClass, planStyle)}`} title={summary.plan ?? undefined}>{summary.plan ?? '—'}</span>;
}
