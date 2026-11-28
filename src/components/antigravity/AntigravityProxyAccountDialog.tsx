import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { Activity, Check, RefreshCw, Server, ShieldCheck, X } from 'lucide-react';
import type { Account } from '../../types/account';
import { ModalErrorMessage } from '../ModalErrorMessage';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import { antigravityProxyApi, type AntigravityProxyView } from '../../services/antigravityProxyService';
import { useAntigravityProxyStore } from '../../stores/useAntigravityProxyStore';
import { cancelProxyCatalog, catalogErrorKey, getProxyCatalog, probeProxyCatalog, type ProxyCatalog } from '../../services/codexProxyCatalogService';
import { restoreExitChoice, sameExitChoice, type CodexProxyExitChoice } from '../../utils/codexProxyDraft';
import { defaultProxySelections, sourceDefaultDraft } from '../../utils/codexProxySelection';
import { proxySourceInspectable } from '../../utils/codexProxyPickerModel';
import { proxySummary } from '../../utils/codexProxyPresentation';
import { CodexProxyPicker } from '../codex/CodexProxyPicker';
import { CodexProxyResources } from '../codex/CodexProxyResources';
import { useProxyLatency } from '../codex/useProxyLatency';
import { useCodexProxyAccountName } from '../codex/useCodexProxyExitEditor';
import type { CodexProxyProbeResult } from '../../services/codexAccountProxyService';
import '../../styles/pages/codex-proxy-accounts.css';

/** Uses the real shared picker, but never the Codex account store or binding commands. */
export function AntigravityProxyAccountDialog({ account, onClose }: { account: Account; onClose(): void }) {
  const { t } = useTranslation();
  const translate = useRef(t);
  translate.current = t;
  const dialog = useRef<HTMLDivElement>(null);
  const accept = useAntigravityProxyStore((state) => state.accept);
  const [view, setView] = useState<AntigravityProxyView | null>(null);
  const [catalog, setCatalog] = useState<ProxyCatalog>({ sources: [] });
  const [loading, setLoading] = useState(true);
  const [revision, setRevision] = useState(0);
  const [mode, setMode] = useState<'follow' | 'independent'>('follow');
  const [draft, setDraft] = useState<CodexProxyExitChoice | null>(null);
  const [busy, setBusy] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState('');
  const [resources, setResources] = useState(false);
  const [result, setResult] = useState<CodexProxyProbeResult | null>(null);
  const live = useRef(false);
  const operation = useRef(false);
  const requestId = useRef<string | null>(null);
  const displayName = useCodexProxyAccountName();
  const restored = useMemo(() => restoreExitChoice(catalog, view?.binding), [catalog, view?.binding]);
  const choice = draft ?? restored;
  const source = catalog.sources.find((entry) => entry.id === choice.sourceId);
  const sources = catalog.sources.filter(proxySourceInspectable);
  const selections = source ? defaultProxySelections(source, choice.itemId, choice.selections) : null;
  const latency = useProxyLatency(source);
  const bound = Boolean(view?.binding || view?.legacyBound || view?.error);
  const unchanged = Boolean(view?.binding) && !view?.error && sameExitChoice(restored, choice);
  const locked = loading || busy || pending;
  const canSave = Boolean(view) && !locked && !latency.running && (mode === 'follow' ? bound : selections !== null && (!unchanged || draft !== null));

  useEscCloseTopmost(true, () => { if (!operation.current && !pending) onClose(); }, dialog);
  useModalScrollLock(true);
  useModalFocusTrap(dialog, true);
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
      if (requestId.current) void cancelProxyCatalog(requestId.current).catch(() => {});
    };
  }, []);
  useEffect(() => {
    let current = true;
    setLoading(true); setError('');
    void Promise.all([antigravityProxyApi.get(account.id), getProxyCatalog()]).then(([saved, next]) => {
      if (!current) return;
      setView(saved); accept(saved); setCatalog(next);
      setMode(saved.binding || saved.legacyBound || saved.error ? 'independent' : 'follow');
      setDraft(null);
    }).catch((caught) => { if (current) setError(translate.current(catalogErrorKey(caught))); })
      .finally(() => { if (current) setLoading(false); });
    return () => { current = false; };
  }, [account.id, revision, accept]);

  const select = (next: CodexProxyExitChoice) => {
    if (operation.current) return;
    setDraft(next); setResult(null); setError('');
  };
  const submit = async () => {
    if (!canSave || operation.current) return;
    operation.current = true; setBusy(true); setError(''); latency.cancel();
    try {
      const updated = mode === 'follow' ? await antigravityProxyApi.clear(account.id)
        : await antigravityProxyApi.bind(account.id, choice.sourceId, choice.itemId, selections!, choice.groupId);
      accept(updated);
      if (live.current) onClose();
    } catch (caught) { if (live.current) setError(t(catalogErrorKey(caught))); }
    finally { operation.current = false; if (live.current) setBusy(false); }
  };
  const probe = async () => {
    if (operation.current || !selections || !source) return;
    operation.current = true; setBusy(true); setError(''); setResult(null); latency.cancel();
    const id = crypto.randomUUID(); requestId.current = id;
    try {
      const checked = await probeProxyCatalog(id, source.id, choice.itemId, selections);
      if (live.current) setResult(checked);
    } catch (caught) { if (live.current) setError(t(catalogErrorKey(caught))); }
    finally { requestId.current = null; operation.current = false; if (live.current) setBusy(false); }
  };
  const title = view?.binding ? proxySummary(view.binding) : view?.legacyBound ? t('codex.proxy.legacyBoundHint') : t('codex.proxy.modeDefault');

  return createPortal(<div className="modal-overlay codex-proxy-account-dialog-overlay">
    <div className="modal codex-proxy-account-dialog" ref={dialog} role="dialog" aria-modal="true" aria-labelledby="antigravity-proxy-title" tabIndex={-1}>
      <header className="modal-header"><div><h2 id="antigravity-proxy-title">{displayName({ id: account.id, email: account.email, account_name: account.name })}</h2><p>{title}</p></div>
        <button type="button" className="btn btn-secondary compact" disabled={busy || pending} onClick={onClose} aria-label={t('common.close')}><X size={18} /></button></header>
      <div className="modal-body">
        <ModalErrorMessage message={error || (view?.error ? t('codex.proxy.unified.errorStale') : '')} />
        {loading ? <p role="status">{t('common.loading')}</p> : !view ? <button type="button" className="btn btn-secondary" onClick={() => setRevision((value) => value + 1)}>{t('common.retry')}</button>
          : resources ? <><button type="button" className="btn btn-secondary compact" onClick={() => { setResources(false); setRevision((value) => value + 1); }}>{t('common.back')}</button>
            <CodexProxyResources onBindingsChanged={async () => { await useAntigravityProxyStore.getState().reload(); }}
              onAssign={(sourceId, itemId, selected, groupId, next) => { setCatalog(next); setMode('independent'); setDraft({ sourceId, itemId, selections: selected, groupId }); setResources(false); }} /></>
            : <>
              <div className="codex-proxy-account-mode-options" role="radiogroup" aria-label={t('codex.proxy.managerAccounts.mode')}>
                {(['follow', 'independent'] as const).map((value) => <button type="button" key={value} role="radio" aria-checked={mode === value}
                  className={`btn codex-proxy-account-mode-option${mode === value ? ' active' : ''}`} disabled={locked} onClick={() => { setMode(value); setResult(null); setError(''); latency.cancel(); }}>
                  <span className="codex-proxy-account-mode-dot">{mode === value && <Check size={12} />}</span><span>
                    <strong>{t(value === 'follow' ? 'codex.proxy.managerAccounts.following' : 'codex.proxy.modeIndependent')}</strong>
                    <small>{t(value === 'follow' ? 'antigravity.proxy.followHint' : 'antigravity.proxy.independentHint')}</small></span></button>)}
              </div>
              {mode === 'follow' ? <div className="codex-proxy-account-follow-preview"><ShieldCheck size={21} /><div><span>{t('settings.network.proxyTitle')}</span><strong>{t('antigravity.proxy.followHint')}</strong></div></div>
                : sources.length === 0 ? <div className="codex-proxy-account-guide"><Server size={24} /><p>{t('codex.proxy.unified.emptyCatalog')}</p>
                  <button type="button" className="btn btn-primary" onClick={() => setResources(true)}>{t('codex.proxy.manager.proxies')}</button></div>
                  : <div className="codex-proxy-account-choice"><label className="codex-proxy-field"><span>{t('codex.proxy.catalog.sources')}</span>
                    <SingleSelectDropdown value={choice.sourceId} disabled={locked} ariaLabel={t('codex.proxy.catalog.sources')}
                      options={sources.map((entry) => ({ value: entry.id, label: entry.name }))} onChange={(sourceId) => {
                        const preset = sourceDefaultDraft(catalog.sources.find((entry) => entry.id === sourceId));
                        select({ sourceId, itemId: preset?.itemId ?? '', groupId: preset?.groupId ?? '', selections: preset?.selections ?? {} });
                      }} /></label>
                    {source && <CodexProxyPicker source={source} itemId={choice.itemId} selectedGroupId={choice.groupId} selections={choice.selections} busy={busy}
                      latency={latency} onCatalogChange={setCatalog} onPendingChange={setPending}
                      choose={(itemId, groupId) => select({ sourceId: choice.sourceId, itemId, groupId, selections: {} })}
                      chooseMember={(id, member) => select({ ...choice, selections: { ...choice.selections, [id]: member } })} />}
                    <div className="codex-proxy-account-check-actions"><button type="button" className="btn btn-secondary compact" disabled={locked || selections === null || latency.running} onClick={() => void probe()}>
                      <Activity size={15} />{t('codex.proxy.test')}</button>
                      {requestId.current && <button type="button" className="btn btn-secondary compact" onClick={() => { if (requestId.current) void cancelProxyCatalog(requestId.current).catch(() => {}); }}>{t('codex.proxy.cancelCheck')}</button>}
                      <button type="button" className="btn btn-secondary compact" disabled={locked || latency.running} onClick={() => setResources(true)}>{t('codex.proxy.manager.proxies')}</button></div>
                  </div>}
              {result && <output className="codex-proxy-check-result" aria-live="polite"><ShieldCheck size={18} /><div><strong>{t('codex.proxy.testPassed')}</strong><span>{result.protocol} · {result.ip} · {result.latencyMs} ms</span><span>{t('codex.proxy.draftTested')}</span></div></output>}
              <p className="codex-proxy-page-note">{t('antigravity.proxy.restartHint')}</p>
              <p className="codex-proxy-page-note">{t('codex.proxy.unified.restartHint')}</p>
              <p className="codex-proxy-page-note">{t('antigravity.proxy.hostHint')}</p>
            </>}
      </div>
      <footer className="modal-footer"><button type="button" className="btn btn-secondary" disabled={busy || pending} onClick={onClose}>{t('common.close')}</button>
        {!resources && <button type="button" className="btn btn-primary" disabled={!canSave} onClick={() => void submit()}>{busy ? <RefreshCw size={15} className="loading-spinner" /> : <Check size={15} />}{t('common.save')}</button>}</footer>
    </div>
  </div>, document.body);
}
