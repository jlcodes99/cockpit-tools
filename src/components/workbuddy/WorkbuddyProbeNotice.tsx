import { AlertTriangle, CheckCircle2, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { dismissWorkbuddyProbeRecord, getWorkbuddyProbeRecords } from '../../utils/workbuddyProbeStatus';

export function WorkbuddyProbeNotice({ accountId }: { accountId: string }) {
  const { t, i18n } = useTranslation();
  const records = getWorkbuddyProbeRecords(accountId).slice(0, 2);
  if (records.length === 0) return null;
  const format = new Intl.DateTimeFormat(i18n.resolvedLanguage, { dateStyle: 'short', timeStyle: 'short' });

  return (
    <div className="wb-probe-notice">
      {records.map((record) => {
        const Icon = record.recoveredAt ? CheckCircle2 : AlertTriangle;
        const { model, resetAt, recoveredAt } = record;
        const message = recoveredAt
          ? t('workbuddy.probeNotice.recovered', { model, time: format.format(recoveredAt) })
          : record.status === 'rate_limited' && resetAt
            ? t('workbuddy.probeNotice.until', { model, time: format.format(resetAt) })
            : t(record.status === 'error' ? 'workbuddy.probeNotice.failed' : 'workbuddy.probeNotice.unavailable', { model });
        return (
          <div key={model} className={`wb-probe-notice-item ${record.status}${recoveredAt ? ' recovered' : ''}`}>
            <Icon size={12} aria-hidden="true" />
            <span title={record.message ? `${message}\n${record.message}` : message}>{message}</span>
            <button
              type="button"
              className="wb-probe-notice-close"
              onClick={(event) => {
                event.stopPropagation();
                dismissWorkbuddyProbeRecord(accountId, model);
              }}
              title={t('common.close')}
            >
              <X size={12} aria-hidden="true" />
            </button>
          </div>
        );
      })}
    </div>
  );
}
