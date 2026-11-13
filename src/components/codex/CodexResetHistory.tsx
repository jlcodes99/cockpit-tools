import { useState, type ReactNode } from 'react';
import { RefreshCw } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { ModalErrorMessage } from '../ModalErrorMessage';
import { useCodexResetHistory } from '../../hooks/useCodexResetHistory';
import type { ResetHistorySource, ResetObservation } from '../../services/codexResetHistoryService';
import { ruleDescriptions } from './CodexResetAutomation';
import './CodexResetHistory.css';

const outcomes: Record<string, string> = {
  granted: 'received', redeemed: 'used', reset: 'used', already_redeemed: 'alreadyRedeemed',
  nothing_to_reset: 'nothingToReset', no_credit: 'noCredit', pending: 'pending', unconfirmed: 'unconfirmed',
};
const reasons: Record<string, string> = { manual: 'manual', expiry: 'expiry', quota: 'weekly' };

function Observation({ value, stage }: { value: ResetObservation; stage: 'before' | 'after' }) {
  const { t, i18n } = useTranslation();
  const percent = new Intl.NumberFormat(i18n.language, { style: 'percent' });
  const fields = [
    value.available_count === null ? null : t('codex.quota.resetCredits', { count: value.available_count }),
    value.hourly && `${t('codex.quota.hourly')} ${percent.format(value.hourly.remaining / 100)}`,
    value.weekly && `${t('codex.quota.weekly')} ${percent.format(value.weekly.remaining / 100)}`,
  ].filter(Boolean);
  return <>
    <p>{t(`codex.resetHistory.${stage}`)} · {fields.length ? fields.join(' · ') : '—'}</p>
    {value.available_card_ids.length > 0 && (
      <p>{t('codex.resetHistory.availableCardIds')} · {value.available_card_ids.join(', ')}</p>
    )}
  </>;
}

function HistoryPanel({ accountId, source }: { accountId: string; source: ResetHistorySource }) {
  const { t, i18n } = useTranslation();
  const history = useCodexResetHistory(accountId, source);
  const date = (at: number | null) => at === null ? null : new Date(at);
  return <div role="tabpanel" className="codex-reset-history-panel">
    <div className="codex-reset-history-heading">
      <p>{t(`codex.resetHistory.${source === 'official' ? 'officialHint' : 'localHint'}`)}</p>
      <button type="button" className="btn btn-secondary" disabled={history.loading}
        onClick={() => void history.refresh()} aria-label={t('common.refresh')}>
        <RefreshCw size={14} className={history.loading ? 'loading-spinner' : ''} />
      </button>
    </div>
    {history.loading && <p role="status" className="codex-reset-history-empty">{t('common.loading')}</p>}
    {!history.loading && !history.error && history.entries.length === 0 && (
      <p className="codex-reset-history-empty">{t(`codex.resetHistory.${source === 'official' ? 'officialEmpty' : 'localEmpty'}`)}</p>
    )}
    <ol className="codex-reset-history-list">{history.entries.map(entry => {
      const time = date(entry.at);
      return <li key={entry.id}>
        <div className="codex-reset-history-event">
          <strong>{t(`codex.resetHistory.${outcomes[entry.kind] ?? 'unknown'}`)}</strong>
          {entry.reason && <span>{t(`codex.resetHistory.${reasons[entry.reason] ?? 'unknown'}`)}</span>}
          {entry.detail && <p>{entry.detail}</p>}
          {entry.requestId && <span className="codex-reset-history-request">{t('codex.resetHistory.requestId')}: {entry.requestId}</span>}
          {(entry.context || entry.receipt) && <details className="codex-reset-history-observations">
            <summary>{t('codex.resetHistory.observations')}</summary>
            {entry.context && <>
              <p>{t('codex.resetHistory.effectiveRule')} · {ruleDescriptions(entry.context.rule, t).join(t('codex.resetAutomation.orSeparator'))}</p>
              <Observation value={entry.context.before} stage="before" />
            </>}
            {entry.receipt?.credit_id && <p>{t('codex.resetHistory.redeemedCardId')} · {entry.receipt.credit_id}</p>}
            {entry.receipt?.windows_reset != null && <p>{t('codex.resetHistory.windowsReset')} · {entry.receipt.windows_reset}</p>}
            {entry.after && <Observation value={entry.after} stage="after" />}
          </details>}
        </div>
        <time dateTime={time?.toISOString()}>
          {time ? <>{time.toLocaleDateString(i18n.language, { month: 'short', day: 'numeric', year: source === 'local' ? 'numeric' : undefined })}
            <span>{time.toLocaleTimeString(i18n.language, { hour: '2-digit', minute: '2-digit', timeZoneName: 'short' })}</span></>
            : t('codex.quota.resetCreditTimeUnknown')}
        </time>
      </li>;
    })}</ol>
    <ModalErrorMessage message={history.error ? `${t('codex.resetHistory.loadFailed')}: ${history.error}` : null}
      scrollKey={history.errorKey} position="bottom" />
    <div className="codex-reset-history-actions">
      {history.error && <button type="button" className="btn btn-secondary" disabled={history.loading}
        onClick={() => void history.retry()}>{t('common.retry')}</button>}
      {history.cursor !== null && !history.error && <button type="button" className="btn btn-secondary" disabled={history.loading}
        onClick={() => void history.more()}>{t('codex.resetHistory.more')}</button>}
    </div>
  </div>;
}

export function CodexResetHistory({ accountId, children }: { accountId: string; children: ReactNode }) {
  const { t } = useTranslation();
  const [tab, setTab] = useState<'available' | ResetHistorySource>('available');
  return <section className="codex-reset-credit-confirm-details codex-reset-history">
    <div role="tablist" aria-label={t('codex.resetHistory.title')} className="codex-reset-history-tabs">
      {(['available', 'official', 'local'] as const).map(value => <button type="button" role="tab" key={value}
        className={`btn btn-secondary ${tab === value ? 'is-active' : ''}`} aria-selected={tab === value}
        onClick={() => setTab(value)}>{t(`codex.resetHistory.${value}`)}</button>)}
    </div>
    {tab === 'available' ? <div role="tabpanel">{children}</div>
      : <HistoryPanel key={tab} accountId={accountId} source={tab} />}
  </section>;
}
