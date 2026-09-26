import { Shield, ShieldCheck } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { CodexAccount } from '../../types/codex';
import { canUseCodexAccountProxy, requestCodexAccountProxy } from '../../utils/codexAccountProxy';
import { proxySummary } from '../../utils/codexProxyPresentation';

/**
 * Cards and table rows keep a stable proxy label; clicking routes to the shared
 * proxy page, which owns every selection, check and bind action.
 */
export function CodexAccountProxyButton({ account }: { account: CodexAccount }) {
  const { t } = useTranslation();
  if (!canUseCodexAccountProxy(account)) return null;
  const saved = account.egress_proxy;
  const exit = saved ? saved.name?.trim() || proxySummary(saved) : '';
  const status = exit || t('codex.proxy.filter_unbound');
  const Icon = saved ? ShieldCheck : Shield;
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
