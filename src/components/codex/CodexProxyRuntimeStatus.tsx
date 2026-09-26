import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { getCodexProxyRuntimeStatus, type CodexProxyRuntimeStatus } from '../../services/codexAccountProxyService';
import { proxyRuntimeLabelKey, proxyRuntimeRows, singleFlightRead } from '../../utils/codexProxyPreview';
import { CodexProxyRuntimeDetails, CodexProxyRuntimePort } from './CodexProxyRuntimeDetails';

const readStatus = singleFlightRead(getCodexProxyRuntimeStatus);

/** This is process liveness, not connectivity or a substitute for an exit test. */
export function useCodexProxyRuntimeStatus(accountId: string, revision: unknown, enabled = true) {
  const [result, setResult] = useState({ accountId, revision, status: null as CodexProxyRuntimeStatus | null, failed: false });
  const { status, failed } = result.accountId === accountId && Object.is(result.revision, revision)
    ? result : { status: null, failed: false };
  useEffect(() => {
    if (!enabled) return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    setResult({ accountId, revision, status: null, failed: false });
    const refresh = async () => {
      try {
        const next = await readStatus(accountId);
        if (!disposed) setResult({ accountId, revision, status: next, failed: false });
      } catch {
        if (!disposed) setResult({ accountId, revision, status: null, failed: true });
      } finally {
        // No overlapping requests. Cleanup never updates a closed/switched dialog.
        if (!disposed) timer = setTimeout(refresh, 5000);
      }
    };
    void refresh();
    return () => { disposed = true; if (timer) clearTimeout(timer); };
  }, [accountId, revision, enabled]);
  return { status: enabled ? status : null, failed: enabled && failed };
}

export function CodexProxyRuntimeStatusPanel({ accountId, revision }: { accountId: string; revision: unknown }) {
  const { t } = useTranslation();
  const { status, failed } = useCodexProxyRuntimeStatus(accountId, revision);
  return <section className="codex-proxy-runtime" aria-label={t('codex.proxy.runtimeTitle')}>
    <strong>{t('codex.proxy.runtimeTitle')}</strong>
    {status ? <div className="codex-proxy-runtime-rows">
      {proxyRuntimeRows(status).map((row) => <div key={row.kind} className={`codex-proxy-runtime-item is-${row.state}`}>
        {t(proxyRuntimeLabelKey(row.kind))}
        <b>{row.state ? t(`codex.proxy.runtime_${row.state}`) : '—'}</b><CodexProxyRuntimePort row={row} />
        {row.node && <small className="codex-proxy-runtime-node">{t('codex.proxy.catalog.actualNode', { name: row.node })}</small>}
        <CodexProxyRuntimeDetails row={row} />
      </div>)}
    </div> : <span>{t(failed ? 'codex.proxy.runtimeUnavailable' : 'codex.proxy.runtimeLoading')}</span>}
    <p>{t('codex.proxy.runtimeHint')}</p>
  </section>;
}
