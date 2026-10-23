import { useTranslation } from 'react-i18next';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import { requestCodexOpenAddAccount, type CodexAddAccountTab } from '../../utils/codexAddAccountRequest';

/** All API Service account sources share this entry; each keeps its own credentials. */
export function CodexApiServiceAddAccount({ disabled = false }: { disabled?: boolean }) {
  const { t } = useTranslation();
  return <SingleSelectDropdown
    className="chatgpt-plan-add-type" value="" disabled={disabled}
    ariaLabel={t('common.shared.addAccount')} placeholder={t('common.shared.addAccount')}
    options={[
      { value: 'tempLogin', label: t('nav.codex') },
      { value: 'apikey', label: t('codex.addModal.token') },
      { value: 'import', label: t('common.shared.addModal.import') },
    ]}
    onChange={(type) => {
      if (['tempLogin', 'apikey', 'import'].includes(type)) {
        requestCodexOpenAddAccount({ tab: type as CodexAddAccountTab, autoJoinApiService: true });
      }
    }} />;
}
