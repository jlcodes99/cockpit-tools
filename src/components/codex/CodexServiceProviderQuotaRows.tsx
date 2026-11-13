import type { TFunction } from 'i18next';
import type { CodexAccount } from '../../types/codex';
import { useCodexRelayBalanceDisabled } from '../../hooks/useCodexRelayBalanceDisabled';
import { canShowCodexProviderBalance } from '../../utils/codexDeepSeekAccess';
import { summarizeCodexProviderPoolQuotas, type CodexProviderPoolQuota } from '../../utils/codexProviderPoolQuota';

const numberFormat = new Intl.NumberFormat(undefined, { maximumFractionDigits: 6 });

function formatQuota(row: CodexProviderPoolQuota, t: TFunction): string {
  if (row.status === 'failed') return t('common.shared.quota.queryFailed');
  if (row.status === 'stale') return t('codex.apiService.providerQuotaStale');
  if (row.status === 'unavailable') return t('common.shared.quota.noData');
  if (row.remaining == null) return t('codex.newApi.quota.unlimited');

  const suffix = row.unit === '%' ? '%' : row.unit ? ` ${row.unit}` : '';
  const remaining = numberFormat.format(row.remaining) + suffix;
  const amount = row.total != null && !row.unlimited
    ? `${remaining} / ${numberFormat.format(row.total) + suffix}`
    : remaining;
  return row.unit ? amount : `${amount} · ${t('codex.apiService.providerQuotaUnitUnknown')}`;
}

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
        {row.name} · {formatQuota(row, t)}
      </span>
    ));
}
