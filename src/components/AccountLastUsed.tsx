import { useTranslation } from 'react-i18next';
import { Clock } from 'lucide-react';

interface AccountLastUsedProps {
  lastUsed: number;
  createdAt: number;
  formatDate: (timestamp: number) => string;
}

// Displays the most recent account switch time. Shows "never switched" when
// last_used is 0/empty instead of misleading creation time. Generic across
// every platform because every account model records last_used on switch.
export function AccountLastUsed({ lastUsed, createdAt, formatDate }: AccountLastUsedProps) {
  const { t } = useTranslation();
  const ts = lastUsed && lastUsed > 0 ? lastUsed : createdAt;
  if (!ts || ts <= 0) return null;
  const switched = lastUsed && lastUsed > 0;
  return (
    <span
      className="card-date account-last-used"
      title={switched ? formatDate(lastUsed) : t('accounts.neverSwitched')}
    >
      <Clock size={12} />
      <span>
        {t('accounts.lastSwitchTime')}: {switched ? formatDate(lastUsed) : t('accounts.neverSwitched', '从未切换')}
      </span>
    </span>
  );
}
