import { RefreshCw } from 'lucide-react';
import { T3codeNativeSettingsPanel } from '../../components/t3code/T3codeNativeSettings';
import type { SettingsGeneralPanelProps } from '../SettingsGeneralPanel';
import { PlatformSettingsSection } from './PlatformSettingsSection';

export function SettingsT3codePlatformPanel(props: SettingsGeneralPanelProps) {
  const {
    getResetLabelByTarget,
    handlePickAppPath,
    handleResetAppPath,
    isAppPathResetDetecting,
    platformSettingsOrder,
    setT3codeAppPath,
    setT3codeAppUiInjectionEnabled,
    t,
    t3codeAppPath,
    t3codeAppUiInjectionEnabled,
  } = props;

  return (
    <PlatformSettingsSection platformId="t3code" order={platformSettingsOrder.t3code}>
      <div className="group-title">{t('settings.general.t3codeTitle', 'T3 Code 设置')}</div>
      <div className="settings-group">
        <T3codeNativeSettingsPanel />

        <div className="settings-row">
          <div className="row-label">
            <div className="row-title">
              {t('settings.general.t3codeAppUiInjection')}
            </div>
            <div className="row-desc">
              {t('settings.general.t3codeAppUiInjectionDesc')}
            </div>
          </div>
          <div className="row-control">
            <label className="switch">
              <input
                type="checkbox"
                checked={t3codeAppUiInjectionEnabled}
                onChange={(e) => setT3codeAppUiInjectionEnabled(e.target.checked)}
              />
              <span className="slider"></span>
            </label>
          </div>
        </div>

        <div className="settings-row">
          <div className="row-label">
            <div className="row-title">{t('settings.general.t3codeAppPath', 'T3 Code 启动路径')}</div>
            <div className="row-desc">{t('settings.general.t3codeAppPathDesc', '留空则使用默认路径')}</div>
          </div>
          <div className="row-control row-control--grow">
            <div style={{ display: 'flex', gap: '8px', alignItems: 'center', width: '100%' }}>
              <input
                type="text"
                className="settings-input settings-input--path"
                value={t3codeAppPath}
                placeholder={t('settings.general.codexAppPathPlaceholder', '默认路径')}
                onChange={(e) => setT3codeAppPath(e.target.value)}
              />
              <button
                className="btn btn-secondary"
                onClick={() => setT3codeAppPath('')}
                disabled={isAppPathResetDetecting('t3code') || !t3codeAppPath.trim()}
              >
                {t('common.clear', '清除')}
              </button>
              <button
                className="btn btn-secondary"
                onClick={() => handlePickAppPath('t3code')}
                disabled={isAppPathResetDetecting('t3code')}
              >
                {t('settings.general.codexPathSelect', '选择')}
              </button>
              <button
                className="btn btn-secondary"
                onClick={() => handleResetAppPath('t3code')}
                disabled={isAppPathResetDetecting('t3code')}
              >
                <RefreshCw size={16} className={isAppPathResetDetecting('t3code') ? 'spin' : undefined} />
                {isAppPathResetDetecting('t3code')
                  ? t('common.loading', '加载中...')
                  : getResetLabelByTarget('t3code')}
              </button>
            </div>
          </div>
        </div>
      </div>
    </PlatformSettingsSection>
  );
}
