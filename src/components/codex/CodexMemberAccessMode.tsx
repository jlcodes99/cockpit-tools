import { useTranslation } from 'react-i18next';
import { SingleSelectDropdown } from '../SingleSelectDropdown';

/** Occupies the existing name column's spare space, before the priority column. */
export function CodexMemberAccessMode({ value, disabled, onChange }: {
  value: 'codex' | 'official'; disabled: boolean; onChange: (value: 'codex' | 'official') => void;
}) {
  const { t } = useTranslation();
  return <SingleSelectDropdown value={value} options={['codex', 'official'].map((mode) => ({ value: mode,
    label: t(mode === 'official' ? 'chatgptPlan.officialFlow.rowOfficial' : 'chatgptPlan.officialFlow.rowLocal') }))}
    className="codex-member-access-mode" menuWidth={150} ariaLabel={t('chatgptPlan.officialFlow.accessMode')}
    disabled={disabled} onChange={(mode) => onChange(mode as 'codex' | 'official')} />;
}
