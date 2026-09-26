import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { CheckSquare, Search, ShieldCheck, Square, X } from 'lucide-react';
import type { CodexAccount } from '../../types/codex';
import { getCodexPlanBadgePresentation } from '../../types/codex';
import { CODEX_PLAN_BADGE_STYLE_CHANGED_EVENT, getCodexPlanBadgeStyle, withCodexPlanBadgeStyle } from '../../utils/codexPreferences';
import { proxySummary } from '../../utils/codexProxyPresentation';
import { canUseCodexAccountProxy } from '../../utils/codexAccountProxy';
import type { ProxyBindingValue } from '../../utils/codexProxyBatch';
import { filterProxyAccounts, resolveExitMode, unifiedFollowingIds, type CodexProxyExitFilter } from '../../utils/codexProxyDraft';
import { useCodexAccountStore } from '../../stores/useCodexAccountStore';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import { CodexProxyBatchBindDialog } from './CodexProxyBatchBindDialog';
import { CodexProxyAccountDialog, CodexProxyFollowDialog } from './CodexProxyAccountDialog';
import { useCodexProxyAccountName } from './useCodexProxyExitEditor';
import { useCodexProxyWorkspace } from './CodexProxyWorkspaceContext';
import '../../styles/pages/codex-proxy-accounts.css';

/** Full-width assignments. Account editors and runtime polling exist only while requested. */
export function CodexProxyAccountsSection() {
  const { t } = useTranslation();
  const { accounts, selectedId, entryAccountId, selectAccount, catalog, catalogLoading, catalogError, acceptCatalog, unified, reloadUnified } = useCodexProxyWorkspace();
  const resolveName = useCodexProxyAccountName();
  const eligible = useMemo(() => accounts.filter(canUseCodexAccountProxy), [accounts]);
  const [applied, setApplied] = useState<Record<string, ProxyBindingValue>>({});
  const [search, setSearch] = useState('');
  const [filter, setFilter] = useState<CodexProxyExitFilter>('all');
  const [planBadgeStyle, setPlanBadgeStyle] = useState(getCodexPlanBadgeStyle);
  const [selection, setSelection] = useState<string[]>([]);
  const [batchOpen, setBatchOpen] = useState(false);
  const [followOpen, setFollowOpen] = useState(false);
  const entryRow = useRef<HTMLTableRowElement>(null);
  const [dialog, setDialog] = useState<{ id: string; tab: 'edit' | 'details' } | null>(null);

  useEffect(() => {
    const update = () => setPlanBadgeStyle(getCodexPlanBadgeStyle());
    window.addEventListener(CODEX_PLAN_BADGE_STYLE_CHANGED_EVENT, update);
    return () => window.removeEventListener(CODEX_PLAN_BADGE_STYLE_CHANGED_EVENT, update);
  }, []);
  const savedValue = useCallback((account: CodexAccount): ProxyBindingValue =>
    Object.prototype.hasOwnProperty.call(applied, account.id) ? applied[account.id] : account.egress_proxy ?? null, [applied]);
  const visible = useMemo(() => filterProxyAccounts(eligible, { search, filter, saved: savedValue }), [eligible, filter, savedValue, search]);
  const following = useMemo(() => new Set(unifiedFollowingIds(eligible, savedValue, unified)), [eligible, savedValue, unified]);
  const pickedAccounts = useMemo(() => eligible.filter((entry) => selection.includes(entry.id)), [selection, eligible]);
  const allVisibleSelected = visible.length > 0 && visible.every((entry) => selection.includes(entry.id));
  const unifiedLabel = unified?.binding ? [unified.binding.sourceName, unified.binding.name, unified.binding.selectedName].filter(Boolean).join(' · ') : '';
  const recordApplied = useCallback((account: CodexAccount) => {
    useCodexAccountStore.getState().applyAccountSnapshot(account);
    setApplied((old) => ({ ...old, [account.id]: account.egress_proxy ?? null }));
  }, []);
  useEffect(() => {
    if (!entryAccountId) return;
    entryRow.current?.scrollIntoView({ block: 'nearest' });
  }, [entryAccountId, eligible.length]);
  const toggle = (id: string) => setSelection((old) => old.includes(id) ? old.filter((entry) => entry !== id) : [...old, id]);
  const toggleAll = () => {
    const ids = visible.map((entry) => entry.id);
    setSelection((old) => ids.every((id) => old.includes(id)) ? old.filter((id) => !ids.includes(id)) : [...new Set([...old, ...ids])]);
  };
  const open = (id: string, tab: 'edit' | 'details') => { selectAccount(id); setDialog({ id, tab }); };

  return <section className="codex-proxy-accounts" aria-label={t('codex.proxy.accounts.title')}>
    <div className="codex-proxy-accounts-toolbar">
      <label className="codex-proxy-search"><Search size={16} /><input value={search} onChange={(event) => setSearch(event.target.value)}
        placeholder={t('codex.proxy.search')} aria-label={t('codex.proxy.search')} /></label>
      <SingleSelectDropdown value={filter} onChange={(next) => setFilter(next as CodexProxyExitFilter)} ariaLabel={t('codex.proxy.filter')}
        options={(['all', 'bound', 'unbound'] as const).map((value) => ({ value, label: t(value === 'all' ? 'codex.proxy.filter_all'
          : value === 'bound' ? 'codex.proxy.modeIndependent' : 'codex.proxy.managerAccounts.following') }))} />
      <span className="codex-proxy-accounts-count">{visible.length} / {eligible.length}</span>
    </div>
    {pickedAccounts.length > 0 && <div className="codex-proxy-accounts-selection" role="region" aria-label={t('codex.proxy.batchSelect')}>
      <strong>{t('codex.proxy.batchSelectedCount', { count: pickedAccounts.length })}</strong>
      <button type="button" className="btn btn-primary compact" onClick={() => setBatchOpen(true)}>{t('codex.proxy.batchBind')}</button>
      <button type="button" className="btn btn-secondary compact" disabled={!pickedAccounts.some((entry) => savedValue(entry))}
        onClick={() => setFollowOpen(true)}>{t('codex.proxy.managerAccounts.follow')}</button>
      <button type="button" className="btn btn-secondary compact codex-proxy-accounts-clear" onClick={() => setSelection([])} aria-label={t('codex.proxy.batchClearAll')}><X size={15} /></button>
    </div>}
    {eligible.length === 0 ? <div className="codex-proxy-accounts-empty">
      <div className="codex-proxy-accounts-empty-icon"><ShieldCheck size={26} /></div>
      <h3>{t('codex.proxy.emptyTitle')}</h3><p>{t('codex.proxy.emptyDescription')}</p>
    </div> : <div className="codex-proxy-accounts-table-wrap">
      <table className="codex-proxy-accounts-table">
        <thead><tr>
          <th className="codex-proxy-accounts-check-cell"><button type="button" role="checkbox" aria-checked={allVisibleSelected ? true : visible.some((entry) => selection.includes(entry.id)) ? 'mixed' : false}
            className="btn btn-secondary compact codex-proxy-accounts-check" aria-label={t('common.selectAll')} disabled={visible.length === 0} onClick={toggleAll}>
            {allVisibleSelected ? <CheckSquare size={17} /> : <Square size={17} />}</button></th>
          <th>{t('codex.proxy.accountsTitle')}</th><th>{t('codex.proxy.managerAccounts.mode')}</th>
          <th>{t('codex.proxy.managerAccounts.proxy')}</th><th><span className="codex-proxy-accounts-sr-only">{t('common.shared.columns.actions')}</span></th>
        </tr></thead>
        <tbody>{visible.map((entry) => {
          const binding = savedValue(entry);
          const mode = resolveExitMode(binding, { catalog, loading: catalogLoading, failed: Boolean(catalogError) }, following.has(entry.id));
          const proxy = binding ? proxySummary(binding) : following.has(entry.id) ? unifiedLabel : t('codex.proxy.modeDefault');
          const plan = entry.plan_type?.trim();
          const planClass = plan ? withCodexPlanBadgeStyle(getCodexPlanBadgePresentation(entry).className, planBadgeStyle) : '';
          return <tr key={entry.id} ref={entry.id === entryAccountId ? entryRow : undefined} className={selection.includes(entry.id) ? 'is-selected' : entry.id === selectedId ? 'is-current' : undefined}>
            <td className="codex-proxy-accounts-check-cell"><button type="button" role="checkbox" aria-checked={selection.includes(entry.id)}
              className="btn btn-secondary compact codex-proxy-accounts-check" onClick={() => toggle(entry.id)} aria-label={resolveName(entry)}>
              {selection.includes(entry.id) ? <CheckSquare size={17} /> : <Square size={17} />}</button></td>
            <th scope="row"><div className="codex-proxy-accounts-identity"><span className="codex-proxy-accounts-avatar">{resolveName(entry).slice(0, 1).toUpperCase()}</span>
              <span className="codex-proxy-accounts-name" title={resolveName(entry)}>{resolveName(entry)}</span>
              {plan && <span className={`tier-badge ${planClass}`} title={plan}>{plan}</span>}</div></th>
            <td><span className={`codex-proxy-accounts-mode is-${mode}`}>{t(binding ? 'codex.proxy.modeIndependent' : 'codex.proxy.managerAccounts.following')}</span></td>
            <td><div className="codex-proxy-accounts-proxy"><span title={proxy}>{proxy}</span>
              {mode === 'stale' && <small>{t('codex.proxy.modeStale')}</small>}</div></td>
            <td><div className="codex-proxy-accounts-row-actions">
              <button type="button" className="btn btn-secondary compact" onClick={() => open(entry.id, 'edit')}>{t('codex.proxy.managerAccounts.change')}</button>
              <button type="button" className="btn btn-secondary compact" onClick={() => open(entry.id, 'details')}>{t('codex.proxy.managerAccounts.details')}</button>
            </div></td>
          </tr>;
        })}</tbody>
      </table>
      {visible.length === 0 && <div className="codex-proxy-accounts-empty-result"><p>{t('codex.proxy.noMatches')}</p>
        <button type="button" className="btn btn-secondary compact" onClick={() => { setSearch(''); setFilter('all'); }}>{t('codex.proxy.accounts.resetFilters')}</button></div>}
    </div>}
    <p className="codex-proxy-accounts-scope">{t('codex.proxy.managerAccounts.scope')}
      {accounts.length > eligible.length && <> {t('codex.proxy.unified.unsupported', { count: accounts.length - eligible.length })}</>}</p>
    {dialog && eligible.some((entry) => entry.id === dialog.id) && <CodexProxyAccountDialog key={dialog.id} accountId={dialog.id} initialTab={dialog.tab}
      onClose={() => setDialog(null)} onApplied={(account) => { setApplied((old) => ({ ...old, [account.id]: account.egress_proxy ?? null })); reloadUnified(); }} />}
    {batchOpen && <CodexProxyBatchBindDialog accounts={pickedAccounts} catalog={catalog} resolveDisplayName={resolveName} savedValue={savedValue} onCatalogChange={acceptCatalog}
      onClose={() => { setBatchOpen(false); reloadUnified(); }} onApplied={recordApplied} />}
    {followOpen && <CodexProxyFollowDialog accounts={pickedAccounts.filter((entry) => savedValue(entry))} onApplied={recordApplied}
      onClose={() => { setFollowOpen(false); reloadUnified(); }} />}
  </section>;
}
