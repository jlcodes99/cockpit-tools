import type { TFunction } from 'i18next';
import type { CodexAccount } from '../../types/codex';
import { useCodexRelayBalanceDisabled } from '../../hooks/useCodexRelayBalanceDisabled';
import { canShowCodexProviderBalance } from '../../utils/codexDeepSeekAccess';
import { formatCodexProviderQuota, summarizeCodexProviderPoolQuotas } from '../../utils/codexProviderPoolQuota';

export function CodexServiceProviderQuotaRows({ accounts, t }: { accounts: readonly CodexAccount[]; t: TFunction }) {
  const hideRelayBalance = useCodexRelayBalanceDisabled();
  const rows = summarizeCodexProviderPoolQuotas(accounts, Math.floor(Date.now() / 1000));
  return rows
    .filter((row) => canShowCodexProviderBalance(hideRelayBalance, row.baseUrl, row.mode))
    .map((row) => (
      <span
        key={`provider-quota:${row.id}`}
        title={row.observedAt == null ? undefined : t('common.shared.quota.lastSuccessfulQuery', {
          time: new Date(row.observedAt * 1000).toLocaleString(),
        })}
      >
        {row.name} · {formatCodexProviderQuota(row, t)}
      </span>
    ));
}
