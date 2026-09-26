import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { Activity, ArrowRight, Check, RefreshCw, Server, ShieldCheck, TriangleAlert, X } from 'lucide-react';
import { proxyErrorKey, type CodexProxyProbeResult } from '../../services/codexAccountProxyService';
import { cancelProxyCatalog, probeProxyCatalog, type ProxyCatalog, type ProxyCatalogSelections } from '../../services/codexProxyCatalogService';
import {
  applyCodexUnifiedProxy, disableCodexUnifiedProxy, previewCodexUnifiedProxy, unifiedProxyErrorKey,
  type CodexUnifiedProxyPreview, type CodexUnifiedProxyView,
} from '../../services/codexUnifiedProxyService';
import { defaultProxySelections, restoreProxySelection, savedRootProxySelections, sourceDefaultDraft } from '../../utils/codexProxySelection';
import { proxySourceInspectable } from '../../utils/codexProxyPickerModel';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import { ModalErrorMessage } from '../ModalErrorMessage';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import { CodexProxyPicker } from './CodexProxyPicker';
import { useProxyLatency } from './useProxyLatency';
import '../../styles/pages/codex-proxy-unified.css';

type UnifiedDialogKind = 'enable' | 'change' | 'disable';

interface Props {
  /** Every account on the page, including kinds that can never use an egress proxy. */
  totalAccounts: number;
  catalog: ProxyCatalog;
  onCatalogChange?: (catalog: ProxyCatalog) => void;
  view: CodexUnifiedProxyView | null;
  onViewChange: (view: CodexUnifiedProxyView) => void;
  onGoResources: () => void;
}

const dialogCopy: Record<UnifiedDialogKind, { title: string; message: string; confirm: string }> = {
  enable: { title: 'codex.proxy.unified.enableTitle', message: 'codex.proxy.unified.enableMessage', confirm: 'codex.proxy.unified.enableConfirm' },
  change: { title: 'codex.proxy.unified.changeTitle', message: 'codex.proxy.unified.changeMessage', confirm: 'codex.proxy.unified.changeConfirm' },
  disable: { title: 'codex.proxy.unified.disableTitle', message: 'codex.proxy.unified.disableMessage', confirm: 'codex.proxy.unified.disableConfirm' },
};

/**
 * Confirms one write. Enabling and changing read a preview first so the copy can name the
 * affected accounts; every failure stays inside the dialog (rules 15-17).
 */
export function CodexUnifiedProxyDialog({ kind, sourceId, itemId, groupId, selections, exitLabel, onClose, onApplied }: {
  kind: UnifiedDialogKind;
  sourceId: string;
  itemId: string;
  groupId: string;
  selections: ProxyCatalogSelections | null;
  exitLabel: string;
  onClose: () => void;
  onApplied: (view: CodexUnifiedProxyView) => void;
}) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDivElement>(null);
  const mounted = useRef(true);
  const asked = useRef<string | null>(null);
  const [preview, setPreview] = useState<CodexUnifiedProxyPreview | null>(null);
  const [loading, setLoading] = useState(kind !== 'disable');
  const [attempt, setAttempt] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const copy = dialogCopy[kind];

  useEscCloseTopmost(true, () => { if (!busy) onClose(); });
  useModalScrollLock(true);
  useModalFocusTrap(dialog, true);

  // The backend guards each source while a snapshot is built, so a replayed effect must not
  // open a second read: one preview per dialog keeps the counts and the error state honest.
  const previewKey = `${kind}|${sourceId}|${itemId}|${groupId}|${attempt}`;
  useEffect(() => {
    mounted.current = true;
    if (kind === 'disable') return () => { mounted.current = false; };
    if (asked.current === previewKey) return () => { mounted.current = false; };
    asked.current = previewKey;
    setLoading(true); setError('');
    void previewCodexUnifiedProxy(sourceId, itemId, selections ?? {}, groupId || undefined)
      .then((next) => { if (mounted.current) { setPreview(next); setLoading(false); } })
      .catch((caught) => {
        if (!mounted.current) return;
        setPreview(null); setLoading(false); setError(t(unifiedProxyErrorKey(caught)));
      });
    return () => { mounted.current = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [previewKey, kind, sourceId, itemId, groupId, selections]);

  const confirm = async () => {
    if (busy || (kind !== 'disable' && !preview)) return;
    setBusy(true); setError('');
    try {
      const next = kind === 'disable'
        ? await disableCodexUnifiedProxy()
        : await applyCodexUnifiedProxy(sourceId, itemId, selections ?? {}, groupId || undefined);
      onApplied(next);
    } catch (caught) {
      if (mounted.current) setError(t(unifiedProxyErrorKey(caught)));
    } finally {
      if (mounted.current) setBusy(false);
    }
  };
  const retry = () => { setError(''); setLoading(true); setAttempt((value) => value + 1); };

  return createPortal(<div className="modal-overlay codex-proxy-unified-overlay">
    <div className="modal codex-proxy-unified-dialog" ref={dialog} role="dialog" aria-modal="true" aria-labelledby="codex-proxy-unified-dialog-title" tabIndex={-1}>
      <header className="modal-header">
        <div className="codex-proxy-unified-dialog-heading">
          <span className="codex-proxy-unified-dialog-icon">{kind === 'disable' ? <TriangleAlert size={24} /> : <ShieldCheck size={24} />}</span>
          <div><h2 id="codex-proxy-unified-dialog-title">{t(copy.title)}</h2><p>{exitLabel}</p></div>
        </div>
        <button type="button" className="btn btn-secondary codex-proxy-unified-dialog-close" disabled={busy} onClick={onClose} aria-label={t('common.close')}><X size={19} /></button>
      </header>
      <div className="modal-body">
        <ModalErrorMessage message={error} />
        {loading && <p className="codex-proxy-page-note" role="status">{t('common.loading')}</p>}
        {/* Counts only ever describe a real preview; a failed read shows the error plus a retry. */}
        {!loading && (kind === 'disable' || preview) && <p className="codex-proxy-unified-dialog-message">{t(copy.message, {
          count: Math.max(0, (preview?.eligibleAccountIds.length ?? 0) - (preview?.independentAccountIds.length ?? 0)),
          independent: preview?.independentAccountIds.length ?? 0,
        })}</p>}
        {!loading && !preview && kind !== 'disable' && <button type="button" className="btn btn-secondary" disabled={busy} onClick={retry}>{t('common.retry')}</button>}
      </div>
      <footer className="modal-footer">
        <button type="button" className="btn btn-secondary" disabled={busy} onClick={onClose}>{t('common.cancel')}</button>
        <button type="button" className={kind === 'disable' ? 'btn btn-danger' : 'btn btn-primary'}
          disabled={busy || loading || (kind !== 'disable' && !preview)} onClick={() => void confirm()}>
          {busy ? <RefreshCw size={15} className="loading-spinner" /> : <Check size={15} />}{t(busy && kind !== 'disable' ? 'common.saving' : copy.confirm)}</button>
      </footer>
    </div>
  </div>, document.body);
}

/**
 * One shared exit for every eligible account. The binding stays a catalog reference on the
 * backend, so accounts added later follow it without any per-account write.
 */
export function CodexUnifiedProxyPanel({ totalAccounts, catalog, view, onViewChange, onGoResources, onCatalogChange }: Props) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState<{ sourceId: string; itemId: string; groupId: string; selections: ProxyCatalogSelections } | null>(null);
  const [editing, setEditing] = useState(false);
  const editorDialog = useRef<HTMLDivElement>(null);
  const [testing, setTesting] = useState(false);
  const [catalogPending, setCatalogPending] = useState(false);
  const [probe, setProbe] = useState<CodexProxyProbeResult | null>(null);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [dialog, setDialog] = useState<UnifiedDialogKind | null>(null);
  const requestId = useRef<string | null>(null);
  const mounted = useRef(true);
  const probeGeneration = useRef(0);

  const active = view?.mode === 'all_accounts' && Boolean(view?.binding);
  const availableSources = useMemo(() => catalog.sources.filter(proxySourceInspectable), [catalog]);
  const restored = useMemo(() => restoreProxySelection(catalog, view?.binding ? {
    protocol: view.binding.protocol, sourceId: view.binding.sourceId, itemId: view.binding.itemId, groupId: view.binding.groupId,
  } : null), [catalog, view]);
  const { sourceId, itemId, groupId } = draft ?? restored;
  const source = availableSources.find((entry) => entry.id === sourceId);
  const selected = source?.nodes.find((entry) => entry.id === itemId) ?? source?.groups.find((entry) => entry.id === itemId);
  const selectedChoices = useMemo(() => draft?.selections ?? (source ? savedRootProxySelections(source, view?.binding ?? null) : {}), [draft, source, view]);
  const selections = useMemo(() => source ? defaultProxySelections(source, itemId, selectedChoices) : null, [source, itemId, selectedChoices]);
  const selectionReady = !!selected?.supported && selections !== null;
  const latency = useProxyLatency(source);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      const pending = requestId.current;
      if (pending) void cancelProxyCatalog(pending).catch(() => {});
    };
  }, []);

  /** The source default only fills the shared-exit draft until the user confirms it. */
  const chooseSource = (id: string) => {
    const preset = sourceDefaultDraft(availableSources.find((entry) => entry.id === id));
    setDraft({ sourceId: id, itemId: preset?.itemId ?? '', groupId: preset?.groupId ?? '', selections: preset?.selections ?? {} });
    setProbe(null); setError(''); setNotice('');
  };
  const chooseItem = (id: string, nextGroup: string) => { setDraft({ sourceId, itemId: id, groupId: nextGroup, selections: {} }); setProbe(null); setError(''); setNotice(''); };
  const chooseMember = (selectorId: string, member: string) => {
    setDraft({ sourceId, itemId, groupId, selections: { ...selectedChoices, [selectorId]: member } });
    setProbe(null); setError(''); setNotice('');
  };
  const stopCheck = () => { const pending = requestId.current; if (pending) void cancelProxyCatalog(pending).catch(() => {}); };
  const checkExit = async () => {
    if (!source || testing || requestId.current || !selectionReady) return;
    const id = crypto.randomUUID();
    const generation = probeGeneration.current;
    requestId.current = id;
    setTesting(true); setError(''); setNotice(''); setProbe(null);
    try {
      const checked = await probeProxyCatalog(id, source.id, itemId, selections ?? {});
      if (mounted.current && probeGeneration.current === generation) setProbe(checked);
    } catch (caught) {
      if (mounted.current && probeGeneration.current === generation) setError(t(proxyErrorKey(caught, 'probeFailed')));
    } finally {
      if (requestId.current === id) requestId.current = null;
      if (mounted.current && probeGeneration.current === generation) setTesting(false);
    }
  };
  const openDialog = (kind: UnifiedDialogKind) => {
    if (dialog) return;
    setError(''); setNotice('');
    if (kind === 'disable') { setDialog('disable'); return; }
    if (!selectionReady || !source) return;
    setDialog(kind);
  };
  const applied = (next: CodexUnifiedProxyView) => {
    onViewChange(next);
    setDialog(null); setDraft(null); setEditing(false); setProbe(null); setError('');
    setNotice(t('codex.proxy.saved'));
  };

  const closeEditor = () => {
    stopCheck(); latency.cancel();
    probeGeneration.current += 1; requestId.current = null; setTesting(false);
    setEditing(false); setDraft(null); setProbe(null); setError('');
  };
  useEscCloseTopmost(editing && !dialog, closeEditor);
  useModalScrollLock(editing);
  useModalFocusTrap(editorDialog, editing && !dialog);

  if (view === null) return <section className="codex-proxy-unified" aria-busy="true">
    <p className="codex-proxy-page-note" role="status">{t('common.loading')}</p>
  </section>;

  const emptyCatalog = availableSources.length === 0;
  const eligible = view.eligibleAccountIds.length;
  const independent = view.independentAccountIds.length;
  const following = Math.max(0, eligible - independent);
  const unsupported = Math.max(0, totalAccounts - eligible);
  const currentLabel = view.binding
    ? [view.binding.sourceName, view.binding.name, view.binding.selectedName ?? ''].filter(Boolean).join(' · ')
    : '';
  const draftLabel = selected ? [source?.name ?? '', selected.name].filter(Boolean).join(' · ') : '';
  const dialogExit = dialog === 'disable' ? currentLabel : draftLabel || currentLabel;
  /** Both a probe and a latency batch hold the per-source guard, so no write may start beside them. */
  const measuring = testing || catalogPending;
  const changeReady = selectionReady && !emptyCatalog && !measuring;

  return <section className="codex-proxy-unified" aria-labelledby="codex-proxy-unified-title">
    <div className="codex-proxy-unified-bar">
      <span className={`codex-proxy-unified-bar-icon${active ? ' active' : ''}`}><ShieldCheck size={22} /></span>
      <div className="codex-proxy-unified-bar-copy"><div><h2 id="codex-proxy-unified-title">{t('codex.proxy.unified.title')}</h2>
        <strong title={currentLabel}>{active ? currentLabel : t('codex.proxy.managerUnified.off')}</strong></div>
        <p>{t('codex.proxy.managerUnified.hint')}
          {active && <span>{t('codex.proxy.unified.eligible', { count: following })}</span>}
          {independent > 0 && <span>{t('codex.proxy.unified.overridden', { count: independent })}</span>}
          {unsupported > 0 && <span>{t('codex.proxy.unified.unsupported', { count: unsupported })}</span>}</p>
      </div>
      <div className="codex-proxy-unified-bar-actions">
        {active && <button type="button" className="btn btn-secondary compact" onClick={() => openDialog('disable')}>{t('codex.proxy.unified.disable')}</button>}
        <button type="button" className="btn btn-primary compact" onClick={() => {
          setError(''); setNotice('');
          if (emptyCatalog) { onGoResources(); return; }
          if (!source) chooseSource(availableSources[0].id);
          setEditing(true);
        }}>{t(active ? 'codex.proxy.managerAccounts.change' : 'codex.proxy.managerUnified.choose')}</button>
      </div>
    </div>
    {view.staleError && <div className={`codex-proxy-unified-warning${view.staleError === 'CATALOG_NOT_FOUND' ? ' is-removed' : ''}`} role="alert">
      <TriangleAlert size={17} /><p>{t(view.staleError === 'CATALOG_NOT_FOUND' ? 'codex.proxy.unified.removedWarning' : 'codex.proxy.unified.staleWarning')}</p>
    </div>}
    {notice && <p className="codex-proxy-unified-notice" role="status">{notice}</p>}
    {editing && !dialog && createPortal(<div className="modal-overlay codex-proxy-unified-overlay">
      <div className="modal codex-proxy-unified-editor" ref={editorDialog} role="dialog" aria-modal="true" aria-labelledby="codex-proxy-unified-editor-title" tabIndex={-1}>
        <header className="modal-header"><div><h2 id="codex-proxy-unified-editor-title">{t('codex.proxy.unified.title')}</h2>
          <p>{t('codex.proxy.managerUnified.hint')}</p></div>
          <button type="button" className="btn btn-secondary compact" onClick={closeEditor} aria-label={t('common.close')}><X size={18} /></button></header>
        <div className="modal-body"><ModalErrorMessage message={error} />
          {emptyCatalog ? <div className="codex-proxy-account-guide"><Server size={24} /><p>{t('codex.proxy.unified.emptyCatalog')}</p>
            <button type="button" className="btn btn-primary" onClick={() => { closeEditor(); onGoResources(); }}>{t('codex.proxy.manager.proxies')}<ArrowRight size={15} /></button></div>
            : <div className="codex-proxy-account-choice">
              <label className="codex-proxy-field"><span>{t('codex.proxy.catalog.sources')}</span>
                <SingleSelectDropdown value={sourceId} disabled={measuring} ariaLabel={t('codex.proxy.catalog.sources')}
                  options={availableSources.map((entry) => ({ value: entry.id, label: entry.name }))} onChange={chooseSource} /></label>
              {source && <CodexProxyPicker key={source.id} source={source} itemId={itemId} selectedGroupId={groupId}
                selections={selectedChoices} busy={testing} latency={latency} choose={chooseItem} chooseMember={chooseMember} onCatalogChange={onCatalogChange} onPendingChange={setCatalogPending} />}
              <div className="codex-proxy-unified-test">
                <button type="button" className="btn btn-secondary compact" disabled={measuring || !selectionReady} onClick={() => void checkExit()}>
                  {testing ? <RefreshCw size={15} className="loading-spinner" /> : <Activity size={15} />}{t(testing ? 'codex.proxy.testing' : 'codex.proxy.test')}</button>
                {testing && <button type="button" className="btn btn-secondary compact" onClick={stopCheck}>{t('codex.proxy.cancelCheck')}</button>}
                {probe && <span className="codex-proxy-unified-probe" role="status"><ShieldCheck size={15} />{probe.protocol} · {probe.ip} · {probe.latencyMs} ms</span>}
              </div>
              {!selectionReady && <p className="codex-proxy-page-note">{t('codex.proxy.unified.pick')}</p>}
            </div>}
          <p className="codex-proxy-page-note">{t('codex.proxy.unified.restartHint')}</p>
        </div>
        <footer className="modal-footer"><button type="button" className="btn btn-secondary" onClick={closeEditor}>{t('common.cancel')}</button>
          <button type="button" className="btn btn-primary" disabled={!changeReady} onClick={() => openDialog(active ? 'change' : 'enable')}>
            <Check size={15} />{t('common.save')}</button></footer>
      </div>
    </div>, document.body)}
    {dialog && <CodexUnifiedProxyDialog kind={dialog} sourceId={sourceId} itemId={itemId} groupId={groupId}
      selections={selections} exitLabel={dialogExit} onClose={() => setDialog(null)} onApplied={applied} />}
  </section>;
}
