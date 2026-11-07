import { useRef, useState } from 'react';
import { ExternalLink, ImageOff, Link2, RefreshCw, Server, Settings2 } from 'lucide-react';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';
import { chatGptPlanStatusKey, type ChatGptPlanAccount } from '../../services/chatgptPlanService';
import type { CodexLocalAccessUsageStats } from '../../types/codexLocalAccess';
import { formatSessionUsageTokensShort } from '../../utils/codexSessionUsageFormat';
import './ChatGptPlanAccountCard.css';

export interface ChatGptPlanAccountCardProps {
  account: ChatGptPlanAccount;
  layout?: 'grid' | 'compact' | 'table';
  /** Only the original subscription value from the account record, never an inferred tier. */
  planType?: string | null;
  onManage: () => void;
  onRefresh?: () => void;
  onOpenService?: () => void;
  maskAccountText?: (value: string) => string;
  stats?: CodexLocalAccessUsageStats | null;
}

/** The overview owns filtering/pagination; plan registrations keep their own authorization actions. */
export function ChatGptPlanAccountCard({ account, layout = 'grid', planType, onManage, onRefresh,
  onOpenService, maskAccountText = (value) => value, stats }: ChatGptPlanAccountCardProps) {
  const { t, i18n } = useTranslation();
  const [usageError, setUsageError] = useState(false);
  const [openingUsage, setOpeningUsage] = useState(false);
  const usagePending = useRef(false);
  const canUse = account.connected && !account.needs_reauth;
  const displayName = maskAccountText(account.email || account.label || t('chatgptPlan.title'));
  const subscription = planType?.trim() ? planType : t('chatgptPlan.card.subscriptionUnknown');
  const status = t(chatGptPlanStatusKey(account));
  const registration = t('chatgptPlan.card.registration', { id: account.id.slice(0, 8) });
  const date = account.created_at > 0 && Number.isFinite(account.created_at)
    ? new Date(account.created_at * 1000).toLocaleDateString(i18n.language) : '—';
  const requests = stats && Number.isFinite(stats.requestCount) && stats.requestCount >= 0
    ? stats.requestCount.toLocaleString(i18n.language) : '—';
  const tokens = stats && Number.isFinite(stats.totalTokens) && stats.totalTokens >= 0
    ? formatSessionUsageTokensShort(stats.totalTokens, i18n.language) : '—';
  const manageService = onOpenService ?? (() => window.dispatchEvent(
    new CustomEvent('app-request-navigate', { detail: 'codex-api-service' }),
  ));
  const openUsage = async () => {
    if (usagePending.current) return;
    usagePending.current = true;
    setUsageError(false);
    setOpeningUsage(true);
    try { await openUrl('https://chatgpt.com/settings/usage'); }
    catch { setUsageError(true); }
    finally { usagePending.current = false; setOpeningUsage(false); }
  };
  const checkbox = <input type="checkbox" disabled checked={false} readOnly
    title={t('chatgptPlan.card.manageSeparately')} aria-label={t('chatgptPlan.card.manageSeparately')} />;
  const source = <span className="tier-badge unknown" title={t('chatgptPlan.card.authorizationMethod')}>{t('chatgptPlan.title')}</span>;
  const meta = <div className="account-sub-line chatgpt-plan-account-meta">
    <span className="codex-login-subline" title={t('chatgptPlan.apiOnlyHint')}>{t('chatgptPlan.card.apiOnly')}</span>
    <span className={`codex-status-pill ${!canUse ? 'quota-error' : 'quota-refresh'}`}>{status}</span>
    <span className="codex-login-subline chatgpt-plan-image-lock" title={t('chatgptPlan.imagePolicyHint')}><ImageOff size={11} aria-hidden="true" />{t('chatgptPlan.imagePolicyLocked')}</span>
  </div>;
  const usage = <div className="chatgpt-plan-local-usage" title={t('chatgptPlan.localStatsHint')}>
    <span className="codex-login-subline">{t('chatgptPlan.card.localUsage')}</span>
    <span>{t('chatgptPlan.card.requests', { value: requests })}</span>
    <span>{t('chatgptPlan.card.tokens', { value: tokens })}</span>
  </div>;
  const error = usageError && <p className="chatgpt-plan-card-error" role="alert">{t('chatgptPlan.error.generic')}</p>;
  const actions = (className: string, size = 14) => <>
    <button type="button" className={className} onClick={onManage}
      title={t(canUse ? 'chatgptPlan.card.manageAuthorization' : 'common.reauthorize')}
      aria-label={t(canUse ? 'chatgptPlan.card.manageAuthorization' : 'common.reauthorize')}><Settings2 size={size} /></button>
    {canUse && !account.in_service && <button type="button" className={className} onClick={onManage}
      title={t('chatgptPlan.officialFlow.useOfficial')} aria-label={t('chatgptPlan.officialFlow.useOfficial')}><Link2 size={size} /></button>}
    {canUse && account.in_service && <button type="button" className={className} onClick={manageService}
      title={t('chatgptPlan.card.manageService')} aria-label={t('chatgptPlan.card.manageService')}><Server size={size} /></button>}
    {onRefresh && <button type="button" className={className} onClick={onRefresh}
      title={t('common.refresh')} aria-label={t('common.refresh')}><RefreshCw size={size} /></button>}
    <button type="button" className={className} disabled={openingUsage} onClick={() => void openUsage()}
      title={t('chatgptPlan.manageUsage')} aria-label={t('chatgptPlan.manageUsage')}><ExternalLink size={size} /></button>
  </>;

  if (layout === 'table') return <tr className="chatgpt-plan-account-row">
    <td>{checkbox}</td>
    <td><div className="account-cell">
      <div className="account-main-line"><span className="account-email-text" title={displayName}>{displayName}</span></div>
      {meta}<span className="card-date" title={date}>{registration}</span>
    </div></td>
    <td>{source}</td>
    <td><span className="codex-subscription-table-empty">{subscription}</span></td>
    <td><div className="quota-grid">{usage}</div>{error}</td>
    <td className="sticky-action-cell table-action-cell"><div className="action-buttons">{actions('action-btn')}</div></td>
  </tr>;

  if (layout === 'compact') return <div className="codex-compact-row chatgpt-plan-account-row">
    <div className="codex-compact-header">
      <div className="codex-compact-select">{checkbox}</div>
      <span className="codex-compact-email" title={displayName}>{displayName}</span>{source}
    </div>
    {meta}
    <div className="codex-compact-quotas">
      <span className="codex-login-subline">{subscription}</span>
      {usage}
    </div>
    {error}
    <div className="codex-compact-actions"><span className="card-date chatgpt-plan-compact-registration" title={date}>{registration}</span>{actions('codex-compact-note-btn', 13)}</div>
  </div>;

  return <div className="codex-account-card chatgpt-plan-account-card">
    <div className="card-top"><div className="card-select">{checkbox}</div><span className="account-email" title={displayName}>{displayName}</span>{source}</div>
    {meta}
    <div className="codex-quota-section">
      <div className="quota-item"><div className="quota-header"><span className="quota-label">{t('chatgptPlan.card.subscription')}</span><strong className="chatgpt-plan-subscription">{subscription}</strong></div></div>
      <div className="quota-item">{usage}</div>
    </div>
    {error}
    <div className="codex-card-bottom">
      <span className="card-date" title={registration}>{date} · {account.id.slice(0, 8)}</span>
      <div className="card-footer"><div className="card-actions">{actions('card-action-btn')}</div></div>
    </div>
  </div>;
}
