import { useEffect, useState } from 'react';
import { AlertTriangle, Clock } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import type { CodexQuota, CodexResetCreditsSnapshot } from '../types/codex';
import { getCodexResetCredits } from '../services/codexService';
import { getResetCreditExpiry } from '../utils/codexResetCreditExpiry';
import './CodexResetCreditExpiry.css';

interface Props {
  accountId: string;
  quota: CodexQuota | undefined;
  disabled: boolean;
  onClick: () => void;
}

export function CodexResetCreditExpiry({ accountId, quota, disabled, onClick }: Props) {
  const { t, i18n } = useTranslation();
  const [snapshot, setSnapshot] = useState<CodexResetCreditsSnapshot | null>(null);
  const [failed, setFailed] = useState(false);
  const [retry, setRetry] = useState(0);
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));
  const availableCount = quota?.reset_credits_available;

  useEffect(() => {
    let cancelled = false;
    setFailed(false);
    void getCodexResetCredits(accountId).then(result => {
      if (!cancelled) {
        setSnapshot({ ...result, credits: Array.isArray(result.credits) ? result.credits : [] });
        setNow(Math.floor(Date.now() / 1000));
      }
    }).catch(() => { if (!cancelled) setFailed(true); });
    return () => { cancelled = true; };
  }, [accountId, availableCount, retry]);

  useEffect(() => {
    const tick = () => setNow(Math.floor(Date.now() / 1000));
    const timer = window.setInterval(tick, 30_000);
    window.addEventListener('focus', tick);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener('focus', tick);
    };
  }, []);

  const expiry = snapshot ? getResetCreditExpiry(snapshot, now) : null;
  if (snapshot?.available_count === 0) return null;
  const absolute = expiry ? new Date(expiry.expiresAt * 1000).toLocaleString(i18n.language, {
    month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hour12: false,
  }) : '';
  const minutes = expiry ? Math.max(1, Math.ceil(expiry.remaining / 60)) : 0;
  const duration = minutes >= 60
    ? t('codex.quota.resetCreditExpiryHours', { hours: Math.floor(minutes / 60), minutes: minutes % 60, defaultValue: '{{hours}}小时{{minutes}}分' })
    : t('codex.quota.resetCreditExpiryMinutes', { minutes, defaultValue: '{{minutes}}分钟' });
  const text = failed
    ? t('codex.quota.resetCreditExpiryFailed', '到期时间查询失败，点击重试')
    : !snapshot ? t('codex.quota.resetCreditExpiryLoading', '查询到期时间…')
    : !expiry ? t('codex.quota.resetCreditTimeUnknown', '时间未知')
    : expiry.expired ? t('codex.quota.resetCreditExpiryExpired', '重置卡已到期，点击刷新')
    : t(expiry.urgent ? 'codex.quota.resetCreditExpiryUrgent' : 'codex.quota.resetCreditExpiryNormal', {
      time: absolute, duration,
      defaultValue: expiry.urgent ? '即将到期：{{time}}（剩余{{duration}}）' : '最近到期：{{time}}（剩余{{duration}}）',
    });
  return (
    <button type="button" onClick={() => {
      if (failed || expiry?.expired) setRetry(value => value + 1);
      else onClick();
    }} disabled={disabled}
      className={`codex-reset-credit-expiry ${expiry?.urgent || expiry?.expired ? 'is-urgent' : ''} ${failed ? 'is-error' : ''}`}
      title={text}>
      {expiry?.urgent || expiry?.expired || failed ? <AlertTriangle size={13} /> : <Clock size={13} />}
      <span>{text}</span>
    </button>
  );
}
