import { useTranslation } from 'react-i18next';
import { SingleSelectDropdown } from '../SingleSelectDropdown';

const LANGUAGES = [
  { value: 'en', label: 'English' },
  { value: 'zh', label: '简体中文' },
  { value: 'de', label: 'Deutsch' },
  { value: 'es', label: 'Español' },
  { value: 'fr', label: 'Français' },
  { value: 'ja', label: '日本語' },
  { value: 'ko', label: '한국어' },
  { value: 'pt-BR', label: 'Português (Brasil)' },
  { value: 'ru', label: 'Русский' },
  { value: 'sv', label: 'Svenska' },
];


export function T3codeLanguageSelect({
  className, label, value, disabled, placeholder, onChange,
}: {
  className?: string;
  label: string;
  value: string;
  disabled?: boolean;
  placeholder?: string;
  onChange: (value: string) => void;
}) {
  const { t } = useTranslation();
  return (
    <SingleSelectDropdown
      className={className}
      menuWidth={224}
      ariaLabel={label}
      placeholder={placeholder}
      value={value}
      disabled={disabled}
      options={[{ value: 'system', label: t('settings.general.themeSystem') }, ...LANGUAGES]}
      onChange={onChange}
    />
  );
}
