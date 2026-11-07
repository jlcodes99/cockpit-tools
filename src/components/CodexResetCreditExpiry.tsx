import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { formatCodexResetTime, type CodexQuota } from '../types/codex';
import { getResetCreditExpiry } from '../utils/codexResetCreditExpiry';
import { codexResetCreditDetails } from '../services/codexResetCreditService';
import './CodexResetCreditExpiry.css';

interface Props {
  accountId: string;
  quota: CodexQuota | undefined;
  disabled: boolean;
  onClick: () => void;
}

export function CodexResetCreditExpiry({ accountId, quota, disabled, onClick }: Props) {
  const { t } = useTranslation();
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));
  const [, setRevision] = useState(0);
  const count = quota?.reset_credits_available ?? 0;
  useEffect(() => codexResetCreditDetails.subscribe(accountId, () => setRevision(value => value + 1)), [accountId]);
  useEffect(() => {
    if (count <= 0) return;
    const tick = () => setNow(Math.floor(Date.now() / 1000));
    tick();
    const timer = window.setInterval(tick, 30_000);
    window.addEventListener('focus', tick);
    return () => { window.clearInterval(timer); window.removeEventListener('focus', tick); };
  }, [count]);
  useEffect(() => {
    if (count <= 0) return;
    if (quota?.reset_credits_next_expires_at || quota?.reset_credits?.some(credit => credit.expires_at)) {
      codexResetCreditDetails.seed(accountId, count, {
        available_count: count, credits: quota.reset_credits ?? [],
        next_expires_at: quota.reset_credits_next_expires_at,
      });
    }
    // The clock updates locally; only stale details enter the staggered queue.
    void codexResetCreditDetails.load(accountId, count).catch(() => {});
  }, [accountId, count, now, quota]);

  if (count <= 0) return null;
  const snapshot = codexResetCreditDetails.peek(accountId, count) ?? {
    available_count: count, credits: quota?.reset_credits ?? [],
    next_expires_at: quota?.reset_credits_next_expires_at,
  };
  if (snapshot.available_count === 0) return null;
  const expiry = getResetCreditExpiry(snapshot, now);
  const time = !expiry ? t('codex.quota.resetCreditTimeUnknown')
    : expiry.expired ? t('codex.quota.resetCreditStatusExpired')
    : formatCodexResetTime(expiry.expiresAt, t);
  const text = t('codex.quota.resetCreditNextExpiry', { time });
  return (
    <button type="button" onClick={() => {
      void codexResetCreditDetails.load(accountId, count, true).catch(() => {});
      onClick();
    }} disabled={disabled}
      className={`codex-quota-mini-reset codex-reset-credit-expiry ${expiry?.urgent || expiry?.expired ? 'is-urgent' : ''}`}
      title={text}>
      {text}
    </button>
  );
}
