import { Shield, ShieldCheck } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { Account } from '../../types/account';

/**
 * Antigravity 账号卡片上的代理入口，交互与视觉对齐 CodexAccountProxyButton：
 * 已绑定显示绿色盾牌与出口地址，未绑定显示未绑定文案；点击打开账号代理弹层。
 */
export function AntigravityAccountProxyButton({ account, onOpen }: {
  account: Account;
  onOpen: (accountId: string) => void;
}) {
  const { t } = useTranslation();
  const saved = account.egress_proxy_url?.trim() ?? '';
  const status = saved || t('codex.proxy.filter_unbound');
  const Icon = saved ? ShieldCheck : Shield;
  return (
    <button
      type="button"
      className={`codex-compact-note-btn codex-account-proxy-button${saved ? ' is-bound' : ''}`}
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
