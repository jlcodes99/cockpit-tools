import { useTranslation } from 'react-i18next';
import { SingleSelectDropdown } from '../../components/SingleSelectDropdown';
import { ALL_PLATFORM_IDS, PLATFORM_PAGE_MAP } from '../../types/platform';
import { getPlatformLabel } from '../../utils/platformMeta';
import { useAntigravityRuntimeTarget } from '../../hooks/useAntigravityRuntimeTarget';

export function StartupPageSetting({ value, onChange }: {
  value: string;
  onChange: (value: string) => void;
}) {
  const { t } = useTranslation();
  const runtimeTarget = useAntigravityRuntimeTarget();
  return (
    <div className="settings-row">
      <div className="row-label">
        <div className="row-title">{t('settings.general.startupPage')}</div>
        <div className="row-desc">{t('settings.general.startupPageDesc')}</div>
      </div>
      <div className="row-control">
        <SingleSelectDropdown
          value={value === 'overview'
            ? runtimeTarget === 'antigravity' ? 'antigravity' : 'antigravity-ide'
            : value}
          onChange={onChange}
          ariaLabel={t('settings.general.startupPage')}
          options={[
            { value: 'last', label: t('settings.general.startupPageLast') },
            { value: 'dashboard', label: t('nav.dashboard') },
            ...ALL_PLATFORM_IDS.map((platform) => ({
              value: platform === 'antigravity' ? 'antigravity'
                : platform === 'antigravity_ide' ? 'antigravity-ide' : PLATFORM_PAGE_MAP[platform],
              label: platform === 'codex_api_service'
                ? t('settings.general.startupPageCodexApi') : getPlatformLabel(platform, t),
            })),
            { value: 'instances', label: t('nav.instances') },
            { value: 'wakeup', label: t('nav.wakeup') },
            { value: '2fa', label: t('nav.2faManager') },
            { value: 'api-relay', label: t('nav.apiRelay') },
            { value: 'manual', label: t('nav.manual') },
            { value: 'settings', label: t('nav.settings') },
          ]}
        />
      </div>
    </div>
  );
}
