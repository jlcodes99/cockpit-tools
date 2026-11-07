import { ShieldCheck } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { requestCodexOfficialAuthorizationManagement } from '../../utils/codexAccountServiceNavigation';
import './CodexOfficialAuthorizationEntry.css';

/** Explicit entry only; no automatic recommendation or attention badge. */
export function CodexOfficialAuthorizationEntry({ disabled = false }: { disabled?: boolean }) {
  const { t } = useTranslation();
  return <button type="button" className="btn btn-secondary codex-official-entry" disabled={disabled}
    title={t('chatgptPlan.officialFlow.entryHint')} onClick={requestCodexOfficialAuthorizationManagement}>
    <span className="codex-official-entry-emblem"><ShieldCheck size={15} /><span>{t('chatgptPlan.officialFlow.badge')}</span></span>
    <span>{t('chatgptPlan.officialFlow.entry')}</span>
  </button>;
}
