import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import type { CodexAccount } from '../../types/codex';
import { refreshCodexApiKeyUsageForAccounts, type CodexApiKeyUsageState } from '../../services/codexApiKeyUsageRefreshService';

export function AinipyUsagePanel({ account, baseUrl, usageState, variant }: {
  account: CodexAccount;
  baseUrl: string;
  usageState?: CodexApiKeyUsageState;
  variant: 'card' | 'table';
}) {
  const { t } = useTranslation();
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState('');
  const summary = usageState?.summary?.mode === 'ainipy' ? usageState.summary : undefined;
  const needsLogin = !summary || usageState?.error?.includes('AINIPY_LOGIN_REQUIRED');
  const tokens = (value?: number | null) =>
    typeof value === 'number' && Number.isFinite(value)
      ? new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 }).format(Math.floor(value))
      : '-';
  const connect = async () => {
    setConnecting(true);
    setError('');
    try {
      await invoke('codex_login_ainipy_usage', { baseUrl, apiKey: account.openai_api_key });
      await refreshCodexApiKeyUsageForAccounts([account], { force: true });
    } catch (cause) {
      const message = String(cause);
      setError(message.includes('ACCOUNT_MISMATCH')
        ? t('codex.ainipy.mismatch', 'This API key does not belong to the signed-in Ainipy account.')
        : message.includes('CANCELLED')
        ? t('codex.ainipy.cancelled', 'Ainipy login cancelled.')
        : message.includes('TIMEOUT')
          ? t('codex.ainipy.timeout', 'Login timed out. Please try again.')
          : t('codex.ainipy.failed', 'Unable to sync Ainipy. Please try again.'));
    } finally {
      setConnecting(false);
    }
  };
  return <div className={`codex-api-key-usage-panel ${variant} sub2api`}>
    <div className="codex-api-key-usage-grid">
      <div><span>{t('codex.ainipy.balance', 'Balance (tokens)')}</span><strong>{tokens(summary?.balance)}</strong></div>
      <div><span>{t('codex.ainipy.today', 'Used today (tokens)')}</span><strong>{tokens(summary?.todayTotalTokens)}</strong></div>
      <div><span>{t('codex.ainipy.total', 'Total used (tokens)')}</span><strong>{tokens(summary?.totalTotalTokens)}</strong></div>
    </div>
    {(needsLogin || connecting) && <button className="btn btn-sm btn-secondary" disabled={connecting || usageState?.loading}
      title={t('codex.ainipy.hint', 'Sign in to the Ainipy account for this card. Sign in again after closing the app.')}
      onClick={(event) => { event.stopPropagation(); void connect(); }}>
      {connecting ? t('codex.ainipy.waiting', 'Waiting for Ainipy login…') : t('codex.ainipy.connect', 'Connect Ainipy website')}
    </button>}
    {(error || (usageState?.error && !usageState.error.includes('AINIPY_LOGIN_REQUIRED'))) &&
      <div className="codex-api-key-usage-empty" role="status">{error || t('codex.ainipy.failed', 'Unable to sync Ainipy. Please try again.')}</div>}
  </div>;
}
