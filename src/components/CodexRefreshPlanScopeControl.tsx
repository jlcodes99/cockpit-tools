import { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { useCodexAccountStore } from '../stores/useCodexAccountStore';
import { isStandardCodexOAuthAccount } from '../types/codex';
import {
  ALL_CODEX_AUTO_REFRESH_PLAN_KEYS,
  buildCodexAutoRefreshPlanOptions,
  sanitizeCodexAutoRefreshPlanKeys,
} from '../utils/codexAutoRefreshPlanScope';
import './CodexRefreshPlanScopeControl.css';

export function CodexRefreshPlanScopeControl({ value, onChange, disabled = false }: {
  value?: readonly string[];
  onChange: (value: string[]) => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  const accounts = useCodexAccountStore((state) => state.accounts);
  const options = useMemo(() => buildCodexAutoRefreshPlanOptions(
    accounts.filter(isStandardCodexOAuthAccount),
  ), [accounts]);
  const selected = sanitizeCodexAutoRefreshPlanKeys(value);
  return <div className="codex-refresh-scope">
    <div className="codex-refresh-scope-options" role="group" aria-label={t('codex.autoRefreshScope.label')}>
      {options.map(({ key, count }) => <button
        key={key}
        type="button"
        className={`btn btn-secondary codex-refresh-scope-option${selected.includes(key) ? ' is-selected' : ''}`}
        disabled={disabled}
        aria-pressed={selected.includes(key)}
        onClick={() => onChange(sanitizeCodexAutoRefreshPlanKeys(
          selected.includes(key) ? selected.filter((item) => item !== key) : [...selected, key],
        ))}
      >
        <span>{t(`codex.autoRefreshScope.plans.${key}`)}</span>
        <span className="codex-refresh-scope-count">{count}</span>
      </button>)}
    </div>
    <div className="codex-refresh-scope-actions">
      <button type="button" className="btn btn-ghost" disabled={disabled || selected.length === options.length}
        onClick={() => onChange([...ALL_CODEX_AUTO_REFRESH_PLAN_KEYS])}>{t('common.selectAll')}</button>
      <button type="button" className="btn btn-ghost" disabled={disabled || selected.length === 0}
        onClick={() => onChange([])}>{t('common.clearAll')}</button>
    </div>
  </div>;
}
