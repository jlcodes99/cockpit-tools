import { Shield, ShieldCheck } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { Account } from '../../types/account';
import { useAntigravityProxyStore } from '../../stores/useAntigravityProxyStore';
import { proxySummary } from '../../utils/codexProxyPresentation';

/**
 * Antigravity 账号卡片与列表上的独立出口代理入口：
 * 已绑定时高亮并显示出口地址提示，未绑定时显示默认未绑定状态；点击打开账号代理弹层。
 */
export function AntigravityAccountProxyButton({ account, onOpen }: {
  account: Account;
  onOpen: (accountId: string) => void;
}) {
  const { t } = useTranslation();
  const view = useAntigravityProxyStore((state) => state.views[account.id]);
  const failed = useAntigravityProxyStore((state) => state.failed);
  const saved = Boolean(view?.binding || view?.legacyBound || view?.error);
  const status = failed ? t('codex.proxy.runtimeReadFailed') : view?.error ? t('codex.proxy.unified.errorStale')
    : view?.binding ? proxySummary(view.binding) : view?.legacyBound ? t('codex.proxy.legacyBoundHint') : t('codex.proxy.filter_unbound');
  const Icon = saved ? ShieldCheck : Shield;
  return (
    <button
      type="button"
      className={`ag-account-proxy-btn${saved ? ' is-bound' : ''}`}
      title={`${status} · ${t('codex.proxy.management')}`}
      onClick={(event) => {
        event.stopPropagation();
        onOpen(account.id);
      }}
    >
      <Icon size={13} aria-hidden="true" />
      {t('codex.proxy.action')}
    </button>
  );
}
