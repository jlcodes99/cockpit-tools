import { useTranslation } from 'react-i18next';
import { SingleSelectDropdown } from './SingleSelectDropdown';

interface MenuBarLeftClickSettingProps {
  opensMainWindow: boolean;
  disabled: boolean;
  onChange: (opensMainWindow: boolean) => void;
  variant?: 'general' | 'quick-settings';
}

export function MenuBarLeftClickSetting({
  opensMainWindow,
  disabled,
  onChange,
  variant = 'general',
}: MenuBarLeftClickSettingProps) {
  const { t } = useTranslation();
  const label = t('settings.general.menuBarLeftClick');
  const description = t('settings.general.menuBarLeftClickDesc');
  const quick = variant === 'quick-settings';
  const dropdown = (
    <SingleSelectDropdown
      value={opensMainWindow ? 'window' : 'quota'}
      options={[
        { value: 'quota', label: t('settings.general.menuBarLeftClickQuotaMenu') },
        { value: 'window', label: t('settings.general.menuBarLeftClickMainWindow') },
      ]}
      onChange={(value) => onChange(value === 'window')}
      ariaLabel={label}
      className={quick ? 'qs-dropdown' : 'settings-dropdown'}
      disabled={disabled}
    />
  );

  if (quick) {
    return (
      <div className="qs-section">
        <div className="qs-row">
          <div className="qs-row-label"><span>{label}</span></div>
          <div className="qs-row-control">{dropdown}</div>
        </div>
        <div className="qs-hint">{description}</div>
      </div>
    );
  }

  return (
    <div className="settings-row">
      <div className="row-label">
        <div className="row-title">{label}</div>
        <div className="row-desc">{description}</div>
      </div>
      <div className="row-control">{dropdown}</div>
    </div>
  );
}
