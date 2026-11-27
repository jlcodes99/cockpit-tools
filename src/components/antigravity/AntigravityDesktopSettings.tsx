import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from 'react-i18next';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import './AntigravityDesktopSettings.css';

export interface AntigravityDesktopSettings {
  antigravity_desktop_chinese_enabled: boolean;
  antigravity_desktop_layout: 'software_engineering' | 'knowledge_work';
  antigravity_desktop_permission: 'request_review' | 'default' | 'auto' | 'turbo';
}

export function readAntigravityDesktopSettings(config?: Partial<AntigravityDesktopSettings>): AntigravityDesktopSettings {
  return {
    antigravity_desktop_chinese_enabled: config?.antigravity_desktop_chinese_enabled ?? false,
    antigravity_desktop_layout: config?.antigravity_desktop_layout ?? 'software_engineering',
    antigravity_desktop_permission: config?.antigravity_desktop_permission ?? 'auto',
  };
}

interface FieldsProps {
  settings: AntigravityDesktopSettings;
  onChange: (updates: Partial<AntigravityDesktopSettings>) => void;
  variant: 'overview' | 'general' | 'quick';
  disabled?: boolean;
}

export function AntigravityDesktopSettingsFields({ settings, onChange, variant, disabled }: FieldsProps) {
  const { t } = useTranslation();
  const fields = [
    {
      key: 'language',
      label: t('antigravityLaunch.desktopLanguage'),
      value: settings.antigravity_desktop_chinese_enabled ? 'zh-CN' : 'en',
      options: [
        { value: 'en', label: t('antigravityLaunch.desktopLanguageEnglish') },
        { value: 'zh-CN', label: t('antigravityLaunch.desktopLanguageChinese') },
      ],
      set: (value: string) => onChange({ antigravity_desktop_chinese_enabled: value === 'zh-CN' }),
    },
    {
      key: 'layout',
      label: t('antigravityLaunch.desktopLayout'),
      value: settings.antigravity_desktop_layout,
      options: [
        { value: 'software_engineering', label: t('antigravityLaunch.desktopLayoutEngineering') },
        { value: 'knowledge_work', label: t('antigravityLaunch.desktopLayoutKnowledge') },
      ],
      set: (value: string) => onChange({ antigravity_desktop_layout: value as AntigravityDesktopSettings['antigravity_desktop_layout'] }),
    },
    {
      key: 'permission',
      label: t('antigravityLaunch.desktopPermission'),
      value: settings.antigravity_desktop_permission,
      options: [
        { value: 'request_review', label: t('antigravityLaunch.desktopPermissionReview') },
        { value: 'default', label: t('antigravityLaunch.desktopPermissionDefault') },
        { value: 'auto', label: t('antigravityLaunch.desktopPermissionAuto') },
        { value: 'turbo', label: t('antigravityLaunch.desktopPermissionTurbo') },
      ],
      set: (value: string) => onChange({ antigravity_desktop_permission: value as AntigravityDesktopSettings['antigravity_desktop_permission'] }),
    },
  ];
  const classes = {
    overview: ['antigravity-common-setting', 'antigravity-common-setting-label', '', 'settings-dropdown antigravity-common-select'],
    general: ['settings-row', 'row-label row-title', 'row-control', 'settings-dropdown'],
    quick: ['qs-row', 'qs-row-label', 'qs-row-control', 'qs-dropdown'],
  }[variant];
  const [rowClass, labelClass, controlClass, dropdownClass] = classes;

  return fields.map(field => (
    <div className={rowClass} key={field.key}>
      <span className={labelClass} title={field.label}>{field.label}</span>
      <div className={controlClass || undefined}>
        <SingleSelectDropdown
          className={dropdownClass}
          ariaLabel={field.label}
          disabled={disabled}
          options={field.options}
          value={field.value}
          onChange={field.set}
        />
      </div>
    </div>
  ));
}

export function AntigravityCommonSettings() {
  const { t } = useTranslation();
  const [settings, setSettings] = useState<AntigravityDesktopSettings | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');

  useEffect(() => {
    let disposed = false;
    let request = 0;
    const load = async () => {
      const version = ++request;
      try {
        const config = await invoke<AntigravityDesktopSettings>('get_general_config');
        if (!disposed && version === request) {
          setSettings(readAntigravityDesktopSettings(config));
          setError('');
        }
      } catch (cause) {
        if (!disposed && version === request) {
          setError(t('quickSettings.error.loadFailed', { error: String(cause) }));
        }
      }
    };
    void load();
    window.addEventListener('config-updated', load);
    return () => {
      disposed = true;
      window.removeEventListener('config-updated', load);
    };
  }, [t]);

  async function save(updates: Partial<AntigravityDesktopSettings>) {
    setSaving(true);
    setError('');
    try {
      await invoke('patch_general_config', { updates });
      setSettings(previous => previous && { ...previous, ...updates });
      window.dispatchEvent(new Event('config-updated'));
    } catch (cause) {
      setError(t('quickSettings.error.saveFailed', { error: String(cause) }));
    } finally {
      setSaving(false);
    }
  }

  return (
    <section className="antigravity-common-settings" aria-label={t('common.commonSettings')}>
      <div className="antigravity-common-settings-controls">
        <AntigravityDesktopSettingsFields
          settings={settings ?? readAntigravityDesktopSettings()}
          variant="overview"
          disabled={!settings || saving}
          onChange={updates => void save(updates)}
        />
      </div>
      {error && <div className="antigravity-common-settings-error" role="alert">{error}</div>}
      {!settings && !error && <span className="antigravity-common-settings-hint" role="status">{t('common.loading')}</span>}
    </section>
  );
}
