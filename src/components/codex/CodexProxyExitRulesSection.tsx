import { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ShieldCheck } from 'lucide-react';
import type { CodexAccount } from '../../types/codex';
import { canUseCodexAccountProxy } from '../../utils/codexAccountProxy';
import type { ProxyBindingValue } from '../../utils/codexProxyBatch';
import {
  codexProxyExitModeKey, resolveExitMode, unifiedFollowingIds,
} from '../../utils/codexProxyDraft';
import type { CodexUnifiedProxyView } from '../../services/codexUnifiedProxyService';
import { CodexUnifiedProxyPanel } from './CodexUnifiedProxyPanel';
import { useCodexProxyAccountName } from './useCodexProxyExitEditor';
import { useCodexProxyWorkspace } from './CodexProxyWorkspaceContext';
import '../../styles/pages/codex-proxy-accounts.css';

const EMPTY_CELL = '—';

/**
 * Read-only resolution order per account plus the shared exit. The table never writes; every row
 * jumps to the account that owns the binding.
 */
export function CodexProxyExitRulesSection() {
  const { t } = useTranslation();
  const { accounts, catalog, catalogLoading, catalogError, reloadCatalog, acceptCatalog, unified, unifiedErrorKey, reloadUnified, selectAccount, goSection } = useCodexProxyWorkspace();
  const resolveName = useCodexProxyAccountName();
  const eligible = useMemo(() => accounts.filter(canUseCodexAccountProxy), [accounts]);
  const [view, setView] = useState<CodexUnifiedProxyView | null>(unified);

  useEffect(() => { setView(unified); }, [unified]);

  const saved = useCallback((account: CodexAccount): ProxyBindingValue => account.egress_proxy ?? null, []);
  const following = useMemo(() => new Set(unifiedFollowingIds(eligible, saved, unified)), [eligible, saved, unified]);
  const state = { catalog, loading: catalogLoading, failed: Boolean(catalogError) };
  /** Source and node come from the catalog when it still resolves; a stale reference is labelled. */
  const cells = (binding: ProxyBindingValue) => {
    if (!binding) return { source: EMPTY_CELL, node: EMPTY_CELL };
    const owner = binding.sourceId ? catalog.sources.find((entry) => entry.id === binding.sourceId) : undefined;
    const item = owner ? [...owner.nodes, ...owner.groups].find((entry) => entry.id === binding.itemId) : undefined;
    const source = owner?.name ?? (binding.sourceId ? t('codex.proxy.modeStale') : binding.sourceName?.trim() || binding.protocol.toUpperCase());
    const node = [item?.name ?? binding.name?.trim(), binding.selectedName?.trim()].filter(Boolean).join(' · ');
    return { source, node: node || EMPTY_CELL };
  };
  const openAccount = (id: string) => { selectAccount(id); goSection('accounts'); };

  return <section className="codex-proxy-rules" aria-label={t('codex.proxy.rules.title')}>
    <header className="codex-proxy-rules-head">
      <p>{t('codex.proxy.rules.intro')}</p>
    </header>
    <div className="codex-proxy-rules-order" role="note" aria-label={t('codex.proxy.rules.order')}>
      <span className="codex-proxy-rules-order-label">{t('codex.proxy.rules.order')}</span>
      <ol className="codex-proxy-rules-order-list">
        <li className="is-independent">{t('codex.proxy.modeIndependent')}</li>
        <li className="is-unified">{t('codex.proxy.modeUnified')}</li>
        <li className="is-default">{t('codex.proxy.modeDefault')}</li>
      </ol>
    </div>
    {catalogError && <div className="codex-proxy-rules-failure" role="alert">
      <div><strong>{t('codex.proxy.rules.loadFailed')}</strong><p>{catalogError}</p></div>
      <div className="codex-proxy-rules-failure-actions">
        <small>{t('codex.proxy.rules.loadFailedHint')}</small>
        <button type="button" className="btn btn-secondary compact" onClick={() => reloadCatalog()}>{t('common.retry')}</button></div></div>}
    {eligible.length === 0 ? <div className="codex-proxy-rules-empty">
      <div className="codex-proxy-rules-empty-icon"><ShieldCheck size={26} /></div>
      <h3>{t('codex.proxy.rules.empty')}</h3><p>{t('codex.proxy.rules.emptyHint')}</p>
    </div> : <>
      <section className="codex-proxy-rules-table-card" aria-labelledby="codex-proxy-rules-table-title">
        <div className="codex-proxy-page-section-heading"><h3 id="codex-proxy-rules-table-title">{t('codex.proxy.rules.tableTitle')}</h3>
          {catalogLoading && <span className="codex-proxy-rules-loading" role="status">{t('common.loading')}</span>}</div>
        <div className="codex-proxy-rules-table-wrap">
          <table className="codex-proxy-rules-table">
            <thead><tr>
              <th scope="col">{t('codex.proxy.rules.columnAccount')}</th>
              <th scope="col">{t('codex.proxy.rules.columnMode')}</th>
              <th scope="col">{t('codex.proxy.rules.columnSource')}</th>
              <th scope="col">{t('codex.proxy.rules.columnNode')}</th>
              <th scope="col">{t('codex.proxy.rules.columnFollow')}</th>
            </tr></thead>
            <tbody>{eligible.map((entry) => {
              const binding = saved(entry);
              const mode = resolveExitMode(binding, state, following.has(entry.id));
              const cell = cells(binding);
              const follows = following.has(entry.id);
              return <tr key={entry.id} onClick={() => openAccount(entry.id)}>
                <th scope="row" title={resolveName(entry)}>
                  <button type="button" className="btn btn-secondary compact codex-proxy-rules-open" onClick={() => openAccount(entry.id)}>
                    {resolveName(entry)}</button>
                </th>
                <td><span className={`codex-proxy-page-state is-${mode}`}><span className="codex-proxy-page-state-dot" />{t(codexProxyExitModeKey(mode))}</span></td>
                <td title={cell.source}>{cell.source}</td>
                <td title={cell.node}>{cell.node}</td>
                <td className={follows ? 'is-following' : undefined}>{t(follows ? 'codex.proxy.rules.followYes' : 'codex.proxy.rules.followNo')}</td>
              </tr>;
            })}</tbody>
          </table>
        </div>
      </section>
      <section className="codex-proxy-rules-unified" aria-labelledby="codex-proxy-rules-unified-title">
        <div className="codex-proxy-page-section-heading"><div className="codex-proxy-section-label">
          <ShieldCheck size={18} aria-hidden="true" /><h3 id="codex-proxy-rules-unified-title">{t('codex.proxy.unified.title')}</h3></div></div>
        <p className="codex-proxy-page-note">{t('codex.proxy.unified.hint')}</p>
        {unifiedErrorKey && <div className="codex-proxy-page-error" role="alert">{t(unifiedErrorKey)}
          <button type="button" className="btn btn-secondary compact" onClick={() => reloadUnified()}>{t('common.retry')}</button></div>}
        {!unifiedErrorKey && <CodexUnifiedProxyPanel totalAccounts={eligible.length} catalog={catalog} view={view} onCatalogChange={acceptCatalog}
          onViewChange={(next) => { setView(next); reloadUnified(); }} onGoResources={() => goSection('resources')} />}
        <p className="codex-proxy-page-note">{t('codex.proxy.unified.restartHint')}</p>
      </section>
    </>}
  </section>;
}
