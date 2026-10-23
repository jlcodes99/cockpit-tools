import { useTranslation } from 'react-i18next';
import { SingleSelectDropdown } from '../../components/SingleSelectDropdown';

export function ThemeColorSetting({ value, onChange }: {
  value: string;
  onChange: (value: string) => void;
}) {
  const { t } = useTranslation();
  const packs = [
    ['default', 'Default'], ['nord', 'Nord'], ['tokyo-night', 'TokyoNight'],
    ['catppuccin', 'Catppuccin'], ['gruvbox', 'Gruvbox'],
    ['everforest', 'Everforest'], ['oled', 'Oled'],
  ];
  return <SingleSelectDropdown
    value={value}
    options={packs.map(([key, label]) => ({
      value: key, label: t(`settings.general.themeColor${label}`),
    }))}
    ariaLabel={t('settings.general.themeColor')}
    onChange={(next) => {
      onChange(next);
      document.documentElement.setAttribute('data-theme-color', next);
    }}
  />;
}
