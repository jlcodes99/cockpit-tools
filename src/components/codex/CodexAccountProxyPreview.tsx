import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Activity, ArrowRight, Clock3, RefreshCw, ShieldCheck, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { CodexAccount } from '../../types/codex';
import { proxyRuntimeRows, proxyRuntimeChanges, proxyRuntimeLabelKey } from '../../utils/codexProxyPreview';
import { useCodexAccountStore } from '../../stores/useCodexAccountStore';
import { measureCodexAccountProxyLatency, proxyErrorKey, proxyEntryRecoveryErrorKey, restoreCodexAccountProxyEntry } from '../../services/codexAccountProxyService';
import { CodexProxyWorkspaceProvider } from './CodexProxyWorkspaceContext';
import { CodexProxyAccountDialog } from './CodexProxyAccountDialog';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import { ModalErrorMessage } from '../ModalErrorMessage';
import { useCodexProxyPreview } from './useCodexProxyPreview';
import { CodexProxyActivityPreview } from './CodexProxyActivityPreview';
import { CodexProxyRuntimeDetails, CodexProxyRuntimePort } from './CodexProxyRuntimeDetails';
import { CodexProxyConnectionSummary } from './CodexProxyConnectionSummary';
import { CodexProxyDisplayControl } from './CodexProxyDisplayControl';
import '../../styles/pages/codex-proxy-preview.css';

interface Props {
  account: CodexAccount;
  displayName: string;
  onClose: () => void;
  onManage: () => void;
}

/**
 * Egress summary with an explicit account-only quick switch. Resource management
 * remains on the shared page; the editor and backend binding transaction are reused.
 */
export function CodexAccountProxyPreview({ account, displayName, onClose, onManage }: Props) {
  const { t } = useTranslation();
  const titleId = useId();
  const dialog = useRef<HTMLDivElement>(null);
  const switchButton = useRef<HTMLButtonElement>(null);
  const latestAccount = useCodexAccountStore((state) => state.accounts.find((entry) => entry.id === account.id)) ?? account;
  const bindingKey = JSON.stringify([account.id, latestAccount.egress_proxy_disabled ?? false,
    latestAccount.egress_proxy ?? null, latestAccount.egress_proxy_url ?? null]);
  const data = useCodexProxyPreview(account.id, bindingKey);
  const operationEpoch = useRef(0);
  const [restoring, setRestoring] = useState(false);
  const [measuring, setMeasuring] = useState(false);
  const measureInFlight = useRef(false);
  const [measureError, setMeasureError] = useState<{ accountId: string; key: string } | null>(null);
  const restoreInFlight = useRef(false);
  const [restoreError, setRestoreError] = useState<{ accountId: string; key: string } | null>(null);
  const [refreshAfterRestore, setRefreshAfterRestore] = useState<string | null>(null);
  const activeAccount = useRef<string | null>(account.id);
  activeAccount.current = account.id;
  useEffect(() => {
    activeAccount.current = account.id;
    operationEpoch.current++;
    measureInFlight.current = false;
    restoreInFlight.current = false;
    setMeasuring(false);
    setRestoring(false);
    setMeasureError(null);
    setRestoreError(null);
    setRefreshAfterRestore(null);
    return () => { activeAccount.current = null; operationEpoch.current++; };
  }, [bindingKey]);
  useEffect(() => {
    if (refreshAfterRestore === account.id && !data.loading) {
      data.refresh();
      setRefreshAfterRestore(null);
    }
  }, [refreshAfterRestore, account.id, data.loading, data.refresh]);
  const restoreEntry = async () => {
    if (restoreInFlight.current || measureInFlight.current) return;
    restoreInFlight.current = true;
    setRestoring(true);
    setRestoreError(null);
    setMeasureError(null);
    const accountId = account.id;
    const epoch = operationEpoch.current;
    try {
      await restoreCodexAccountProxyEntry(accountId);
    } catch (error) {
      if (activeAccount.current === accountId && operationEpoch.current === epoch) setRestoreError({ accountId, key: proxyEntryRecoveryErrorKey(error) });
    } finally {
      if (activeAccount.current === accountId && operationEpoch.current === epoch) {
        restoreInFlight.current = false;
        setRestoring(false);
        if (activeAccount.current === accountId) setRefreshAfterRestore(accountId);
      }
    }
  };
  const measureLatency = async () => {
    if (measureInFlight.current || restoreInFlight.current) return;
    measureInFlight.current = true;
    setMeasuring(true);
    setMeasureError(null);
    setRestoreError(null);
    const accountId = account.id;
    const epoch = operationEpoch.current;
    try {
      await measureCodexAccountProxyLatency(accountId);
    } catch (error) {
      if (activeAccount.current === accountId && operationEpoch.current === epoch) setMeasureError({ accountId, key: proxyErrorKey(error, 'probeFailed') });
    } finally {
      if (activeAccount.current === accountId && operationEpoch.current === epoch) {
        measureInFlight.current = false;
        setMeasuring(false);
      }
    }
  };
  const accounts = useMemo(() => [latestAccount], [latestAccount]);
  const [editingAccount, setEditingAccount] = useState<string | null>(null);
  const editing = editingAccount === account.id;
  const [editMode, setEditMode] = useState<'follow' | 'independent' | 'disabled'>('independent');
  const openEditor = (mode: typeof editMode) => { setRestoreError(null); setMeasureError(null); setEditMode(mode); setEditingAccount(account.id); };
  const closeSwitch = () => {
    setEditingAccount(null);
    requestAnimationFrame(() => switchButton.current?.focus({ preventScroll: true }));
  };
  const [activityView, setActivityView] = useState<'api' | 'proxy'>('api');
  useEscCloseTopmost(!editing, onClose);
  useModalScrollLock(true);
  useModalFocusTrap(dialog, true);

  const error = [measureError?.accountId === account.id ? t(measureError.key) : '',
    restoreError?.accountId === account.id ? t(restoreError.key) : '',
    data.statusError ? t('codex.proxy.runtimeUnavailable') : '',
    data.requestsError ? t('codex.proxy.recentFailed') : ''].filter(Boolean).join(' · ');

  return createPortal(<><div className="modal-overlay codex-proxy-preview-overlay" inert={editing} aria-hidden={editing || undefined}>
    <div className="modal codex-proxy-preview" ref={dialog} role="dialog" aria-modal={!editing || undefined} aria-labelledby={titleId} tabIndex={-1}>
      <header className="modal-header">
        <div className="codex-proxy-preview-heading"><span className="codex-proxy-preview-icon"><ShieldCheck size={25} /></span>
          <div><h2 id={titleId}>{t('codex.proxy.previewTitle')}</h2><p>{displayName}</p></div></div>
        <button type="button" className="btn btn-secondary codex-proxy-preview-close" onClick={onClose} aria-label={t('common.close')}><X size={19} /></button>
      </header>
      <div className="modal-body">
        <ModalErrorMessage message={error} />
        <CodexProxyDisplayControl showSample={false} />
        <CodexProxyConnectionSummary account={latestAccount} status={data.status} failed={data.statusError}
          switchButtonRef={switchButton} onSwitch={() => openEditor('independent')}
          onFollow={() => openEditor('follow')} onDisable={() => openEditor('disabled')}
          onMeasure={() => void measureLatency()} measuring={measuring || restoring} />
        <div className="codex-proxy-preview-help"><span>{t('codex.proxy.runtimeHint')}</span></div>
        <section className="codex-proxy-preview-runtime" aria-label={t('codex.proxy.runtimeTitle')}>
          {proxyRuntimeRows(data.status).map((row) => <div key={row.kind}>
            <span>{t(proxyRuntimeLabelKey(row.kind))}</span><strong className={`is-${row.state ?? 'unknown'}`}>
              <i />{row.state ? t(`codex.proxy.runtime_${row.state}`) : data.statusError ? '—' : t('codex.proxy.runtimeLoading')}</strong>
            <small title={row.node || undefined}>{row.node || '—'}</small><CodexProxyRuntimePort row={row} />
            <CodexProxyRuntimeDetails row={row} />
          </div>)}
        </section>
        <div className="codex-proxy-preview-activity">
          <section className="codex-proxy-preview-panel">
            <h3><Clock3 size={17} />{t('codex.proxy.sessionHistory')}<span>{data.history.length}</span></h3>
            <p>{t('codex.proxy.sessionHistoryHint')}</p>
            <ol className="codex-proxy-preview-timeline" tabIndex={0} aria-label={t('codex.proxy.sessionHistory')}>{data.history.map((entry, index) => <li key={`${entry.timestamp}-${index}`}>
              <time dateTime={new Date(entry.timestamp).toISOString()}>{new Date(entry.timestamp).toLocaleTimeString()}</time>
              <div>{proxyRuntimeChanges(entry.status, data.history[index + 1]?.status).map((row) => <div className="codex-proxy-preview-change" key={row.kind}>
                <span>{t(proxyRuntimeLabelKey(row.kind))}<b>{row.state ? t(`codex.proxy.runtime_${row.state}`) : '—'}</b></span>
                {row.node && <small>{row.node}</small>}{row.port && <CodexProxyRuntimePort row={row} />}
                <CodexProxyRuntimeDetails row={row} />
              </div>)}
              </div></li>)}</ol>
            {data.history.length === 0 && !data.statusError && <p className="codex-proxy-preview-empty">{t('common.loading')}</p>}
          </section>
          <section className="codex-proxy-preview-panel">
            <div className="codex-proxy-preview-activity-switch" role="tablist" aria-label={t('codex.proxy.activity.title')}>
              <button type="button" className={`btn ${activityView === 'api' ? 'is-active' : ''}`} role="tab" aria-selected={activityView === 'api'} onClick={() => setActivityView('api')}><Activity size={15} />{t('codex.proxy.recentTitle')}<span>{data.requests.length}</span></button>
              <button type="button" className={`btn ${activityView === 'proxy' ? 'is-active' : ''}`} role="tab" aria-selected={activityView === 'proxy'} onClick={() => setActivityView('proxy')}>{t('codex.proxy.activity.logs')}</button>
            </div>
            {activityView === 'api' ? <>
              <p>{t('codex.proxy.recentHint')}</p>
              {data.requests.length === 0 ? !data.requestsError && <p className="codex-proxy-preview-empty">{t(data.loading ? 'common.loading' : 'codex.proxy.recentEmpty')}</p>
                : <ol className="codex-proxy-preview-requests" tabIndex={0} aria-label={t('codex.proxy.recentTitle')}>{data.requests.map((request, index) => <li key={`${request.timestamp}-${index}`}>
                  <div><strong>{request.modelId || t('codex.proxy.recentUnknownModel')}</strong><span className={request.success ? 'is-success' : 'is-failure'}>
                    {t(request.success ? 'codex.proxy.recentSuccess' : 'codex.proxy.recentFailure')}{request.httpStatus ? ` · HTTP ${request.httpStatus}` : ''}</span></div>
                  <small><time dateTime={new Date(request.timestamp).toISOString()}>{new Date(request.timestamp).toLocaleString()}</time><span>{Math.max(0, request.latencyMs)} ms</span></small>
                </li>)}</ol>}
            </> : <CodexProxyActivityPreview accountId={account.id} onManage={onManage} />}
          </section>
        </div>
      </div>
      <footer className="modal-footer">
        <div><button type="button" className="btn btn-secondary" disabled={data.loading || restoring || measuring} onClick={() => { setRestoreError(null); setMeasureError(null); data.refresh(); }}><RefreshCw size={15} className={data.loading ? 'loading-spinner' : undefined} />{t('common.refresh')}</button>
          {(restoring || data.status?.desktopEntry?.state !== 'listening') && <button type="button" className="btn btn-secondary" disabled={data.loading || restoring || measuring} onClick={() => void restoreEntry()}>
            <RefreshCw size={15} className={restoring ? 'loading-spinner' : undefined} />{t(restoring ? 'codex.proxy.restoringEntry' : 'codex.proxy.restoreEntry')}
          </button>}</div>
        <div><button type="button" className="btn btn-secondary" onClick={onClose}>{t('common.close')}</button>
          <button type="button" className="btn btn-primary" onClick={onManage}>{t('codex.proxy.management')}<ArrowRight size={16} /></button></div>
      </footer>
    </div>
  </div>
    {editing && <CodexProxyWorkspaceProvider key={account.id} accounts={accounts} accountId={account.id}>
      <CodexProxyAccountDialog accountId={account.id} initialTab="edit" initialMode={editMode}
        onClose={closeSwitch} onResources={onManage} onApplied={() => data.refresh()} />
    </CodexProxyWorkspaceProvider>}
  </>, document.body);
}
