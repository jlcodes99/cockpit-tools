import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { Activity, Check, RefreshCw, Server, ShieldCheck, StickyNote, TriangleAlert, X } from 'lucide-react';
import type { CodexAccount } from '../../types/codex';
import { proxySummary } from '../../utils/codexProxyPresentation';
import { sourceDefaultDraft } from '../../utils/codexProxySelection';
import { proxySourceInspectable } from '../../utils/codexProxyPickerModel';
import { executeProxyBatch, proxyBatchCompletedSuccessfully, type BatchBindResult } from '../../utils/codexProxyBatch';
import { summarizeProxyBatch, unifiedProxyActive } from '../../utils/codexProxyDraft';
import { proxyErrorKey } from '../../services/codexAccountProxyService';
import { useCodexAccountStore } from '../../stores/useCodexAccountStore';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import { ModalErrorMessage } from '../ModalErrorMessage';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import { CodexProxyPicker } from './CodexProxyPicker';
import { CodexProxyRecentRequests } from './CodexProxyRecentRequests';
import { CodexProxyRuntimeStatusPanel, useCodexProxyRuntimeStatus } from './CodexProxyRuntimeStatus';
import { CodexProxyActivityPanel } from './CodexProxyActivityPanel';
import { useProxyLatency } from './useProxyLatency';
import { useCodexProxyAccountName, useCodexProxyExitEditor } from './useCodexProxyExitEditor';
import { useCodexProxyWorkspace } from './CodexProxyWorkspaceContext';

/** An account's saved exit and its draft are kept separate until the explicit Save action. */
export function CodexProxyAccountDialog({ accountId, initialTab, onClose, onApplied }: {
  accountId: string; initialTab: 'edit' | 'details'; onClose: () => void; onApplied: (account: CodexAccount) => void;
}) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDivElement>(null);
  const editor = useCodexProxyExitEditor(accountId);
  const { catalog, catalogLoading, catalogError, reloadCatalog, acceptCatalog, unified, goSection } = useCodexProxyWorkspace();
  const resolveName = useCodexProxyAccountName();
  const [tab, setTab] = useState(initialTab);
  const [mode, setMode] = useState<'follow' | 'independent'>(() => editor.bound ? 'independent' : 'follow');
  const [noteOpen, setNoteOpen] = useState(false);
  const [submitted, setSubmitted] = useState(false);
  const [catalogPending, setCatalogPending] = useState(false);
  const previousBusy = useRef(editor.busy);
  const applied = useRef(onApplied);
  applied.current = onApplied;
  const closeAfterSave = useRef(onClose);
  closeAfterSave.current = onClose;
  const latency = useProxyLatency(editor.source);
  const measuring = Boolean(editor.busy) || catalogPending;
  const runtime = useCodexProxyRuntimeStatus(accountId, editor.savedBinding, tab === 'edit' && editor.saved);
  const availableSources = useMemo(() => catalog.sources.filter(proxySourceInspectable), [catalog]);
  const activeUnified = unifiedProxyActive(unified);
  const sharedLabel = unified?.binding ? [unified.binding.sourceName, unified.binding.name, unified.binding.selectedName].filter(Boolean).join(' · ') : '';
  const effectiveLabel = editor.bound ? proxySummary(editor.savedBinding) : activeUnified ? sharedLabel : t('codex.proxy.modeDefault');
  const canSave = mode === 'follow' ? editor.bound : editor.selectionReady && !editor.saved;

  useEscCloseTopmost(!noteOpen, () => { if (!editor.busy || editor.testing) onClose(); });
  useModalScrollLock(true);
  useModalFocusTrap(dialog, !noteOpen);
  useEffect(() => {
    if ((previousBusy.current === 'save' || previousBusy.current === 'unbind') && !editor.busy && !editor.error && editor.account) {
      applied.current({ ...editor.account, egress_proxy: editor.savedBinding });
      closeAfterSave.current();
    }
    previousBusy.current = editor.busy;
  }, [editor.account, editor.busy, editor.error, editor.savedBinding]);

  const chooseMode = (next: 'follow' | 'independent') => {
    setMode(next); setSubmitted(false);
    editor.select({ sourceId: editor.sourceId, itemId: editor.itemId, groupId: editor.groupId, selections: editor.selections });
  };
  const submit = () => { setSubmitted(true); if (mode === 'follow') editor.unbind(); else editor.save(); };
  const chooseSource = (id: string) => {
    const preset = sourceDefaultDraft(catalog.sources.find((entry) => entry.id === id));
    editor.select({ sourceId: id, itemId: preset?.itemId ?? '', groupId: preset?.groupId ?? '', selections: preset?.selections ?? {} });
    setSubmitted(false);
  };

  return createPortal(<div className="modal-overlay codex-proxy-account-dialog-overlay">
    <div className="modal codex-proxy-account-dialog" ref={dialog} role="dialog" aria-modal="true" aria-labelledby="codex-proxy-account-dialog-title" tabIndex={-1}>
      <header className="modal-header"><div><h2 id="codex-proxy-account-dialog-title">{resolveName(editor.account)}</h2>
        <p>{effectiveLabel}</p></div>
        <button type="button" className="btn btn-secondary compact" disabled={Boolean(editor.busy) && !editor.testing} onClick={onClose} aria-label={t('common.close')}><X size={18} /></button>
      </header>
      <div className="codex-proxy-account-dialog-tabs" role="tablist" aria-label={t('codex.proxy.accounts.title')}>
        {(['edit', 'details'] as const).map((value) => <button type="button" key={value} role="tab" aria-selected={tab === value}
          className={`btn btn-secondary compact${tab === value ? ' active' : ''}`} disabled={measuring} onClick={() => { setTab(value); editor.clearError(); }}>
          {t(value === 'edit' ? 'codex.proxy.managerAccounts.change' : 'codex.proxy.managerAccounts.details')}</button>)}
      </div>
      <div className="modal-body">
        <ModalErrorMessage message={editor.error} />
        {submitted && editor.notice && <p className="codex-proxy-account-saved" role="status"><Check size={16} />{t('codex.proxy.saved')}</p>}
        {tab === 'edit' ? <>
          <div className="codex-proxy-account-mode-options" role="radiogroup" aria-label={t('codex.proxy.managerAccounts.mode')}>
            {(['follow', 'independent'] as const).map((value) => <button type="button" key={value} role="radio" aria-checked={mode === value}
              className={`btn codex-proxy-account-mode-option${mode === value ? ' active' : ''}`} disabled={measuring} onClick={() => chooseMode(value)}>
              <span className="codex-proxy-account-mode-dot">{mode === value && <Check size={12} />}</span>
              <span><strong>{t(value === 'follow' ? 'codex.proxy.managerAccounts.following' : 'codex.proxy.modeIndependent')}</strong>
                <small>{t(value === 'follow' ? 'codex.proxy.managerAccounts.followHint' : 'codex.proxy.managerAccounts.independentHint')}</small></span>
            </button>)}
          </div>
          {mode === 'follow' ? <div className="codex-proxy-account-follow-preview"><ShieldCheck size={21} /><div>
            <span>{t('codex.proxy.managerAccounts.proxy')}</span><strong>{activeUnified ? sharedLabel : t('codex.proxy.modeDefault')}</strong>
          </div></div> : <div className="codex-proxy-account-choice">
            {catalogError ? <div><ModalErrorMessage message={catalogError} />
              <button type="button" className="btn btn-secondary compact" onClick={reloadCatalog}>{t('common.retry')}</button></div>
              : catalogLoading && availableSources.length === 0 ? <p role="status">{t('common.loading')}</p>
                : availableSources.length === 0 ? <div className="codex-proxy-account-guide"><Server size={24} /><p>{t('codex.proxy.unified.emptyCatalog')}</p>
                  <button type="button" className="btn btn-primary" onClick={() => { onClose(); goSection('resources'); }}>{t('codex.proxy.manager.proxies')}</button></div>
                  : <><label className="codex-proxy-field"><span>{t('codex.proxy.catalog.sources')}</span>
                    <SingleSelectDropdown value={editor.sourceId} disabled={measuring} ariaLabel={t('codex.proxy.catalog.sources')}
                      options={availableSources.map((entry) => ({ value: entry.id, label: entry.name }))} onChange={chooseSource} /></label>
                    {editor.source && <CodexProxyPicker source={editor.source} itemId={editor.itemId} selectedGroupId={editor.groupId}
                      selections={editor.selections} busy={Boolean(editor.busy)} latency={latency} onCatalogChange={acceptCatalog} onPendingChange={setCatalogPending} runtimeStatus={runtime.status}
                      choose={(id, groupId) => { setSubmitted(false); editor.select({ sourceId: editor.sourceId, itemId: id, groupId, selections: {} }); }}
                      chooseMember={(id, member) => { setSubmitted(false); editor.select({ sourceId: editor.sourceId, itemId: editor.itemId,
                        groupId: editor.groupId, selections: { ...editor.selections, [id]: member } }); }} />}
                  </>}
            <div className="codex-proxy-account-check-actions">
              <button type="button" className="btn btn-secondary compact" disabled={measuring || (!editor.selectionReady && !editor.bound)} onClick={() => editor.test()}>
                {editor.testing ? <RefreshCw size={15} className="loading-spinner" /> : <Activity size={15} />}{t(editor.testing ? 'codex.proxy.testing' : 'codex.proxy.test')}</button>
              {editor.testing && <button type="button" className="btn btn-secondary compact" onClick={editor.cancelTest}>{t('codex.proxy.cancelCheck')}</button>}
            </div>
          </div>}
          <p className="codex-proxy-page-note">{t('codex.proxy.restartHint')}</p>
        </> : <div className="codex-proxy-account-details">
          <CodexProxyRuntimeStatusPanel accountId={accountId} revision={editor.savedBinding} />
          <div className="codex-proxy-account-check-actions">
            <button type="button" className="btn btn-secondary compact" disabled={measuring || !editor.bound} onClick={() => editor.test()}>
              {editor.testing ? <RefreshCw size={15} className="loading-spinner" /> : <Activity size={15} />}{t('codex.proxy.test')}</button>
            {editor.testing && <button type="button" className="btn btn-secondary compact" onClick={editor.cancelTest}>{t('codex.proxy.cancelCheck')}</button>}
            <button type="button" className="btn btn-secondary compact" disabled={measuring} onClick={() => setNoteOpen(true)}><StickyNote size={15} />{t('codex.proxy.accounts.note')}</button>
            {editor.account?.account_note?.trim() && <span className="codex-proxy-account-note-preview">{editor.account.account_note.trim()}</span>}
          </div>
          <CodexProxyRecentRequests inModal accountId={accountId} revision={JSON.stringify(editor.savedBinding)} />
          <CodexProxyActivityPanel inModal accountId={accountId} probeResult={editor.result && editor.resultScope === 'saved' ? editor.result : undefined} />
          <p className="codex-proxy-page-note">{t('codex.proxy.runtimeHint')}</p>
        </div>}
        {editor.result && <output className="codex-proxy-check-result" aria-live="polite"><ShieldCheck size={18} /><div>
          <strong>{t('codex.proxy.testPassed')}</strong><span>{editor.result.protocol} · {editor.result.ip} · {editor.result.latencyMs} ms</span>
          {editor.resultScope === 'selection' && <span>{t('codex.proxy.draftTested')}</span>}
          <time dateTime={new Date(editor.result.checkedAt).toISOString()}>{new Date(editor.result.checkedAt).toLocaleString()}</time></div></output>}
        {(tab === 'details' || mode === 'independent') && <p className="codex-proxy-page-note">{t('codex.proxy.testNotice')}</p>}
        {tab === 'details' && <div className="codex-proxy-tun-notice" role="note"><TriangleAlert size={17} /><p>{t('codex.proxy.catalog.tunNotice')}</p></div>}
      </div>
      <footer className="modal-footer"><button type="button" className="btn btn-secondary" disabled={Boolean(editor.busy) && !editor.testing} onClick={onClose}>{t('common.close')}</button>
        {tab === 'edit' && <button type="button" className="btn btn-primary" disabled={measuring || !canSave} onClick={submit}>
          {editor.busy === 'save' || editor.busy === 'unbind' ? <RefreshCw size={15} className="loading-spinner" /> : <Check size={15} />}{t(editor.busy === 'save' ? 'common.saving' : 'common.save')}</button>}
      </footer>
    </div>
    {noteOpen && editor.account && <CodexProxyNoteDialog account={editor.account} onClose={() => setNoteOpen(false)} />}
  </div>, document.body);
}

/** Restoring shared defaults is a partial-success operation; failures remain in this dialog. */
export function CodexProxyFollowDialog({ accounts, onApplied, onClose }: {
  accounts: CodexAccount[]; onApplied: (account: CodexAccount) => void; onClose: () => void;
}) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDivElement>(null);
  const [targets] = useState(() => accounts.map((account) => ({ account, willOverwrite: true })));
  const [results, setResults] = useState<BatchBindResult[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const cancelled = useRef(false);
  const running = useRef(false);
  const mounted = useRef(true);
  const resolveName = useCodexProxyAccountName();
  const summary = summarizeProxyBatch(results, accounts);
  const remaining = targets.filter((entry) => !results.some((result) => result.accountId === entry.account.id && result.ok));
  useEscCloseTopmost(true, () => { if (!busy) onClose(); });
  useModalScrollLock(true);
  useModalFocusTrap(dialog, true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; cancelled.current = true; }; }, []);
  const run = async () => {
    if (running.current || remaining.length === 0) return;
    running.current = true; cancelled.current = false; setBusy(true); setError('');
    const complete = results.filter((entry) => entry.ok);
    try {
      let completed: BatchBindResult[] = [];
      await executeProxyBatch(remaining, {
        cancelled: () => cancelled.current,
        bind: (entry) => useCodexAccountStore.getState().updateAccountEgressProxy(entry.id, null),
        applied: onApplied,
        errorKey: proxyErrorKey,
        progress: (next) => {
          completed = next;
          if (!mounted.current) return;
          const combined = [...complete, ...next];
          setResults(combined);
          if (combined.some((entry) => !entry.ok)) setError(t('codex.proxy.accounts.batchFailedHint'));
        },
      });
      if (mounted.current && proxyBatchCompletedSuccessfully(completed, remaining.length, cancelled.current)) onClose();
    } catch (caught) { if (mounted.current) setError(t(proxyErrorKey(caught))); }
    finally { running.current = false; if (mounted.current) setBusy(false); }
  };
  return createPortal(<div className="modal-overlay codex-proxy-account-dialog-overlay">
    <div className="modal codex-proxy-follow-dialog" ref={dialog} role="dialog" aria-modal="true" aria-labelledby="codex-proxy-follow-title" tabIndex={-1}>
      <header className="modal-header"><h2 id="codex-proxy-follow-title">{t('codex.proxy.managerAccounts.follow')}</h2>
        <button type="button" className="btn btn-secondary compact" disabled={busy} onClick={onClose} aria-label={t('common.close')}><X size={18} /></button></header>
      <div className="modal-body"><ModalErrorMessage message={error} /><p>{t('codex.proxy.managerAccounts.followConfirm', { count: targets.length })}</p>
        {results.length > 0 && <p role="status">{t('codex.proxy.batchResult', { success: summary.success, failed: summary.failed })}</p>}
        {summary.failed > 0 && <ul className="codex-proxy-account-follow-failures">{results.filter((entry) => !entry.ok).map((entry) => <li key={entry.accountId}>
          <strong>{resolveName(targets.find((target) => target.account.id === entry.accountId)?.account)}</strong><span>{t(entry.errorKey ?? 'codex.proxy.saveFailed')}</span>
        </li>)}</ul>}
      </div>
      <footer className="modal-footer"><button type="button" className="btn btn-secondary" onClick={() => busy ? cancelled.current = true : onClose()}>{t(busy ? 'common.cancel' : 'common.close')}</button>
        <button type="button" className="btn btn-primary" disabled={busy || remaining.length === 0} onClick={() => void run()}>
          {busy && <RefreshCw size={15} className="loading-spinner" />}{t('common.confirm')}</button></footer>
    </div>
  </div>, document.body);
}

/** The account note entry writes only the note field; the full note form stays in the account overview. */
function CodexProxyNoteDialog({ account, onClose }: { account: CodexAccount; onClose: () => void }) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDivElement>(null);
  const updateAccountNote = useCodexAccountStore((state) => state.updateAccountNote);
  const [note, setNote] = useState(account.account_note ?? '');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');

  useEscCloseTopmost(true, () => { if (!busy) onClose(); });
  useModalScrollLock(true);
  useModalFocusTrap(dialog, true);

  const submit = async () => {
    if (busy) return;
    setBusy(true); setError(''); setNotice('');
    try {
      await updateAccountNote(account.id, { note });
      setNotice(t('codex.proxy.accounts.noteSaved'));
    } catch {
      setError(t('codex.proxy.accounts.noteFailed'));
    } finally {
      setBusy(false);
    }
  };

  return createPortal(<div className="modal-overlay codex-proxy-accounts-note-overlay">
    <div className="modal codex-proxy-accounts-note" ref={dialog} role="dialog" aria-modal="true"
      aria-labelledby="codex-proxy-accounts-note-title" tabIndex={-1}>
      <header className="modal-header">
        <div><h2 id="codex-proxy-accounts-note-title">{t('codex.proxy.accounts.noteTitle')}</h2>
          <p>{t('codex.proxy.accounts.noteHint')}</p></div>
        <button type="button" className="btn btn-secondary compact" disabled={busy} onClick={onClose} aria-label={t('common.close')}><X size={18} /></button>
      </header>
      <div className="modal-body">
        <ModalErrorMessage message={error} />
        <label className="codex-proxy-accounts-note-field"><span>{t('codex.accountNote.label')}</span>
          <textarea className="codex-proxy-accounts-note-input" value={note} disabled={busy}
            placeholder={t('codex.accountNote.placeholder')} onChange={(event) => setNote(event.target.value)} /></label>
        {notice && <p className="codex-proxy-page-note" role="status">{notice}</p>}
      </div>
      <footer className="modal-footer">
        <button type="button" className="btn btn-secondary" disabled={busy} onClick={onClose}>{t('common.close')}</button>
        <button type="button" className="btn btn-primary" disabled={busy} onClick={() => void submit()}>
          {busy ? <RefreshCw size={15} className="loading-spinner" /> : <Check size={15} />}{t('common.save')}</button>
      </footer>
    </div>
  </div>, document.body);
}
