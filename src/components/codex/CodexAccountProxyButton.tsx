import { Shield, ShieldCheck, ShieldOff } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { CodexAccount } from '../../types/codex';
import { canUseCodexAccountProxy, requestCodexAccountProxy } from '../../utils/codexAccountProxy';
import { useCodexProxyTargets } from './useCodexProxyTargets';
import { proxySummary } from '../../utils/codexProxyPresentation';

/**
 * Cards and table rows keep a stable proxy label; clicking routes to the shared
 * proxy page, which owns every selection, check and bind action.
 */
export function CodexAccountProxyButton({ account: sourceAccount }: { account: CodexAccount }) {
  const { t } = useTranslation();
  const [account] = useCodexProxyTargets([sourceAccount]);
  if (!canUseCodexAccountProxy(account)) return null;
  const saved = account.egress_proxy;
  const exit = saved ? saved.name?.trim() || proxySummary(saved) : '';
  const status = account.egress_proxy_error ? t('codex.proxy.modeStale') : account.proxyTargetError ? t('codex.proxy.runtimeReadFailed') : account.proxyTargetLoading ? t('common.loading') : account.egress_proxy_disabled ? t('codex.proxy.modeDisabled') : exit || t('codex.proxy.filter_unbound');
  const Icon = account.egress_proxy_disabled ? ShieldOff : saved ? ShieldCheck : Shield;
  return (
    <button
      type="button"
      className={`codex-compact-note-btn codex-account-proxy-button${saved ? ' is-bound' : ''}`}
      title={`${status} · ${t('codex.proxy.management')}`}
      onClick={(event) => {
        event.stopPropagation();
        requestCodexAccountProxy(account.id);
      }}
    >
      <Icon size={13} aria-hidden="true" />
      {t('codex.proxy.action')}
    </button>
  );
}
