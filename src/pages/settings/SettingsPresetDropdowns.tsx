import { useTranslation } from 'react-i18next';
import { SingleSelectDropdown, type SingleSelectOption } from '../../components/SingleSelectDropdown';

interface SettingsDropdownProps {
  value: string;
  options: SingleSelectOption[];
  onChange: (value: string) => void;
  disabled?: boolean;
  ariaLabel?: string;
  className?: string;
  menuWidth?: number;
  menuMaxHeight?: number;
}

/** 统一设置页使用的自定义单选下拉控件。 */
export function SettingsDropdown({
  value,
  options,
  onChange,
  disabled = false,
  ariaLabel,
  className,
  menuWidth,
  menuMaxHeight = 280,
}: SettingsDropdownProps) {
  return (
    <SingleSelectDropdown
      value={value}
      options={options}
      onChange={onChange}
      disabled={disabled}
      ariaLabel={ariaLabel}
      className={['settings-dropdown', className ?? ''].filter(Boolean).join(' ')}
      menuWidth={menuWidth}
      menuMaxHeight={menuMaxHeight}
    />
  );
}

interface SettingsRefreshPresetDropdownProps {
  value: string;
  isPreset: boolean;
  setValue: (value: string) => void;
  setCustomMode: (enabled: boolean) => void;
  disabledNonPreset?: boolean;
  ariaLabel?: string;
}

/** 各平台自动刷新间隔的预设/自定义下拉控件。 */
export function SettingsRefreshPresetDropdown({
  value,
  isPreset,
  setValue,
  setCustomMode,
  disabledNonPreset = false,
  ariaLabel,
}: SettingsRefreshPresetDropdownProps) {
  const { t } = useTranslation();
  const minutesLabel = t('settings.general.minutes');
  const options: SingleSelectOption[] = [
    ...(!isPreset ? [{ value, label: `${value} ${minutesLabel}` }] : []),
    { value: '-1', label: t('settings.general.autoRefreshDisabled'), disabled: disabledNonPreset },
    { value: '2', label: `2 ${minutesLabel}` },
    { value: '5', label: `5 ${minutesLabel}`, disabled: disabledNonPreset },
    { value: '10', label: `10 ${minutesLabel}`, disabled: disabledNonPreset },
    { value: '15', label: `15 ${minutesLabel}`, disabled: disabledNonPreset },
    { value: 'custom', label: t('settings.general.autoRefreshCustom'), disabled: disabledNonPreset },
  ];

  return (
    <SettingsDropdown
      value={value}
      options={options}
      ariaLabel={ariaLabel}
      onChange={(nextValue) => {
        if (nextValue === 'custom') {
          setCustomMode(true);
          setValue(value !== '-1' ? value : '1');
          return;
        }
        setCustomMode(false);
        setValue(nextValue);
      }}
    />
  );
}

interface SettingsThresholdPresetDropdownProps {
  value: string;
  isPreset: boolean;
  setValue: (value: string) => void;
  setCustomMode: (enabled: boolean) => void;
  presets?: readonly string[];
  defaultCustomValue?: string;
  unit?: string;
  ariaLabel?: string;
}

/** 配额预警/自动切号阈值的预设/自定义下拉控件。 */
export function SettingsThresholdPresetDropdown({
  value,
  isPreset,
  setValue,
  setCustomMode,
  presets = ['0', '20', '40', '60'],
  defaultCustomValue = '20',
  unit = '%',
  ariaLabel,
}: SettingsThresholdPresetDropdownProps) {
  const { t } = useTranslation();
  const options: SingleSelectOption[] = [
    ...(!isPreset ? [{ value, label: `${value}${unit}` }] : []),
    ...presets.map((preset) => ({ value: preset, label: `${preset}${unit}` })),
    { value: 'custom', label: t('settings.general.autoRefreshCustom') },
  ];

  return (
    <SettingsDropdown
      value={value}
      options={options}
      ariaLabel={ariaLabel}
      onChange={(nextValue) => {
        if (nextValue === 'custom') {
          setCustomMode(true);
          setValue(value || defaultCustomValue);
          return;
        }
        setCustomMode(false);
        setValue(nextValue);
      }}
    />
  );
}
