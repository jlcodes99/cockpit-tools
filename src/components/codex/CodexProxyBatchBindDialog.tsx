import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { preflightCodexProxyEngine } from '../../services/codexProxyEngineService';
import { Activity, Check, RefreshCw, ShieldCheck, TriangleAlert, X } from 'lucide-react';
import type { CodexAccount } from '../../types/codex';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import { ModalErrorMessage } from '../ModalErrorMessage';
import { CodexProxyPicker } from './CodexProxyPicker';
import { useProxyLatency } from './useProxyLatency';
import { proxySourceInspectable } from '../../utils/codexProxyPickerModel';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import {
  bindProxyCatalog, cancelProxyCatalog, catalogErrorKey, probeProxyCatalog,
  type ProxyCatalog, type ProxyCatalogSelections,
} from '../../services/codexProxyCatalogService';
import type { CodexProxyProbeResult } from '../../services/codexAccountProxyService';
import { defaultProxySelections, sourceDefaultDraft } from '../../utils/codexProxySelection';
import { batchBindSkipped, batchBindSummary, batchBindTargets, executeProxyBatch, proxyBatchCompletedSuccessfully, type BatchBindResult } from '../../utils/codexProxyBatch';
import '../../styles/pages/codex-proxy-preview.css';
import '../../styles/pages/codex-proxy-batch.css';

interface Props {
  /** The accounts picked on the page; binding only ever touches this list. */
  accounts: CodexAccount[];
  catalog: ProxyCatalog;
  onCatalogChange?: (catalog: ProxyCatalog) => void;
  initialSourceId?: string;
  initialItemId?: string;
  initialGroupId?: string;
  initialSelections?: ProxyCatalogSelections;
  resolveDisplayName: (account: CodexAccount) => string;
  savedValue: (account: CodexAccount) => CodexAccount['egress_proxy'];
  onClose: () => void;
  /** Feeds the page's optimistic overrides; the account store stays the source of truth. */
  onApplied: (account: CodexAccount) => void;
}

/**
 * Bind one node or group to several accounts, one account at a time, with per-account results.
 * Nothing is written until the user confirms, and each account keeps an independent binding.
 */
export function CodexProxyBatchBindDialog({
  accounts, catalog, initialSourceId, initialItemId, initialGroupId, initialSelections,
  resolveDisplayName, savedValue, onClose, onApplied, onCatalogChange,
}: Props) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDivElement>(null);
  const requestId = useRef<string | null>(null);
  const cancelled = useRef(false);
  const mounted = useRef(false);
  const running = useRef(false);
  const availableSources = useMemo(() => catalog.sources.filter(proxySourceInspectable), [catalog]);
  /** Only a source default may prefill the dialog; an explicit seed from the resources page wins. */
  const initialSource = availableSources.find((entry) => entry.id === initialSourceId) ?? availableSources[0];
  const initialDefault = useMemo(() => sourceDefaultDraft(initialSource), [initialSource]);
  const [sourceId, setSourceId] = useState(initialSourceId ?? initialSource?.id ?? '');
  const [itemId, setItemId] = useState(initialItemId ?? initialDefault?.itemId ?? '');
  const [groupId, setGroupId] = useState(initialGroupId ?? initialDefault?.groupId ?? '');
  const [selectedChoices, setSelectedChoices] = useState<ProxyCatalogSelections>(initialSelections ?? initialDefault?.selections ?? {});
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<{ current: number; total: number } | null>(null);
  const [testing, setTesting] = useState(false);
  const [catalogPending, setCatalogPending] = useState(false);
  const [probe, setProbe] = useState<CodexProxyProbeResult | null>(null);
  const [error, setError] = useState('');
  const [results, setResults] = useState<BatchBindResult[]>([]);
  const source = availableSources.find((entry) => entry.id === sourceId) ?? availableSources[0];
  const latency = useProxyLatency(source);
  const selected = source?.nodes.find((entry) => entry.id === itemId) ?? source?.groups.find((entry) => entry.id === itemId);
  const selections = useMemo(() => source ? defaultProxySelections(source, itemId, selectedChoices) : null, [source, itemId, selectedChoices]);
  const ready = !!selected?.supported && selections !== null;
  const targets = useMemo(() => source && ready ? batchBindTargets(accounts, savedValue, source.id, itemId, groupId, selections ?? undefined) : [],
    [accounts, savedValue, source, itemId, groupId, ready, selections]);
  const skipped = useMemo(() => source ? batchBindSkipped(accounts, savedValue, source.id, itemId, groupId, selections ?? undefined) : { same: 0 },
    [accounts, savedValue, source, itemId, groupId, selections]);
  const overwrite = targets.filter((entry) => entry.willOverwrite).length;
  const summary = batchBindSummary(results);

  useEscCloseTopmost(true, () => { if (!busy) onClose(); });
  useModalScrollLock(true);
  useModalFocusTrap(dialog, true);

  // StrictMode replays setup/cleanup. Unmount and user cancellation are different states.
  useEffect(() => {
    mounted.current = true;
    cancelled.current = false;
    return () => {
      mounted.current = false;
      cancelled.current = true;
      if (requestId.current) void cancelProxyCatalog(requestId.current).catch(() => {});
    };
  }, []);
  useEffect(() => { if (source && source.id !== sourceId) setSourceId(source.id); }, [source, sourceId]);

  const chooseSource = (next: string) => {
    // Switching sources brings that source's default into the draft; nothing is written yet.
    const prefill = sourceDefaultDraft(availableSources.find((entry) => entry.id === next));
    setSourceId(next); setItemId(prefill?.itemId ?? ''); setGroupId(prefill?.groupId ?? '');
    setSelectedChoices(prefill?.selections ?? {}); setProbe(null); setError(''); setResults([]);
  };
  const chooseItem = (next: string, nextGroup: string) => {
    setItemId(next); setGroupId(nextGroup); setSelectedChoices({}); setProbe(null); setError(''); setResults([]);
  };
  const chooseMember = (selectorId: string, member: string) => {
    setSelectedChoices((old) => ({ ...old, [selectorId]: member })); setProbe(null); setError(''); setResults([]);
  };
  const checkNode = async () => {
    if (!source || running.current || requestId.current || !ready) return;
    const id = crypto.randomUUID();
    requestId.current = id;
    setTesting(true); setError(''); setProbe(null);
    try {
      const checked = await probeProxyCatalog(id, source.id, itemId, selections ?? {});
      if (mounted.current) setProbe(checked);
    } catch (caught) {
      if (mounted.current) setError(t(catalogErrorKey(caught)));
    } finally {
      requestId.current = null;
      if (mounted.current) setTesting(false);
    }
  };
  const run = async () => {
    if (!source || running.current || requestId.current || !ready || targets.length === 0) return;
    running.current = true;
    cancelled.current = false;
    setBusy(true); setError(''); setResults([]);
    setProgress({ current: 0, total: targets.length });
    try {
      await preflightCodexProxyEngine();
      if (!mounted.current || cancelled.current) return;
      let completed: BatchBindResult[] = [];
      await executeProxyBatch(targets, {
        cancelled: () => cancelled.current,
        bind: (entry) => bindProxyCatalog(entry.id, source.id, itemId, selections ?? {}, groupId),
        // Even after cancellation/unmount, persisted writes must reach the parent account store.
        applied: onApplied,
        errorKey: catalogErrorKey,
        progress: (next) => {
          completed = next;
          if (!mounted.current) return;
          setResults(next);
          setProgress({ current: next.length, total: targets.length });
          const nextSummary = batchBindSummary(next);
          if (nextSummary.failed) setError(t('codex.proxy.batchResult', nextSummary));
        },
      });
      if (mounted.current && proxyBatchCompletedSuccessfully(completed, targets.length, cancelled.current)) onClose();
    } catch (caught) {
      if (mounted.current) setError(t(catalogErrorKey(caught)));
    } finally {
      running.current = false;
      if (mounted.current) { setBusy(false); setProgress(null); }
    }
  };

  return createPortal(<div className="modal-overlay codex-proxy-batch-overlay">
    <div className="modal codex-proxy-batch" ref={dialog} role="dialog" aria-modal="true" aria-labelledby="codex-proxy-batch-title" tabIndex={-1}>
      <header className="modal-header">
        <div className="codex-proxy-preview-heading"><span className="codex-proxy-preview-icon"><ShieldCheck size={25} /></span>
          <div><h2 id="codex-proxy-batch-title">{t('codex.proxy.batchTitle')}</h2>
            <p>{t('codex.proxy.batchSubtitle', { count: accounts.length })}</p></div></div>
        <button type="button" className="btn btn-secondary codex-proxy-preview-close" disabled={busy} onClick={onClose} aria-label={t('common.close')}><X size={19} /></button>
      </header>
      <div className="modal-body">
        <ModalErrorMessage message={error} />
        <div className="codex-proxy-batch-choice">
          <label className="codex-proxy-field"><span>{t('codex.proxy.catalog.sources')}</span>
            <SingleSelectDropdown value={source?.id ?? ''} disabled={busy || testing || catalogPending} ariaLabel={t('codex.proxy.catalog.sources')}
              options={availableSources.map((entry) => ({ value: entry.id, label: entry.name }))} onChange={chooseSource} /></label>
          {source && <CodexProxyPicker key={source.id} source={source} itemId={itemId} selectedGroupId={groupId} selections={selectedChoices}
            busy={busy || testing} latency={latency} choose={chooseItem} chooseMember={chooseMember} onCatalogChange={onCatalogChange} onPendingChange={setCatalogPending} />}
          <div className="codex-proxy-batch-test">
            <button type="button" className="btn btn-secondary" disabled={busy || testing || catalogPending || !ready} onClick={() => void checkNode()}>
              {testing ? <RefreshCw size={15} className="loading-spinner" /> : <Activity size={15} />}
              {t(testing ? 'codex.proxy.testing' : 'codex.proxy.test')}</button>
            {probe && <span className="codex-proxy-batch-probe"><ShieldCheck size={15} />{probe.protocol} · {probe.ip} · {probe.latencyMs} ms</span>}
          </div>
        </div>
        <p className="codex-proxy-batch-summary">
          {ready && selected ? t('codex.proxy.batchSummary', { node: selected.name, count: targets.length }) : t('codex.proxy.batchPickNode')}
        </p>
        {ready && <p className="codex-proxy-batch-notes">
          {overwrite > 0 && <span className="is-warning"><TriangleAlert size={13} />{t('codex.proxy.batchOverwrite', { count: overwrite })}</span>}
          {skipped.same > 0 && <span>{t('codex.proxy.batchAlreadySame', { count: skipped.same })}</span>}
          {targets.length === 0 && <span>{t('codex.proxy.batchEmpty')}</span>}
        </p>}
        {results.length > 0 && <section className="codex-proxy-batch-results" aria-live="polite">
          <strong>{t('codex.proxy.batchResult', { success: summary.success, failed: summary.failed })}</strong>
          {summary.failed > 0 && <ul>{results.filter((entry) => !entry.ok).map((entry) => {
            const target = accounts.find((item) => item.id === entry.accountId);
            return <li key={entry.accountId}><span>{target ? resolveDisplayName(target) : entry.accountId}</span>
              <span className="is-failure">{t(entry.errorKey ?? 'codex.proxy.catalog.failed')}</span></li>;
          })}</ul>}
        </section>}
        <p className="codex-proxy-preview-notice">{t('codex.proxy.restartHint')}</p>
      </div>
      <footer className="modal-footer">
        <span className="codex-proxy-batch-progress" role="status">
          {progress ? t('codex.proxy.batchProgress', { current: progress.current, total: progress.total }) : ''}
        </span>
        <div>
          {busy
            ? <button type="button" className="btn btn-secondary" onClick={() => { cancelled.current = true; }}>{t('common.cancel')}</button>
            : <button type="button" className="btn btn-secondary" onClick={onClose}>{t('common.close')}</button>}
          <button type="button" className="btn btn-primary" disabled={busy || testing || catalogPending || !ready || targets.length === 0} onClick={() => void run()}>
            {busy ? <RefreshCw size={15} className="loading-spinner" /> : <Check size={15} />}
            {t(busy ? 'codex.proxy.batchRunning' : 'codex.proxy.batchApply')}</button>
        </div>
      </footer>
    </div>
  </div>, document.body);
}
