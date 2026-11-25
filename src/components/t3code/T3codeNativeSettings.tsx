import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { RefreshCw } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import { getT3codeNativeSettings, updateT3codeNativeSettings } from '../../services/t3codeService';
import type { T3CodeNativeModel, T3CodeNativeSettings } from '../../types/t3code';
import './T3codeNativeSettings.css';

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

const modelKey = (model: Pick<T3CodeNativeModel, 'instanceId' | 'model'>) =>
  JSON.stringify([model.instanceId, model.model]);
type UpdateArgs = Parameters<typeof updateT3codeNativeSettings>;

function SettingRow({
  compact,
  label,
  children,
}: {
  compact: boolean;
  label: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className={compact ? 'qs-row' : 'settings-row'}>
      <div className={compact ? 'qs-row-label' : 'row-label'}>
        <span className={compact ? undefined : 'row-title'}>{label}</span>
      </div>
      {children && <div className={compact ? 'qs-row-control' : 'row-control'}>{children}</div>}
    </div>
  );
}

export function T3codeNativeSettingsPanel({ compact = false }: { compact?: boolean }) {
  const { t } = useTranslation();
  const [settings, setSettings] = useState<T3CodeNativeSettings>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [saved, setSaved] = useState(false);
  const request = useRef(0);

  const load = useCallback(async () => {
    const version = ++request.current;
    setBusy(true);
    setError('');
    setSaved(false);
    try {
      const data = await getT3codeNativeSettings();
      if (request.current === version) setSettings(data);
    } catch (err) {
      if (request.current === version) setError(String(err));
    } finally {
      if (request.current === version) setBusy(false);
    }
  }, []);

  useEffect(() => {
    void load();
    return () => {
      request.current += 1;
    };
  }, [load]);

  async function save(...args: UpdateArgs) {
    const version = ++request.current;
    setBusy(true);
    setError('');
    setSaved(false);
    try {
      const data = await updateT3codeNativeSettings(...args);
      if (request.current === version) {
        setSettings(data);
        setSaved(true);
      }
    } catch (err) {
      if (request.current === version) setError(String(err));
    } finally {
      if (request.current === version) setBusy(false);
    }
  }

  const tx = (name: string) => t(`t3code.nativeSettings.${name}`);
  const selection = settings?.defaultModelSelection;
  const selected = settings?.models.find(
    (model) => selection && modelKey(model) === modelKey(selection),
  );
  const effort = selected?.effort;
  const currentEffort = selection?.options?.find((item) => item.id === effort?.id)?.value;
  const defaultOption = { value: '', label: tx('clientDefault') };
  const models = [
    defaultOption,
    ...(selection && !selected
      ? [{ value: modelKey(selection), label: selection.model, disabled: true }]
      : []),
    ...(settings?.models ?? []).map((model) => ({
      value: modelKey(model),
      label: `${model.providerName} · ${model.label}`,
    })),
  ];

  const dropdownClass = `${compact ? 'qs-dropdown' : 'settings-dropdown'} t3-native-settings-select`;
  const refreshLabel = t('common.refresh');
  const feedback =
    error ||
    (busy
      ? t('common.loading')
      : saved
        ? settings?.clientConnected
          ? t('common.success')
          : tx('savedForLaunch')
        : settings && !settings.clientConnected
          ? tx('connectRequired')
          : '');

  const refreshBtn = (
    <button
      type="button"
      className={compact ? 'qs-btn' : 'btn btn-secondary btn-icon'}
      disabled={busy}
      aria-label={refreshLabel}
      title={refreshLabel}
      onClick={() => void load()}
    >
      <RefreshCw size={14} className={busy ? 'loading-spinner' : undefined} aria-hidden="true" />
    </button>
  );

  return (
    <>
      {settings && (
        <>
          <SettingRow compact={compact} label={t('settings.general.language')}>
            <SingleSelectDropdown
              className={dropdownClass}
              menuWidth={224}
              ariaLabel={t('settings.general.language')}
              value={settings.language}
              disabled={busy}
              options={[{ value: 'system', label: t('settings.general.themeSystem') }, ...LANGUAGES]}
              onChange={(value) => void save('language', value)}
            />
          </SettingRow>
          <SettingRow compact={compact} label={tx('defaultModel')}>
            <SingleSelectDropdown
              className={dropdownClass}
              menuWidth={336}
              ariaLabel={tx('defaultModel')}
              value={selection ? modelKey(selection) : ''}
              disabled={busy || (!selection && settings.models.length === 0)}
              options={models}
              search={{ label: t('common.search'), emptyLabel: t('common.none') }}
              onChange={(value) => {
                const model = settings.models.find((item) => modelKey(item) === value);
                if (model) {
                  void save('defaultModelSelection', {
                    instanceId: model.instanceId,
                    model: model.model,
                  });
                } else if (!value) {
                  void save('defaultModelSelection', null);
                }
              }}
            />
            {refreshBtn}
          </SettingRow>
          {selected && effort && (
            <SettingRow compact={compact} label={tx('reasoning')}>
              <SingleSelectDropdown
                className={dropdownClass}
                menuWidth={224}
                ariaLabel={tx('reasoning')}
                value={typeof currentEffort === 'string' ? currentEffort : ''}
                disabled={busy}
                options={[
                  defaultOption,
                  ...effort.options.map((item) => ({ value: item.id, label: item.label })),
                ]}
                onChange={(value) =>
                  void save('defaultModelSelection', {
                    instanceId: selected.instanceId,
                    model: selected.model,
                    effort: value || undefined,
                  })
                }
              />
            </SettingRow>
          )}
          <SettingRow compact={compact} label={tx('notifications')}>
            <SingleSelectDropdown
              className={dropdownClass}
              menuWidth={224}
              ariaLabel={tx('notifications')}
              value={settings.notificationMode}
              disabled={busy}
              options={[
                { value: 'off', label: t('common.disabled') },
                { value: 'notifications', label: tx('desktop') },
                { value: 'sound', label: tx('sound') },
                { value: 'notifications-and-sound', label: tx('both') },
              ]}
              onChange={(value) => void save('notificationMode', value)}
            />
          </SettingRow>
        </>
      )}
      {feedback && (
        <div
          className={`t3-native-settings-feedback${compact ? ' is-compact qs-hint' : ' settings-row'}${error ? ' is-error' : ''}`}
          role={error ? 'alert' : 'status'}
        >
          <span>{feedback}</span>
          {!settings && error ? refreshBtn : null}
        </div>
      )}
    </>
  );
}
