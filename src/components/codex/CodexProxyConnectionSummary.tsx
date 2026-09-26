import type { Ref } from 'react';
import { ArrowLeftRight, Server } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { CodexProxyRuntimeStatus } from '../../services/codexAccountProxyService';
import { getCodexPlanBadgePresentation, type CodexAccount } from '../../types/codex';
import { proxyPreviewBinding, proxyRuntimeRows, proxyRuntimeLabelKey } from '../../utils/codexProxyPreview';
import { proxySummary } from '../../utils/codexProxyPresentation';
import { withCodexPlanBadgeStyle } from '../../utils/codexPreferences';

export function CodexProxyConnectionSummary({ account, status, failed, onSwitch, switchButtonRef }: {
  account: CodexAccount;
  status: CodexProxyRuntimeStatus | null;
  failed: boolean;
  onSwitch?: () => void;
  switchButtonRef?: Ref<HTMLButtonElement>;
}) {
  const { t } = useTranslation();
  const binding = proxyPreviewBinding(account.egress_proxy, status);
  const saved = binding.summary;
  // A saved automatic group is a policy, never evidence of its current leaf.
  const liveRows = proxyRuntimeRows(status).filter((row) => (row.kernelState ?? row.state) === 'running' && row.node);
  const nodes = [...new Set(liveRows.map((row) => row.node!))].map((name) => {
    const rows = liveRows.filter((row) => row.node === name);
    const selection = rows.flatMap((row) => row.selection ? [row.selection] : [])
      .sort((a, b) => (b.checkedAt ?? 0) - (a.checkedAt ?? 0))[0];
    return { name, rows, selection };
  });
  const rawPlan = account.plan_type?.trim();
  const planClass = rawPlan ? withCodexPlanBadgeStyle(getCodexPlanBadgePresentation(account).className) : '';
  const sourceKey = binding.source === 'account' ? 'codex.proxy.modeIndependent' : binding.source === 'unified'
    ? 'codex.proxy.modeUnified' : binding.source === 'unknown'
      ? failed ? 'codex.proxy.runtimeUnavailable' : 'codex.proxy.runtimeLoading' : 'codex.proxy.filter_unbound';
  return <section className="codex-proxy-preview-connection" aria-label={t('codex.proxy.connectionTitle')}>
    <div className="codex-proxy-preview-node">
      <div className="codex-proxy-preview-eyebrow"><Server size={16} />
        <span className={`codex-proxy-preview-badge${saved ? ' is-bound' : ''}`}>{t(sourceKey)}</span>
        {rawPlan && <span className={`tier-badge ${planClass}`} title={rawPlan}>{rawPlan}</span>}
      </div>
      <h3>{saved?.name || (saved ? proxySummary(saved) : t(binding.source === 'none' ? 'codex.proxy.unboundHint' : sourceKey))}</h3>
      <div className="codex-proxy-preview-metadata">
        {saved?.sourceName && <span>{t('codex.proxy.catalog.sources')}<strong>{saved.sourceName}</strong></span>}
        {saved && !['catalog', 'resource'].includes(saved.protocol.toLowerCase()) && <span>{t('codex.proxy.protocol')}<strong>{saved.protocol.toUpperCase()}</strong></span>}
        {saved?.server && <span>{t('codex.proxy.host')}<strong>{saved.server}{saved.port ? `:${saved.port}` : ''}</strong></span>}
      </div>
      {binding.source === 'unified' && <p>{t('codex.proxy.modeUnifiedHint')}</p>}
      {binding.source === 'none' && <p>{t('codex.proxy.managementHint')}</p>}
    </div>
    <div className="codex-proxy-preview-current">
      <div className="codex-proxy-preview-current-content">
        <span className="codex-proxy-preview-current-label">{t('codex.proxy.currentNode')}</span>
        {nodes.length ? nodes.map(({ name, rows, selection }) => <div className="codex-proxy-preview-current-node" key={name}>
          <strong>{name}</strong>
          <span className="codex-proxy-preview-current-latency" title={selection?.checkedAt != null
            ? `${t('codex.proxy.latencyCheckedAt')}: ${new Date(selection.checkedAt).toLocaleString()}` : undefined}>
            {selection?.delayMs != null ? `${selection.delayMs} ms`
              : t(selection?.checkedAt != null ? 'common.failed' : 'codex.proxy.latencyPending')}
          </span>
          {nodes.length > 1 && <small>{rows.map((row) => t(proxyRuntimeLabelKey(row.kind))).join(' · ')}</small>}
        </div>) : <span className="codex-proxy-preview-current-empty">{t(failed ? 'codex.proxy.runtimeUnavailable'
          : !status ? 'codex.proxy.runtimeLoading' : 'codex.proxy.noCurrentNode')}</span>}
      </div>
      {onSwitch && <button ref={switchButtonRef} type="button" className="btn btn-secondary" aria-haspopup="dialog" onClick={onSwitch}>
        <ArrowLeftRight size={14} />{t('codex.proxy.quickSwitch')}
      </button>}
    </div>
  </section>;
}
