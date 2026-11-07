import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { listen } from '@tauri-apps/api/event';
import { openUrl } from '@tauri-apps/plugin-opener';
import { CircleAlert } from 'lucide-react';
import type { CodexAccount } from '../../types/codex';
import { refreshCodexQuota } from '../../services/codexService';
import { useCodexDeviceRestrictions, allowCodexDeviceRestrictionRetry,
  type CodexDeviceRestriction } from '../../stores/useCodexDeviceRestrictions';
import './CodexDeviceRestrictions.css';

const HELP = 'https://help.openai.com/en/articles/20001559-understanding-card-and-device-limits-in-chatgpt';
const SUPPORT = 'https://help.openai.com/en/articles/6614161-how-can-i-contact-support';

export function CodexDeviceRestrictionBadge({ accountId }: { accountId: string }) {
  const { t } = useTranslation();
  const restricted = useCodexDeviceRestrictions(s => s.records.some(r => r.subject === accountId && r.status === 'restricted'));
  if (!restricted) return null;
  return <span className="codex-status-pill quota-error codex-device-restriction-badge"
    title={t('common.deviceRestriction.note')}><CircleAlert size={12} />{t('common.deviceRestriction.badge')}</span>;
}

function RestrictionRow({ record, name }: { record: CodexDeviceRestriction; name: string }) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const act = async (work: () => Promise<unknown>) => {
    setFailed(false); setBusy(true);
    try { await work(); } catch { setFailed(true); } finally { setBusy(false); }
  };
  const state = record.status === 'recovered' ? 'recovered' : record.subject.startsWith('login:') ? 'badge' : record.suspended ? 'pausedLabel' : 'retryEnabled';
  return <div className="codex-device-restriction-row">
    <div className="codex-device-restriction-heading"><strong>{name}</strong>
      <span>{t(`common.deviceRestriction.${record.scope}`)}</span>
      <span>{t(`common.deviceRestriction.${state}`)}</span></div>
    <small>{t('common.deviceRestriction.observed', { first: new Date(record.firstSeenAt).toLocaleString(), last: new Date(record.lastSeenAt).toLocaleString(), count: record.occurrences })}</small>
    {record.instanceId && <small>{t('codex.switchAuth.observationInstance', { instance: record.instanceId })}</small>}
    <div className="codex-device-restriction-actions">
      {record.status === 'restricted' && record.scope === 'quota' && !record.subject.startsWith('login:') &&
        <button type="button" className="btn btn-secondary btn-sm" disabled={busy}
          onClick={() => void act(async () => {
            try { await refreshCodexQuota(record.subject); }
            finally { await useCodexDeviceRestrictions.getState().refresh(); }
          })}>{t('common.deviceRestriction.check')}</button>}
      {record.suspended && !record.subject.startsWith('login:') && <button type="button" className="btn btn-secondary btn-sm"
        disabled={busy} title={t('common.deviceRestriction.retryNote')}
        onClick={() => void act(() => allowCodexDeviceRestrictionRetry(record))}>{t('common.deviceRestriction.allowRetry')}</button>}
      <button type="button" className="btn btn-secondary btn-sm" disabled={busy}
        onClick={() => void act(() => openUrl(HELP))}>{t('common.deviceRestriction.help')}</button>
      <button type="button" className="btn btn-secondary btn-sm" disabled={busy}
        onClick={() => void act(() => openUrl(SUPPORT))}>{t('common.deviceRestriction.support')}</button>
    </div>
    {busy && <small role="status">{t('common.loading')}</small>}
    {failed && <p className="codex-device-restriction-error" role="alert">{t('common.deviceRestriction.failed')}</p>}
  </div>;
}

export function CodexDeviceRestrictions({ accounts, maskAccountText }: { accounts: CodexAccount[]; maskAccountText: (text: string) => string }) {
  const { t } = useTranslation();
  const { records, loading, failed, refresh } = useCodexDeviceRestrictions();
  const [expanded, setExpanded] = useState(false);
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen('codex:device-restrictions-changed', () => void refresh()).then(stop => {
      if (disposed) stop(); else unlisten = stop;
    }).catch(() => { /* Manual refresh remains available if event subscription fails. */ });
    void refresh();
    return () => { disposed = true; unlisten?.(); };
  }, [refresh]);
  if (!records.length && !failed) return null;
  const accountMap = new Map(accounts.map(a => [a.id, a]));
  return <section className="codex-device-restrictions">
    <div className="codex-device-restriction-actions">
      <button type="button" className="btn btn-secondary btn-sm" aria-expanded={expanded}
        onClick={() => setExpanded(v => !v)}><CircleAlert size={14} />{t('common.deviceRestriction.title')}</button>
      <button type="button" className="btn btn-secondary btn-sm" disabled={loading} onClick={() => void refresh()}>{t('common.refresh')}</button>
      {loading && <span role="status">{t('common.loading')}</span>}
    </div>
    {failed && <p role="alert">{t('common.deviceRestriction.failed')}</p>}
    {expanded && <div className="codex-device-restriction-body">
      <p>{t('common.deviceRestriction.note')}</p>
      <p>{t('common.deviceRestriction.retryNote')}</p>
      {records.map(record => <RestrictionRow key={`${record.subject}:${record.scope}`} record={record}
        name={record.subject.startsWith('login:') ? t('common.deviceRestriction.loginAttempt')
          : maskAccountText(accountMap.get(record.subject)?.account_name || accountMap.get(record.subject)?.email || t('common.deviceRestriction.removed'))} />)}
    </div>}
  </section>;
}
