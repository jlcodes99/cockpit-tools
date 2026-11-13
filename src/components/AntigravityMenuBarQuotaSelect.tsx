import { useTranslation } from 'react-i18next';
import type { Account } from '../types/account';
import windows from '../utils/antigravityQuotaWindows.json';
import { SingleSelectDropdown } from './SingleSelectDropdown';

export function AntigravityMenuBarQuotaSelect({ accounts, value, onChange }: {
  accounts: Account[];
  value: string;
  onChange: (value: string) => void;
}) {
  const { t } = useTranslation();
  const label = t('settings.general.menuBarAntigravityMetric');
  const weekly = t('common.quota.weeklyWindow');
  const choices = new Map(windows.map((window) => [
    window.id,
    `${window.model} (${window.weekly ? weekly : '5h'})`,
  ]));
  const aliases = new Set(windows.flatMap((window) => window.aliases));
  const modelLabels = new Map<string, string>();
  for (const account of accounts) {
    for (const model of account.quota?.models ?? []) {
      const id = model.name.trim();
      if (id && !aliases.has(id)) {
        modelLabels.set(id, model.display_name?.trim() || id);
      }
    }
  }
  for (const [id, modelLabel] of [...modelLabels].sort(([, left], [, right]) => left.localeCompare(right))) {
    choices.set(id, modelLabel);
  }
  if (value && !choices.has(value)) choices.set(value, value);

  return <SingleSelectDropdown
    ariaLabel={label}
    value={value}
    options={[
      { value: '', label: t('settings.general.menuBarQuotaLowest') },
      ...Array.from(choices, ([value, label]) => ({ value, label })),
    ]}
    onChange={onChange}
  />;
}
