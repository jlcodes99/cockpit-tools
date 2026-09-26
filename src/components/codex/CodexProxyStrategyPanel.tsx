import { useEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { useCodexProxyAccountName } from './useCodexProxyExitEditor';
import { ArrowDown, ArrowUp, Check, Layers, Pencil, Plus, RefreshCw, Search, Trash2, TriangleAlert, X } from 'lucide-react';
import { useEscCloseTopmost } from '../../hooks/useEscClose';
import { useModalFocusTrap } from '../../hooks/useModalFocusTrap';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import { ModalErrorMessage } from '../ModalErrorMessage';
import { SingleSelectFilterDropdown } from '../SingleSelectFilterDropdown';
import { proxyRemovalErrorKey, refreshCodexProxyAccounts, removeProxySourceWithRefresh, type ProxyRemovalProgress } from '../../utils/codexProxyRemoval';
import { getProxyCatalogDependencies, type ProxyCatalog, type ProxyCatalogDependencies, type ProxyCatalogSource } from '../../services/codexProxyCatalogService';
import {
  PROXY_STRATEGY_DEFAULTS, PROXY_STRATEGY_KINDS, PROXY_STRATEGY_LIMITS, PROXY_STRATEGY_OPTION_FIELDS, strategyOptionsForm, removeProxyStrategy,
  saveProxyStrategy, strategyCandidates, filterStrategyCandidates, isPossibleProxyNotice, strategyEditorMembers, strategyErrorKey, strategyHintKey, strategyKindKey, strategyKindOf,
  strategyMemberId, strategyMemberViews, strategyNameTaken, strategyNoticeKey, strategyOptionErrors, strategyOptions, strategyViews,
  type ProxyStrategyCandidate, type ProxyStrategyKind, type ProxyStrategyOptionField, type ProxyStrategyOptionsForm,
} from '../../services/codexProxyStrategyService';
import '../../styles/pages/codex-proxy-strategy.css';

type StrategyMemberItem = Pick<ProxyStrategyCandidate, 'sourceId' | 'itemId' | 'name' | 'sourceName'>;

/** Health check fields, in the order the engine reads them. */
const ADVANCED_FIELDS: { field: ProxyStrategyOptionField; labelKey: string }[] = [
  { field: 'url', labelKey: 'codex.proxy.catalog.strategyHealthUrl' },
  { field: 'interval', labelKey: 'codex.proxy.catalog.strategyInterval' },
  { field: 'timeout', labelKey: 'codex.proxy.catalog.strategyTimeout' },
  { field: 'tolerance', labelKey: 'codex.proxy.catalog.strategyTolerance' },
];

interface PanelProps {
  sources: ProxyCatalogSource[];
  /** A proxy page operation is running: strategy writes wait for it. */
  busy: boolean;
  onCatalog: (next: ProxyCatalog) => void;
  /** A removed strategy may have unbound accounts; the page refreshes those bindings. */
  onRemoved: (sourceId: string) => void;
}

/** Own list of saved strategies. Members are read back from the catalog view, never guessed. */
export function CodexProxyStrategyPanel({ sources, busy, onCatalog, onRemoved }: PanelProps) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState('');
  const [deleting, setDeleting] = useState('');
  const strategies = useMemo(() => strategyViews(sources), [sources]);
  const candidates = useMemo(() => strategyCandidates(sources), [sources]);
  const editingSource = editing && editing !== 'new' ? sources.find((entry) => entry.id === editing) ?? null : null;
  const deletingSource = deleting ? sources.find((entry) => entry.id === deleting) ?? null : null;
  const dialogOpen = editing !== '' || deleting !== '';

  return <section className="codex-strategy" aria-labelledby="codex-strategy-title">
    <div className="codex-strategy-head">
      <div>
        <h3 id="codex-strategy-title"><Layers size={16} aria-hidden="true" />{t('codex.proxy.catalog.strategyTitle')}</h3>
        <p>{t('codex.proxy.catalog.strategyIntro')}</p>
      </div>
      <button type="button" className="btn btn-secondary" disabled={busy || dialogOpen || !candidates.length} onClick={() => setEditing('new')}>
        <Plus size={15} aria-hidden="true" />{t('codex.proxy.catalog.strategyCreate')}
      </button>
    </div>
    {strategies.length === 0
      ? <p className="codex-proxy-page-note">{candidates.length ? t('codex.proxy.catalog.strategyEmpty') : t('codex.proxy.catalog.strategyNoCandidates')}</p>
      : <ul className="codex-strategy-list">{strategies.map((strategy) => {
        const missing = strategy.members.filter((member) => !member.matched).length;
        const originRemoved = strategy.members.filter((member) => member.originRemoved).length;
        const order = strategy.members.map((member) => member.name).join(' → ');
        return <li key={strategy.id} className="codex-strategy-card">
          <div className="codex-strategy-card-head">
            <div className="codex-strategy-card-title">
              <strong>{strategy.name}</strong>
              <span className="codex-strategy-tag">{t(strategyKindKey(strategy.kind ?? ''))}</span>
            </div>
            <div className="codex-strategy-card-actions">
              <button type="button" className="btn btn-secondary compact" disabled={busy || dialogOpen}
                onClick={() => setEditing(strategy.id)}><Pencil size={14} aria-hidden="true" />{t('codex.proxy.catalog.strategyEdit')}</button>
              <button type="button" className="btn btn-secondary compact" disabled={busy || dialogOpen} aria-label={t('common.delete')}
                onClick={() => setDeleting(strategy.id)}><Trash2 size={15} aria-hidden="true" /></button>
            </div>
          </div>
          <p className="codex-strategy-order" title={order}>
            <span className="codex-strategy-count">{t('codex.proxy.catalog.strategyMemberCount', { count: strategy.members.length })}</span>
            <span className="codex-strategy-order-text">{order}</span>
          </p>
          {missing > 0 && <p className="codex-strategy-warning" role="status">
            <TriangleAlert size={13} aria-hidden="true" />{t('codex.proxy.catalog.strategyMemberMissing', { count: missing })}</p>}
          {originRemoved > 0 && <p className="codex-strategy-warning" role="status">
            <TriangleAlert size={13} aria-hidden="true" />{t('codex.proxy.catalog.strategyMemberOriginRemoved', { count: originRemoved })}</p>}
          <div className="codex-strategy-card-meta">
            <span>{t('codex.proxy.catalog.updatedAt', { time: new Date(strategy.updatedAt).toLocaleString() })}</span>
            {strategy.error && <span className="codex-strategy-error" role="alert">{t(strategyErrorKey(strategy.error))}</span>}
          </div>
        </li>;
      })}</ul>}
    {editing !== '' && <StrategyEditorDialog source={editingSource} sources={sources} busy={busy}
      onClose={() => setEditing('')} onSaved={(next) => { onCatalog(next); setEditing(''); }} />}
    {deletingSource && <StrategyDeleteDialog source={deletingSource} busy={busy} onClose={() => setDeleting('')}
      onRemoved={(next) => { onCatalog(next); onRemoved(deletingSource.id); setDeleting(''); }} />}
  </section>;
}

/** Creates or edits one strategy. Every failure stays in the dialog (rules 15-17). */
export function StrategyEditorDialog({ source, sources, busy, onClose, onSaved }: {
  source: ProxyCatalogSource | null;
  sources: ProxyCatalogSource[];
  busy: boolean;
  onClose: () => void;
  onSaved: (next: ProxyCatalog) => void;
}) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDivElement>(null);
  const body = useRef<HTMLDivElement>(null);
  const mounted = useRef(true);
  // Members are read once on open: a replayed render must not rebuild the member order.
  const [initial] = useState(() => {
    const editor = strategyEditorMembers(source ? strategyMemberViews(source, sources) : []);
    return {
      name: source?.name ?? '',
      kind: ((source ? strategyKindOf(source) : null) ?? 'fallback') as ProxyStrategyKind,
      // 原来源已删除的成员仍靠策略内的副本工作：原样进入编辑列表，保存后继续沿用副本。
      members: editor.members,
      kept: editor.kept,
      unmatched: editor.unmatched,
      options: strategyOptionsForm(source),
    };
  });
  const [name, setName] = useState(initial.name);
  const [kind, setKind] = useState<ProxyStrategyKind>(initial.kind);
  const [members, setMembers] = useState<StrategyMemberItem[]>(initial.members);
  const [query, setQuery] = useState('');
  const [candidateSource, setCandidateSource] = useState('');
  const [form, setForm] = useState<ProxyStrategyOptionsForm>(initial.options);
  const [error, setError] = useState('');
  const [submitted, setSubmitted] = useState(false);
  const [advanced, setAdvanced] = useState(false);
  const [saving, setSaving] = useState(false);
  const candidates = useMemo(() => strategyCandidates(sources), [sources]);
  const optionFields = PROXY_STRATEGY_OPTION_FIELDS[kind];
  const optionErrors = strategyOptionErrors(form, kind);
  const selected = useMemo(() => new Set(members.map(strategyMemberId)), [members]);
  const visible = useMemo(() => {
    return filterStrategyCandidates(candidates, query, candidateSource)
      .filter((entry) => !selected.has(strategyMemberId(entry)));
  }, [candidates, query, candidateSource, selected]);
  const membersInvalid = submitted && !members.length;
  /** 内核按名称区分成员：同名成员只能保留一个，弹框内先行拦截（规则 15-17）。 */
  const duplicateNames = useMemo(() => {
    const seen = new Set<string>();
    const duplicates = new Set<string>();
    for (const member of members) {
      const key = member.name.trim();
      if (!key) continue;
      if (seen.has(key)) duplicates.add(key);
      else seen.add(key);
    }
    return duplicates;
  }, [members]);
  const membersDuplicated = submitted && duplicateNames.size > 0;
  const full = members.length >= PROXY_STRATEGY_LIMITS.members;
  /** The engine resolves group members by name, so the strategy may not shadow one of them. */
  const nameTaken = strategyNameTaken(name, members.map((entry) => entry.name));
  const nameInvalid = submitted && (!name.trim() || nameTaken);
  /** Any edit clears the previous failure so nothing is reported twice (rule 17). */
  const edit = () => { if (error) setError(''); };

  useEscCloseTopmost(true, () => { if (!saving) onClose(); });
  useModalScrollLock(true);
  useModalFocusTrap(dialog, true);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);
  // Field errors are revealed together; focus and scroll land on the first invalid control.
  useEffect(() => {
    if (!submitted) return;
    const frame = requestAnimationFrame(() => {
      const field = body.current?.querySelector<HTMLElement>('[aria-invalid="true"]');
      if (!field) return;
      field.scrollIntoView({ block: 'nearest' });
      field.focus({ preventScroll: true });
    });
    return () => cancelAnimationFrame(frame);
  }, [submitted]);

  const add = (candidate: ProxyStrategyCandidate) => {
    edit();
    setMembers((old) => old.length >= PROXY_STRATEGY_LIMITS.members
      || old.some((entry) => strategyMemberId(entry) === strategyMemberId(candidate))
      ? old
      : [...old, { sourceId: candidate.sourceId, itemId: candidate.itemId, name: candidate.name, sourceName: candidate.sourceName }]);
  };
  const move = (index: number, delta: number) => setMembers((old) => {
    const target = index + delta;
    if (target < 0 || target >= old.length) return old;
    const next = [...old];
    [next[index], next[target]] = [next[target], next[index]];
    return next;
  });
  const submit = async () => {
    if (saving || busy) return;
    setSubmitted(true);
    setError('');
    if (Object.keys(optionErrors).length) setAdvanced(true);
    if (!name.trim() || nameTaken || !members.length || duplicateNames.size || Object.keys(optionErrors).length) return;
    setSaving(true);
    try {
      const next = await saveProxyStrategy({
        ...(source ? { id: source.id } : {}), name, kind, members, options: strategyOptions(form, kind),
      });
      if (mounted.current) onSaved(next);
    } catch (caught) {
      if (mounted.current) setError(t(strategyErrorKey(caught)));
    } finally {
      if (mounted.current) setSaving(false);
    }
  };
  const fieldError = (field: ProxyStrategyOptionField) => optionErrors[field]
    ? field === 'url'
      ? t(optionErrors[field]!)
      : t(optionErrors[field]!, { min: PROXY_STRATEGY_LIMITS[field].min, max: PROXY_STRATEGY_LIMITS[field].max })
    : '';

  return createPortal(<div className="modal-overlay codex-strategy-overlay">
    <div className="modal codex-strategy-dialog" ref={dialog} role="dialog" aria-modal="true" aria-labelledby="codex-strategy-dialog-title" tabIndex={-1}>
      <header className="modal-header">
        <h2 id="codex-strategy-dialog-title">{t(source ? 'codex.proxy.managerResources.editGroup' : 'codex.proxy.managerResources.createGroup')}</h2>
        <button type="button" className="btn btn-secondary compact" disabled={saving} aria-label={t('common.close')} onClick={onClose}><X size={16} /></button>
      </header>
      <div className="modal-body codex-strategy-dialog-body" ref={body}>
        <ModalErrorMessage message={error} />
        <label className="codex-strategy-field">
          <span>{t('codex.proxy.catalog.strategyNameLabel')}</span>
          <input className="codex-strategy-input" value={name} maxLength={PROXY_STRATEGY_LIMITS.name} autoComplete="off" spellCheck={false}
            disabled={saving} aria-invalid={nameInvalid} placeholder={t('codex.proxy.catalog.strategyNamePlaceholder')}
            onChange={(event) => { setName(event.target.value); edit(); }} />
          {nameInvalid && <span className="codex-strategy-field-error" role="alert">
            {t(nameTaken ? 'codex.proxy.catalog.strategyErrorNameTaken' : 'codex.proxy.catalog.strategyErrorName')}</span>}
        </label>
        <div className="codex-strategy-block">
          <span className="codex-strategy-label">{t('codex.proxy.catalog.strategyTypeLabel')}</span>
          <div className="codex-strategy-kind" role="group" aria-label={t('codex.proxy.catalog.strategyTypeLabel')}>
            {PROXY_STRATEGY_KINDS.map((option) => <button key={option} type="button" disabled={saving} aria-pressed={kind === option}
              className={'btn btn-secondary' + (kind === option ? ' active' : '')} onClick={() => { setKind(option); edit(); }}>{t(strategyKindKey(option))}</button>)}
          </div>
          <p className="codex-proxy-page-note">{t(strategyHintKey(kind))}</p>
        </div>
        <div className="codex-strategy-block" aria-invalid={membersInvalid || membersDuplicated}
          tabIndex={membersInvalid || membersDuplicated ? -1 : undefined}>
          <div className="codex-strategy-block-head">
            <span className="codex-strategy-label">{t('codex.proxy.catalog.strategyMemberPick')}</span>
            <span className="codex-proxy-page-note">{t('codex.proxy.catalog.strategyMemberLimit', { count: PROXY_STRATEGY_LIMITS.members })}</span>
          </div>
          <SingleSelectFilterDropdown value={candidateSource} disabled={saving}
            ariaLabel={t('codex.proxy.catalog.strategySourceFilter')}
            options={[{ value: '', label: t('codex.proxy.catalog.strategyAllSources') },
              ...sources.filter((entry) => entry.kind !== 'strategy').map((entry) => ({ value: entry.id, label: entry.name }))]}
            onChange={setCandidateSource} />
          <label className="codex-proxy-search codex-strategy-search"><Search size={15} aria-hidden="true" />
            <input value={query} disabled={saving} aria-label={t('codex.proxy.catalog.strategyMemberSearch')}
              placeholder={t('codex.proxy.catalog.strategyMemberSearch')} onChange={(event) => setQuery(event.target.value)} /></label>
          <div className="codex-strategy-columns">
            <div className="codex-strategy-candidates" role="group" aria-label={t('codex.proxy.catalog.strategyMemberPick')}>
              {visible.map((candidate) => <button key={strategyMemberId(candidate)} type="button" className="codex-strategy-candidate"
                disabled={saving || full} onClick={() => add(candidate)} title={`${candidate.name} · ${candidate.sourceName}`}>
                <Plus size={14} aria-hidden="true" />
                <span><strong>{candidate.name}</strong><small>{candidate.sourceName} · {candidate.protocol.toUpperCase()}</small>
                  {candidate.server && <small>{candidate.server}{candidate.port ? `:${candidate.port}` : ''}</small>}
                  {isPossibleProxyNotice(candidate.name) && <small>{t('codex.proxy.catalog.strategyPossibleNotice')}</small>}
                </span>
              </button>)}
              {!visible.length && <p className="codex-proxy-page-note">{t('codex.proxy.catalog.noResults')}</p>}
            </div>
            <div className="codex-strategy-selected">
              <span className="codex-strategy-label">{t('codex.proxy.catalog.strategyMemberSelected')}</span>
              {!members.length && <p className="codex-proxy-page-note">{t('codex.proxy.catalog.strategyNoMembers')}</p>}
              <ol className="codex-strategy-orderlist">{members.map((member, index) => <li key={strategyMemberId(member)}>
                <span className="codex-strategy-index" aria-hidden="true">{index + 1}</span>
                <span className="codex-strategy-member"><strong>{member.name}</strong><small>{member.sourceName}</small></span>
                <span className="codex-strategy-member-actions">
                  <button type="button" className="btn btn-secondary compact" disabled={saving || index === 0}
                    aria-label={t('codex.proxy.catalog.strategyMemberUp')} title={t('codex.proxy.catalog.strategyMemberUp')}
                    onClick={() => move(index, -1)}><ArrowUp size={14} aria-hidden="true" /></button>
                  <button type="button" className="btn btn-secondary compact" disabled={saving || index === members.length - 1}
                    aria-label={t('codex.proxy.catalog.strategyMemberDown')} title={t('codex.proxy.catalog.strategyMemberDown')}
                    onClick={() => move(index, 1)}><ArrowDown size={14} aria-hidden="true" /></button>
                  <button type="button" className="btn btn-secondary compact" disabled={saving}
                    aria-label={t('codex.proxy.catalog.strategyMemberRemove')} title={t('codex.proxy.catalog.strategyMemberRemove')}
                    onClick={() => { edit(); setMembers((old) => old.filter((_, position) => position !== index)); }}><X size={14} aria-hidden="true" /></button>
                </span>
              </li>)}</ol>
            </div>
          </div>
          {membersInvalid && <span className="codex-strategy-field-error" role="alert">{t('codex.proxy.catalog.strategyErrorMembers')}</span>}
          {membersDuplicated && <span className="codex-strategy-field-error" role="alert">
            {t('codex.proxy.catalog.strategyErrorDuplicateName')}</span>}
          {!!initial.kept.length && <>
            <p className="codex-strategy-warning" role="status">
              <TriangleAlert size={13} aria-hidden="true" />{t('codex.proxy.catalog.strategyOriginKept', { count: initial.kept.length })}</p>
            <p className="codex-proxy-page-note">{initial.kept.map((entry) => entry.name).join(' · ')}</p>
          </>}
          {!!initial.unmatched.length && <p className="codex-strategy-warning" role="status">
            <TriangleAlert size={13} aria-hidden="true" />{t('codex.proxy.catalog.strategyUnmatched', { count: initial.unmatched.length })}</p>}
        </div>
        {/* Only the settings this kind applies are offered: manual selection runs no health check. */}
        {optionFields.length > 0 && <details className="codex-proxy-page-details codex-strategy-advanced" open={advanced} onToggle={(event) => setAdvanced(event.currentTarget.open)}>
          <summary>{t('common.advancedSettings')}</summary>
          <p className="codex-proxy-page-note">{t('codex.proxy.catalog.strategyAdvancedHint')}</p>
          <div className="codex-strategy-fields">
            {ADVANCED_FIELDS.filter((entry) => optionFields.includes(entry.field)).map((entry) => <label className="codex-strategy-field" key={entry.field}>
              <span>{t(entry.labelKey)}</span>
              {entry.field === 'url'
                ? <input className="codex-strategy-input" value={form.url} placeholder={PROXY_STRATEGY_DEFAULTS.url} disabled={saving}
                  autoComplete="off" spellCheck={false} aria-invalid={!!optionErrors.url}
                  onChange={(event) => { setForm((old) => ({ ...old, url: event.target.value })); edit(); }} />
                : <input className="codex-strategy-input" value={form[entry.field]} inputMode="numeric" disabled={saving} autoComplete="off" spellCheck={false}
                  placeholder={String(PROXY_STRATEGY_DEFAULTS[entry.field])} aria-invalid={!!optionErrors[entry.field]}
                  onChange={(event) => { setForm((old) => ({ ...old, [entry.field]: event.target.value })); edit(); }} />}
              {optionErrors[entry.field] && <span className="codex-strategy-field-error" role="alert">{fieldError(entry.field)}</span>}
            </label>)}
          </div>
          <div className="codex-strategy-lazy">
            <button type="button" role="switch" aria-checked={form.lazy} disabled={saving}
              className={'codex-strategy-switch' + (form.lazy ? ' on' : '')} aria-label={t('codex.proxy.catalog.strategyLazy')}
              onClick={() => { setForm((old) => ({ ...old, lazy: !old.lazy })); edit(); }}><span /></button>
            <span>{t('codex.proxy.catalog.strategyLazy')}</span>
          </div>
        </details>}
        <div className="codex-strategy-summary" role="group" aria-label={t('codex.proxy.catalog.strategySummary')}>
          <p className="codex-strategy-summary-title">{t('codex.proxy.catalog.strategySummary')}</p>
          <p><span>{t('codex.proxy.catalog.strategySummaryType')}</span><strong>{t(strategyKindKey(kind))}</strong></p>
          <p><span>{t('codex.proxy.catalog.strategySummaryOrder')}</span><strong>{members.length
            ? members.map((member, index) => `${index + 1}. ${member.name}`).join(' → ')
            : t('codex.proxy.catalog.strategyNoMembers')}</strong></p>
          <p className={'codex-strategy-notice' + (members.length <= 1 ? ' is-strong' : '')} role="note">{t(strategyNoticeKey(kind, members.length))}</p>
        </div>
      </div>
      <footer className="modal-footer">
        <button type="button" className="btn btn-secondary" disabled={saving} onClick={onClose}>{t('common.cancel')}</button>
        <button type="button" className="btn btn-primary" disabled={saving || busy} onClick={() => void submit()}>
          {saving ? <RefreshCw size={15} className="loading-spinner" /> : <Check size={15} />}
          {t(saving ? 'common.processing' : 'common.save')}</button>
      </footer>
    </div>
  </div>, document.body);
}

/** Deletion reuses the source dependency preview: the dialog names the accounts that lose the proxy. */
export function StrategyDeleteDialog({ source, busy, onClose, onRemoved, onBindingsChanged = refreshCodexProxyAccounts }: {
  source: ProxyCatalogSource;
  busy: boolean;
  onClose: () => void;
  onRemoved: (next: ProxyCatalog) => void;
  onBindingsChanged?: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const dialog = useRef<HTMLDivElement>(null);
  const mounted = useRef(true);
  const [impact, setImpact] = useState<ProxyCatalogDependencies | null>(null);
  const resolveName = useCodexProxyAccountName();
  const [impactError, setImpactError] = useState('');
  const [loading, setLoading] = useState(true);
  const [attempt, setAttempt] = useState(0);
  const [removing, setRemoving] = useState(false);
  const [error, setError] = useState('');
  const removalProgress = useRef<ProxyRemovalProgress>({});
  const close = () => {
    if (removalProgress.current.catalog) onRemoved(removalProgress.current.catalog);
    else onClose();
  };

  useEscCloseTopmost(true, () => { if (!removing) close(); });
  useModalScrollLock(true);
  useModalFocusTrap(dialog, true);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);
  useEffect(() => {
    let cancelled = false;
    setImpact(null);
    setImpactError('');
    setLoading(true);
    void getProxyCatalogDependencies(source.id).then((next) => {
      if (!cancelled && mounted.current) setImpact(next);
    }).catch(() => {
      if (!cancelled && mounted.current) setImpactError(t('codex.proxy.catalog.deleteImpactLoadFailed'));
    }).finally(() => {
      if (!cancelled && mounted.current) setLoading(false);
    });
    return () => { cancelled = true; };
    // The preview is re-read on retry, never on a language change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [source.id, attempt]);

  const confirm = async () => {
    if (removing || busy || !impact) return;
    setRemoving(true);
    setError('');
    try {
      const next = await removeProxySourceWithRefresh(removalProgress.current,
        () => removeProxyStrategy(source.id), onBindingsChanged);
      if (mounted.current) onRemoved(next);
    } catch (caught) {
      if (mounted.current) {
        setError(t(proxyRemovalErrorKey(caught, strategyErrorKey)));
        if (!removalProgress.current.catalog) setAttempt((value) => value + 1);
      }
    } finally {
      if (mounted.current) setRemoving(false);
    }
  };

  return createPortal(<div className="modal-overlay codex-strategy-overlay">
    <div className="modal codex-strategy-confirm" ref={dialog} role="alertdialog" aria-modal="true"
      aria-labelledby="codex-strategy-delete-title" tabIndex={-1}>
      <header className="modal-header">
        <h2 id="codex-strategy-delete-title">{t('common.confirmDelete')}</h2>
        <button type="button" className="btn btn-secondary compact" disabled={removing} aria-label={t('common.close')} onClick={close}><X size={16} /></button>
      </header>
      <div className="modal-body codex-strategy-dialog-body">
        <ModalErrorMessage message={error} />
        <strong className="codex-strategy-delete-name">{source.name}</strong>
        <div className="codex-strategy-delete-impact" aria-live="polite">
          <h3>{t('codex.proxy.catalog.deleteImpactTitle')}</h3>
          {loading && <p className="codex-proxy-page-note" role="status">{t('common.loading')}</p>}
          {!loading && impactError && <div className="codex-strategy-impact-error" role="alert">
            <span>{impactError}</span>
            <button type="button" className="btn btn-secondary compact" disabled={removing} onClick={() => setAttempt((value) => value + 1)}>{t('common.retry')}</button>
          </div>}
          {!loading && impact && <>
            <p className="codex-strategy-delete-count">{t('codex.proxy.catalog.deleteImpactAccounts', { count: impact.accountCount })}</p>
            {impact.accounts.length === 0
              ? <p className="codex-proxy-page-note">{t('codex.proxy.catalog.deleteImpactNone')}</p>
              : <ul className="codex-strategy-delete-accounts">{impact.accounts.map((entry) => <li key={entry.id}>
                <strong>{resolveName({ id: entry.id, account_name: entry.name })}</strong>{entry.email && entry.email !== entry.name && <span>{resolveName({ id: entry.id, email: entry.email })}</span>}
              </li>)}</ul>}
            <p className="codex-proxy-page-note">{impact.unifiedProxy ? t('codex.proxy.catalog.deleteImpactUnifiedClose') : t('codex.proxy.catalog.deleteImpactUnifiedKept')}</p>
          </>}
        </div>
        <p className="codex-proxy-page-note">{t('codex.proxy.catalog.strategyDeleteNotice')}</p>
      </div>
      <footer className="modal-footer">
        <button type="button" className="btn btn-secondary" disabled={removing} onClick={close}>{t('common.cancel')}</button>
        <button type="button" className="btn btn-danger" disabled={removing || busy || !impact} onClick={() => void confirm()}>
          {removing ? <RefreshCw size={15} className="loading-spinner" /> : <Trash2 size={15} />}
          {t(removing ? 'common.processing' : removalProgress.current.catalog ? 'common.retry' : 'common.confirmDelete')}</button>
      </footer>
    </div>
  </div>, document.body);
}
