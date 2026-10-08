import type { Ref } from 'react';
import { ArrowLeftRight, Gauge, RefreshCw, Server } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { CodexProxyRuntimeStatus } from '../../services/codexAccountProxyService';
import { getCodexPlanBadgePresentation, type CodexAccount } from '../../types/codex';
import { proxyPreviewBinding, proxyRuntimeLabelKey } from '../../utils/codexProxyPreview';
import { codexProxyCardPresentation } from '../../utils/codexProxyDisplay';
import { proxySummary } from '../../utils/codexProxyPresentation';
import { withCodexPlanBadgeStyle } from '../../utils/codexPreferences';

export function CodexProxyConnectionSummary({ account, status, failed, onSwitch, onFollow, onDisable, onMeasure, measuring = false, switchButtonRef }: {
  account: CodexAccount;
  status: CodexProxyRuntimeStatus | null;
  failed: boolean;
  onSwitch?: () => void;
  onFollow?: () => void;
  onDisable?: () => void;
  onMeasure?: () => void;
  measuring?: boolean;
  switchButtonRef?: Ref<HTMLButtonElement>;
}) {
  const { t } = useTranslation();
  const binding = proxyPreviewBinding(account.egress_proxy, status);
  const saved = binding.summary;
  // Cards and preview use the same leaf/measurement selection, including merged channels.
  const { nodes } = codexProxyCardPresentation(account, status);
  const badge = getCodexPlanBadgePresentation(account, { preserveRawNonProLabel: true });
  const rawPlan = account.plan_type?.trim() ? badge.label : undefined;
  const planClass = rawPlan ? withCodexPlanBadgeStyle(badge.className) : '';
  const sourceKey = account.egress_proxy_disabled ? 'codex.proxy.modeDisabled' : binding.source === 'account' ? 'codex.proxy.modeIndependent' : binding.source === 'unified'
    ? 'codex.proxy.modeUnified' : binding.source === 'unknown'
      ? failed ? 'codex.proxy.runtimeUnavailable' : 'codex.proxy.runtimeLoading' : 'codex.proxy.filter_unbound';
  return <section className="codex-proxy-preview-connection" aria-label={t('codex.proxy.connectionTitle')}>
    <div className="codex-proxy-preview-node">
      <div className="codex-proxy-preview-eyebrow"><Server size={16} />
        <span className={`codex-proxy-preview-badge${saved ? ' is-bound' : ''}`}>{t(sourceKey)}</span>
        {rawPlan && <span className={`tier-badge ${planClass}`} title={rawPlan}>{rawPlan}</span>}
      </div>
      <h3>{account.egress_proxy_disabled ? t('codex.proxy.modeDisabled') : saved?.name || (saved ? proxySummary(saved) : t(binding.source === 'none' ? 'codex.proxy.unboundHint' : sourceKey))}</h3>
      <div className="codex-proxy-preview-metadata">
        {saved?.sourceName && <span>{t('codex.proxy.catalog.sources')}<strong>{saved.sourceName}</strong></span>}
        {saved && !['catalog', 'resource'].includes(saved.protocol.toLowerCase()) && <span>{t('codex.proxy.protocol')}<strong>{saved.protocol.toUpperCase()}</strong></span>}
        {saved?.server && <span>{t('codex.proxy.host')}<strong>{saved.server}{saved.port ? `:${saved.port}` : ''}</strong></span>}
      </div>
      {account.egress_proxy_disabled && <p>{t('codex.proxy.disabledHint')}</p>}
      {!account.egress_proxy_disabled && binding.source === 'unified' && <p>{t('codex.proxy.modeUnifiedHint')}</p>}
      {!account.egress_proxy_disabled && binding.source === 'none' && <p>{t('codex.proxy.managementHint')}</p>}
    </div>
    <div className="codex-proxy-preview-current">
      <div className="codex-proxy-preview-current-content">
        <span className="codex-proxy-preview-current-label">{t('codex.proxy.currentNode')}</span>
        {nodes.length ? nodes.map(({ name, kinds, selection }) => <div className="codex-proxy-preview-current-node" key={name}>
          <strong>{name}</strong>
          <span className="codex-proxy-preview-current-latency" title={selection?.checkedAt != null
            ? `${t('codex.proxy.latencyCheckedAt')}: ${new Date(selection.checkedAt).toLocaleString()}` : undefined}>
            {selection?.delayMs != null && selection.checkedAt != null ? `${selection.delayMs} ms`
              : t(selection?.checkedAt != null ? 'common.failed' : 'codex.proxy.latencyPending')}
          </span>
          {nodes.length > 1 && <small>{kinds.map((kind) => t(proxyRuntimeLabelKey(kind))).join(' · ')}</small>}
          {selection?.checkedAt != null && <small className="codex-proxy-preview-current-checked">
            {t('codex.proxy.latencyCheckedAt')}: <time dateTime={new Date(selection.checkedAt).toISOString()}>{new Date(selection.checkedAt).toLocaleString()}</time>
          </small>}
        </div>) : <span className="codex-proxy-preview-current-empty">{t(failed ? 'codex.proxy.runtimeUnavailable'
          : !status ? 'codex.proxy.runtimeLoading' : 'codex.proxy.noCurrentNode')}</span>}
      </div>
      <div className="codex-proxy-preview-routing-actions">
        {onMeasure && <button type="button" className="btn btn-secondary" disabled={measuring || failed || nodes.length === 0}
          title={t(nodes.length ? 'codex.proxy.currentLatencyHint' : 'codex.proxy.currentLatencyNotRunning')} onClick={onMeasure}>
          {measuring ? <RefreshCw size={14} className="loading-spinner" /> : <Gauge size={14} />}
          {t(measuring ? 'codex.proxy.measuringCurrentLatency' : 'codex.proxy.measureCurrentLatency')}
        </button>}
        {onSwitch && <button ref={switchButtonRef} type="button" className="btn btn-secondary" disabled={measuring} aria-haspopup="dialog" onClick={onSwitch}>
          <ArrowLeftRight size={14} />{t(account.egress_proxy ? 'codex.proxy.quickSwitch' : 'codex.proxy.setIndependent')}
        </button>}
        {onFollow && (account.egress_proxy || account.egress_proxy_disabled) && <button type="button" className="btn btn-secondary" disabled={measuring} aria-haspopup="dialog" onClick={onFollow}>{t('codex.proxy.managerAccounts.follow')}</button>}
        {onDisable && !account.egress_proxy_disabled && <button type="button" className="btn btn-secondary" disabled={measuring} aria-haspopup="dialog" onClick={onDisable}>{t('codex.proxy.disableAction')}</button>}
      </div>
    </div>
  </section>;
}
