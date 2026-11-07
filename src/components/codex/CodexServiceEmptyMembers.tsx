import { FolderPlus } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import './CodexServiceEmptyMembers.css';

/** An enabled listener with an empty pool still needs an explicit member state. */
export function CodexServiceEmptyMembers({ onManage, disabled = false }: {
  onManage?: () => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  return <div className="codex-service-empty-members" role="status">
    <strong>{t('codex.localAccess.accountPoolHealth.emptyTitle')}</strong>
    <p>{t('codex.localAccess.accountPoolHealth.emptyHint')}</p>
    {onManage && <button type="button" className="btn btn-secondary btn-sm" disabled={disabled} onClick={onManage}>
      <FolderPlus size={14} />{t('codex.localAccess.modal.manageMembers')}
    </button>}
  </div>;
}
