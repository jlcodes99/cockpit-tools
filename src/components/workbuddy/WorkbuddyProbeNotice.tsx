import { useEffect, useState } from 'react';
import { AlertTriangle, CheckCircle2, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { subscribeWorkbuddyProbeStatus } from '../../utils/workbuddyProbeStatus';
import {
  dismissWorkbuddyMergedNotice,
  getMergedWorkbuddyNotices,
  subscribeWorkbuddyLogLimits,
} from '../../utils/workbuddyLogLimits';

export function WorkbuddyProbeNotice({ accountId }: { accountId: string }) {
  const { t, i18n } = useTranslation();
  const [, setVersion] = useState(0);

  useEffect(() => {
    const bump = () => setVersion((v) => v + 1);
    const unsubProbe = subscribeWorkbuddyProbeStatus(bump);
    const unsubLog = subscribeWorkbuddyLogLimits(bump);
    return () => {
      unsubProbe();
      unsubLog();
    };
  }, []);

  const records = getMergedWorkbuddyNotices(accountId).slice(0, 2);
  if (records.length === 0) return null;
  const format = new Intl.DateTimeFormat(i18n.resolvedLanguage, { dateStyle: 'short', timeStyle: 'short' });

  return (
    <div className="wb-probe-notice">
      {records.map((record) => {
        const Icon = record.recoveredAt ? CheckCircle2 : AlertTriangle;
        const { model, resetAt, recoveredAt, source, hitCount } = record;
        const formattedReset = resetAt ? format.format(resetAt) : '';

        let message: string;
        if (recoveredAt) {
          message = t('workbuddy.probeNotice.recovered', {
            model,
            time: format.format(recoveredAt),
          });
        } else if (source === 'client_log') {
          message = model
            ? t('workbuddy.probeNotice.logUntil', {
                model,
                time: formattedReset,
                defaultValue: '{{model}} 客户端日志受限，恢复时间 {{time}}',
              })
            : t('workbuddy.probeNotice.logUnknownUntil', {
                time: formattedReset,
                defaultValue: '客户端日志检测到限流，恢复时间 {{time}}',
              });
        } else if (record.status === 'rate_limited' && resetAt) {
          message = t('workbuddy.probeNotice.until', {
            model,
            time: formattedReset,
          });
        } else {
          message = t(
            record.status === 'error'
              ? 'workbuddy.probeNotice.failed'
              : 'workbuddy.probeNotice.unavailable',
            { model },
          );
        }

        const detailParts = [message];
        if (record.hasLogRecord && hitCount > 1) {
          detailParts.push(
            t('workbuddy.probeNotice.logHits', {
              count: hitCount,
              defaultValue: '日志命中 {{count}} 次',
            }),
          );
        }
        if (record.message) {
          detailParts.push(record.message);
        }

        return (
          <div
            key={record.key}
            className={`wb-probe-notice-item ${record.status}${recoveredAt ? ' recovered' : ''}`}
          >
            <Icon size={12} aria-hidden="true" />
            <span title={detailParts.join('\n')}>{message}</span>
            <button
              type="button"
              className="wb-probe-notice-close"
              onClick={(event) => {
                event.stopPropagation();
                dismissWorkbuddyMergedNotice(record);
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
