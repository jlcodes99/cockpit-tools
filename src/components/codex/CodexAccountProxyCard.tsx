import { ChevronRight, RefreshCw, Shield, ShieldCheck, ShieldOff } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { CodexAccount } from '../../types/codex';
import { useCodexProxyDisplay } from '../../hooks/useCodexProxyDisplay';
import { canUseCodexAccountProxy, requestCodexAccountProxy } from '../../utils/codexAccountProxy';
import { codexProxyCardPresentation } from '../../utils/codexProxyDisplay';
import { proxySummary } from '../../utils/codexProxyPresentation';
import { proxyRuntimeLabelKey } from '../../utils/codexProxyPreview';
import { useCodexProxyCardData } from './useCodexProxyCardData';
import '../../styles/pages/codex-proxy-display.css';

export function CodexAccountProxyCard({ account, placement }: {
  account: CodexAccount; placement: 'summary' | 'detailed';
}) {
  const display = useCodexProxyDisplay();
  if (display !== placement || !canUseCodexAccountProxy(account)) return null;
  return <ProxyCard key={account.id} account={account} detailed={display === 'detailed'} />;
}

function ProxyCard({ account, detailed }: { account: CodexAccount; detailed: boolean }) {
  const { t } = useTranslation();
  const data = useCodexProxyCardData(account, detailed);
  const { saved, modeKey, nodes, rows } = codexProxyCardPresentation(account, data.status);
  const Icon = account.egress_proxy_disabled ? ShieldOff : saved ? ShieldCheck : Shield;
  const configured = saved ? saved.selectedName || saved.name || proxySummary(saved) : '';
  const latest = data.request;
  const statusErrorKey = data.statusErrorKind === 'busy' ? 'codex.proxy.runtimeBusy'
    : data.statusErrorKind === 'capacity' ? 'codex.proxy.runtimeCapacity' : 'codex.proxy.runtimeReadFailed';
  const latency = (delay?: number | null, checked?: number | null) => delay != null && checked != null
    ? `${delay} ms` : t(checked != null ? 'common.failed' : 'codex.proxy.latencyPending');
  return <div ref={data.ref} className={`codex-account-proxy-card is-${detailed ? 'detailed' : 'summary'}`}>
    <button type="button" className="btn codex-account-proxy-card-open" aria-haspopup="dialog"
      title={t('codex.proxy.previewTitle')} onClick={(event) => {
        event.stopPropagation(); requestCodexAccountProxy(account.id);
      }}>
      <span className="codex-account-proxy-card-emblem"><Icon size={16} aria-hidden="true" /></span>
      <span className="codex-account-proxy-card-heading">
        <span className="codex-account-proxy-card-mode" title={t(modeKey)}>{t(modeKey)}</span>
        {detailed && saved?.sourceName && <small title={`${t('codex.proxy.catalog.sources')}: ${saved.sourceName}`}>{saved.sourceName}</small>}
      </span>
      <ChevronRight className="codex-account-proxy-card-chevron" size={15} aria-hidden="true" />
      <span className="codex-account-proxy-card-nodes">
        {nodes.length > 0 ? nodes.map((node) => <span className="codex-account-proxy-card-node" key={node.name}>
          <span className="codex-account-proxy-card-node-label" title={`${t('codex.proxy.currentNode')}: ${node.name}`}>
            {detailed && <small>{t('codex.proxy.currentNode')}</small>}<strong>{node.name}</strong>
          </span>
          <span className={`codex-account-proxy-card-latency${node.selection?.delayMs != null && node.selection?.checkedAt != null ? ' is-measured' : ''}`} title={node.selection?.checkedAt != null
            ? `${t('codex.proxy.latencyCheckedAt')}: ${new Date(node.selection.checkedAt).toLocaleString()}`
            : t('codex.proxy.latencyPending')}>
            {latency(node.selection?.delayMs, node.selection?.checkedAt)}
          </span>
          {detailed && nodes.length > 1 && <small>{node.kinds.map((kind) => t(proxyRuntimeLabelKey(kind))).join(' · ')}</small>}
          {detailed && node.selection?.checkedAt != null && <small className="codex-account-proxy-card-checked"
            title={`${t('codex.proxy.latencyCheckedAt')}: ${new Date(node.selection.checkedAt).toLocaleString()}`}>
            {t('codex.proxy.latencyCheckedAt')}
            <time dateTime={new Date(node.selection.checkedAt).toISOString()}>
              {new Date(node.selection.checkedAt).toLocaleString(undefined, { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' })}
            </time>
          </small>}
        </span>) : configured ? <span className="codex-account-proxy-card-node">
          <span className="codex-account-proxy-card-node-label" title={configured}>
            <small>{t('codex.proxy.display.configuredNode')}</small><strong>{configured}</strong>
          </span>
          <span className="codex-account-proxy-card-latency">{t('codex.proxy.latencyPending')}</span>
        </span> : !account.egress_proxy_disabled && <small>{t(data.statusError ? 'codex.proxy.runtimeUnavailable'
          : !data.status ? 'codex.proxy.runtimeLoading' : 'codex.proxy.noCurrentNode')}</small>}
      </span>
      {detailed && <>
        <span className="codex-account-proxy-card-details">
          {rows.map((row) => <span className="codex-account-proxy-card-channel" key={row.kind}>
            <span>{t(proxyRuntimeLabelKey(row.kind))}</span>
            <strong className={`is-${data.statusError ? 'unknown' : row.state ?? 'unknown'}`}>
              <i aria-hidden="true" />{t(data.statusError ? 'codex.proxy.runtimeUnavailable' : row.state ? `codex.proxy.runtime_${row.state}` : 'codex.proxy.runtimeLoading')}
            </strong>
          </span>)}
        </span>
        <span className="codex-account-proxy-card-request">
          <span className="codex-account-proxy-card-request-heading">
            <span>{t('codex.proxy.display.lastRequest')}</span>
            {latest ? <strong className={`codex-account-proxy-card-result ${latest.success ? 'is-success' : 'is-failure'}`}>
              {t(latest.success ? 'codex.proxy.recentSuccess' : 'codex.proxy.recentFailure')}{latest.httpStatus ? ` · HTTP ${latest.httpStatus}` : ''}
            </strong> : <small>{t(data.requestsError ? 'codex.proxy.recentFailed' : data.loading ? 'common.loading' : 'codex.proxy.recentEmpty')}</small>}
          </span>
          {latest && <span className="codex-account-proxy-card-request-meta">
            <span title={latest.modelId}>{latest.modelId || t('codex.proxy.recentUnknownModel')}</span>
            <time dateTime={new Date(latest.timestamp).toISOString()} title={new Date(latest.timestamp).toLocaleString()}>
              {new Date(latest.timestamp).toLocaleString(undefined, { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' })}
            </time>
          </span>}
        </span>
      </>}
    </button>
    {(data.statusError || data.requestsError) && <div className={`codex-account-proxy-card-error${data.statusErrorKind === 'busy' ? ' is-busy' : ''}`} role="status">
      <span>{t(data.statusError ? statusErrorKey : 'codex.proxy.recentFailed')}</span>
      <button type="button" className="btn btn-secondary" disabled={data.loading} aria-label={t('codex.proxy.display.refresh')}
        onClick={(event) => { event.stopPropagation(); data.refresh(); }}><RefreshCw size={12} />{t('common.retry')}</button>
    </div>}
  </div>;
}
