import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { queryCodexOverloadAttempts, queryCodexOverloadCounts, type CodexOverloadAttempt } from '../services/codexLocalAccessService';
import { formatRequestDiagnosticError } from '../utils/codexRequestDiagnostics';

const PAGE_SIZE = 50;

export function useCodexOverloadCounts(startAt: number, endAt: number, visible: boolean) {
  const [counts, setCounts] = useState<Record<string, number> | null>(null);
  const [error, setError] = useState(false);
  useEffect(() => {
    if (!visible) return;
    let disposed = false;
    setCounts(null);
    setError(false);
    let loading = false;
    const load = async () => {
      if (loading) return;
      loading = true;
      try {
        const value = await queryCodexOverloadCounts(startAt, endAt);
        if (!disposed) { setCounts(value); setError(false); }
      } catch {
        if (!disposed) setError(true);
      } finally {
        loading = false;
      }
    };
    void load();
    const timer = window.setInterval(() => void load(), 5_000);
    return () => { disposed = true; window.clearInterval(timer); };
  }, [startAt, endAt, visible]);
  return { counts, error };
}

export function CodexOverloadHistory({ accountId, accountName, startAt, endAt, onClose }: {
  accountId: string; accountName: string; startAt: number; endAt: number; onClose: () => void;
}) {
  const { t, i18n } = useTranslation();
  const [page, setPage] = useState<Array<CodexOverloadAttempt | undefined>>([undefined]);
  const [attempts, setAttempts] = useState<CodexOverloadAttempt[] | null>(null);
  const [error, setError] = useState('');
  const [refresh, setRefresh] = useState(0);
  const before = page[page.length - 1];

  useEffect(() => {
    let disposed = false;
    let loading = false;
    setAttempts(null);
    setError('');
    const load = async () => {
      if (loading) return;
      loading = true;
      try {
        const rows = await queryCodexOverloadAttempts(accountId, startAt, endAt, before);
        if (!disposed) { setAttempts(rows); setError(''); }
      } catch (cause) {
        if (!disposed) setError(formatRequestDiagnosticError(cause, t));
      } finally {
        loading = false;
      }
    };
    void load();
    const timer = window.setInterval(() => void load(), 5_000);
    return () => { disposed = true; window.clearInterval(timer); };
  }, [accountId, startAt, endAt, before, refresh, t]);

  return <section className="codex-api-service-panel">
    <div className="codex-api-service-panel-head">
      <h2>{t('codex.overload.history')} · {accountName}</h2>
      <button type="button" className="btn btn-secondary btn-sm" onClick={onClose}>{t('common.close')}</button>
    </div>
    <p className="codex-request-diagnostics-hint">{t('codex.overload.countHint')}</p>
    {error && <p role="alert">{error} <button type="button" className="btn btn-secondary btn-sm"
      onClick={() => setRefresh((value) => value + 1)}>{t('common.retry')}</button></p>}
    {!attempts && !error && <p role="status">{t('common.loading')}</p>}
    {attempts?.length === 0 && <p>{t('codex.overload.empty')}</p>}
    {!!attempts?.length && <div className="codex-api-service-config-list">
      {attempts.slice(0, PAGE_SIZE).map((attempt) => {
        const outcome = attempt.outcome === 'recovered'
          ? (attempt.finalAccountId === attempt.accountId ? 'sameAccount' : 'otherAccount')
          : (attempt.outcome ?? 'pending');
        return <div key={attempt.id} className="codex-api-service-account-meta codex-overload-attempt">
          <time dateTime={new Date(attempt.timestamp).toISOString()}>{new Date(attempt.timestamp).toLocaleString(i18n.language)}</time>
          <code>{attempt.model}</code>
          <span>{t('codex.overload.attempt', { number: attempt.sequence })}</span>
          <span>{t(`codex.overload.${outcome}`)}</span>
          <code title={attempt.requestId}>{attempt.requestId}</code>
        </div>;
      })}
      <div className="codex-api-service-head-actions">
        <button type="button" className="btn btn-secondary btn-sm" disabled={page.length < 2}
          onClick={() => setPage((value) => value.slice(0, -1))}>{t('pagination.prev')}</button>
        <button type="button" className="btn btn-secondary btn-sm" disabled={attempts.length <= PAGE_SIZE}
          onClick={() => setPage((value) => [...value, attempts[PAGE_SIZE - 1]])}>{t('pagination.next')}</button>
      </div>
    </div>}
  </section>;
}
