import { useCallback, useEffect, useRef, useState } from 'react';
import { RefreshCw } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { ModalErrorMessage } from '../ModalErrorMessage';
import {
  getCodexAccountProxyRecentRequests,
  type CodexProxyRecentRequest,
} from '../../services/codexAccountProxyService';

interface Props {
  accountId: string;
  revision: string;
  inModal?: boolean;
}

export function CodexProxyRecentRequests({ accountId, revision, inModal = false }: Props) {
  const { t } = useTranslation();
  const [requests, setRequests] = useState<CodexProxyRecentRequest[]>([]);
  const [loading, setLoading] = useState(false);
  const [failed, setFailed] = useState(false);
  const requestSequence = useRef(0);

  const refresh = useCallback(async () => {
    const sequence = ++requestSequence.current;
    setLoading(true);
    setFailed(false);
    try {
      const next = await getCodexAccountProxyRecentRequests(accountId);
      if (requestSequence.current === sequence) setRequests(next);
    } catch {
      if (requestSequence.current === sequence) {
        setRequests([]);
        setFailed(true);
      }
    } finally {
      if (requestSequence.current === sequence) setLoading(false);
    }
  }, [accountId]);

  useEffect(() => {
    void refresh();
    return () => { requestSequence.current += 1; };
  }, [refresh, revision]);

  return <section className="codex-proxy-recent" aria-label={t('codex.proxy.recentTitle')}>
    <div className="codex-proxy-recent-head">
      <strong>{t('codex.proxy.recentTitle')}</strong>
      <button type="button" className="btn btn-secondary compact" onClick={() => void refresh()} disabled={loading}>
        <RefreshCw size={14} className={loading ? 'loading-spinner' : undefined} />
        {t('common.refresh')}
      </button>
    </div>
    <p className="codex-proxy-muted">{t('codex.proxy.recentHint')}</p>
    {failed ? (inModal ? <ModalErrorMessage message={t('codex.proxy.recentFailed')} />
      : <p className="codex-proxy-recent-empty" role="status">{t('codex.proxy.recentFailed')}</p>)
      : loading && requests.length === 0 ? <p className="codex-proxy-recent-empty" role="status">{t('common.loading')}</p>
      : requests.length === 0 ? <p className="codex-proxy-recent-empty">{t('codex.proxy.recentEmpty')}</p>
      : <ol className="codex-proxy-recent-list">{requests.map((request, index) => <li key={`${request.timestamp}-${index}`}>
        <span className="codex-proxy-recent-model">{request.modelId || t('codex.proxy.recentUnknownModel')}</span>
        <span className={request.success ? 'is-success' : 'is-failure'}>
          {t(request.success ? 'codex.proxy.recentSuccess' : 'codex.proxy.recentFailure')}
          {request.httpStatus ? ` · HTTP ${request.httpStatus}` : ''}
        </span>
        <small>{Math.max(0, request.latencyMs)} ms · {new Date(request.timestamp).toLocaleString()}</small>
      </li>)}</ol>}
  </section>;
}
