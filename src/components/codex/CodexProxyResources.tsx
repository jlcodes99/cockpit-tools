import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { Activity, ArrowRight, ChevronDown, Download, Layers, Link2, MoreHorizontal, Plus, RefreshCw, Search, ShieldCheck, Star, Trash2, TriangleAlert, Pencil, X } from 'lucide-react';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { CodexProxyPicker } from './CodexProxyPicker';
import { useProxyLatency } from './useProxyLatency';
import { useCodexProxyAccountName } from './useCodexProxyExitEditor';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import { defaultProxySelections, sourceDefaultDraft } from '../../utils/codexProxySelection';
import { CodexProxyResourceForm, type ProxySourceDraft } from './CodexProxyResourceForm';
import { StrategyEditorDialog, StrategyDeleteDialog } from './CodexProxyStrategyPanel';
import { CodexProxyResourceNodes } from './CodexProxyResourceNodes';
import { CodexProxyLatencyBadge } from './CodexProxyLatencyBadge';
import { CodexProxySetupGuide } from './CodexProxySetupGuide';
import { preflightCodexProxyEngine } from '../../services/codexProxyEngineService';
import { proxyRemovalErrorKey, removeProxySourceWithRefresh, type ProxyRemovalProgress } from '../../utils/codexProxyRemoval';
import { strategyCandidates, strategyKindKey, strategyKindOf } from '../../services/codexProxyStrategyService';
import { formatProxyBytes, formatProxyDateTime } from '../../utils/codexProxyFormat';
import type { CodexProxyProbeResult } from '../../services/codexAccountProxyService';
import { cancelProxyCatalog, catalogErrorKey, clearProxyCatalogDefault, getProxyCatalog, importProxyCatalog, probeProxyCatalog,
  setProxyCatalogDefault, setProxyNodeInsecure, refreshProxyCatalog, removeProxyCatalog, setProxyCatalogAutoUpdate, renameProxySource, getProxyCatalogDependencies,
  type ProxyCatalog, type ProxyCatalogSource, type ProxyCatalogDependencies, type ProxyCatalogSelections } from '../../services/codexProxyCatalogService';
import '../../styles/pages/codex-proxy-resources.css';

export function CodexProxyResources({ onAssign, onBindingsChanged }: {
  onAssign: (sourceId: string, itemId: string, selections: ProxyCatalogSelections, groupId: string, catalog: ProxyCatalog) => void;
  onBindingsChanged: (sourceId: string, sourceRemoved: boolean) => Promise<void>;
}) {
  const { t } = useTranslation();
  const resolveName = useCodexProxyAccountName();
  const [catalog, setCatalog] = useState<ProxyCatalog>({ sources: [] });
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState('');
  const [catalogPending, setCatalogPending] = useState(false);
  const [error, setError] = useState('');
  const [formError, setFormError] = useState('');
  const [adding, setAdding] = useState(false);
  const [sourceId, setSourceId] = useState('');
  const [expanded, setExpanded] = useState(false);
  const [query, setQuery] = useState('');
  const [menuId, setMenuId] = useState('');
  const [strategyEditing, setStrategyEditing] = useState('');
  const [strategyDeleting, setStrategyDeleting] = useState('');
  const [advanced, setAdvanced] = useState(false);
  const [itemId, setItemId] = useState('');
  const [groupId, setGroupId] = useState('');
  const [selectedChoices, setSelectedChoices] = useState<ProxyCatalogSelections>({});
  const [renaming, setRenaming] = useState(false);
  const [renameValue, setRenameValue] = useState('');
  const [renameError, setRenameError] = useState('');
  const [deleting, setDeleting] = useState<{ id: string; name: string } | null>(null);
  const [deleteError, setDeleteError] = useState('');
  const [deleteImpact, setDeleteImpact] = useState<ProxyCatalogDependencies | null>(null);
  const [deleteImpactError, setDeleteImpactError] = useState('');
  const [deleteImpactLoading, setDeleteImpactLoading] = useState(false);
  const [deleteImpactRetry, setDeleteImpactRetry] = useState(0);
  const [result, setResult] = useState<CodexProxyProbeResult | null>(null);
  const [defaultPending, setDefaultPending] = useState<'' | 'set' | 'clear'>('');
  const [defaultError, setDefaultError] = useState('');
  const [defaultRetry, setDefaultRetry] = useState<'' | 'set' | 'clear'>('');
  const mounted = useRef(true);
  const operation = useRef(false);
  const request = useRef<string | null>(null);
  const section = useRef<HTMLElement>(null);
  const deleteButton = useRef<HTMLButtonElement>(null);
  const deleteDialog = useRef<HTMLDivElement>(null);
  const cancelDeleteButton = useRef<HTMLButtonElement>(null);
  const deleteErrorElement = useRef<HTMLDivElement>(null);
  const defaultErrorElement = useRef<HTMLDivElement>(null);
  const catalogRevision = useRef(0);
  const removalProgress = useRef<ProxyRemovalProgress>({});
  const source = catalog.sources.find((entry) => entry.id === sourceId);
  const deletingRevision = catalog.sources.find((entry) => entry.id === deleting?.id)?.revision;
  const latency = useProxyLatency(source);
  const locked = !!busy || !!defaultPending || catalogPending;
  const hasCandidates = useMemo(() => strategyCandidates(catalog.sources).length > 0, [catalog.sources]);
  const visibleSources = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    return catalog.sources.filter((entry) => !needle || [entry.name, ...entry.nodes.map((node) => node.name)].some((value) => value.toLocaleLowerCase().includes(needle)));
  }, [catalog.sources, query]);
  const selected = source?.nodes.find((entry) => entry.id === itemId) ?? source?.groups.find((entry) => entry.id === itemId);
  const selections = useMemo(() => source ? defaultProxySelections(source, itemId, selectedChoices) : null, [source, itemId, selectedChoices]);
  const selectionReady = !!selected?.supported && selections !== null;
  const defaultItem = source?.default ? [...source.nodes, ...source.groups].find((entry) => entry.id === source.default?.itemId) : undefined;
  const defaultMember = source?.default ? source.default.selections[source.default.itemId] : '';
  const defaultLabel = defaultItem ? [defaultItem.name, defaultMember].filter(Boolean).join(' · ') : t('codex.proxy.catalog.defaultUnavailable');
  // A failed or pending initial read is not an empty configuration.
  const catalogKnown = catalogRevision.current > 0;
  const emptyCatalog = catalogKnown && catalog.sources.length === 0;
  const hasUsableProxies = catalogKnown ? catalog.sources.some((entry) => entry.nodes.some((node) => node.supported)
    || entry.groups.some((group) => group.supported)) : null;

  const apply = (next: ProxyCatalog) => {
    catalogRevision.current += 1;
    setCatalog(next);
    setSourceId((previous) => next.sources.some((entry) => entry.id === previous) ? previous : '');
  };
  useEffect(() => {
    mounted.current = true;
    const initialRevision = catalogRevision.current;
    void getProxyCatalog().then((next) => { if (mounted.current && catalogRevision.current === initialRevision) apply(next); })
      .catch((caught) => { if (mounted.current && catalogRevision.current === initialRevision) setError(t(catalogErrorKey(caught))); })
      .finally(() => { if (mounted.current) setLoading(false); });
    return () => { mounted.current = false; if (request.current) void cancelProxyCatalog(request.current).catch(() => {}); };
    // Initial local read only; changing language must not repeat it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(() => {
    if (error || renameError) {
      const target = section.current?.querySelector<HTMLElement>('[aria-invalid="true"]')
        ?? section.current?.querySelector<HTMLElement>(':scope > [role="alert"]');
      target?.scrollIntoView({ block: 'nearest' });
      target?.focus({ preventScroll: true });
    }
  }, [error, renameError]);
  useModalFocusTrap(deleteDialog, !!deleting);
  useModalScrollLock(!!deleting);
  useEffect(() => { if (deleting) cancelDeleteButton.current?.focus(); }, [deleting]);
  useEffect(() => {
    if (!menuId) return;
    const outside = (event: PointerEvent) => {
      if (!(event.target instanceof Element) || !event.target.closest('.codex-resource-more')) setMenuId('');
    };
    const escape = (event: KeyboardEvent) => { if (event.key === 'Escape') setMenuId(''); };
    document.addEventListener('pointerdown', outside);
    document.addEventListener('keydown', escape);
    return () => { document.removeEventListener('pointerdown', outside); document.removeEventListener('keydown', escape); };
  }, [menuId]);
  useEffect(() => {
    if (deleteError) {
      deleteErrorElement.current?.scrollIntoView({ block: 'nearest' });
      deleteErrorElement.current?.focus();
    }
  }, [deleteError]);
  useEffect(() => {
    if (defaultError) {
      defaultErrorElement.current?.scrollIntoView({ block: 'nearest' });
      defaultErrorElement.current?.focus();
    }
  }, [defaultError]);

  const closeDelete = () => {
    if (removalProgress.current.catalog) apply(removalProgress.current.catalog);
    removalProgress.current = {};
    setDeleting(null);
    setDeleteError('');
    setDeleteImpact(null);
    setDeleteImpactError('');
    setDeleteImpactLoading(false);
    requestAnimationFrame(() => deleteButton.current?.focus());
  };
  // 删除弹框打开即预览真实影响；来源版本变化（如后台自动更新）后重新取一次。
  useEffect(() => {
    if (!deleting) return;
    let cancelled = false;
    setDeleteImpact(null);
    setDeleteImpactError('');
    setDeleteImpactLoading(true);
    void getProxyCatalogDependencies(deleting.id).then((next) => {
      if (!cancelled && mounted.current) setDeleteImpact(next);
    }).catch(() => {
      if (!cancelled && mounted.current) setDeleteImpactError(t('codex.proxy.catalog.deleteImpactLoadFailed'));
    }).finally(() => {
      if (!cancelled && mounted.current) setDeleteImpactLoading(false);
    });
    return () => { cancelled = true; };
    // 语言切换不能重读账号与来源；预览只在打开弹框或来源 revision 变化时重新请求。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [deleting, deletingRevision, deleteImpactRetry]);
  useEscCloseTopmost(!!deleting && busy !== 'remove', closeDelete);

  const run = async (action: string, job: (requestId: string) => Promise<void>, cancellable = false) => {
    if (operation.current || catalogPending) return;
    operation.current = true; setBusy(action); setError(''); setFormError(''); setRenameError(''); setDeleteError('');
    const id = crypto.randomUUID();
    if (cancellable) request.current = id;
    try { await job(id); } catch (caught) {
      if (mounted.current) (action === 'import' ? setFormError : action === 'rename' ? setRenameError : action === 'remove' ? setDeleteError : setError)(t(proxyRemovalErrorKey(caught, catalogErrorKey)));
      if (mounted.current && action === 'remove' && deleting) {
        // 部分失败可能已解绑账号，重新取一次影响预览再让用户重试。
        if (!removalProgress.current.catalog) setDeleteImpactRetry((value) => value + 1);
      }
      if (action === 'refresh') {
        try { const next = await getProxyCatalog(); if (mounted.current) apply(next); } catch { /* Preserve the last successful list and the original error. */ }
      }
    } finally {
      if (request.current === id) request.current = null;
      operation.current = false;
      if (mounted.current) setBusy('');
    }
  };
  const changeSource = (id: string) => {
    const nextSource = catalog.sources.find((entry) => entry.id === id);
    const draft = nextSource ? sourceDefaultDraft(nextSource) : null;
    const automatic = nextSource?.kind === 'strategy' ? nextSource.groups[0] : nextSource?.nodes.length === 1 && !nextSource.groups.length ? nextSource.nodes[0] : undefined;
    setSourceId(id); setItemId(draft?.itemId ?? automatic?.id ?? ''); setGroupId(draft?.groupId ?? (nextSource?.kind === 'strategy' ? automatic?.id ?? '' : '')); setSelectedChoices(draft?.selections ?? {});
    setExpanded(true); setAdvanced(false); setMenuId(''); setResult(null); setError(''); setDeleting(null); setDeleteError(''); setRenaming(false); setRenameError('');
    setDefaultError(''); setDefaultRetry('');
  };
  const choose = (id: string, nextGroup: string) => { setItemId(id); setGroupId(nextGroup); setSelectedChoices({}); setResult(null); setError(''); };
  const chooseMember = (selectorId: string, member: string) => {
    setSelectedChoices((old) => ({ ...old, [selectorId]: member })); setResult(null); setError('');
  };
  /** Saves or clears the source default. Failures stay beside the controls with a retry. */
  const applyDefault = async (action: 'set' | 'clear') => {
    if (operation.current || catalogPending || !source) return;
    operation.current = true;
    setDefaultPending(action); setDefaultError(''); setDefaultRetry('');
    try {
      const next = action === 'set'
        ? await setProxyCatalogDefault(source.id, itemId, selections ?? {}, groupId)
        : await clearProxyCatalogDefault(source.id);
      if (mounted.current) apply(next);
    } catch (caught) {
      if (mounted.current) { setDefaultError(t(catalogErrorKey(caught))); setDefaultRetry(action); }
    } finally {
      operation.current = false;
      if (mounted.current) setDefaultPending('');
    }
  };
  const submit = (draft: ProxySourceDraft) => void run('import', async (id) => {
    const next = await importProxyCatalog(id, draft.name, draft.input, draft.kind, draft.options);
    if (!mounted.current) return;
    const added = next.sources.find((entry) => !catalog.sources.some((old) => old.id === entry.id));
    apply(next); if (added) { setSourceId(added.id); setExpanded(true); setItemId(added.nodes.length === 1 && !added.groups.length ? added.nodes[0].id : ''); setGroupId(''); setSelectedChoices({}); } setQuery(''); setAdding(false);
  }, true);

  const assign = (entry: ProxyCatalogSource) => {
    // An incomplete explicit choice must never fall back to an older source default.
    if (entry.id === sourceId && itemId && !selectionReady) { setExpanded(true); return; }
    const remembered = entry.id === sourceId && selectionReady ? { itemId, groupId, selections: selections ?? {} } : sourceDefaultDraft(entry);
    const single = entry.kind === 'strategy' ? entry.groups[0] : entry.nodes.length === 1 && !entry.groups.length ? entry.nodes[0] : undefined;
    const choices = single ? defaultProxySelections(entry, single.id) : null;
    const draft = remembered ?? (single && choices ? { itemId: single.id, groupId: entry.kind === 'strategy' ? single.id : '', selections: choices } : null);
    if (!draft) { if (sourceId !== entry.id) changeSource(entry.id); else setExpanded(true); return; }
    void run('prepare-assign', async () => {
      await preflightCodexProxyEngine();
      if (mounted.current) onAssign(entry.id, draft.itemId, draft.selections, draft.groupId, catalog);
    });
  };
  const editSource = (entry: ProxyCatalogSource) => {
    changeSource(entry.id); setRenaming(true); setRenameValue(entry.name);
  };
  const cancelImport = () => {
    if (request.current) void cancelProxyCatalog(request.current).catch((caught) => setFormError(t(catalogErrorKey(caught))));
  };
  const openAdd = () => void run('prepare-add', async () => {
    await preflightCodexProxyEngine();
    if (mounted.current) { setAdding(true); setFormError(''); setMenuId(''); }
  });
  const openStrategy = (id: string) => void run('prepare-strategy', async () => {
    await preflightCodexProxyEngine();
    if (mounted.current) { setStrategyEditing(id); setMenuId(''); }
  });

  return <section ref={section} className="codex-proxy-resources" aria-label={t('codex.proxy.managerResources.title')}>
    <CodexProxySetupGuide empty={emptyCatalog} hasUsableProxies={hasUsableProxies} addDisabled={locked || adding}
      onAdd={openAdd} />
    {!emptyCatalog && <div className="codex-resource-toolbar">
      <label className="codex-proxy-search codex-resource-catalog-search"><Search size={17} aria-hidden="true" />
        <input value={query} aria-label={t('codex.proxy.managerResources.search')} placeholder={t('codex.proxy.managerResources.search')} onChange={(event) => setQuery(event.target.value)} />
        {query && <button type="button" className="btn btn-secondary compact" aria-label={t('common.close')} onClick={() => setQuery('')}><X size={14} /></button>}
      </label>
      <div className="codex-resource-toolbar-actions">
        <button type="button" className="btn btn-secondary" disabled={locked || !hasCandidates} title={!hasCandidates ? t('codex.proxy.catalog.strategyNoCandidates') : undefined}
          onClick={() => openStrategy('new')}><Layers size={16} />{t('codex.proxy.managerResources.createGroup')}</button>
        <button type="button" className="btn btn-primary" disabled={locked || adding} onClick={openAdd}><Plus size={16} />{t('codex.proxy.managerResources.add')}</button>
      </div>
    </div>}
    {adding && <CodexProxyResourceForm busy={busy === 'import'} error={formError} onEdit={() => setFormError('')} onSubmit={submit} onCancel={cancelImport} onClose={() => { setAdding(false); setFormError(''); }} />}
    {busy && busy !== 'import' && <div className="codex-resource-progress" role="status"><RefreshCw size={15} className="loading-spinner" /><span>{t('common.processing')}</span>
      {request.current && <button type="button" className="btn btn-secondary compact" onClick={() => void cancelProxyCatalog(request.current!).catch((caught) => setError(t(catalogErrorKey(caught))))}>{t('common.cancel')}</button>}</div>}
    {error && <div className="codex-proxy-page-error" role="alert" tabIndex={-1}>{error}
      {!source && <button type="button" className="btn btn-secondary compact" disabled={!!busy} onClick={() => void run('load', async () => { const next = await getProxyCatalog(); if (mounted.current) apply(next); })}>{t('common.retry')}</button>}</div>}
    {latency.running && (!expanded || !visibleSources.some((entry) => entry.id === sourceId)) && <div className="codex-resource-progress"><button type="button" className="btn btn-secondary compact" onClick={latency.cancel}>{t('codex.proxy.cancelCheck')}</button></div>}
    {loading && !catalogKnown ? <p className="codex-proxy-page-note" role="status">{t('common.loading')}</p> : catalog.sources.length === 0 ? null : <>
      <div className="codex-resource-list-caption"><span>{t('codex.proxy.managerResources.count', { count: catalog.sources.length })}</span><span>{t('codex.proxy.managerResources.listHint')}</span></div>
      <div className="codex-resource-list">{visibleSources.map((entry) => {
        const open = expanded && sourceId === entry.id;
        const strategy = entry.kind === 'strategy';
        const firstNode = entry.nodes.length === 1 ? entry.nodes[0] : null;
        const measured = firstNode && entry.id === sourceId ? latency.results[firstNode.id] : null;
        const typeLabel = strategy ? t(strategyKindKey(strategyKindOf(entry) ?? '')) : t(entry.kind === 'subscription' ? 'codex.proxy.catalog.input_subscription' : 'codex.proxy.managerResources.manual');
        return <article key={entry.id} className={'codex-resource-entry' + (open ? ' is-expanded' : '')}>
          <div className="codex-resource-entry-row">
            <button type="button" className="codex-resource-entry-main" aria-expanded={open} aria-controls={`proxy-resource-${entry.id}`} disabled={!!busy}
              onClick={() => { if (sourceId === entry.id) { if (open) latency.cancel(); setExpanded(!open); } else changeSource(entry.id); }}>
              <span className={'codex-resource-entry-icon ' + entry.kind}>{strategy ? <Layers size={19} /> : entry.kind === 'subscription' ? <Download size={19} /> : <Link2 size={19} />}</span>
              <span className="codex-resource-entry-name"><strong title={entry.name}>{entry.name}</strong><small>{typeLabel}{firstNode && !strategy ? ` · ${firstNode.protocol.toUpperCase()}` : ` · ${t('codex.proxy.catalog.strategyMemberCount', { count: entry.nodes.length })}`}</small></span>
              <ChevronDown size={16} className="codex-resource-entry-chevron" aria-hidden="true" />
            </button>
            <div className="codex-resource-entry-status">
              {entry.defaultInvalidated || entry.error ? <span className="codex-resource-entry-warning"><TriangleAlert size={13} />{t(entry.defaultInvalidated ? 'codex.proxy.catalog.defaultUnavailable' : catalogErrorKey(entry.error))}</span>
                : entry.kind === 'subscription' && entry.usage ? <span>{t('codex.proxy.resources.usageLine', { used: formatProxyBytes(entry.usage.upload + entry.usage.download), total: entry.usage.total > 0 ? formatProxyBytes(entry.usage.total) : t('codex.proxy.resources.usageUnknown') })}</span>
                  : measured?.status === 'success' ? <CodexProxyLatencyBadge result={measured} /> : <span>{t('codex.proxy.catalog.availabilityCount', { count: entry.nodes.length, available: entry.nodes.filter((node) => node.supported).length })}</span>}
              {entry.kind === 'subscription' && entry.usage?.expireAt ? <small>{t('codex.proxy.resources.expireLine', { date: formatProxyDateTime(entry.usage.expireAt) })}</small>
                : <small>{t('codex.proxy.catalog.updatedAt', { time: formatProxyDateTime(entry.updatedAt) })}</small>}
            </div>
            <div className="codex-resource-entry-actions">
              <button type="button" className="btn btn-primary compact" disabled={locked || (!entry.nodes.some((node) => node.supported) && !entry.groups.some((group) => group.supported))} onClick={() => assign(entry)}>{t('codex.proxy.managerResources.assign')}<ArrowRight size={14} /></button>
              <div className="codex-resource-more">
                <button type="button" className="btn btn-secondary compact codex-resource-more-trigger" disabled={locked} aria-expanded={menuId === entry.id} aria-label={t('codex.proxy.managerResources.more')}
                  onClick={() => setMenuId(menuId === entry.id ? '' : entry.id)}><MoreHorizontal size={18} /></button>
                {menuId === entry.id && <div className="codex-resource-more-menu" role="group" aria-label={t('codex.proxy.managerResources.more')}>
                  {strategy ? <button type="button" className="btn btn-secondary" onClick={() => openStrategy(entry.id)}><Pencil size={14} />{t('codex.proxy.catalog.strategyEdit')}</button>
                    : <button type="button" className="btn btn-secondary" onClick={() => editSource(entry)}><Pencil size={14} />{t('codex.proxy.catalog.rename')}</button>}
                  {entry.kind === 'subscription' && <button type="button" className="btn btn-secondary" onClick={() => { setMenuId(''); void run('refresh', async (id) => { const next = await refreshProxyCatalog(id, entry.id); if (mounted.current) { apply(next); setResult(null); } }, true); }}><RefreshCw size={14} />{t('common.refresh')}</button>}
                  <button type="button" className="btn btn-secondary" onClick={() => { if (sourceId !== entry.id) changeSource(entry.id); setExpanded(true); setAdvanced(true); setMenuId(''); }}><Star size={14} />{t('common.advancedSettings')}</button>
                  <button type="button" className="btn btn-secondary codex-resource-delete-action" onClick={(event) => { deleteButton.current = event.currentTarget.closest('.codex-resource-more')?.querySelector<HTMLButtonElement>('.codex-resource-more-trigger') ?? null; setMenuId(''); if (strategy) setStrategyDeleting(entry.id); else { setDeleting({ id: entry.id, name: entry.name }); setDeleteError(''); } }}><Trash2 size={14} />{t('common.delete')}</button>
                </div>}
              </div>
            </div>
          </div>
          {open && source && <div className="codex-resource-expanded" id={`proxy-resource-${entry.id}`}>
            {renaming && <div className="codex-resource-rename"><label className="codex-proxy-secret"><input autoFocus value={renameValue} maxLength={80} disabled={!!busy} aria-label={t('codex.proxy.catalog.sourceName')} aria-invalid={!!renameError} onChange={(event) => { setRenameValue(event.target.value); setRenameError(''); }} /></label>
              <button type="button" className="btn btn-primary compact" disabled={locked || !renameValue.trim()} onClick={() => void run('rename', async () => { const next = await renameProxySource(source.id, renameValue); if (mounted.current) { apply(next); setRenaming(false); } })}>{t('common.save')}</button>
              <button type="button" className="btn btn-secondary compact" disabled={!!busy} onClick={() => { setRenaming(false); setRenameError(''); }}>{t('common.cancel')}</button>
              {renameError && <div className="codex-proxy-page-error" role="alert">{renameError}</div>}</div>}
            {source.kind === 'strategy' ? <CodexProxyPicker key={source.id} source={source} itemId={itemId} selectedGroupId={groupId} selections={selectedChoices} busy={!!busy || !!defaultPending} latency={latency} choose={choose} chooseMember={chooseMember}
              onCatalogChange={(next) => { apply(next); setResult(null); }} onPendingChange={setCatalogPending} />
              : <>
                <CodexProxyResourceNodes key={source.id} source={source} itemId={itemId} busy={locked} latency={latency} onChoose={(id) => choose(id, '')}
                  onInsecure={(nodeId, enabled) => void run('insecure', async () => { const next = await setProxyNodeInsecure(source.id, nodeId, source.revision, enabled); if (mounted.current) { apply(next); setResult(null); } })} />
                {!!source.groups.length && <details className="codex-resource-group-options"><summary>{t('codex.proxy.catalog.groups')}</summary>
                  <CodexProxyPicker key={source.id} source={source} itemId={itemId} selectedGroupId={groupId} selections={selectedChoices} busy={!!busy || !!defaultPending} latency={latency} choose={choose} chooseMember={chooseMember} onCatalogChange={(next) => { apply(next); setResult(null); }} onPendingChange={setCatalogPending} />
                </details>}
              </>}
            <div className="codex-resource-selection-bar">
              <span className="codex-resource-selection"><ShieldCheck size={17} /><strong>{selected?.name ?? t('codex.proxy.managerResources.pickNode')}</strong></span>
              <div className="codex-resource-bind-controls"><button type="button" className="btn btn-secondary compact" disabled={locked || !selectionReady} onClick={() => void run('probe', async (id) => { const checked = await probeProxyCatalog(id, source.id, itemId, selections ?? {}); if (mounted.current) setResult(checked); }, true)}><Activity size={15} />{t('codex.proxy.test')}</button>
                <button type="button" className="btn btn-primary compact" disabled={locked || !selectionReady} onClick={() => assign(source)}>{t('codex.proxy.managerResources.assign')}<ArrowRight size={14} /></button></div>
            </div>
            {result && <output className="codex-proxy-check-result" aria-live="polite"><ShieldCheck size={18} /><div><strong>{t('codex.proxy.testPassed')}</strong><span>{result.protocol} · {result.ip} · {result.latencyMs} ms</span><time dateTime={new Date(result.checkedAt).toISOString()}>{new Date(result.checkedAt).toLocaleString()}</time></div></output>}
            <details className="codex-resource-advanced" open={advanced} onToggle={(event) => setAdvanced(event.currentTarget.open)}><summary>{t('common.advancedSettings')}</summary>
              {source.kind === 'subscription' && <div className="codex-resource-auto"><button type="button" role="switch" aria-checked={source.autoUpdate} className={`codex-resource-switch${source.autoUpdate ? ' on' : ''}`} disabled={locked} aria-label={t('codex.proxy.catalog.autoUpdate')}
                onClick={() => void run('auto', async () => { const next = await setProxyCatalogAutoUpdate(source.id, !source.autoUpdate); if (mounted.current) apply(next); })}><span /></button><span>{t('codex.proxy.catalog.autoUpdate')}</span></div>}
              <div className={`codex-resource-default${source.default ? ' is-set' : ''}`}><div className="codex-resource-default-head"><Star size={16} /><strong>{t('codex.proxy.catalog.defaultLabel')}</strong><span>{source.default ? defaultLabel : t('codex.proxy.catalog.defaultEmpty')}</span></div>
                <div className="codex-resource-default-actions"><button type="button" className="btn btn-secondary compact" disabled={locked || !selectionReady} onClick={() => void applyDefault('set')}>{defaultPending === 'set' && <RefreshCw size={14} className="loading-spinner" />}{t('codex.proxy.catalog.setDefault')}</button>
                  {(source.default || source.defaultInvalidated) && <button type="button" className="btn btn-secondary compact" disabled={locked} onClick={() => void applyDefault('clear')}>{t('codex.proxy.catalog.clearDefault')}</button>}</div></div>
              {source.defaultInvalidated && <p className="codex-proxy-page-error" role="alert">{t('codex.proxy.catalog.defaultInvalidated')}</p>}
              {defaultError && <div ref={defaultErrorElement} className="codex-proxy-page-error" role="alert" tabIndex={-1}><span>{defaultError}</span>{defaultRetry && <button type="button" className="btn btn-secondary compact" disabled={locked} onClick={() => void applyDefault(defaultRetry)}>{t('common.retry')}</button>}</div>}
              <p className="codex-proxy-page-note">{t('codex.proxy.catalog.defaultHint')}</p><p className="codex-proxy-page-note">{t('codex.proxy.catalog.snapshotNotice')}</p><p className="codex-proxy-page-note">{t('codex.proxy.testNotice')}</p>
            </details>
          </div>}
        </article>;
      })}</div>
      {!visibleSources.length && <div className="codex-resource-no-results">{t('codex.proxy.catalog.noResults')}</div>}
    </>}
    {strategyEditing && <StrategyEditorDialog source={catalog.sources.find((entry) => entry.id === strategyEditing) ?? null} sources={catalog.sources} busy={locked} onClose={() => setStrategyEditing('')}
      onSaved={(next) => { apply(next); setStrategyEditing(''); setQuery(''); }} />}
    {strategyDeleting && catalog.sources.some((entry) => entry.id === strategyDeleting) && <StrategyDeleteDialog source={catalog.sources.find((entry) => entry.id === strategyDeleting)!} busy={locked} onClose={() => setStrategyDeleting('')}
      onBindingsChanged={() => onBindingsChanged(strategyDeleting, true)}
      onRemoved={(next) => { apply(next); setStrategyDeleting(''); }} />}
    {deleting && typeof document !== 'undefined' && createPortal(<div className="modal-overlay codex-resource-delete-overlay">
      <div ref={deleteDialog} tabIndex={-1} className="modal-content codex-resource-delete-modal" role="alertdialog" aria-modal="true" aria-labelledby="codex-resource-delete-title" aria-describedby="codex-resource-delete-name codex-resource-delete-notice">
        <div className="modal-header"><h2 id="codex-resource-delete-title">{t('common.confirmDelete')}</h2>
          <button type="button" className="btn btn-secondary compact" disabled={busy === 'remove'} aria-label={t('common.close')} onClick={closeDelete}><X size={16} /></button></div>
        <div className="modal-body"><strong id="codex-resource-delete-name" className="codex-resource-delete-name">{deleting.name}</strong>
          <div className="codex-resource-delete-impact" aria-live="polite">
            <h3>{t('codex.proxy.catalog.deleteImpactTitle')}</h3>
            {deleteImpactLoading && <p className="codex-resource-delete-pending" role="status">{t('common.loading')}</p>}
            {!deleteImpactLoading && deleteImpactError && <div className="codex-proxy-page-error" role="alert">
              <span>{deleteImpactError}</span>
              <button type="button" className="btn btn-secondary compact" disabled={busy === 'remove'} onClick={() => setDeleteImpactRetry((value) => value + 1)}>{t('common.retry')}</button></div>}
            {!deleteImpactLoading && deleteImpact && <>
              <p className="codex-resource-delete-count">{t('codex.proxy.catalog.deleteImpactAccounts', { count: deleteImpact.accountCount })}</p>
              {deleteImpact.accounts.length === 0 ? <p className="codex-resource-delete-none">{t('codex.proxy.catalog.deleteImpactNone')}</p>
                : <ul className="codex-resource-delete-accounts">{deleteImpact.accounts.map((entry) => <li key={entry.id}>
                  <strong>{resolveName({ id: entry.id, account_name: entry.name })}</strong>{entry.email && entry.email !== entry.name ? <span>{resolveName({ id: entry.id, email: entry.email })}</span> : null}</li>)}</ul>}
              <p className="codex-resource-delete-unified">{deleteImpact.unifiedProxy ? t('codex.proxy.catalog.deleteImpactUnifiedClose') : t('codex.proxy.catalog.deleteImpactUnifiedKept')}</p>
              {deleteImpact.strategyCount > 0 && <>
                <p className="codex-resource-delete-count">{t('codex.proxy.catalog.deleteImpactStrategies', { count: deleteImpact.strategyCount })}</p>
                <ul className="codex-resource-delete-accounts">{deleteImpact.strategies.map((entry, index) => <li key={`${index}-${entry}`}>
                  <strong>{entry}</strong></li>)}</ul>
                <p>{t('codex.proxy.catalog.deleteImpactStrategyNotice')}</p>
              </>}
            </>}
          </div>
          <p id="codex-resource-delete-notice">{t('codex.proxy.catalog.deleteNotice')}</p>
          {deleteError && <div ref={deleteErrorElement} className="codex-proxy-page-error" role="alert" tabIndex={-1}>{deleteError}</div>}</div>
        <div className="modal-footer"><button ref={cancelDeleteButton} type="button" className="btn btn-secondary" disabled={busy === 'remove'} onClick={closeDelete}>{t('common.cancel')}</button>
          <button type="button" className="btn btn-danger" disabled={locked || !deleteImpact} onClick={() => void run('remove', async () => {
            const next = await removeProxySourceWithRefresh(removalProgress.current,
              () => removeProxyCatalog(deleting.id), () => onBindingsChanged(deleting.id, Boolean(removalProgress.current.catalog)));
            if (mounted.current) { apply(next); removalProgress.current = {}; setItemId(''); setGroupId(''); setResult(null); setDeleting(null); }
          })}>{busy === 'remove' ? <><RefreshCw size={15} className="loading-spinner" />{t('common.processing')}</> : t(removalProgress.current.catalog ? 'common.retry' : 'common.confirmDelete')}</button></div>
      </div>
    </div>, document.body)}
  </section>;
}
