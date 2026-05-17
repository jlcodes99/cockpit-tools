import { useState, useEffect } from 'react';
import { X } from 'lucide-react';

export interface DosageNotifyUsage {
  dosageNotifyCode?: string;
  dosageNotifyZh?: string;
  dosageNotifyEn?: string;
  isNormal: boolean;
}

interface DosageNotifyUsageStatusProps {
  usage: DosageNotifyUsage;
  locale: string;
  accountLabel: string;
  normalText: string;
  abnormalText: string;
  viewDetailText: string;
  detailTitle: string;
  accountText: string;
  confirmText: string;
  closeText: string;
  classPrefix: 'codebuddy' | 'workbuddy';
}

function resolveDosageNotifyDetailText(usage: DosageNotifyUsage, locale: string, fallbackText: string): string {
  const detailRaw = locale.startsWith('zh')
    ? (usage.dosageNotifyZh || usage.dosageNotifyCode)
    : (usage.dosageNotifyEn || usage.dosageNotifyCode);
  const detailText = (detailRaw || usage.dosageNotifyCode || '').trim();
  return detailText || fallbackText;
}

export function DosageNotifyUsageStatus({
  usage,
  locale,
  accountLabel,
  normalText,
  abnormalText,
  viewDetailText,
  detailTitle,
  accountText,
  confirmText,
  closeText,
  classPrefix,
}: DosageNotifyUsageStatusProps) {
  const [showDetail, setShowDetail] = useState(false);

  useEffect(() => {
    if (!showDetail) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        setShowDetail(false);
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [showDetail]);
  if (usage.isNormal) {
    return <span className="quota-value high">{normalText}</span>;
  }

  const detailText = resolveDosageNotifyDetailText(usage, locale, abnormalText);

  return (
    <>
      <span className={`${classPrefix}-usage-status`}>
        <span className="quota-value critical">{abnormalText}</span>
        <button
          type="button"
          className={`${classPrefix}-usage-detail-trigger`}
          onClick={() => setShowDetail(true)}
          title={viewDetailText}
        >
          {viewDetailText}
        </button>
      </span>
      {showDetail && (
        <div className="modal-overlay" onClick={() => setShowDetail(false)}>
          <div className={`modal confirm-modal ${classPrefix}-usage-detail-modal`} onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{detailTitle}</h2>
              <button
                className="modal-close"
                onClick={() => setShowDetail(false)}
                aria-label={closeText}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <p>{accountText}: {accountLabel}</p>
              <div className="quota-error-detail">{detailText}</div>
            </div>
            <div className="modal-footer">
              <button className="btn btn-primary" onClick={() => setShowDetail(false)}>
                {confirmText}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
