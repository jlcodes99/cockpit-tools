import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { preflightCodexProxyEngine } from '../../services/codexProxyEngineService';
import { Check, CheckSquare, RefreshCw, Search, ShieldCheck, Square, Users, X } from 'lucide-react';
import type { ProxyCatalog, ProxyCatalogSelections } from '../../services/codexProxyCatalogService';
import { bindProxyCatalog, catalogErrorKey } from '../../services/codexProxyCatalogService';
import { applyCodexUnifiedProxy, previewCodexUnifiedProxy, unifiedProxyErrorKey, type CodexUnifiedProxyPreview } from '../../services/codexUnifiedProxyService';
import { batchBindSummary, batchBindTargets, executeProxyBatch, proxyBatchCompletedSuccessfully, type BatchBindResult } from '../../utils/codexProxyBatch';
import { proxyAssignmentAccounts } from '../../utils/codexProxyAssignment';
import { canUseCodexAccountProxy } from '../../utils/codexAccountProxy';
import { useCodexAccountStore } from '../../stores/useCodexAccountStore';
import { useCodexProxyWorkspace } from './CodexProxyWorkspaceContext';
import { useCodexProxyAccountName } from './useCodexProxyExitEditor';
import { ModalErrorMessage } from '../ModalErrorMessage';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import '../../styles/pages/codex-proxy-assign.css';

export interface ProxyAssignment {
  sourceId: string;
  itemId: string;
  groupId: string;
  selections: ProxyCatalogSelections;
  catalog: ProxyCatalog;
}

/** Choose a scope and confirm in place; importing or browsing never writes an account. */
export function CodexProxyAssignDialog({ choice, onClose }: { choice: ProxyAssignment; onClose: () => void }) {
  const { t } = useTranslation();
  const { accounts, reloadUnified, acceptUnified } = useCodexProxyWorkspace();
  const name = useCodexProxyAccountName();
  const dialog = useRef<HTMLDivElement>(null);
  const mounted = useRef(false);
  const running = useRef(false);
  const cancelled = useRef(false);
  const previewTask = useRef<Promise<CodexUnifiedProxyPreview> | null>(null);
  const [scope, setScope] = useState<'accounts' | 'unified'>('accounts');
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [search, setSearch] = useState('');
  const [preview, setPreview] = useState<CodexUnifiedProxyPreview | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [previewError, setPreviewError] = useState('');
  const [attempt, setAttempt] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [results, setResults] = useState<BatchBindResult[]>([]);
  const [unifiedSaved, setUnifiedSaved] = useState(false);
  const [appliedIds, setAppliedIds] = useState<string[]>([]);
  const source = choice.catalog.sources.find((entry) => entry.id === choice.sourceId);
  const item = source?.nodes.find((entry) => entry.id === choice.itemId) ?? source?.groups.find((entry) => entry.id === choice.itemId);
  const eligible = useMemo(() => accounts.filter(canUseCodexAccountProxy), [accounts]);
  const visible = eligible.filter((entry) => name(entry).toLowerCase().includes(search.trim().toLowerCase()));
  const picked = proxyAssignmentAccounts(accounts, selectedIds);
  const targets = batchBindTargets(picked.filter((entry) => !appliedIds.includes(entry.id)), (entry) => entry.egress_proxy,
    choice.sourceId, choice.itemId, choice.groupId, choice.selections);
  const overwrite = targets.filter((entry) => entry.willOverwrite).length;
  const summary = batchBindSummary(results);
  const allVisible = visible.length > 0 && visible.every((entry) => selectedIds.includes(entry.id));

  useEscCloseTopmost(true, () => { if (!running.current) onClose(); });
  useModalScrollLock(true);
  useModalFocusTrap(dialog, true);
  useEffect(() => {
    mounted.current = true;
    cancelled.current = false;
    return () => { mounted.current = false; cancelled.current = true; };
  }, []);
  useEffect(() => {
    if (scope !== 'unified') return;
    let live = true;
    setPreview(null); setPreviewLoading(true); setPreviewError('');
    // The promise is shared across StrictMode's replay; source snapshots take a backend lock.
    const task = previewTask.current ?? previewCodexUnifiedProxy(choice.sourceId, choice.itemId, choice.selections, choice.groupId || undefined);
    previewTask.current = task;
    void task.then((next) => { if (live) setPreview(next); }).catch((caught) => {
      if (live) { setPreview(null); setPreviewError(t(unifiedProxyErrorKey(caught))); }
    }).finally(() => {
      // The source guard outlives a scope change. Release the shared pending state even
      // when its result is no longer visible, and never cache completed impact counts.
      if (previewTask.current === task) {
        previewTask.current = null;
        if (mounted.current) setPreviewLoading(false);
      }
    });
    return () => { live = false; };
    // Language changes should not repeat backend work.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scope, attempt, choice]);

  const clearFeedback = () => { setError(''); setResults([]); setUnifiedSaved(false); };
  const changeScope = (next: 'accounts' | 'unified') => {
    if (running.current || next === scope) return;
    setScope(next); setPreview(null); clearFeedback();
  };
  const toggle = (id: string) => { clearFeedback(); setSelectedIds((old) => old.includes(id) ? old.filter((value) => value !== id) : [...old, id]); };
  const save = async () => {
    if (running.current || previewTask.current || previewLoading || !item?.supported || (scope === 'unified' ? !preview || unifiedSaved : !targets.length)) return;
    running.current = true; cancelled.current = false; setBusy(true); setError(''); setResults([]);
    try {
      await preflightCodexProxyEngine();
      if (!mounted.current || cancelled.current) return;
      if (scope === 'unified') {
        const next = await applyCodexUnifiedProxy(choice.sourceId, choice.itemId, choice.selections, choice.groupId || undefined);
        acceptUnified(next);
        if (mounted.current) setUnifiedSaved(true);
        if (mounted.current) onClose();
      } else {
        let completed: BatchBindResult[] = [];
        await executeProxyBatch(targets, {
          cancelled: () => cancelled.current,
          bind: (account) => bindProxyCatalog(account.id, choice.sourceId, choice.itemId, choice.selections, choice.groupId),
          applied: (updated) => {
            useCodexAccountStore.getState().applyAccountSnapshot(updated);
            if (mounted.current) setAppliedIds((old) => [...old, updated.id]);
          },
          errorKey: catalogErrorKey,
          progress: (next) => {
            completed = next;
            if (!mounted.current) return;
            setResults(next);
            if (next.some((entry) => !entry.ok)) setError(t('codex.proxy.batchResult', batchBindSummary(next)));
          },
        });
        if (mounted.current) setPreview(null);
        reloadUnified();
        if (mounted.current && proxyBatchCompletedSuccessfully(completed, targets.length, cancelled.current)) onClose();
      }
    } catch (caught) {
      if (mounted.current) setError(t(scope === 'unified' ? unifiedProxyErrorKey(caught) : catalogErrorKey(caught)));
    } finally {
      running.current = false;
      if (mounted.current) setBusy(false);
    }
  };

  return createPortal(<div className="modal-overlay codex-proxy-assign-overlay">
    <div ref={dialog} className="modal codex-proxy-assign-dialog" role="dialog" aria-modal="true" aria-labelledby="codex-proxy-assign-title" tabIndex={-1}>
      <header className="modal-header">
        <div><h2 id="codex-proxy-assign-title">{t('codex.proxy.manager.assign')}</h2>
          <p>{[source?.name, item?.name].filter(Boolean).join(' · ')}</p></div>
        <button type="button" className="btn btn-secondary compact" disabled={busy} aria-label={t('common.close')} onClick={onClose}><X size={18} /></button>
      </header>
      <div className="modal-body">
        <div className="codex-proxy-assign-scopes" role="group" aria-label={t('codex.proxy.manager.assign')}>
          <button type="button" className={`btn codex-proxy-assign-scope${scope === 'accounts' ? ' is-active' : ''}`} disabled={busy} aria-pressed={scope === 'accounts'} onClick={() => changeScope('accounts')}>
            <Users size={19} /><span>{t('codex.proxy.manager.specific')}</span></button>
          <button type="button" className={`btn codex-proxy-assign-scope${scope === 'unified' ? ' is-active' : ''}`} disabled={busy} aria-pressed={scope === 'unified'} onClick={() => changeScope('unified')}>
            <ShieldCheck size={19} /><span>{t('codex.proxy.manager.shared')}</span></button>
        </div>
        {scope === 'accounts' ? <>
          <div className="codex-proxy-assign-search">
            <label className="codex-proxy-search"><Search size={16} /><input value={search} disabled={busy} onChange={(event) => setSearch(event.target.value)} placeholder={t('codex.proxy.search')} aria-label={t('codex.proxy.search')} /></label>
            <button type="button" className="btn btn-secondary compact" disabled={busy || !visible.length} onClick={() => {
              clearFeedback(); const ids = new Set(visible.map((entry) => entry.id));
              setSelectedIds((old) => allVisible ? old.filter((id) => !ids.has(id)) : [...new Set([...old, ...ids])]);
            }}>{t(allVisible ? 'codex.proxy.batchClearAll' : 'common.selectAll')}</button>
          </div>
          <div className="codex-proxy-assign-accounts">{visible.map((account) => <button key={account.id} type="button" className={`btn codex-proxy-assign-account${selectedIds.includes(account.id) ? ' is-active' : ''}`} disabled={busy} aria-pressed={selectedIds.includes(account.id)} onClick={() => toggle(account.id)}>
            {selectedIds.includes(account.id) ? <CheckSquare size={17} /> : <Square size={17} />}<span>{name(account)}</span>
          </button>)}</div>
          {!eligible.length && <p className="codex-proxy-page-note">{t('codex.proxy.emptyDescription')}</p>}
          {eligible.length > 0 && !visible.length && <p className="codex-proxy-page-note">{t('codex.proxy.noMatches')}</p>}
          <p className="codex-proxy-page-note">{t('codex.proxy.batchSelectedCount', { count: picked.length })}
            {overwrite > 0 && <span> · {t('codex.proxy.batchOverwrite', { count: overwrite })}</span>}</p>
          {picked.length > 0 && !targets.length && !results.length && <p className="codex-proxy-page-note">{t('codex.proxy.batchEmpty')}</p>}
        </> : <div className="codex-proxy-assign-shared">
          <p>{t('codex.proxy.manager.sharedHint')}</p>
          {previewLoading && <p role="status">{t('common.loading')}</p>}
          {previewError && <><ModalErrorMessage message={previewError} /><button type="button" className="btn btn-secondary compact" disabled={busy} onClick={() => { previewTask.current = null; setAttempt((old) => old + 1); }}>{t('common.retry')}</button></>}
          {!previewLoading && preview && <p>{t('codex.proxy.unified.enableMessage', {
            count: Math.max(0, preview.eligibleAccountIds.length - preview.independentAccountIds.length), independent: preview.independentAccountIds.length,
          })}</p>}
        </div>}
        <ModalErrorMessage message={error} />
        {results.length > 0 && <div className="codex-proxy-assign-result" role="status">
          {!error && <strong>{t('codex.proxy.batchResult', summary)}</strong>}
          <ul>{results.filter((entry) => !entry.ok).map((entry) => <li key={entry.accountId}><span>{name(accounts.find((account) => account.id === entry.accountId))}</span><span>{t(entry.errorKey ?? 'codex.proxy.catalog.failed')}</span></li>)}</ul>
        </div>}
        {unifiedSaved && <p className="codex-proxy-assign-result" role="status"><Check size={16} />{t('codex.proxy.saved')}</p>}
        <p className="codex-proxy-page-note">{t('codex.proxy.restartHint')}</p>
      </div>
      <footer className="modal-footer">
        {busy && scope === 'accounts' ? <button type="button" className="btn btn-secondary" onClick={() => { cancelled.current = true; }}>{t('common.cancel')}</button>
          : <button type="button" className="btn btn-secondary" disabled={busy} onClick={onClose}>{t('common.close')}</button>}
        <button type="button" className="btn btn-primary" disabled={busy || previewLoading || !item?.supported || (scope === 'unified' ? !preview || unifiedSaved : targets.length === 0)} onClick={() => void save()}>
          {busy || previewLoading ? <RefreshCw size={15} className="loading-spinner" /> : <Check size={15} />}{t(previewLoading ? 'common.loading' : busy ? 'common.processing' : 'codex.proxy.manager.apply')}</button>
      </footer>
    </div>
  </div>, document.body);
}
