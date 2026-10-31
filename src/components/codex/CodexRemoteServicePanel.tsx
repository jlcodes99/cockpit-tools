import type { TFunction } from 'i18next';
import type { CodexApiKeyUsageState } from '../../services/codexApiKeyUsageRefreshService';
import { CodexDailyUsageMetrics } from './CodexDailyUsagePanel';
import './CodexRemoteService.css';

export function CodexRemoteServicePanel({ state, variant, t }: {
  state: CodexApiKeyUsageState;
  variant: 'card' | 'table';
  t: TFunction;
}) {
  return <div className={`codex-api-key-usage-panel codex-remote-service-panel ${variant}`}>
    <CodexDailyUsageMetrics summary={state.summary} t={t} />
  </div>;
}
