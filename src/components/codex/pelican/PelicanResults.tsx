import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { openUrl } from '@tauri-apps/plugin-opener';
import { ExternalLink, LoaderCircle } from 'lucide-react';
import { useCodexPelicanStore } from '../../../stores/useCodexPelicanStore';
import { useCodexAccountStore } from '../../../stores/useCodexAccountStore';
import { PelicanPlanBadge } from './PelicanAccountSummary';
import { artifactPelican, browserPelican, retryPelican } from '../../../services/codexPelicanService';
import { isPelicanRunning, type CodexPelicanArtifact, type CodexPelicanBatch, type CodexPelicanItem } from '../../../types/codexPelican';
import { ModalErrorMessage, useModalErrorState } from '../../ModalErrorMessage';
import { pelicanError } from './pelicanUtils';
import { pelicanPreviewDocument } from './pelicanPreviewDocument';

// Kept for existing account-group listeners; results no longer edit account groups.
export const PELICAN_GROUPS_CHANGED = 'codex-pelican-groups-changed';

interface PelicanTokenUsage {
  input: number;
  output: number;
  total: number;
}

function readPelicanTokenUsage(usage: unknown): PelicanTokenUsage | null {
  if (!usage || typeof usage !== 'object') return null;
  const record = usage as Record<string, unknown>;
  const read = (key: string) => {
    const value = record[key];
    return typeof value === 'number' && Number.isFinite(value) ? value : undefined;
  };
  const input = read('input_tokens') ?? read('inputTokens');
  const output = read('output_tokens') ?? read('outputTokens');
  const total = read('total_tokens') ?? read('totalTokens');
  if (input == null && output == null && total == null) return null;
  return {
    input: input ?? 0,
    output: output ?? 0,
    total: total ?? (input ?? 0) + (output ?? 0),
  };
}

function formatPelicanNumber(value: number): string {
  return new Intl.NumberFormat().format(Math.round(value));
}

function formatPelicanPercent(value: number): string {
  if (value > 0 && value < 0.1) return '<0.1';
  const rounded = Math.round(value * 10) / 10;
  return Number.isInteger(rounded) ? String(rounded) : rounded.toFixed(1);
}

function PelicanItem({ batch, item, mask }: { batch: CodexPelicanBatch; item: CodexPelicanItem; mask: (value: string) => string }) {
  const { t } = useTranslation();
  const [artifact, setArtifact] = useState<CodexPelicanArtifact | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loadAttempt, setLoadAttempt] = useState(0);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [source, setSource] = useState<'preview' | 'rawReply' | 'error'>('preview');
  const [busy, setBusy] = useState(false);
  const cellRef = useRef<HTMLDivElement>(null);
  const previewRef = useRef<HTMLButtonElement>(null);
  const error = useModalErrorState();
  const terminal = item.status !== 'running' && item.status !== 'queued';
  const retryable = ['failed', 'cancelled', 'interrupted'].includes(item.status);
  const elapsed = item.startedAt == null ? 0 : Math.max(0, (item.finishedAt ?? Date.now()) - item.startedAt) / 1000;
  const tokenUsage = readPelicanTokenUsage(item.usage);
  const tokenLabel = tokenUsage ? t('pelican.usageTokens', { tokens: formatPelicanNumber(tokenUsage.total) }) : null;
  const quotaLabel = item.quota?.consumedPercent != null
    ? t('pelican.quotaUsed', { percent: formatPelicanPercent(item.quota.consumedPercent) })
    : item.quota?.remainingPercent != null
      ? t('pelican.quotaLeft', { percent: formatPelicanPercent(item.quota.remainingPercent) })
      : null;

  useEffect(() => {
    setArtifact(null); setLoadError(null);
    if (!terminal) return;
    let disposed = false;
    const observer = new IntersectionObserver((entries) => {
      if (!entries.some((entry) => entry.isIntersecting)) return;
      observer.disconnect();
      void artifactPelican(batch.id, item.id)
        .then((value) => { if (!disposed) setArtifact(value); })
        .catch((cause) => { if (!disposed) setLoadError(pelicanError(cause, t)); });
    }, { rootMargin: '100px' });
    if (cellRef.current) observer.observe(cellRef.current);
    return () => { disposed = true; observer.disconnect(); };
  }, [batch.id, item.id, item.startedAt, terminal, loadAttempt, t]);

  const run = async (operation: () => Promise<void>) => {
    if (busy) return;
    setBusy(true); error.clear();
    try { await operation(); } catch (cause) { error.set(pelicanError(cause, t)); }
    finally { setBusy(false); }
  };
  const retry = () => void run(async () => {
    const next = await retryPelican(batch.id, item.id);
    useCodexPelicanStore.getState().receive(next);
    setArtifact(null); setLoadError(null);
  });
  const closeDetails = () => {
    setDetailsOpen(false); error.clear(); previewRef.current?.focus();
  };
  const html = terminal ? artifact?.html : null;
  const document = html ? pelicanPreviewDocument(html) : undefined;
  return <div className="pelican-canvas-cell" ref={cellRef}>
    {html ? <button ref={previewRef} type="button" className="pelican-canvas" aria-label={`${mask(item.accountEmail || item.accountId)} · ${t('pelican.preview')}`}
      onClick={() => { error.clear(); setSource('preview'); setDetailsOpen(true); }}>
      <iframe title={mask(item.accountEmail || item.accountId)} srcDoc={document} sandbox="allow-scripts" tabIndex={-1} loading="lazy" />
    </button> : <div className={`pelican-canvas-placeholder${retryable ? ' is-failed' : ''}`}>
      {!terminal && <><LoaderCircle className={item.status === 'running' ? 'pelican-spinner' : undefined} size={24} /><strong>{t(`pelican.${item.status}`)}</strong><span className="pelican-elapsed">{elapsed.toFixed(0)}s</span></>}
      {terminal && <>
        <strong>{t(retryable ? `pelican.${item.status}` : item.hasHtml ? 'common.loading' : 'pelican.noHtml')}</strong>
        {loadError && <><p className="pelican-cell-error">{loadError}</p><button className="btn btn-secondary" disabled={busy} onClick={() => { error.clear(); setLoadAttempt((value) => value + 1); }}>{t('common.refresh')}</button></>}
        {item.startedAt != null && <span className="pelican-elapsed">{elapsed.toFixed(1)}s</span>}
        {terminal && <div className="pelican-cell-actions">
          {item.error && <button className="btn btn-secondary" disabled={busy} onClick={() => { error.clear(); setSource('error'); setDetailsOpen(true); }}>{t('pelican.viewError')}</button>}
          {retryable && <button className="btn btn-secondary" disabled={busy || batch.status === 'cancelling'} onClick={retry}>{t(busy ? 'common.loading' : 'common.windowsOperation.retry')}</button>}
          <button ref={previewRef} className="btn btn-secondary" disabled={busy} onClick={() => void run(async () => { if (!artifact) await artifactPelican(batch.id, item.id).then(setArtifact); setSource('rawReply'); setDetailsOpen(true); })}>{t('pelican.rawReply')}</button>
        </div>}
      </>}
    </div>}
    {html && <div className="pelican-cell-caption"><span className={`pelican-status pelican-status-${item.status}`}>{t(`pelican.${item.status}`)}</span><span>{elapsed.toFixed(1)}s</span></div>}
    {(tokenLabel || quotaLabel) && <div className="pelican-cell-usage">
      {tokenLabel && <span>{tokenLabel}</span>}
      {quotaLabel && <span>{quotaLabel}</span>}
    </div>}
    {!detailsOpen && <ModalErrorMessage message={error.message} scrollKey={error.scrollKey} />}
    {detailsOpen && createPortal(<div className="pelican-detail-overlay">
      <section className="pelican-dialog pelican-detail-dialog" role="dialog" aria-modal="true" aria-labelledby={`pelican-detail-${item.id}`} onKeyDown={(event) => {
        event.stopPropagation();
        if (event.key === 'Escape') closeDetails();
        if (event.key === 'Tab') {
          const controls = event.currentTarget.querySelectorAll<HTMLElement>('button:not(:disabled), [tabindex="0"]');
          const first = controls[0]; const last = controls[controls.length - 1];
          if (first && event.shiftKey && window.document.activeElement === first) { event.preventDefault(); last.focus(); }
          if (last && !event.shiftKey && window.document.activeElement === last) { event.preventDefault(); first.focus(); }
        }
      }}>
        <header className="pelican-detail-header"><h3 id={`pelican-detail-${item.id}`}>{mask(item.accountEmail || item.accountId)}</h3><button className="btn btn-secondary" autoFocus onClick={closeDetails}>{t('common.close')}</button></header>
        <div className="pelican-detail-body">
          <div className="pelican-actions pelican-detail-actions">
            {source === 'error' && <span className="pelican-detail-error-label">{t('pelican.viewError')}</span>}
            {source !== 'error' && <button className="btn btn-secondary" disabled={busy || !artifact?.rawReply} onClick={() => { error.clear(); setSource('rawReply'); }}>{t('pelican.rawReply')}</button>}
            <button className="btn btn-secondary" disabled={busy || !html} onClick={() => void run(async () => { await openUrl(await browserPelican(batch.id, item.id)); })}><ExternalLink size={14} />{t('pelican.openBrowser')}</button>
          </div>
          <ModalErrorMessage message={error.message} scrollKey={error.scrollKey} />
          {(tokenUsage || item.quota) && <div className="pelican-detail-usage">
            {tokenUsage && <span>{t('pelican.usageTokens', { tokens: formatPelicanNumber(tokenUsage.total) })}</span>}
            {item.quota?.remainingPercent != null && <span>{t('pelican.quotaLeft', { percent: formatPelicanPercent(item.quota.remainingPercent) })}</span>}
            {item.quota?.consumedPercent != null && <span>{t('pelican.quotaUsed', { percent: formatPelicanPercent(item.quota.consumedPercent) })}</span>}
            {item.quota?.resetAt != null && <span>{t('common.shared.quota.resetAt', { time: new Date(item.quota.resetAt * 1000).toLocaleString(), defaultValue: 'Reset: {{time}}' })}</span>}
          </div>}
          {source === 'preview' && document ? <iframe className="pelican-detail-canvas" title={t('pelican.preview')} srcDoc={document} sandbox="allow-scripts" />
            : <pre className="pelican-detail-output" tabIndex={0}>{source === 'error' ? pelicanError(item.error ?? 'pelican.error.workerFailed', t) : artifact?.rawReply ?? item.replyPreview ?? t('pelican.noHtml')}</pre>}
        </div>
      </section>
    </div>, window.document.body)}
  </div>;
}

export function PelicanResults({ batch, mask }: { batch: CodexPelicanBatch; mask: (value: string) => string }) {
  const { t } = useTranslation();
  const accounts = useCodexAccountStore((state) => state.accounts);
  const [, tick] = useState(0);
  useEffect(() => {
    if (!isPelicanRunning(batch)) return;
    const timer = window.setInterval(() => tick((value) => value + 1), 1000);
    return () => clearInterval(timer);
  }, [batch.status]);
  return <>
    <ModalErrorMessage message={batch.error ? pelicanError(batch.error, t) : undefined} />
    <div className="pelican-table-scroll" tabIndex={0} role="region" aria-label={t('pelican.results')}>
      <div className="pelican-canvas-table" aria-label={t('pelican.results')}>
        {batch.items.map((item) => <article className="pelican-account-tile" key={item.id}>
          <h3><span className="pelican-account-email">{mask(item.accountEmail || item.accountId)}</span>{accounts.find((entry) => entry.id === item.accountId) && <span className="pelican-account-plan"><PelicanPlanBadge account={accounts.find((entry) => entry.id === item.accountId)!} /></span>}</h3>
          <PelicanItem batch={batch} item={item} mask={mask} />
        </article>)}
      </div>
    </div>
  </>;
}
