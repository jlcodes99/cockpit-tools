import { useTranslation } from 'react-i18next';
import type { Account } from '../types/account';
import { getAccountProjectId, isGcpTosAccount } from '../utils/account';

export function AntigravityGcpTosBadge({ account }: { account: Account }) {
  const { t } = useTranslation();
  if (!isGcpTosAccount(account)) return null;

  return (
    <span
      className="tier-badge gcp-tos"
      title={getAccountProjectId(account) ?? t('accounts.badge.gcpTosTitle')}
    >
      {t('accounts.badge.gcpTos')}
    </span>
  );
}
