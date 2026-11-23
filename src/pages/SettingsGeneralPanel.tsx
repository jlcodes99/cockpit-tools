import { LaunchPathSettingsButton } from '../components/LaunchPathSettingsButton';
import { MenuBarLeftClickSetting } from '../components/MenuBarLeftClickSetting';
import { invoke } from '@tauri-apps/api/core';
import { normalizeLanguage } from '../i18n';
import { AutoSwitchAccountScopeSelector } from '../components/AutoSwitchAccountScopeSelector';
import { UI_SCALE_OPTION_STRINGS as UI_SCALE_OPTIONS } from '../utils/uiScale';
import { setClaudeQuotaDisplayRemainingEnabled } from '../utils/claudeQuotaDisplayPreference';
import { SettingsAccountTransferSection } from '../components/SettingsAccountTransferSection';
import type { SideNavLayoutMode } from '../stores/useSideNavLayoutStore';
import './settings/Settings.css';
import { Save, AlertCircle, RefreshCw } from 'lucide-react';
import type { SettingsPageViewProps } from "./SettingsPageView";
import { FloatingCardSettings, FloatingCardShowSetting } from "./settings/FloatingCardSettings";
import { StartupPageSetting } from "./settings/StartupPageSetting";
import { ThemeColorSetting } from './settings/ThemeColorSetting';
import { AppBackgroundSettings } from './settings/AppBackgroundSettings';
import { DataDirectorySetting } from './settings/DataDirectorySetting';
import { SettingsCodexPlatformPanel } from "./SettingsCodexPlatformPanel";
import { PlatformSettingsSection } from "./settings/PlatformSettingsSection";
import {
  SettingsDropdown,
  SettingsRefreshPresetDropdown,
  SettingsThresholdPresetDropdown,
} from "./settings/SettingsPresetDropdowns";
import { useUiConfigStore } from "../stores/useUiConfigStore";


/** 渲染 SettingsPageView 的 activeTab === 'general' 业务面板。 */
export function SettingsGeneralPanel(props: SettingsPageViewProps) {
  const {
    antigravityAccountGroups,
    antigravityAppPath,
    antigravityLegacyAppPath,
    antigravityDualSwitchNoRestartEnabled,
    antigravityLaunchOnSwitch,
    antigravityScopeAccounts,
    antigravityScopeTypeOptions,
    antigravitySeamlessSwitchUnlocked,
    appAutoLaunchEnabled,
    autoImportFromLocalEnabled,
    autoImportScanBusy,
    autoImportScanSeqRef,
    autoImportScanStatus,
    autoInstall,
    autoInstallLoaded,
    autoInstallTouchedRef,
    autoRefresh,
    autoRefreshCustomMode,
    autoRefreshIsPreset,
    autoSwitchAccountScopeMode,
    autoSwitchCreditsEnabled,
    autoSwitchCreditsThreshold,
    autoSwitchCreditsThresholdCustomMode,
    autoSwitchCreditsThresholdIsPreset,
    autoSwitchEnabled,
    autoSwitchSelectedAccountIds,
    autoSwitchThreshold,
    agWindowThresholdsEnabled,
    agFiveHourThreshold,
    agWeeklyThreshold,
    autoSwitchThresholdCustomMode,
    autoSwitchThresholdIsPreset,
    claudeAppPath,
    claudeAutoRefresh,
    claudeAutoRefreshCustomMode,
    claudeAutoRefreshIsPreset,
    claudeLaunchCandidates,
    claudeQuotaAlertEnabled,
    claudeQuotaAlertThreshold,
    claudeQuotaAlertThresholdCustomMode,
    claudeQuotaAlertThresholdIsPreset,
    claudeQuotaDisplayRemaining,
    claudeShareCodeSessionsOnSwitch,
    setClaudeShareCodeSessionsOnSwitch,
    closeBehavior,
    codebuddyAppPath,
    codebuddyAutoRefresh,
    codebuddyAutoRefreshCustomMode,
    codebuddyAutoRefreshIsPreset,
    codebuddyCnAppPath,
    codebuddyCnAutoRefresh,
    codebuddyCnAutoRefreshCustomMode,
    codebuddyCnAutoRefreshIsPreset,
    codebuddyCnQuotaAlertEnabled,
    codebuddyCnQuotaAlertThreshold,
    codebuddyCnQuotaAlertThresholdCustomMode,
    codebuddyCnQuotaAlertThresholdIsPreset,
    codebuddyCnShareSessionsOnSwitch,
    codebuddyQuotaAlertEnabled,
    codebuddyQuotaAlertThreshold,
    codebuddyQuotaAlertThresholdCustomMode,
    codebuddyQuotaAlertThresholdIsPreset,
    codebuddyShareSessionsOnSwitch,
    cursorAppPath,
    cursorAutoRefresh,
    cursorAutoRefreshCustomMode,
    cursorAutoRefreshIsPreset,
    cursorQuotaAlertEnabled,
    cursorQuotaAlertThreshold,
    cursorQuotaAlertThresholdCustomMode,
    cursorQuotaAlertThresholdIsPreset,
    defaultTerminal,
    errorReportingEnabled,
    errorReportingSaving,
    externalNetworkEnabled,
    floatingCardAlwaysOnTop,
    floatingCardShowOnStartup,
    generalLoaded,
    generalLoadFailed,
    getResetLabelByTarget,
    ghcpAutoRefresh,
    ghcpAutoRefreshCustomMode,
    ghcpAutoRefreshIsPreset,
    ghcpQuotaAlertEnabled,
    ghcpQuotaAlertThreshold,
    ghcpQuotaAlertThresholdCustomMode,
    ghcpQuotaAlertThresholdIsPreset,
    grokAutoRefresh,
    grokCliPath,
    grokCliSaving,
    grokCliStatus,
    grokCliStatusError,
    grokOpencodeAuthOverwriteOnSwitch,
    grokOpencodeSyncOnSwitch,
    grokQuotaAlertEnabled,
    grokQuotaAlertThreshold,
    grokSyncOfficialAuthOnSwitch,
    handleErrorReportingEnabledChange,
    handlePickAppPath,
    handleResetAppPath,
    handleSelectClaudeLaunchCandidate,
    handleSelectTraeLaunchCandidate,
    hasActiveResetTasks,
    hideDockIcon,
    isAppPathResetDetecting,
    isMacOS,
    isWindows,
    kiroAppPath,
    kiroAutoRefresh,
    kiroAutoRefreshCustomMode,
    kiroAutoRefreshIsPreset,
    kiroQuotaAlertEnabled,
    kiroQuotaAlertThreshold,
    kiroQuotaAlertThresholdCustomMode,
    kiroQuotaAlertThresholdIsPreset,
    language,
    languageOptions,
    loadGeneralConfig,
    loadUpdateSettings,
    menuBarQuotaEnabled,
    menuBarLeftClickOpensMainWindow,
    setMenuBarLeftClickOpensMainWindow,
    menuBarQuotaPlatform,
    menuBarQuotaPlatformOptions,
    normalizeNumberInput,
    openMenuBarQuotaModal,
    platformSettingsOrder,
    qoderAppPath,
    qoderAutoRefresh,
    qoderAutoRefreshCustomMode,
    qoderAutoRefreshIsPreset,
    qoderQuotaAlertEnabled,
    qoderQuotaAlertThreshold,
    qoderQuotaAlertThresholdCustomMode,
    qoderQuotaAlertThresholdIsPreset,
    quotaAlertEnabled,
    quotaAlertThreshold,
    quotaAlertThresholdCustomMode,
    quotaAlertThresholdIsPreset,
    reducedMotionEnabled,
    REFRESH_PRESET_VALUES,
    rememberMainWindowState,
    renderAccountLevelRefreshConfig,
    renderCurrentAccountRefreshRow,
    renderPlatformAutoRefreshRow,
    renderPlatformQuotaAlertRows,
    renderSessionSharingRow,
    renderTraeVariantSettingsGroup,
    sanitizeNumberInput,
    saveGrokCliPath,
    setAntigravityAppPath,
    setAntigravityLegacyAppPath,
    setAntigravityDualSwitchNoRestartEnabled,
    setAntigravityLaunchOnSwitch,
    setAppAutoLaunchEnabled,
    setAutoImportFromLocalEnabled,
    setAutoImportScanBusy,
    setAutoImportScanStatus,
    setAutoInstall,
    setAutoRefresh,
    setAutoRefreshCustomMode,
    setAutoSwitchAccountScopeMode,
    setAutoSwitchCreditsEnabled,
    setAutoSwitchCreditsThreshold,
    setAutoSwitchCreditsThresholdCustomMode,
    setAutoSwitchEnabled,
    setAutoSwitchSelectedAccountIds,
    setAutoSwitchThreshold,
    setAgWindowThresholdsEnabled,
    setAgFiveHourThreshold,
    setAgWeeklyThreshold,
    setAutoSwitchThresholdCustomMode,
    setClaudeAppPath,
    setClaudeAutoRefresh,
    setClaudeAutoRefreshCustomMode,
    setClaudeQuotaAlertEnabled,
    setClaudeQuotaAlertThreshold,
    setClaudeQuotaAlertThresholdCustomMode,
    setClaudeQuotaDisplayRemaining,
    setCloseBehavior,
    setCodebuddyAppPath,
    setCodebuddyAutoRefresh,
    setCodebuddyAutoRefreshCustomMode,
    setCodebuddyCnAppPath,
    setCodebuddyCnAutoRefresh,
    setCodebuddyCnAutoRefreshCustomMode,
    setCodebuddyCnQuotaAlertEnabled,
    setCodebuddyCnQuotaAlertThreshold,
    setCodebuddyCnQuotaAlertThresholdCustomMode,
    setCodebuddyCnShareSessionsOnSwitch,
    setCodebuddyQuotaAlertEnabled,
    setCodebuddyQuotaAlertThreshold,
    setCodebuddyQuotaAlertThresholdCustomMode,
    setCodebuddyShareSessionsOnSwitch,
    setCursorAppPath,
    setCursorAutoRefresh,
    setCursorAutoRefreshCustomMode,
    setCursorQuotaAlertEnabled,
    setCursorQuotaAlertThreshold,
    setCursorQuotaAlertThresholdCustomMode,
    setDefaultTerminal,
    setExternalNetworkEnabled,
    setFloatingCardAlwaysOnTop,
    setFloatingCardShowOnStartup,
    setGhcpAutoRefresh,
    setGhcpAutoRefreshCustomMode,
    setGhcpQuotaAlertEnabled,
    setGhcpQuotaAlertThreshold,
    setGhcpQuotaAlertThresholdCustomMode,
    setGrokAutoRefresh,
    setGrokCliPath,
    setGrokCliStatusError,
    setGrokOpencodeAuthOverwriteOnSwitch,
    setGrokOpencodeSyncOnSwitch,
    setGrokQuotaAlertEnabled,
    setGrokQuotaAlertThreshold,
    setGrokSyncOfficialAuthOnSwitch,
    setHideDockIcon,
    setShowTrayIcon,
    setKiroAppPath,
    setKiroAutoRefresh,
    setKiroAutoRefreshCustomMode,
    setKiroQuotaAlertEnabled,
    setKiroQuotaAlertThreshold,
    setKiroQuotaAlertThresholdCustomMode,
    setLanguage,
    setMenuBarQuotaEnabled,
    setQoderAppPath,
    setQoderAutoRefresh,
    setQoderAutoRefreshCustomMode,
    setQoderQuotaAlertEnabled,
    setQoderQuotaAlertThreshold,
    setQoderQuotaAlertThresholdCustomMode,
    setQuotaAlertEnabled,
    setQuotaAlertThreshold,
    setQuotaAlertThresholdCustomMode,
    setReducedMotionEnabled,
    setRememberMainWindowState,
    setSideNavLayoutMode,
    setStartupMinimized,
    setStartupPage,
    setTheme,
    setThemeColor,
    setTokenKeeperEnabled,
    setTopRightAdVisible,
    setTraeAppPath,
    setTraeAutoRefresh,
    setTraeAutoRefreshCustomMode,
    setTraeCnAutoRefresh,
    setTraeCnAutoRefreshCustomMode,
    setTraeCnQuotaAlertEnabled,
    setTraeCnQuotaAlertThreshold,
    setTraeCnQuotaAlertThresholdCustomMode,
    setTraeQuotaAlertEnabled,
    setTraeQuotaAlertThreshold,
    setTraeQuotaAlertThresholdCustomMode,
    setTraeSoloAutoRefresh,
    setTraeSoloAutoRefreshCustomMode,
    setTraeSoloCnAutoRefresh,
    setTraeSoloCnAutoRefreshCustomMode,
    setTraeSoloCnQuotaAlertEnabled,
    setTraeSoloCnQuotaAlertThreshold,
    setTraeSoloCnQuotaAlertThresholdCustomMode,
    setTraeSoloQuotaAlertEnabled,
    setTraeSoloQuotaAlertThreshold,
    setTraeSoloQuotaAlertThresholdCustomMode,
    setTrayIconStyle,
    setUiScale,
    setUpdateRemindersEnabled,
    setVscodeAppPath,
    setWebdavAllowedDomains,
    setWindsurfAppPath,
    setWindsurfAutoRefresh,
    setWindsurfAutoRefreshCustomMode,
    setWindsurfQuotaAlertEnabled,
    setWindsurfQuotaAlertThreshold,
    setWindsurfQuotaAlertThresholdCustomMode,
    setWorkbuddyAppPath,
    setWorkbuddyAutoRefresh,
    setWorkbuddyAutoRefreshCustomMode,
    setWorkbuddyQuotaAlertEnabled,
    setWorkbuddyQuotaAlertThreshold,
    setWorkbuddyQuotaAlertThresholdCustomMode,
    setWorkbuddyShareSessionsOnSwitch,
    setZcodeAppPath,
    setZcodeAutoRefresh,
    setZcodeAutoRefreshCustomMode,
    setZedAppPath,
    setZedAutoRefresh,
    setZedAutoRefreshCustomMode,
    setZedQuotaAlertEnabled,
    setZedQuotaAlertThreshold,
    setZedQuotaAlertThresholdCustomMode,
    sideNavLayoutMode,
    startupMinimized,
    startupPage,
    t,
    terminalOptions,
    theme,
    themeColor,
    THRESHOLD_PRESET_VALUES,
    tokenKeeperEnabled,
    topRightAdVisible,
    traeAppPath,
    traeAutoRefresh,
    traeAutoRefreshCustomMode,
    traeAutoRefreshIsPreset,
    traeCnAutoRefresh,
    traeCnAutoRefreshCustomMode,
    traeCnAutoRefreshIsPreset,
    traeCnQuotaAlertEnabled,
    traeCnQuotaAlertThreshold,
    traeCnQuotaAlertThresholdCustomMode,
    traeCnQuotaAlertThresholdIsPreset,
    traeLaunchCandidates,
    traeLaunchCandidatesTarget,
    traeQuotaAlertEnabled,
    traeQuotaAlertThreshold,
    traeQuotaAlertThresholdCustomMode,
    traeQuotaAlertThresholdIsPreset,
    traeSoloAutoRefresh,
    traeSoloAutoRefreshCustomMode,
    traeSoloAutoRefreshIsPreset,
    traeSoloCnAutoRefresh,
    traeSoloCnAutoRefreshCustomMode,
    traeSoloCnAutoRefreshIsPreset,
    traeSoloCnQuotaAlertEnabled,
    traeSoloCnQuotaAlertThreshold,
    traeSoloCnQuotaAlertThresholdCustomMode,
    traeSoloCnQuotaAlertThresholdIsPreset,
    traeSoloQuotaAlertEnabled,
    traeSoloQuotaAlertThreshold,
    traeSoloQuotaAlertThresholdCustomMode,
    traeSoloQuotaAlertThresholdIsPreset,
    trayIconStyle,
    showTrayIcon,
    uiScale,
    updateRemindersEnabled,
    updateRemindersLoaded,
    updateRemindersTouchedRef,
    updateSettingsLoadFailed,
    vscodeAppPath,
    webdavAllowedDomains,
    windsurfAppPath,
    windsurfAutoRefresh,
    windsurfAutoRefreshCustomMode,
    windsurfAutoRefreshIsPreset,
    windsurfQuotaAlertEnabled,
    windsurfQuotaAlertThreshold,
    windsurfQuotaAlertThresholdCustomMode,
    windsurfQuotaAlertThresholdIsPreset,
    workbuddyAppPath,
    workbuddyAutoRefresh,
    workbuddyAutoRefreshCustomMode,
    workbuddyAutoRefreshIsPreset,
    workbuddyQuotaAlertEnabled,
    workbuddyQuotaAlertThreshold,
    workbuddyQuotaAlertThresholdCustomMode,
    workbuddyQuotaAlertThresholdIsPreset,
    workbuddyShareSessionsOnSwitch,
    minimaxCodeAppPath,
    minimaxCodeAutoRefresh,
    minimaxCodeAutoRefreshCustomMode,
    minimaxCodeAutoRefreshIsPreset,
    minimaxCodeQuotaAlertEnabled,
    minimaxCodeQuotaAlertThreshold,
    minimaxCodeQuotaAlertThresholdCustomMode,
    minimaxCodeQuotaAlertThresholdIsPreset,
    setMinimaxCodeAppPath,
    setMinimaxCodeAutoRefresh,
    setMinimaxCodeAutoRefreshCustomMode,
    setMinimaxCodeQuotaAlertEnabled,
    setMinimaxCodeQuotaAlertThreshold,
    setMinimaxCodeQuotaAlertThresholdCustomMode,
    zcodeAppPath,
    zcodeAutoRefresh,
    zcodeAutoRefreshCustomMode,
    zcodeAutoRefreshIsPreset,
    zedAppPath,
    zedAutoRefresh,
    zedAutoRefreshCustomMode,
    zedAutoRefreshIsPreset,
    zedQuotaAlertEnabled,
    zedQuotaAlertThreshold,
    zedQuotaAlertThresholdCustomMode,
    zedQuotaAlertThresholdIsPreset,
  } = props;

  const { timeDisplayMode, setTimeDisplayMode } = useUiConfigStore();
  return (
          <>
          {(generalLoadFailed || updateSettingsLoadFailed) && (
            <div className="settings-load-error" role="alert">
              <AlertCircle size={16} />
              <span>{t('common.failed')}</span>
              <button
                type="button"
                className="btn btn-secondary"
                onClick={() => {
                  if (generalLoadFailed) void loadGeneralConfig();
                  if (updateSettingsLoadFailed) void loadUpdateSettings();
                }}
              >
                <RefreshCw size={14} />
                {t('common.refresh')}
              </button>
            </div>
          )}
          <SettingsAccountTransferSection directoryEntryOnly />
          <AppBackgroundSettings />
          <fieldset
            className="settings-general-fieldset"
            disabled={!generalLoaded}
            aria-busy={!generalLoaded}
          >
            <div className="group-title">{t('settings.general.commonTitle', '通用')}</div>
            <div className="settings-group">
              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.language')}</div>
                  <div className="row-desc">{t('settings.general.languageDesc')}</div>
                </div>
                <div className="row-control">
                  <SettingsDropdown
                    value={language}
                    options={languageOptions.map((option) => ({
                      value: option.value,
                      label: option.label,
                    }))}
                    onChange={(nextValue) => setLanguage(normalizeLanguage(nextValue))}
                    ariaLabel={t('settings.general.language')}
                  />
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.theme')}</div>
                  <div className="row-desc">{t('settings.general.themeDesc')}</div>
                </div>
                <div className="row-control">
                  <SettingsDropdown
                    value={theme}
                    options={[
                      { value: 'light', label: t('settings.general.themeLight') },
                      { value: 'dark', label: t('settings.general.themeDark') },
                      { value: 'system', label: t('settings.general.themeSystem') },
                    ]}
                    onChange={setTheme}
                    ariaLabel={t('settings.general.theme')}
                  />
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">
                    {t('settings.general.reducedMotion', '减少动画')}
                  </div>
                  <div className="row-desc">
                    {t(
                      'settings.general.reducedMotionDesc',
                      '降低页面淡入、弹层过渡、阴影、模糊和平滑滚动，仅保留必要加载反馈'
                    )}
                  </div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={reducedMotionEnabled}
                      onChange={(event) => setReducedMotionEnabled(event.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.defaultTerminal', '默认终端')}</div>
                  <div className="row-desc">{t('settings.general.defaultTerminalDesc', 'CLI 打开时使用的终端')}</div>
                </div>
                <div className="row-control">
                  <SettingsDropdown
                    value={defaultTerminal}
                    options={terminalOptions.map((option) => ({
                      value: option.value,
                      label: option.label,
                    }))}
                    onChange={setDefaultTerminal}
                    ariaLabel={t('settings.general.defaultTerminal', '默认终端')}
                  />
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.sideNavLayout', '侧边栏布局')}</div>
                  <div className="row-desc">{t('settings.general.sideNavLayoutDesc', '切换原始布局或经典布局')}</div>
                </div>
                <div className="row-control">
                  <SettingsDropdown
                    value={sideNavLayoutMode}
                    options={[
                      { value: 'original', label: t('settings.general.sideNavLayoutOriginal', '原始布局') },
                      { value: 'classic', label: t('settings.general.sideNavLayoutClassic', '经典布局') },
                    ]}
                    onChange={(nextValue) => setSideNavLayoutMode(nextValue as SideNavLayoutMode)}
                    ariaLabel={t('settings.general.sideNavLayout', '侧边栏布局')}
                  />
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.uiScale')}</div>
                  <div className="row-desc">{t('settings.general.uiScaleDesc')}</div>
                </div>
                <div className="row-control">
                  <SettingsDropdown
                    value={uiScale}
                    options={UI_SCALE_OPTIONS.map((value) => ({
                      value,
                      label: `${Math.round(Number.parseFloat(value) * 100)}%`,
                    }))}
                    onChange={setUiScale}
                    ariaLabel={t('settings.general.uiScale')}
                  />
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.closeBehavior')}</div>
                  <div className="row-desc">{t('settings.general.closeBehaviorDesc')}</div>
                </div>
                <div className="row-control">
                  <SettingsDropdown
                    value={closeBehavior}
                    options={[
                      { value: 'ask', label: t('settings.general.closeBehaviorAsk') },
                      {
                        value: 'minimize',
                        label: t(showTrayIcon ? 'settings.general.closeBehaviorMinimize' : 'closeDialog.minimizeWindow'),
                      },
                      { value: 'quit', label: t('settings.general.closeBehaviorQuit') },
                    ]}
                    onChange={(nextValue) => setCloseBehavior(nextValue as 'ask' | 'minimize' | 'quit')}
                    ariaLabel={t('settings.general.closeBehavior')}
                  />
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">
                    {t('settings.general.startupMinimized', '启动后自动最小化')}
                  </div>
                  <div className="row-desc">
                    {t(
                      'settings.general.startupMinimizedDesc',
                      '应用启动完成后自动最小化主窗口，可从 Dock、任务栏或托盘恢复'
                    )}
                  </div>
                </div>
                <div className="row-control">
                  <SettingsDropdown
                    value={startupMinimized ? 'true' : 'false'}
                    options={[
                      { value: 'false', label: t('common.disable', '停用') },
                      { value: 'true', label: t('common.enable', '启用') },
                    ]}
                    onChange={(nextValue) => setStartupMinimized(nextValue === 'true')}
                    ariaLabel={t('settings.general.startupMinimized', '启动后自动最小化')}
                  />
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">
                    {t('settings.general.rememberMainWindowState', '记住主窗口位置和大小')}
                  </div>
                  <div className="row-desc">
                    {t(
                      'settings.general.rememberMainWindowStateDesc',
                      '重启或从托盘重新打开时恢复主窗口位置和大小；默认关闭'
                    )}
                  </div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={rememberMainWindowState}
                      onChange={(event) => setRememberMainWindowState(event.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.timeDisplayMode', '时间显示')}</div>
                  <div className="row-desc">{t('settings.general.timeDisplayModeDesc', '账号卡片与列表中的时间区域显示内容（默认切换时间）')}</div>
                </div>
                <div className="row-control">
                  <SettingsDropdown
                    value={timeDisplayMode}
                    options={[
                      { value: 'switch', label: t('accounts.lastSwitchTime', '切换时间') },
                      { value: 'refresh', label: t('accounts.refreshTime', '刷新时间') },
                    ]}
                    onChange={(nextValue) => setTimeDisplayMode(nextValue as 'switch' | 'refresh')}
                    ariaLabel={t('settings.general.timeDisplayMode', '时间显示')}
                  />
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.autoUpdate')}</div>
                  <div className="row-desc">{t('settings.general.autoUpdateDesc')}</div>
                </div>
                <div className="row-control">
                  <SettingsDropdown
                    value={autoInstall ? 'true' : 'false'}
                    disabled={!autoInstallLoaded}
                    options={[
                      { value: 'false', label: t('settings.general.autoUpdateOff') },
                      { value: 'true', label: t('settings.general.autoUpdateOn') },
                    ]}
                    onChange={(nextValue) => {
                      autoInstallTouchedRef.current = true;
                      setAutoInstall(nextValue === 'true');
                    }}
                    ariaLabel={t('settings.general.autoUpdate')}
                  />
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.updateReminder')}</div>
                  <div className="row-desc">{t('settings.general.updateReminderDesc')}</div>
                </div>
                <div className="row-control">
                  <SettingsDropdown
                    value={updateRemindersEnabled ? 'true' : 'false'}
                    disabled={!updateRemindersLoaded}
                    options={[
                      { value: 'true', label: t('settings.general.updateReminderOn') },
                      { value: 'false', label: t('settings.general.updateReminderOff') },
                    ]}
                    onChange={(nextValue) => {
                      updateRemindersTouchedRef.current = true;
                      setUpdateRemindersEnabled(nextValue === 'true');
                    }}
                    ariaLabel={t('settings.general.updateReminder')}
                  />
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.showTrayIcon')}</div>
                  <div className="row-desc">{t('settings.general.showTrayIconDesc')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      aria-label={t('settings.general.showTrayIcon')}
                      checked={showTrayIcon}
                      onChange={(event) => setShowTrayIcon(event.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>

              {isMacOS && (
                <>
                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">
                        {t('settings.general.hideDockIcon', '是否隐藏Dock图标（仅 macOS）')}
                      </div>
                      <div className="row-desc">
                        {t(
                          'settings.general.hideDockIconDesc',
                          '独立控制程序坞图标显示状态，不受窗口最小化行为影响'
                        )}
                      </div>
                    </div>
                    <div className="row-control">
                      <SettingsDropdown
                        value={hideDockIcon ? 'true' : 'false'}
                        options={[
                          { value: 'false', label: t('settings.general.hideDockIconOff', '否（显示Dock图标）') },
                          { value: 'true', label: t('settings.general.hideDockIconOn', '是（隐藏Dock图标）') },
                        ]}
                        onChange={(nextValue) => setHideDockIcon(nextValue === 'true')}
                        ariaLabel={t('settings.general.hideDockIcon', '是否隐藏Dock图标（仅 macOS）')}
                      />
                    </div>
                  </div>

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">
                        {t('settings.general.trayIconStyle', '菜单栏图标样式（仅 macOS）')}
                      </div>
                      <div className="row-desc">
                        {t(
                          'settings.general.trayIconStyleDesc',
                          '选择系统单色图标或原彩色 App 图标'
                        )}
                      </div>
                    </div>
                    <div className="row-control">
                      <SettingsDropdown
                        value={trayIconStyle}
                        disabled={!showTrayIcon}
                        options={[
                          { value: 'template', label: t('settings.general.trayIconStyleTemplate', '单色图标') },
                          { value: 'color', label: t('settings.general.trayIconStyleColor', '彩色图标') },
                        ]}
                        onChange={(nextValue) =>
                          setTrayIconStyle(nextValue === 'color' ? 'color' : 'template')
                        }
                        ariaLabel={t('settings.general.trayIconStyle', '菜单栏图标样式（仅 macOS）')}
                      />
                    </div>
                  </div>

                  <MenuBarLeftClickSetting
                    opensMainWindow={menuBarLeftClickOpensMainWindow}
                    disabled={!showTrayIcon}
                    onChange={setMenuBarLeftClickOpensMainWindow}
                  />

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">
                        {t('settings.general.menuBarQuota', '菜单栏显示实时额度')}
                      </div>
                      <div className="row-desc">
                        {showTrayIcon && menuBarQuotaEnabled
                          ? t(
                              'settings.general.menuBarQuotaEnabledDesc',
                              '已启用 · {{platform}} · 显示该平台当前账号的可用额度',
                              {
                                platform:
                                  menuBarQuotaPlatformOptions.find(
                                    (option) => option.value === menuBarQuotaPlatform
                                  )?.label ?? menuBarQuotaPlatform,
                              }
                            )
                          : t(
                              'settings.general.menuBarQuotaDesc',
                              '启用后在菜单栏图标旁显示所选平台当前账号的剩余额度；平台等专属选项在弹框中配置'
                            )}
                      </div>
                    </div>
                    <div className="row-control">
                      <SettingsDropdown
                        value={showTrayIcon && menuBarQuotaEnabled ? 'true' : 'false'}
                        disabled={!showTrayIcon}
                        options={[
                          { value: 'false', label: t('common.disable', '停用') },
                          { value: 'true', label: t('common.enable', '启用') },
                        ]}
                        onChange={(nextValue) => {
                          if (nextValue === 'true') {
                            openMenuBarQuotaModal(menuBarQuotaEnabled ? 'edit' : 'enable');
                            return;
                          }
                          setMenuBarQuotaEnabled(false);
                        }}
                        ariaLabel={t('settings.general.menuBarQuota', '菜单栏显示实时额度')}
                      />
                      {showTrayIcon && menuBarQuotaEnabled ? (
                        <button
                          type="button"
                          className="btn btn-secondary"
                          onClick={() => openMenuBarQuotaModal('edit')}
                        >
                          {t('settings.general.menuBarQuotaConfigure', '配置')}
                        </button>
                      ) : null}
                    </div>
                  </div>
                </>
              )}

              <FloatingCardSettings
                startup={floatingCardShowOnStartup} onTop={floatingCardAlwaysOnTop}
                onStartupChange={setFloatingCardShowOnStartup} onTopChange={setFloatingCardAlwaysOnTop}
              />

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.appAutoLaunch')}</div>
                  <div className="row-desc">{t('settings.general.appAutoLaunchDesc')}</div>
                </div>
                <div className="row-control">
                  <SettingsDropdown
                    value={appAutoLaunchEnabled ? 'true' : 'false'}
                    options={[
                      { value: 'false', label: t('common.disable', '停用') },
                      { value: 'true', label: t('common.enable', '启用') },
                    ]}
                    onChange={(nextValue) => setAppAutoLaunchEnabled(nextValue === 'true')}
                    ariaLabel={t('settings.general.appAutoLaunch')}
                  />
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">
                    {t('settings.general.tokenKeeper', '后台授权保活')}
                  </div>
                  <div className="row-desc">
                    {t(
                      'settings.general.tokenKeeperDesc',
                      '仅在授权快过期时分批刷新账号 Token，降低大量账号场景下的后台请求压力。',
                    )}
                  </div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={tokenKeeperEnabled}
                      onChange={(e) => setTokenKeeperEnabled(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">
                    {t('settings.general.autoImportFromLocal', '本机账号自动导入')}
                  </div>
                  <div className="row-desc">
                    {t(
                      'settings.general.autoImportFromLocalDesc',
                      '开启后会立即扫描本机客户端登录状态并导入当前账号；之后当官方客户端更换登录时自动导入。首次可能弹出系统钥匙串授权，请选择「始终允许」。',
                    )}
                    {autoImportScanStatus ? (
                      <div
                        className="row-desc-extra"
                        style={{
                          marginTop: 6,
                          opacity: autoImportScanBusy ? 0.9 : 1,
                        }}
                      >
                        {autoImportScanStatus}
                      </div>
                    ) : null}
                  </div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={autoImportFromLocalEnabled}
                      onChange={(e) => {
                        const enabled = e.target.checked;
                        setAutoImportFromLocalEnabled(enabled);
                        if (!enabled) {
                          // 作废进行中的扫描结果，避免关了之后又刷回完成文案
                          autoImportScanSeqRef.current += 1;
                          setAutoImportScanBusy(false);
                          setAutoImportScanStatus(
                            autoImportScanBusy
                              ? t(
                                  'settings.general.autoImportFromLocalScanCancelled',
                                  '已关闭，后台扫描将忽略结果',
                                )
                              : '',
                          );
                          return;
                        }
                        const scanSeq = ++autoImportScanSeqRef.current;
                        setAutoImportScanBusy(true);
                        setAutoImportScanStatus(
                          t(
                            'settings.general.autoImportFromLocalScanning',
                            '正在扫描本机账号，可随时关闭…',
                          ),
                        );
                        void (async () => {
                          try {
                            // 等 patch 自动保存生效后再扫（debounce 300ms）
                            await new Promise((resolve) => window.setTimeout(resolve, 450));
                            if (autoImportScanSeqRef.current !== scanSeq) {
                              return;
                            }
                            const result = await invoke<{
                              scanned: number;
                              imported: number;
                              failed: number;
                              platforms: string[];
                            }>('scan_auto_local_import');
                            if (autoImportScanSeqRef.current !== scanSeq) {
                              return;
                            }
                            if (result.imported > 0) {
                              setAutoImportScanStatus(
                                t('settings.general.autoImportFromLocalScanDone', {
                                  imported: result.imported,
                                  scanned: result.scanned,
                                  defaultValue:
                                    '扫描完成：发现 {{scanned}} 个本机登录，已导入 {{imported}} 个',
                                }),
                              );
                            } else if (result.failed > 0) {
                              setAutoImportScanStatus(
                                t(
                                  'settings.general.autoImportFromLocalScanPartial',
                                  '扫描完成：未成功导入，部分平台失败，可到账号页手动导入',
                                ),
                              );
                            } else {
                              setAutoImportScanStatus(
                                t(
                                  'settings.general.autoImportFromLocalScanEmpty',
                                  '扫描完成：未发现可导入的本机登录',
                                ),
                              );
                            }
                          } catch (err) {
                            if (autoImportScanSeqRef.current !== scanSeq) {
                              return;
                            }
                            console.error('本机账号自动导入扫描失败:', err);
                            setAutoImportScanStatus(
                              t('settings.general.autoImportFromLocalScanFailed', {
                                error: String(err),
                                defaultValue: '扫描失败：{{error}}',
                              }),
                            );
                          } finally {
                            if (autoImportScanSeqRef.current === scanSeq) {
                              setAutoImportScanBusy(false);
                            }
                          }
                        })();
                      }}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">
                    {t('settings.general.errorReporting')}
                  </div>
                  <div className="row-desc">
                    {t('settings.general.errorReportingDesc')}
                  </div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={errorReportingEnabled}
                      disabled={errorReportingSaving}
                      onChange={(e) => void handleErrorReportingEnabledChange(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>

              <FloatingCardShowSetting />

              <DataDirectorySetting />

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">
                    {t('settings.general.topRightAdVisible', '显示顶部推广')}
                  </div>
                  <div className="row-desc">
                    {t(
                      'settings.general.topRightAdVisibleDesc',
                      '关闭后隐藏应用顶部推广位。'
                    )}
                  </div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={topRightAdVisible}
                      onChange={(e) => setTopRightAdVisible(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>

              <StartupPageSetting value={startupPage} onChange={setStartupPage} />

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">
                    {t('settings.general.themeColor', '主题色套件')}
                  </div>
                  <div className="row-desc">
                    {t(
                      'settings.general.themeColorDesc',
                      '在浅色/深色之上叠加配色包（Nord、Tokyo Night 等）'
                    )}
                  </div>
                </div>
                <div className="row-control">
                  <ThemeColorSetting value={themeColor} onChange={setThemeColor} />
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">
                    {t('settings.general.externalNetwork', '允许外连')}
                  </div>
                  <div className="row-desc">
                    {t(
                      'settings.general.externalNetworkDesc',
                      '关闭后阻断 WebDAV 同步与 OpenRouter 用量刷新（不影响应用更新等其他网络）'
                    )}
                  </div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={externalNetworkEnabled}
                      onChange={(e) => setExternalNetworkEnabled(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">
                    {t('settings.general.webdavAllowedDomains', 'WebDAV 域名白名单')}
                  </div>
                  <div className="row-desc">
                    {t(
                      'settings.general.webdavAllowedDomainsDesc',
                      '逗号分隔；留空不限制。非空时同步 URL 主机必须匹配'
                    )}
                  </div>
                </div>
                <div className="row-control">
                  <input
                    className="settings-input"
                    value={webdavAllowedDomains}
                    onChange={(e) => setWebdavAllowedDomains(e.target.value)}
                    placeholder="example.com, dav.example.org"
                  />
                </div>
              </div>
            </div>
            <div style={{ display: 'flex', flexDirection: 'column' }}>
              <PlatformSettingsSection platformId={'antigravity'} order={platformSettingsOrder.antigravity}>
                <div className="group-title">{t('settings.general.antigravitySettingsTitle', 'Antigravity IDE 设置')}</div>
                <div className="settings-group">
                  <div className="settings-row"><LaunchPathSettingsButton app="antigravity_cli" /></div>
              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.autoRefresh')}</div>
                  <div className="row-desc">{t('settings.general.autoRefreshDesc')}</div>
                </div>
                <div className="row-control">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                    {autoRefreshCustomMode ? (
                      <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                        <input
                          type="number"
                          min={1}
                          max={999}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={autoRefresh}
                          placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                          onChange={(e) => setAutoRefresh(sanitizeNumberInput(e.target.value))}
                        onBlur={() => {
                          const normalized = normalizeNumberInput(autoRefresh, 1, 999);
                          setAutoRefresh(normalized);
                          setAutoRefreshCustomMode(false);
                        }}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') {
                            e.preventDefault();
                            const normalized = normalizeNumberInput(autoRefresh, 1, 999);
                            setAutoRefresh(normalized);
                            setAutoRefreshCustomMode(false);
                          }
                        }}
                      />
                        <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                      </div>
                    ) : (
                      <SettingsRefreshPresetDropdown
                        value={autoRefresh}
                        isPreset={autoRefreshIsPreset}
                        setValue={setAutoRefresh}
                        setCustomMode={setAutoRefreshCustomMode}
                        disabledNonPreset={hasActiveResetTasks}
                        ariaLabel={t('settings.general.autoRefresh')}
                      />
                    )}
                  </div>
                  
                  {hasActiveResetTasks && (
                    <div style={{
                      display: 'flex',
                      alignItems: 'flex-start',
                      gap: '8px',
                      padding: '12px',
                      marginTop: '8px',
                      background: 'rgba(59, 130, 246, 0.1)',
                      borderRadius: '8px',
                      fontSize: '13px',
                      color: 'var(--accent)',
                      lineHeight: '1.5'
                    }}>
                      <AlertCircle size={16} style={{ marginTop: '2px', flexShrink: 0 }} />
                      <span>{t('settings.general.refreshIntervalLimited')}</span>
                    </div>
                  )}
                </div>
              </div>

	              {renderCurrentAccountRefreshRow('antigravity')}
	              {renderAccountLevelRefreshConfig('antigravity')}

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">
                    {t(
                      'settings.general.antigravityLaunchOnSwitch',
                      '切换时启动 Antigravity',
                    )}
                  </div>
                  <div className="row-desc">
                    {t(
                      'antigravityLaunch.quickSwitchSetting',
                    )}
                  </div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={antigravityLaunchOnSwitch}
                      onChange={(e) => setAntigravityLaunchOnSwitch(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>

              {([
                ['antigravity_legacy', 'Antigravity', antigravityLegacyAppPath, setAntigravityLegacyAppPath],
                ['antigravity', 'Antigravity IDE', antigravityAppPath, setAntigravityAppPath],
              ] as const).map(([target, appName, path, setPath]) => (
                <div className="settings-row" key={target}>
                  <div className="row-label">
                    <div className="row-title">{t('settings.general.antigravityAppPath', { appName })}</div>
                    <div className="row-desc">{t('settings.general.codexAppPathDesc', '留空则使用默认路径')}</div>
                  </div>
                  <div className="row-control row-control--grow">
                    <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                      <input
                        type="text"
                        className="settings-input settings-input--path"
                        value={path}
                        placeholder={t('settings.general.codexAppPathPlaceholder', '默认路径')}
                        onChange={(e) => setPath(e.target.value)}
                      />
                      <button
                        className="btn btn-secondary"
                        onClick={() => handlePickAppPath(target)}
                        disabled={isAppPathResetDetecting(target)}
                      >
                        {t('settings.general.codexPathSelect', '选择')}
                      </button>
                      <button
                        className="btn btn-secondary"
                        onClick={() => handleResetAppPath(target)}
                        disabled={isAppPathResetDetecting(target)}
                      >
                        <RefreshCw size={16} className={isAppPathResetDetecting(target) ? 'spin' : undefined} />
                        {isAppPathResetDetecting(target)
                          ? t('common.loading', '加载中...')
                          : getResetLabelByTarget(target)}
                      </button>
                    </div>
                  </div>
                </div>
              ))}

              {antigravitySeamlessSwitchUnlocked && (
                <div className="settings-row">
                  <div className="row-label">
                    <div className="row-title">
                      {t(
                        'settings.general.antigravityDualSwitchNoRestart',
                        '无感双通道切号（不重启）'
                      )}
                    </div>
                    <div className="row-desc">
                      {t(
                        'settings.general.antigravityDualSwitchNoRestartDesc',
                        '切号时同时执行本地落盘与扩展无感切号，不再自动重启 Antigravity IDE。'
                      )}
                    </div>
                  </div>
                  <div className="row-control">
                    <label className="switch">
                      <input
                        type="checkbox"
                        checked={antigravityDualSwitchNoRestartEnabled}
                        onChange={(e) =>
                          setAntigravityDualSwitchNoRestartEnabled(e.target.checked)
                        }
                      />
                      <span className="slider"></span>
                    </label>
                  </div>
                </div>
              )}

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('quickSettings.autoSwitch.enable', '自动切号')}</div>
                  <div className="row-desc">{t('quickSettings.autoSwitch.hint', '命中监控的模型分组阈值时会自动切号；启用 Credits 监控后，剩余 Credits 低于阈值时也会触发。')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={autoSwitchEnabled}
                      onChange={(e) => setAutoSwitchEnabled(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>
              {autoSwitchEnabled && (
                <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                  <div className="row-label">
                    <div className="row-title">{t('quickSettings.autoSwitch.threshold', '切号阈值')}</div>
                    <div className="row-desc">{t('quickSettings.autoSwitch.thresholdDesc', '任意模型配额低于此百分比时触发自动切号')}</div>
                  </div>
                  <div className="row-control">
                    {autoSwitchThresholdCustomMode ? (
                      <div className="settings-inline-input">
                        <input
                          type="number"
                          min={0}
                          max={100}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={autoSwitchThreshold}
                          placeholder={t('quickSettings.inputPercent', '输入百分比')}
                          onChange={(e) => setAutoSwitchThreshold(sanitizeNumberInput(e.target.value))}
                        onBlur={() => {
                          const normalized = normalizeNumberInput(autoSwitchThreshold, 0, 100);
                          setAutoSwitchThreshold(normalized);
                          setAutoSwitchThresholdCustomMode(false);
                        }}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') {
                            e.preventDefault();
                            const normalized = normalizeNumberInput(autoSwitchThreshold, 0, 100);
                            setAutoSwitchThreshold(normalized);
                            setAutoSwitchThresholdCustomMode(false);
                          }
                        }}
                      />
                        <span className="settings-input-unit">%</span>
                      </div>
                    ) : (
                      <SettingsThresholdPresetDropdown
                        value={autoSwitchThreshold}
                        isPreset={autoSwitchThresholdIsPreset}
                        setValue={setAutoSwitchThreshold}
                        setCustomMode={setAutoSwitchThresholdCustomMode}
                      />
                    )}
                  </div>
                </div>
              )}
              {autoSwitchEnabled && (
                <div className="settings-row">
                  <div className="row-label">
                    <div className="row-title">{t('quickSettings.autoSwitch.windowThresholdsEnable')}</div>
                    <div className="row-desc">{t('quickSettings.autoSwitch.windowThresholdsHint')}</div>
                  </div>
                  <div className="row-control">
                    <label className="switch">
                      <input type="checkbox" checked={agWindowThresholdsEnabled}
                        aria-label={t('quickSettings.autoSwitch.windowThresholdsEnable')}
                        onChange={(event) => setAgWindowThresholdsEnabled(event.target.checked)} />
                      <span className="slider" />
                    </label>
                  </div>
                </div>
              )}
              {autoSwitchEnabled && agWindowThresholdsEnabled && ([
                ['fiveHourThreshold', agFiveHourThreshold, setAgFiveHourThreshold],
                ['weeklyThreshold', agWeeklyThreshold, setAgWeeklyThreshold],
              ] as const).map(([key, value, setValue]) => {
                const label = t(`quickSettings.autoSwitch.${key}`);
                return (
                  <div className="settings-row" key={key}>
                    <div className="row-label"><div className="row-title">{label}</div></div>
                    <div className="row-control settings-inline-input">
                      <input type="number" min={0} max={100} value={value} aria-label={label}
                        className="settings-select settings-select--input-mode settings-select--with-unit"
                        onChange={(event) => setValue(sanitizeNumberInput(event.target.value))}
                        onBlur={() => setValue(normalizeNumberInput(value || '5', 0, 100))} />
                      <span className="settings-input-unit">%</span>
                    </div>
                  </div>
                );
              })}
              {autoSwitchEnabled && (
                <div className="settings-row">
                  <div className="row-label">
                    <div className="row-title">
                      {t('quickSettings.autoSwitch.creditsEnable', '监控 Credits')}
                    </div>
                    <div className="row-desc">
                      {t('quickSettings.autoSwitch.creditsThresholdDesc', '剩余 Credits 小于等于此值时也会触发自动切号')}
                    </div>
                  </div>
                  <div className="row-control">
                    <label className="switch">
                      <input
                        type="checkbox"
                        checked={autoSwitchCreditsEnabled}
                        onChange={(e) => setAutoSwitchCreditsEnabled(e.target.checked)}
                      />
                      <span className="slider"></span>
                    </label>
                  </div>
                </div>
              )}
              {autoSwitchEnabled && autoSwitchCreditsEnabled && (
                <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                  <div className="row-label">
                    <div className="row-title">
                      {t('quickSettings.autoSwitch.creditsThreshold', 'Credits 阈值')}
                    </div>
                    <div className="row-desc">
                      {t('quickSettings.autoSwitch.creditsThresholdDesc', '剩余 Credits 小于等于此值时也会触发自动切号')}
                    </div>
                  </div>
                  <div className="row-control">
                    {autoSwitchCreditsThresholdCustomMode ? (
                      <div className="settings-inline-input">
                        <input
                          type="number"
                          min={0}
                          className="settings-select settings-select--input-mode"
                          value={autoSwitchCreditsThreshold}
                          placeholder={t('quickSettings.inputCredits', '输入 Credits')}
                          onChange={(e) =>
                            setAutoSwitchCreditsThreshold(sanitizeNumberInput(e.target.value))
                          }
                          onBlur={() => {
                            const normalized = normalizeNumberInput(autoSwitchCreditsThreshold, 0);
                            setAutoSwitchCreditsThreshold(normalized);
                            setAutoSwitchCreditsThresholdCustomMode(false);
                          }}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter') {
                              e.preventDefault();
                              const normalized = normalizeNumberInput(autoSwitchCreditsThreshold, 0);
                              setAutoSwitchCreditsThreshold(normalized);
                              setAutoSwitchCreditsThresholdCustomMode(false);
                            }
                          }}
                        />
                      </div>
                    ) : (
                      <SettingsThresholdPresetDropdown
                        value={autoSwitchCreditsThreshold}
                        isPreset={autoSwitchCreditsThresholdIsPreset}
                        setValue={setAutoSwitchCreditsThreshold}
                        setCustomMode={setAutoSwitchCreditsThresholdCustomMode}
                        presets={['0', '5', '10', '20']}
                        defaultCustomValue="5"
                        unit=""
                      />
                    )}
                  </div>
                </div>
              )}
              {autoSwitchEnabled && (
                <div className="settings-row settings-row--align-start" style={{ animation: 'fadeUp 0.3s ease both' }}>
                  <div className="row-label">
                    <div className="row-title">
                      {t('settings.general.autoSwitchAccountScope')}
                    </div>
                    <div className="row-desc">
                      {t('settings.general.autoSwitchAccountScopeDesc')}
                    </div>
                  </div>
                  <div className="row-control row-control--grow">
                    <AutoSwitchAccountScopeSelector
                      mode={autoSwitchAccountScopeMode}
                      onModeChange={setAutoSwitchAccountScopeMode}
                      selectedAccountIds={autoSwitchSelectedAccountIds}
                      onSelectedAccountIdsChange={setAutoSwitchSelectedAccountIds}
                      accounts={antigravityScopeAccounts}
                      groups={antigravityAccountGroups}
                      typeOptions={antigravityScopeTypeOptions}
                      useDialog
                    />
                  </div>
                </div>
              )}

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
                  <div className="row-desc">{t('quickSettings.quotaAlert.hint', '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={quotaAlertEnabled}
                      onChange={(e) => setQuotaAlertEnabled(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>
              {quotaAlertEnabled && (
                <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                  <div className="row-label">
                    <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                    <div className="row-desc">{t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}</div>
                  </div>
                  <div className="row-control">
                    {quotaAlertThresholdCustomMode ? (
                      <div className="settings-inline-input">
                        <input
                          type="number"
                          min={0}
                          max={100}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={quotaAlertThreshold}
                          placeholder={t('quickSettings.inputPercent', '输入百分比')}
                          onChange={(e) => setQuotaAlertThreshold(sanitizeNumberInput(e.target.value))}
                          onBlur={() => {
                            const normalized = normalizeNumberInput(quotaAlertThreshold, 0, 100);
                            setQuotaAlertThreshold(normalized);
                            setQuotaAlertThresholdCustomMode(false);
                          }}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter') {
                              e.preventDefault();
                              const normalized = normalizeNumberInput(quotaAlertThreshold, 0, 100);
                              setQuotaAlertThreshold(normalized);
                              setQuotaAlertThresholdCustomMode(false);
                            }
                          }}
                        />
                        <span className="settings-input-unit">%</span>
                      </div>
                    ) : (
                      <SettingsThresholdPresetDropdown
                        value={quotaAlertThreshold}
                        isPreset={quotaAlertThresholdIsPreset}
                        setValue={setQuotaAlertThreshold}
                        setCustomMode={setQuotaAlertThresholdCustomMode}
                      />
                    )}
                  </div>
                </div>
              )}
            </div>

              </PlatformSettingsSection>

              <SettingsCodexPlatformPanel {...props} />

              <PlatformSettingsSection platformId={'claude_manager'} order={platformSettingsOrder.claude_manager}>
                <div className="group-title">
                  {t('settings.general.claudeSettingsTitle', 'Claude 设置')}
                </div>
                <div className="settings-group">
                  <div className="settings-row"><LaunchPathSettingsButton app="claude_cli" /></div>
                  {renderPlatformAutoRefreshRow({
                    title: t(
                      'settings.general.claudeAutoRefresh',
                      'Claude 自动刷新配额',
                    ),
                    description: t(
                      'settings.general.claudeAutoRefreshDesc',
                      '后台自动更新 Claude 账号配额缓存',
                    ),
                    value: claudeAutoRefresh,
                    setValue: setClaudeAutoRefresh,
                    customMode: claudeAutoRefreshCustomMode,
                    setCustomMode: setClaudeAutoRefreshCustomMode,
                    isPreset: claudeAutoRefreshIsPreset,
                  })}
                  {renderCurrentAccountRefreshRow('claude')}
                  {renderAccountLevelRefreshConfig('claude')}
                  {isMacOS && (
                    <div className="settings-row">
                      <div className="row-label">
                        <div className="row-title">{t('settings.general.claudeShareCodeSessionsOnSwitch')}</div>
                        <div className="row-desc">{t('settings.general.claudeShareCodeSessionsOnSwitchDesc')}</div>
                      </div>
                      <div className="row-control">
                        <label className="switch">
                          <input
                            type="checkbox"
                            aria-label={t('settings.general.claudeShareCodeSessionsOnSwitch')}
                            checked={claudeShareCodeSessionsOnSwitch}
                            onChange={(event) => setClaudeShareCodeSessionsOnSwitch(event.target.checked)}
                          />
                          <span className="slider" />
                        </label>
                      </div>
                    </div>
                  )}
                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">
                        {t(
                          'settings.general.claudeQuotaDisplayRemaining',
                          'Claude 额度显示剩余%',
                        )}
                      </div>
                      <div className="row-desc">
                        {t(
                          'settings.general.claudeQuotaDisplayRemainingDesc',
                          '默认显示已用百分比；开启后改为显示剩余百分比。自动切号与预警仍按已用比例计算。',
                        )}
                      </div>
                    </div>
                    <div className="row-control">
                      <label className="switch">
                        <input
                          type="checkbox"
                          checked={claudeQuotaDisplayRemaining}
                          onChange={(e) => {
                            const enabled = e.target.checked;
                            setClaudeQuotaDisplayRemaining(enabled);
                            setClaudeQuotaDisplayRemainingEnabled(enabled);
                          }}
                        />
                        <span className="slider" />
                      </label>
                    </div>
                  </div>
                  <div className="settings-row settings-row--align-start">
                    <div className="row-label">
                      <div className="row-title">
                        {t('settings.general.claudeAppPath', 'Claude Desktop 启动目标')}
                      </div>
                      <div className="row-desc">
                        {t(
                          'settings.general.claudeAppPathDesc',
                          '默认实例可使用 Microsoft Store 应用目标；应用多开请选择真实 Claude.exe。',
                        )}
                      </div>
                    </div>
                    <div className="row-control row-control--grow settings-claude-launch-control">
                      <div className="settings-claude-launch-row">
                        <input
                          type="text"
                          className="settings-input settings-input--path"
                          value={claudeAppPath}
                          placeholder={t(
                            'quickSettings.claude.appTargetPlaceholder',
                            'Claude.exe 路径或 shell:AppsFolder\\...',
                          )}
                          onChange={(e) => setClaudeAppPath(e.target.value)}
                        />
                        <button
                          className="btn btn-secondary"
                          onClick={() => handlePickAppPath('claude')}
                          disabled={isAppPathResetDetecting('claude')}
                        >
                          {t('settings.general.codexPathSelect', '选择')}
                        </button>
                        <button
                          className="btn btn-secondary"
                          onClick={() => handleResetAppPath('claude')}
                          disabled={isAppPathResetDetecting('claude')}
                        >
                          <RefreshCw
                            size={16}
                            className={isAppPathResetDetecting('claude') ? 'spin' : undefined}
                          />
                          {isAppPathResetDetecting('claude')
                            ? t('common.loading', '加载中...')
                            : getResetLabelByTarget('claude')}
                        </button>
                      </div>
                      {claudeLaunchCandidates.length > 0 ? (
                        <div className="settings-claude-candidate-list">
                          {claudeLaunchCandidates.map((candidate) => (
                            <button
                              key={`${candidate.target_type}:${candidate.target}`}
                              type="button"
                              className={`settings-claude-candidate-item${
                                claudeAppPath.trim() === candidate.target ? ' selected' : ''
                              }`}
                              onClick={() => handleSelectClaudeLaunchCandidate(candidate)}
                            >
                              <div className="settings-claude-candidate-main">
                                <span>{candidate.label || 'Claude Desktop'}</span>
                                <span className="settings-claude-candidate-badge">
                                  {candidate.target_type === 'windows_app'
                                    ? t('appPath.missing.windowsApp', 'Microsoft Store')
                                    : 'EXE'}
                                </span>
                              </div>
                              <div className="settings-claude-candidate-target">
                                {candidate.target}
                              </div>
                              {!candidate.supports_multi_instance ? (
                                <div className="settings-claude-candidate-note">
                                  {t(
                                    'appPath.missing.defaultOnly',
                                    '仅适用于默认桌面端；应用多开请选择真实 Claude.exe',
                                  )}
                                </div>
                              ) : null}
                            </button>
                          ))}
                        </div>
                      ) : null}
                    </div>
                  </div>
                  {renderPlatformQuotaAlertRows({
                    enabled: claudeQuotaAlertEnabled,
                    setEnabled: setClaudeQuotaAlertEnabled,
                    threshold: claudeQuotaAlertThreshold,
                    setThreshold: setClaudeQuotaAlertThreshold,
                    customMode: claudeQuotaAlertThresholdCustomMode,
                    setCustomMode: setClaudeQuotaAlertThresholdCustomMode,
                    isPreset: claudeQuotaAlertThresholdIsPreset,
                  })}
                </div>
              </PlatformSettingsSection>

              <PlatformSettingsSection platformId={'github-copilot'} order={platformSettingsOrder['github-copilot']}>
                <div className="group-title">{t('settings.general.githubCopilotSettingsTitle', 'GitHub Copilot 设置')}</div>
                <div className="settings-group">
              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.ghcpAutoRefresh', 'GitHub Copilot 自动刷新配额')}</div>
                  <div className="row-desc">{t('settings.general.ghcpAutoRefreshDesc', '后台自动更新频率')}</div>
                </div>
                <div className="row-control">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                    {ghcpAutoRefreshCustomMode ? (
                      <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                        <input
                          type="number"
                          min={1}
                          max={999}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={ghcpAutoRefresh}
                          placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                          onChange={(e) => setGhcpAutoRefresh(sanitizeNumberInput(e.target.value))}
                        onBlur={() => {
                          const normalized = normalizeNumberInput(ghcpAutoRefresh, 1, 999);
                          setGhcpAutoRefresh(normalized);
                          setGhcpAutoRefreshCustomMode(false);
                        }}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') {
                            e.preventDefault();
                            const normalized = normalizeNumberInput(ghcpAutoRefresh, 1, 999);
                            setGhcpAutoRefresh(normalized);
                            setGhcpAutoRefreshCustomMode(false);
                          }
                        }}
                      />
                        <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                      </div>
                    ) : (
                      <SettingsRefreshPresetDropdown
                        value={ghcpAutoRefresh}
                        isPreset={ghcpAutoRefreshIsPreset}
                        setValue={setGhcpAutoRefresh}
                        setCustomMode={setGhcpAutoRefreshCustomMode}
                      />
                    )}
                  </div>
                </div>
              </div>

              {renderCurrentAccountRefreshRow('ghcp')}
              {renderAccountLevelRefreshConfig('ghcp')}

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.vscodeAppPath', 'VS Code 启动路径')}</div>
                  <div className="row-desc">{t('settings.general.vscodeAppPathDesc', '留空则使用默认路径')}</div>
                </div>
                <div className="row-control row-control--grow">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                    <input
                      type="text"
                      className="settings-input settings-input--path"
                      value={vscodeAppPath}
                      placeholder={t('settings.general.vscodeAppPathPlaceholder', '默认路径')}
                      onChange={(e) => setVscodeAppPath(e.target.value)}
                    />
                    <button
                      className="btn btn-secondary"
                      onClick={() => handlePickAppPath('vscode')}
                      disabled={isAppPathResetDetecting('vscode')}
                    >
                      {t('settings.general.vscodePathSelect', '选择')}
                    </button>
                    <button
                      className="btn btn-secondary"
                      onClick={() => handleResetAppPath('vscode')}
                      disabled={isAppPathResetDetecting('vscode')}
                    >
                      <RefreshCw size={16} className={isAppPathResetDetecting('vscode') ? 'spin' : undefined} />
                      {isAppPathResetDetecting('vscode')
                        ? t('common.loading', '加载中...')
                        : getResetLabelByTarget('vscode')}
                    </button>
                  </div>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
                  <div className="row-desc">{t('quickSettings.quotaAlert.hint', '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={ghcpQuotaAlertEnabled}
                      onChange={(e) => setGhcpQuotaAlertEnabled(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>
              {ghcpQuotaAlertEnabled && (
                <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                  <div className="row-label">
                    <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                    <div className="row-desc">{t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}</div>
                  </div>
                  <div className="row-control">
                    {ghcpQuotaAlertThresholdCustomMode ? (
                      <div className="settings-inline-input">
                        <input
                          type="number"
                          min={0}
                          max={100}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={ghcpQuotaAlertThreshold}
                          placeholder={t('quickSettings.inputPercent', '输入百分比')}
                          onChange={(e) => setGhcpQuotaAlertThreshold(sanitizeNumberInput(e.target.value))}
                          onBlur={() => {
                            const normalized = normalizeNumberInput(ghcpQuotaAlertThreshold, 0, 100);
                            setGhcpQuotaAlertThreshold(normalized);
                            setGhcpQuotaAlertThresholdCustomMode(false);
                          }}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter') {
                              e.preventDefault();
                              const normalized = normalizeNumberInput(ghcpQuotaAlertThreshold, 0, 100);
                              setGhcpQuotaAlertThreshold(normalized);
                              setGhcpQuotaAlertThresholdCustomMode(false);
                            }
                          }}
                        />
                        <span className="settings-input-unit">%</span>
                      </div>
                    ) : (
                      <SettingsThresholdPresetDropdown
                        value={ghcpQuotaAlertThreshold}
                        isPreset={ghcpQuotaAlertThresholdIsPreset}
                        setValue={setGhcpQuotaAlertThreshold}
                        setCustomMode={setGhcpQuotaAlertThresholdCustomMode}
                      />
                    )}
                  </div>
                </div>
              )}
            </div>

              </PlatformSettingsSection>

              <PlatformSettingsSection platformId={'windsurf'} order={platformSettingsOrder.windsurf}>
                <div className="group-title">{t('settings.general.windsurfSettingsTitle', 'Windsurf 设置')}</div>
                <div className="settings-group">
              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.windsurfAutoRefresh', 'Windsurf 自动刷新配额')}</div>
                  <div className="row-desc">{t('settings.general.windsurfAutoRefreshDesc', '后台自动更新频率')}</div>
                </div>
                <div className="row-control">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                    {windsurfAutoRefreshCustomMode ? (
                      <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                        <input
                          type="number"
                          min={1}
                          max={999}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={windsurfAutoRefresh}
                          placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                          onChange={(e) => setWindsurfAutoRefresh(sanitizeNumberInput(e.target.value))}
                        onBlur={() => {
                          const normalized = normalizeNumberInput(windsurfAutoRefresh, 1, 999);
                          setWindsurfAutoRefresh(normalized);
                          setWindsurfAutoRefreshCustomMode(false);
                        }}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') {
                            e.preventDefault();
                            const normalized = normalizeNumberInput(windsurfAutoRefresh, 1, 999);
                            setWindsurfAutoRefresh(normalized);
                            setWindsurfAutoRefreshCustomMode(false);
                          }
                        }}
                      />
                        <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                      </div>
                    ) : (
                      <SettingsRefreshPresetDropdown
                        value={windsurfAutoRefresh}
                        isPreset={windsurfAutoRefreshIsPreset}
                        setValue={setWindsurfAutoRefresh}
                        setCustomMode={setWindsurfAutoRefreshCustomMode}
                      />
                    )}
                  </div>
                </div>
              </div>

              {renderCurrentAccountRefreshRow('windsurf')}
              {renderAccountLevelRefreshConfig('windsurf')}

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.windsurfAppPath', 'Windsurf 启动路径')}</div>
                  <div className="row-desc">{t('settings.general.windsurfAppPathDesc', '留空则使用默认路径')}</div>
                </div>
                <div className="row-control row-control--grow">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                    <input
                      type="text"
                      className="settings-input settings-input--path"
                      value={windsurfAppPath}
                      placeholder={t('settings.general.windsurfAppPathPlaceholder', '默认路径')}
                      onChange={(e) => setWindsurfAppPath(e.target.value)}
                    />
                    <button
                      className="btn btn-secondary"
                      onClick={() => handlePickAppPath('windsurf')}
                      disabled={isAppPathResetDetecting('windsurf')}
                    >
                      {t('settings.general.windsurfPathSelect', '选择')}
                    </button>
                    <button
                      className="btn btn-secondary"
                      onClick={() => handleResetAppPath('windsurf')}
                      disabled={isAppPathResetDetecting('windsurf')}
                    >
                      <RefreshCw size={16} className={isAppPathResetDetecting('windsurf') ? 'spin' : undefined} />
                      {isAppPathResetDetecting('windsurf')
                        ? t('common.loading', '加载中...')
                        : getResetLabelByTarget('windsurf')}
                    </button>
                  </div>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
                  <div className="row-desc">{t('quickSettings.quotaAlert.hint', '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={windsurfQuotaAlertEnabled}
                      onChange={(e) => setWindsurfQuotaAlertEnabled(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>
              {windsurfQuotaAlertEnabled && (
                <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                  <div className="row-label">
                    <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                    <div className="row-desc">{t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}</div>
                  </div>
                  <div className="row-control">
                    {windsurfQuotaAlertThresholdCustomMode ? (
                      <div className="settings-inline-input">
                        <input
                          type="number"
                          min={0}
                          max={100}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={windsurfQuotaAlertThreshold}
                          placeholder={t('quickSettings.inputPercent', '输入百分比')}
                          onChange={(e) => setWindsurfQuotaAlertThreshold(sanitizeNumberInput(e.target.value))}
                          onBlur={() => {
                            const normalized = normalizeNumberInput(windsurfQuotaAlertThreshold, 0, 100);
                            setWindsurfQuotaAlertThreshold(normalized);
                            setWindsurfQuotaAlertThresholdCustomMode(false);
                          }}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter') {
                              e.preventDefault();
                              const normalized = normalizeNumberInput(windsurfQuotaAlertThreshold, 0, 100);
                              setWindsurfQuotaAlertThreshold(normalized);
                              setWindsurfQuotaAlertThresholdCustomMode(false);
                            }
                          }}
                        />
                        <span className="settings-input-unit">%</span>
                      </div>
                    ) : (
                      <SettingsThresholdPresetDropdown
                        value={windsurfQuotaAlertThreshold}
                        isPreset={windsurfQuotaAlertThresholdIsPreset}
                        setValue={setWindsurfQuotaAlertThreshold}
                        setCustomMode={setWindsurfQuotaAlertThresholdCustomMode}
                      />
                    )}
                  </div>
                </div>
              )}
            </div>

              </PlatformSettingsSection>

              <PlatformSettingsSection platformId={'kiro'} order={platformSettingsOrder.kiro}>
                <div className="group-title">{t('settings.general.kiroSettingsTitle', 'Kiro 设置')}</div>
                <div className="settings-group">
              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.kiroAutoRefresh', 'Kiro 自动刷新配额')}</div>
                  <div className="row-desc">{t('settings.general.kiroAutoRefreshDesc', '后台自动更新频率')}</div>
                </div>
                <div className="row-control">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                    {kiroAutoRefreshCustomMode ? (
                      <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                        <input
                          type="number"
                          min={1}
                          max={999}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={kiroAutoRefresh}
                          placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                          onChange={(e) => setKiroAutoRefresh(sanitizeNumberInput(e.target.value))}
                          onBlur={() => {
                            const normalized = normalizeNumberInput(kiroAutoRefresh, 1, 999);
                            setKiroAutoRefresh(normalized);
                            setKiroAutoRefreshCustomMode(false);
                          }}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter') {
                              e.preventDefault();
                              const normalized = normalizeNumberInput(kiroAutoRefresh, 1, 999);
                              setKiroAutoRefresh(normalized);
                              setKiroAutoRefreshCustomMode(false);
                            }
                          }}
                        />
                        <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                      </div>
                    ) : (
                      <SettingsRefreshPresetDropdown
                        value={kiroAutoRefresh}
                        isPreset={kiroAutoRefreshIsPreset}
                        setValue={setKiroAutoRefresh}
                        setCustomMode={setKiroAutoRefreshCustomMode}
                      />
                    )}
                  </div>
                </div>
              </div>

              {renderCurrentAccountRefreshRow('kiro')}
              {renderAccountLevelRefreshConfig('kiro')}

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.kiroAppPath', 'Kiro 启动路径')}</div>
                  <div className="row-desc">{t('settings.general.kiroAppPathDesc', '留空则使用默认路径')}</div>
                </div>
                <div className="row-control row-control--grow">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                    <input
                      type="text"
                      className="settings-input settings-input--path"
                      value={kiroAppPath}
                      placeholder={t('settings.general.kiroAppPathPlaceholder', '默认路径')}
                      onChange={(e) => setKiroAppPath(e.target.value)}
                    />
                    <button
                      className="btn btn-secondary"
                      onClick={() => handlePickAppPath('kiro')}
                      disabled={isAppPathResetDetecting('kiro')}
                    >
                      {t('settings.general.kiroPathSelect', '选择')}
                    </button>
                    <button
                      className="btn btn-secondary"
                      onClick={() => handleResetAppPath('kiro')}
                      disabled={isAppPathResetDetecting('kiro')}
                    >
                      <RefreshCw size={16} className={isAppPathResetDetecting('kiro') ? 'spin' : undefined} />
                      {isAppPathResetDetecting('kiro')
                        ? t('common.loading', '加载中...')
                        : getResetLabelByTarget('kiro')}
                    </button>
                  </div>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
                  <div className="row-desc">{t('quickSettings.quotaAlert.hint', '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={kiroQuotaAlertEnabled}
                      onChange={(e) => setKiroQuotaAlertEnabled(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>
              {kiroQuotaAlertEnabled && (
                <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                  <div className="row-label">
                    <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                    <div className="row-desc">{t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}</div>
                  </div>
                  <div className="row-control">
                    {kiroQuotaAlertThresholdCustomMode ? (
                      <div className="settings-inline-input">
                        <input
                          type="number"
                          min={0}
                          max={100}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={kiroQuotaAlertThreshold}
                          placeholder={t('quickSettings.inputPercent', '输入百分比')}
                          onChange={(e) => setKiroQuotaAlertThreshold(sanitizeNumberInput(e.target.value))}
                          onBlur={() => {
                            const normalized = normalizeNumberInput(kiroQuotaAlertThreshold, 0, 100);
                            setKiroQuotaAlertThreshold(normalized);
                            setKiroQuotaAlertThresholdCustomMode(false);
                          }}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter') {
                              e.preventDefault();
                              const normalized = normalizeNumberInput(kiroQuotaAlertThreshold, 0, 100);
                              setKiroQuotaAlertThreshold(normalized);
                              setKiroQuotaAlertThresholdCustomMode(false);
                            }
                          }}
                        />
                        <span className="settings-input-unit">%</span>
                      </div>
                    ) : (
                      <SettingsThresholdPresetDropdown
                        value={kiroQuotaAlertThreshold}
                        isPreset={kiroQuotaAlertThresholdIsPreset}
                        setValue={setKiroQuotaAlertThreshold}
                        setCustomMode={setKiroQuotaAlertThresholdCustomMode}
                      />
                    )}
                  </div>
                </div>
              )}
            </div>
              </PlatformSettingsSection>

              <PlatformSettingsSection platformId={'codebuddy'} order={platformSettingsOrder.codebuddy}>
                <div className="group-title">{t('settings.general.codebuddySettingsTitle', 'CodeBuddy 设置')}</div>
                <div className="settings-group">
                  <div className="settings-row"><LaunchPathSettingsButton app="codebuddy_cli" /></div>
              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.codebuddyAutoRefresh', 'CodeBuddy 自动刷新配额')}</div>
                  <div className="row-desc">{t('settings.general.codebuddyAutoRefreshDesc', '后台自动更新频率')}</div>
                </div>
                <div className="row-control">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                    {codebuddyAutoRefreshCustomMode ? (
                      <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                        <input
                          type="number"
                          min={1}
                          max={999}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={codebuddyAutoRefresh}
                          placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                          onChange={(e) => setCodebuddyAutoRefresh(sanitizeNumberInput(e.target.value))}
                          onBlur={() => {
                            const normalized = normalizeNumberInput(codebuddyAutoRefresh, 1, 999);
                            if (REFRESH_PRESET_VALUES.includes(normalized)) {
                              setCodebuddyAutoRefreshCustomMode(false);
                            }
                            setCodebuddyAutoRefresh(normalized);
                          }}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter') {
                              e.preventDefault();
                              const normalized = normalizeNumberInput(codebuddyAutoRefresh, 1, 999);
                              setCodebuddyAutoRefreshCustomMode(false);
                              setCodebuddyAutoRefresh(normalized);
                            }
                          }}
                        />
                        <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                      </div>
                    ) : (
                      <SettingsRefreshPresetDropdown
                        value={codebuddyAutoRefresh}
                        isPreset={codebuddyAutoRefreshIsPreset}
                        setValue={setCodebuddyAutoRefresh}
                        setCustomMode={setCodebuddyAutoRefreshCustomMode}
                      />
                    )}
                  </div>
                </div>
              </div>

              {renderCurrentAccountRefreshRow('codebuddy')}
              {renderAccountLevelRefreshConfig('codebuddy')}

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">
                    {t('settings.general.codebuddyShareSessionsOnSwitch')}
                  </div>
                  <div className="row-desc">
                    {t('settings.general.codebuddyShareSessionsOnSwitchDesc')}
                  </div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={codebuddyShareSessionsOnSwitch}
                      onChange={(event) => setCodebuddyShareSessionsOnSwitch(event.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.codebuddyAppPath', 'CodeBuddy 启动路径')}</div>
                  <div className="row-desc">{t('settings.general.codebuddyAppPathDesc', '留空则使用默认路径')}</div>
                </div>
                <div className="row-control row-control--grow">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                    <input
                      type="text"
                      className="settings-input settings-input--path"
                      value={codebuddyAppPath}
                      placeholder={t('settings.general.codebuddyAppPathPlaceholder', '默认路径')}
                      onChange={(e) => setCodebuddyAppPath(e.target.value)}
                    />
                    <button
                      className="btn btn-secondary"
                      onClick={() => handlePickAppPath('codebuddy')}
                      disabled={isAppPathResetDetecting('codebuddy')}
                    >
                      {t('settings.general.codebuddyPathSelect', '选择')}
                    </button>
                    <button
                      className="btn btn-secondary"
                      onClick={() => handleResetAppPath('codebuddy')}
                      disabled={isAppPathResetDetecting('codebuddy')}
                    >
                      <RefreshCw size={16} className={isAppPathResetDetecting('codebuddy') ? 'spin' : undefined} />
                      {isAppPathResetDetecting('codebuddy')
                        ? t('common.loading', '加载中...')
                        : getResetLabelByTarget('codebuddy')}
                    </button>
                  </div>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
                  <div className="row-desc">{t('quickSettings.quotaAlert.hint', '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={codebuddyQuotaAlertEnabled}
                      onChange={(e) => setCodebuddyQuotaAlertEnabled(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>
              {codebuddyQuotaAlertEnabled && (
                <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                  <div className="row-label">
                    <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                    <div className="row-desc">{t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}</div>
                  </div>
                  <div className="row-control">
                    {codebuddyQuotaAlertThresholdCustomMode ? (
                      <div className="settings-inline-input">
                        <input
                          type="number"
                          min={0}
                          max={100}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={codebuddyQuotaAlertThreshold}
                          placeholder={t('quickSettings.inputPercent', '输入百分比')}
                          onChange={(e) => setCodebuddyQuotaAlertThreshold(sanitizeNumberInput(e.target.value))}
                          onBlur={() => {
                            const normalized = normalizeNumberInput(codebuddyQuotaAlertThreshold, 0, 100);
                            if (THRESHOLD_PRESET_VALUES.includes(normalized)) {
                              setCodebuddyQuotaAlertThresholdCustomMode(false);
                            }
                            setCodebuddyQuotaAlertThreshold(normalized);
                          }}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter') {
                              e.preventDefault();
                              const normalized = normalizeNumberInput(codebuddyQuotaAlertThreshold, 0, 100);
                              setCodebuddyQuotaAlertThresholdCustomMode(false);
                              setCodebuddyQuotaAlertThreshold(normalized);
                            }
                          }}
                        />
                        <span className="settings-input-unit">%</span>
                      </div>
                    ) : (
                      <SettingsThresholdPresetDropdown
                        value={codebuddyQuotaAlertThreshold}
                        isPreset={codebuddyQuotaAlertThresholdIsPreset}
                        setValue={setCodebuddyQuotaAlertThreshold}
                        setCustomMode={setCodebuddyQuotaAlertThresholdCustomMode}
                      />
                    )}
                  </div>
                </div>
              )}
            </div>
              </PlatformSettingsSection>

              <PlatformSettingsSection platformId={'codebuddy_cn'} order={platformSettingsOrder.codebuddy_cn}>
                <div className="group-title">{t('settings.general.codebuddyCnSettingsTitle', 'CodeBuddy CN 设置')}</div>
                <div className="settings-group">
                  <div className="settings-row"><LaunchPathSettingsButton app="codebuddy_cli" /></div>
                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('settings.general.codebuddyCnAutoRefresh', 'CodeBuddy CN 自动刷新配额')}</div>
                      <div className="row-desc">{t('settings.general.codebuddyCnAutoRefreshDesc', '后台自动更新频率')}</div>
                    </div>
                    <div className="row-control">
                      <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                        {codebuddyCnAutoRefreshCustomMode ? (
                          <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                            <input
                              type="number"
                              min={1}
                              max={999}
                              className="settings-select settings-select--input-mode settings-select--with-unit"
                              value={codebuddyCnAutoRefresh}
                              placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                              onChange={(e) => setCodebuddyCnAutoRefresh(sanitizeNumberInput(e.target.value))}
                              onBlur={() => {
                                const normalized = normalizeNumberInput(codebuddyCnAutoRefresh, 1, 999);
                                if (REFRESH_PRESET_VALUES.includes(normalized)) {
                                  setCodebuddyCnAutoRefreshCustomMode(false);
                                }
                                setCodebuddyCnAutoRefresh(normalized);
                              }}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  const normalized = normalizeNumberInput(codebuddyCnAutoRefresh, 1, 999);
                                  setCodebuddyCnAutoRefreshCustomMode(false);
                                  setCodebuddyCnAutoRefresh(normalized);
                                }
                              }}
                            />
                            <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                          </div>
                        ) : (
                          <SettingsRefreshPresetDropdown
                            value={codebuddyCnAutoRefresh}
                            isPreset={codebuddyCnAutoRefreshIsPreset}
                            setValue={setCodebuddyCnAutoRefresh}
                            setCustomMode={setCodebuddyCnAutoRefreshCustomMode}
                          />
                        )}
                      </div>
                    </div>
                  </div>

                  {renderCurrentAccountRefreshRow('codebuddy_cn')}
                  {renderAccountLevelRefreshConfig('codebuddy_cn')}
                  {renderSessionSharingRow(
                    'CodeBuddy CN',
                    codebuddyCnShareSessionsOnSwitch,
                    setCodebuddyCnShareSessionsOnSwitch,
                    true,
                  )}

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('settings.general.codebuddyCnAppPath', 'CodeBuddy CN 启动路径')}</div>
                      <div className="row-desc">{t('settings.general.codebuddyCnAppPathDesc', '留空则使用默认路径')}</div>
                    </div>
                    <div className="row-control row-control--grow">
                      <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                        <input
                          type="text"
                          className="settings-input settings-input--path"
                          value={codebuddyCnAppPath}
                          placeholder={t('settings.general.codebuddyCnAppPathPlaceholder', '默认路径')}
                          onChange={(e) => setCodebuddyCnAppPath(e.target.value)}
                        />
                        <button
                          className="btn btn-secondary"
                          onClick={() => handlePickAppPath('codebuddy_cn')}
                          disabled={isAppPathResetDetecting('codebuddy_cn')}
                        >
                          {t('settings.general.codebuddyCnPathSelect', '选择')}
                        </button>
                        <button
                          className="btn btn-secondary"
                          onClick={() => handleResetAppPath('codebuddy_cn')}
                          disabled={isAppPathResetDetecting('codebuddy_cn')}
                        >
                          <RefreshCw size={16} className={isAppPathResetDetecting('codebuddy_cn') ? 'spin' : undefined} />
                          {isAppPathResetDetecting('codebuddy_cn')
                            ? t('common.loading', '加载中...')
                            : getResetLabelByTarget('codebuddy_cn')}
                        </button>
                      </div>
                    </div>
                  </div>

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
                      <div className="row-desc">{t('quickSettings.quotaAlert.hint', '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。')}</div>
                    </div>
                    <div className="row-control">
                      <label className="switch">
                        <input
                          type="checkbox"
                          checked={codebuddyCnQuotaAlertEnabled}
                          onChange={(e) => setCodebuddyCnQuotaAlertEnabled(e.target.checked)}
                        />
                        <span className="slider"></span>
                      </label>
                    </div>
                  </div>
                  {codebuddyCnQuotaAlertEnabled && (
                    <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                      <div className="row-label">
                        <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                        <div className="row-desc">{t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}</div>
                      </div>
                      <div className="row-control">
                        {codebuddyCnQuotaAlertThresholdCustomMode ? (
                          <div className="settings-inline-input">
                            <input
                              type="number"
                              min={0}
                              max={100}
                              className="settings-select settings-select--input-mode settings-select--with-unit"
                              value={codebuddyCnQuotaAlertThreshold}
                              placeholder={t('quickSettings.inputPercent', '输入百分比')}
                              onChange={(e) => setCodebuddyCnQuotaAlertThreshold(sanitizeNumberInput(e.target.value))}
                              onBlur={() => {
                                const normalized = normalizeNumberInput(codebuddyCnQuotaAlertThreshold, 0, 100);
                                if (THRESHOLD_PRESET_VALUES.includes(normalized)) {
                                  setCodebuddyCnQuotaAlertThresholdCustomMode(false);
                                }
                                setCodebuddyCnQuotaAlertThreshold(normalized);
                              }}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  const normalized = normalizeNumberInput(codebuddyCnQuotaAlertThreshold, 0, 100);
                                  setCodebuddyCnQuotaAlertThresholdCustomMode(false);
                                  setCodebuddyCnQuotaAlertThreshold(normalized);
                                }
                              }}
                            />
                            <span className="settings-input-unit">%</span>
                          </div>
                        ) : (
                          <SettingsThresholdPresetDropdown
                            value={codebuddyCnQuotaAlertThreshold}
                            isPreset={codebuddyCnQuotaAlertThresholdIsPreset}
                            setValue={setCodebuddyCnQuotaAlertThreshold}
                            setCustomMode={setCodebuddyCnQuotaAlertThresholdCustomMode}
                          />
                        )}
                      </div>
                    </div>
                  )}
                </div>
              </PlatformSettingsSection>

              <PlatformSettingsSection platformId={'qoder'} order={platformSettingsOrder.qoder}>
                <div className="group-title">{t('quickSettings.qoder.title', 'Qoder 设置')}</div>
                <div className="settings-group">
                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('settings.general.qoderAutoRefresh', 'Qoder 自动刷新配额')}</div>
                      <div className="row-desc">{t('settings.general.qoderAutoRefreshDesc', '后台自动更新频率')}</div>
                    </div>
                    <div className="row-control">
                      <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                        {qoderAutoRefreshCustomMode ? (
                          <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                            <input
                              type="number"
                              min={1}
                              max={999}
                              className="settings-select settings-select--input-mode settings-select--with-unit"
                              value={qoderAutoRefresh}
                              placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                              onChange={(e) => setQoderAutoRefresh(sanitizeNumberInput(e.target.value))}
                              onBlur={() => {
                                const normalized = normalizeNumberInput(qoderAutoRefresh, 1, 999);
                                if (REFRESH_PRESET_VALUES.includes(normalized)) {
                                  setQoderAutoRefreshCustomMode(false);
                                }
                                setQoderAutoRefresh(normalized);
                              }}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  const normalized = normalizeNumberInput(qoderAutoRefresh, 1, 999);
                                  setQoderAutoRefreshCustomMode(false);
                                  setQoderAutoRefresh(normalized);
                                }
                              }}
                            />
                            <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                          </div>
                        ) : (
                          <SettingsRefreshPresetDropdown
                            value={qoderAutoRefresh}
                            isPreset={qoderAutoRefreshIsPreset}
                            setValue={setQoderAutoRefresh}
                            setCustomMode={setQoderAutoRefreshCustomMode}
                          />
                        )}
                      </div>
                    </div>
                  </div>

                  {renderCurrentAccountRefreshRow('qoder')}
                  {renderAccountLevelRefreshConfig('qoder')}

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('settings.general.qoderAppPath', 'Qoder 启动路径')}</div>
                      <div className="row-desc">{t('settings.general.qoderAppPathDesc', '留空则使用默认路径')}</div>
                    </div>
                    <div className="row-control row-control--grow">
                      <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                        <input
                          type="text"
                          className="settings-input settings-input--path"
                          value={qoderAppPath}
                          placeholder={t('settings.general.qoderAppPathPlaceholder', '默认路径')}
                          onChange={(e) => setQoderAppPath(e.target.value)}
                        />
                        <button
                          className="btn btn-secondary"
                          onClick={() => handlePickAppPath('qoder')}
                          disabled={isAppPathResetDetecting('qoder')}
                        >
                          {t('settings.general.qoderPathSelect', '选择')}
                        </button>
                        <button
                          className="btn btn-secondary"
                          onClick={() => handleResetAppPath('qoder')}
                          disabled={isAppPathResetDetecting('qoder')}
                        >
                          <RefreshCw size={16} className={isAppPathResetDetecting('qoder') ? 'spin' : undefined} />
                          {isAppPathResetDetecting('qoder')
                            ? t('common.loading', '加载中...')
                            : getResetLabelByTarget('qoder')}
                        </button>
                      </div>
                    </div>
                  </div>

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
                      <div className="row-desc">{t('quickSettings.quotaAlert.hint', '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。')}</div>
                    </div>
                    <div className="row-control">
                      <label className="switch">
                        <input
                          type="checkbox"
                          checked={qoderQuotaAlertEnabled}
                          onChange={(e) => setQoderQuotaAlertEnabled(e.target.checked)}
                        />
                        <span className="slider"></span>
                      </label>
                    </div>
                  </div>
                  {qoderQuotaAlertEnabled && (
                    <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                      <div className="row-label">
                        <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                        <div className="row-desc">{t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}</div>
                      </div>
                      <div className="row-control">
                        {qoderQuotaAlertThresholdCustomMode ? (
                          <div className="settings-inline-input">
                            <input
                              type="number"
                              min={0}
                              max={100}
                              className="settings-select settings-select--input-mode settings-select--with-unit"
                              value={qoderQuotaAlertThreshold}
                              placeholder={t('quickSettings.inputPercent', '输入百分比')}
                              onChange={(e) => setQoderQuotaAlertThreshold(sanitizeNumberInput(e.target.value))}
                              onBlur={() => {
                                const normalized = normalizeNumberInput(qoderQuotaAlertThreshold, 0, 100);
                                if (THRESHOLD_PRESET_VALUES.includes(normalized)) {
                                  setQoderQuotaAlertThresholdCustomMode(false);
                                }
                                setQoderQuotaAlertThreshold(normalized);
                              }}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  const normalized = normalizeNumberInput(qoderQuotaAlertThreshold, 0, 100);
                                  setQoderQuotaAlertThresholdCustomMode(false);
                                  setQoderQuotaAlertThreshold(normalized);
                                }
                              }}
                            />
                            <span className="settings-input-unit">%</span>
                          </div>
                        ) : (
                          <SettingsThresholdPresetDropdown
                            value={qoderQuotaAlertThreshold}
                            isPreset={qoderQuotaAlertThresholdIsPreset}
                            setValue={setQoderQuotaAlertThreshold}
                            setCustomMode={setQoderQuotaAlertThresholdCustomMode}
                          />
                        )}
                      </div>
                    </div>
                  )}
                </div>
              </PlatformSettingsSection>

              <PlatformSettingsSection platformId={'zcode'} order={platformSettingsOrder.zcode}>
                <div className="group-title">{t('quickSettings.zcode.title', 'ZCode 设置')}</div>
                <div className="settings-group">
                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('settings.general.zcodeAutoRefresh', 'ZCode 自动刷新配额')}</div>
                      <div className="row-desc">{t('settings.general.zcodeAutoRefreshDesc', '后台自动更新频率')}</div>
                    </div>
                    <div className="row-control">
                      <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                        {zcodeAutoRefreshCustomMode ? (
                          <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                            <input
                              type="number"
                              min={1}
                              max={999}
                              className="settings-select settings-select--input-mode settings-select--with-unit"
                              value={zcodeAutoRefresh}
                              placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                              onChange={(event) => setZcodeAutoRefresh(sanitizeNumberInput(event.target.value))}
                              onBlur={() => {
                                const normalized = normalizeNumberInput(zcodeAutoRefresh, 1, 999);
                                if (REFRESH_PRESET_VALUES.includes(normalized)) {
                                  setZcodeAutoRefreshCustomMode(false);
                                }
                                setZcodeAutoRefresh(normalized);
                              }}
                              onKeyDown={(event) => {
                                if (event.key === 'Enter') {
                                  event.preventDefault();
                                  const normalized = normalizeNumberInput(zcodeAutoRefresh, 1, 999);
                                  setZcodeAutoRefreshCustomMode(false);
                                  setZcodeAutoRefresh(normalized);
                                }
                              }}
                            />
                            <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                          </div>
                        ) : (
                          <SettingsRefreshPresetDropdown
                            value={zcodeAutoRefresh}
                            isPreset={zcodeAutoRefreshIsPreset}
                            setValue={setZcodeAutoRefresh}
                            setCustomMode={setZcodeAutoRefreshCustomMode}
                          />
                        )}
                      </div>
                    </div>
                  </div>

                  {renderCurrentAccountRefreshRow('zcode')}
                  {renderAccountLevelRefreshConfig('zcode')}

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('settings.general.zcodeAppPath', 'ZCode 启动路径')}</div>
                      <div className="row-desc">{t('settings.general.zcodeAppPathDesc', '留空则使用默认路径')}</div>
                    </div>
                    <div className="row-control row-control--grow">
                      <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                        <input
                          type="text"
                          className="settings-input settings-input--path"
                          value={zcodeAppPath}
                          placeholder={t('settings.general.codexAppPathPlaceholder', '默认路径')}
                          onChange={(event) => setZcodeAppPath(event.target.value)}
                        />
                        <button
                          className="btn btn-secondary"
                          onClick={() => setZcodeAppPath('')}
                          disabled={isAppPathResetDetecting('zcode') || !zcodeAppPath.trim()}
                        >
                          {t('common.clear', '清除')}
                        </button>
                        <button
                          className="btn btn-secondary"
                          onClick={() => handlePickAppPath('zcode')}
                          disabled={isAppPathResetDetecting('zcode')}
                        >
                          {t('settings.general.codexPathSelect', '选择')}
                        </button>
                        <button
                          className="btn btn-secondary"
                          onClick={() => handleResetAppPath('zcode')}
                          disabled={isAppPathResetDetecting('zcode')}
                        >
                          <RefreshCw size={16} className={isAppPathResetDetecting('zcode') ? 'spin' : undefined} />
                          {isAppPathResetDetecting('zcode')
                            ? t('common.loading', '加载中...')
                            : getResetLabelByTarget('zcode')}
                        </button>
                      </div>
                    </div>
                  </div>
                </div>
              </PlatformSettingsSection>

              <PlatformSettingsSection platformId={'trae'} order={platformSettingsOrder.trae}>
                <div className="group-title">{t('quickSettings.trae.title', 'Trae 设置')}</div>
                <div className="settings-group">
                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('settings.general.traeAutoRefresh', 'Trae 自动刷新配额')}</div>
                      <div className="row-desc">{t('settings.general.traeAutoRefreshDesc', '后台自动更新频率')}</div>
                    </div>
                    <div className="row-control">
                      <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                        {traeAutoRefreshCustomMode ? (
                          <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                            <input
                              type="number"
                              min={1}
                              max={999}
                              className="settings-select settings-select--input-mode settings-select--with-unit"
                              value={traeAutoRefresh}
                              placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                              onChange={(e) => setTraeAutoRefresh(sanitizeNumberInput(e.target.value))}
                              onBlur={() => {
                                const normalized = normalizeNumberInput(traeAutoRefresh, 1, 999);
                                if (REFRESH_PRESET_VALUES.includes(normalized)) {
                                  setTraeAutoRefreshCustomMode(false);
                                }
                                setTraeAutoRefresh(normalized);
                              }}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  const normalized = normalizeNumberInput(traeAutoRefresh, 1, 999);
                                  setTraeAutoRefreshCustomMode(false);
                                  setTraeAutoRefresh(normalized);
                                }
                              }}
                            />
                            <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                          </div>
                        ) : (
                          <SettingsRefreshPresetDropdown
                            value={traeAutoRefresh}
                            isPreset={traeAutoRefreshIsPreset}
                            setValue={setTraeAutoRefresh}
                            setCustomMode={setTraeAutoRefreshCustomMode}
                          />
                        )}
                      </div>
                    </div>
                  </div>

                  {renderCurrentAccountRefreshRow('trae')}
                  {renderAccountLevelRefreshConfig('trae')}

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('settings.general.traeAppPath', 'Trae 启动路径')}</div>
                      <div className="row-desc">{t('settings.general.traeAppPathDesc', '留空则使用默认路径')}</div>
                    </div>
                    <div className="row-control row-control--grow settings-claude-launch-control">
                      <div className="settings-claude-launch-row">
                        <input
                          type="text"
                          className="settings-input settings-input--path"
                          value={traeAppPath}
                          placeholder={t('settings.general.traeAppPathPlaceholder', '默认路径')}
                          onChange={(e) => setTraeAppPath(e.target.value)}
                        />
                        <button
                          className="btn btn-secondary"
                          onClick={() => handlePickAppPath('trae')}
                          disabled={isAppPathResetDetecting('trae')}
                        >
                          {t('settings.general.traePathSelect', '选择')}
                        </button>
                        <button
                          className="btn btn-secondary"
                          onClick={() => handleResetAppPath('trae')}
                          disabled={isAppPathResetDetecting('trae')}
                        >
                          <RefreshCw size={16} className={isAppPathResetDetecting('trae') ? 'spin' : undefined} />
                          {isAppPathResetDetecting('trae')
                            ? t('common.loading', '加载中...')
                            : getResetLabelByTarget('trae')}
                        </button>
                      </div>
                      {isWindows && traeLaunchCandidatesTarget === 'trae' && traeLaunchCandidates.length > 0 ? (
                        <div className="settings-claude-candidate-list">
                          {traeLaunchCandidates.map((candidate) => (
                            <button
                              key={`${candidate.target_type}:${candidate.target}`}
                              type="button"
                              className={`settings-claude-candidate-item${
                                traeAppPath.trim() === candidate.target ? ' selected' : ''
                              }`}
                              onClick={() => handleSelectTraeLaunchCandidate('trae', candidate)}
                            >
                              <div className="settings-claude-candidate-main">
                                <span>{candidate.label || 'Trae'}</span>
                                <span className="settings-claude-candidate-badge">EXE</span>
                              </div>
                              <div className="settings-claude-candidate-target">
                                {candidate.target}
                              </div>
                            </button>
                          ))}
                        </div>
                      ) : null}
                    </div>
                  </div>

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
                      <div className="row-desc">{t('quickSettings.quotaAlert.hint', '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。')}</div>
                    </div>
                    <div className="row-control">
                      <label className="switch">
                        <input
                          type="checkbox"
                          checked={traeQuotaAlertEnabled}
                          onChange={(e) => setTraeQuotaAlertEnabled(e.target.checked)}
                        />
                        <span className="slider"></span>
                      </label>
                    </div>
                  </div>
                  {traeQuotaAlertEnabled && (
                    <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                      <div className="row-label">
                        <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                        <div className="row-desc">{t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}</div>
                      </div>
                      <div className="row-control">
                        {traeQuotaAlertThresholdCustomMode ? (
                          <div className="settings-inline-input">
                            <input
                              type="number"
                              min={0}
                              max={100}
                              className="settings-select settings-select--input-mode settings-select--with-unit"
                              value={traeQuotaAlertThreshold}
                              placeholder={t('quickSettings.inputPercent', '输入百分比')}
                              onChange={(e) => setTraeQuotaAlertThreshold(sanitizeNumberInput(e.target.value))}
                              onBlur={() => {
                                const normalized = normalizeNumberInput(traeQuotaAlertThreshold, 0, 100);
                                if (THRESHOLD_PRESET_VALUES.includes(normalized)) {
                                  setTraeQuotaAlertThresholdCustomMode(false);
                                }
                                setTraeQuotaAlertThreshold(normalized);
                              }}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  const normalized = normalizeNumberInput(traeQuotaAlertThreshold, 0, 100);
                                  setTraeQuotaAlertThresholdCustomMode(false);
                                  setTraeQuotaAlertThreshold(normalized);
                                }
                              }}
                            />
                            <span className="settings-input-unit">%</span>
                          </div>
                        ) : (
                          <SettingsThresholdPresetDropdown
                            value={traeQuotaAlertThreshold}
                            isPreset={traeQuotaAlertThresholdIsPreset}
                            setValue={setTraeQuotaAlertThreshold}
                            setCustomMode={setTraeQuotaAlertThresholdCustomMode}
                          />
                        )}
                      </div>
                    </div>
                  )}
                </div>
              </PlatformSettingsSection>

              {renderTraeVariantSettingsGroup({
                target: 'trae_solo',
                order: platformSettingsOrder.trae_solo,
                titleKey: 'quickSettings.traeSolo.title',
                titleDefault: 'TraeWork 设置',
                appPathTitleKey: 'settings.general.traeSoloAppPath',
                appPathTitleDefault: 'TraeWork 启动路径',
                autoRefresh: traeSoloAutoRefresh,
                setAutoRefresh: setTraeSoloAutoRefresh,
                autoRefreshCustomMode: traeSoloAutoRefreshCustomMode,
                setAutoRefreshCustomMode: setTraeSoloAutoRefreshCustomMode,
                autoRefreshIsPreset: traeSoloAutoRefreshIsPreset,
                quotaAlertEnabled: traeSoloQuotaAlertEnabled,
                setQuotaAlertEnabled: setTraeSoloQuotaAlertEnabled,
                quotaAlertThreshold: traeSoloQuotaAlertThreshold,
                setQuotaAlertThreshold: setTraeSoloQuotaAlertThreshold,
                quotaAlertThresholdCustomMode: traeSoloQuotaAlertThresholdCustomMode,
                setQuotaAlertThresholdCustomMode: setTraeSoloQuotaAlertThresholdCustomMode,
                quotaAlertThresholdIsPreset: traeSoloQuotaAlertThresholdIsPreset,
              })}

              {renderTraeVariantSettingsGroup({
                target: 'trae_cn',
                order: platformSettingsOrder.trae_cn,
                titleKey: 'quickSettings.traeCn.title',
                titleDefault: 'Trae CN 设置',
                appPathTitleKey: 'settings.general.traeCnAppPath',
                appPathTitleDefault: 'Trae CN 启动路径',
                autoRefresh: traeCnAutoRefresh,
                setAutoRefresh: setTraeCnAutoRefresh,
                autoRefreshCustomMode: traeCnAutoRefreshCustomMode,
                setAutoRefreshCustomMode: setTraeCnAutoRefreshCustomMode,
                autoRefreshIsPreset: traeCnAutoRefreshIsPreset,
                quotaAlertEnabled: traeCnQuotaAlertEnabled,
                setQuotaAlertEnabled: setTraeCnQuotaAlertEnabled,
                quotaAlertThreshold: traeCnQuotaAlertThreshold,
                setQuotaAlertThreshold: setTraeCnQuotaAlertThreshold,
                quotaAlertThresholdCustomMode: traeCnQuotaAlertThresholdCustomMode,
                setQuotaAlertThresholdCustomMode: setTraeCnQuotaAlertThresholdCustomMode,
                quotaAlertThresholdIsPreset: traeCnQuotaAlertThresholdIsPreset,
              })}

              {renderTraeVariantSettingsGroup({
                target: 'trae_solo_cn',
                order: platformSettingsOrder.trae_solo_cn,
                titleKey: 'quickSettings.traeSoloCn.title',
                titleDefault: 'TraeWork CN 设置',
                appPathTitleKey: 'settings.general.traeSoloCnAppPath',
                appPathTitleDefault: 'TraeWork CN 启动路径',
                autoRefresh: traeSoloCnAutoRefresh,
                setAutoRefresh: setTraeSoloCnAutoRefresh,
                autoRefreshCustomMode: traeSoloCnAutoRefreshCustomMode,
                setAutoRefreshCustomMode: setTraeSoloCnAutoRefreshCustomMode,
                autoRefreshIsPreset: traeSoloCnAutoRefreshIsPreset,
                quotaAlertEnabled: traeSoloCnQuotaAlertEnabled,
                setQuotaAlertEnabled: setTraeSoloCnQuotaAlertEnabled,
                quotaAlertThreshold: traeSoloCnQuotaAlertThreshold,
                setQuotaAlertThreshold: setTraeSoloCnQuotaAlertThreshold,
                quotaAlertThresholdCustomMode: traeSoloCnQuotaAlertThresholdCustomMode,
                setQuotaAlertThresholdCustomMode: setTraeSoloCnQuotaAlertThresholdCustomMode,
                quotaAlertThresholdIsPreset: traeSoloCnQuotaAlertThresholdIsPreset,
              })}

              <PlatformSettingsSection platformId={'workbuddy'} order={platformSettingsOrder.workbuddy}>
                <div className="group-title">{t('quickSettings.workbuddy.title', 'WorkBuddy 设置')}</div>
                <div className="settings-group">
                  <div className="settings-row"><LaunchPathSettingsButton app="workbuddy_ai" /></div>
                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('settings.general.workbuddyAutoRefresh', 'WorkBuddy 自动刷新配额')}</div>
                      <div className="row-desc">{t('settings.general.workbuddyAutoRefreshDesc', '后台自动更新频率')}</div>
                    </div>
                    <div className="row-control">
                      <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                        {workbuddyAutoRefreshCustomMode ? (
                          <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                            <input
                              type="number"
                              min={1}
                              max={999}
                              className="settings-select settings-select--input-mode settings-select--with-unit"
                              value={workbuddyAutoRefresh}
                              placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                              onChange={(e) => setWorkbuddyAutoRefresh(sanitizeNumberInput(e.target.value))}
                              onBlur={() => {
                                const normalized = normalizeNumberInput(workbuddyAutoRefresh, 1, 999);
                                if (REFRESH_PRESET_VALUES.includes(normalized)) {
                                  setWorkbuddyAutoRefreshCustomMode(false);
                                }
                                setWorkbuddyAutoRefresh(normalized);
                              }}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  const normalized = normalizeNumberInput(workbuddyAutoRefresh, 1, 999);
                                  setWorkbuddyAutoRefreshCustomMode(false);
                                  setWorkbuddyAutoRefresh(normalized);
                                }
                              }}
                            />
                            <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                          </div>
                        ) : (
                          <SettingsRefreshPresetDropdown
                            value={workbuddyAutoRefresh}
                            isPreset={workbuddyAutoRefreshIsPreset}
                            setValue={setWorkbuddyAutoRefresh}
                            setCustomMode={setWorkbuddyAutoRefreshCustomMode}
                          />
                        )}
                      </div>
                    </div>
                  </div>

                  {renderCurrentAccountRefreshRow('workbuddy')}
                  {renderAccountLevelRefreshConfig('workbuddy')}

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">
                        {t('settings.general.workbuddyShareSessionsOnSwitch')}
                      </div>
                      <div className="row-desc">
                        {t('settings.general.workbuddyShareSessionsOnSwitchDesc')}
                      </div>
                    </div>
                    <div className="row-control">
                      <label className="switch">
                        <input
                          type="checkbox"
                          checked={workbuddyShareSessionsOnSwitch}
                          onChange={(event) =>
                            setWorkbuddyShareSessionsOnSwitch(event.target.checked)
                          }
                        />
                        <span className="slider"></span>
                      </label>
                    </div>
                  </div>

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('settings.general.workbuddyAppPath')}</div>
                      <div className="row-desc">{t('settings.general.workbuddyAppPathDesc', '留空则使用默认路径')}</div>
                    </div>
                    <div className="row-control row-control--grow">
                      <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                        <input
                          type="text"
                          className="settings-input settings-input--path"
                          value={workbuddyAppPath}
                          placeholder={t('settings.general.workbuddyAppPathPlaceholder', '默认路径')}
                          onChange={(e) => setWorkbuddyAppPath(e.target.value)}
                        />
                        <button
                          className="btn btn-secondary"
                          onClick={() => handlePickAppPath('workbuddy')}
                          disabled={isAppPathResetDetecting('workbuddy')}
                        >
                          {t('settings.general.workbuddyPathSelect', '选择')}
                        </button>
                        <button
                          className="btn btn-secondary"
                          onClick={() => handleResetAppPath('workbuddy')}
                          disabled={isAppPathResetDetecting('workbuddy')}
                        >
                          <RefreshCw size={16} className={isAppPathResetDetecting('workbuddy') ? 'spin' : undefined} />
                          {isAppPathResetDetecting('workbuddy')
                            ? t('common.loading', '加载中...')
                            : getResetLabelByTarget('workbuddy')}
                        </button>
                      </div>
                    </div>
                  </div>

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
                      <div className="row-desc">{t('quickSettings.quotaAlert.hint', '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。')}</div>
                    </div>
                    <div className="row-control">
                      <label className="switch">
                        <input
                          type="checkbox"
                          checked={workbuddyQuotaAlertEnabled}
                          onChange={(e) => setWorkbuddyQuotaAlertEnabled(e.target.checked)}
                        />
                        <span className="slider"></span>
                      </label>
                    </div>
                  </div>
                  {workbuddyQuotaAlertEnabled && (
                    <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                      <div className="row-label">
                        <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                        <div className="row-desc">{t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}</div>
                      </div>
                      <div className="row-control">
                        {workbuddyQuotaAlertThresholdCustomMode ? (
                          <div className="settings-inline-input">
                            <input
                              type="number"
                              min={0}
                              max={100}
                              className="settings-select settings-select--input-mode settings-select--with-unit"
                              value={workbuddyQuotaAlertThreshold}
                              placeholder={t('quickSettings.inputPercent', '输入百分比')}
                              onChange={(e) => setWorkbuddyQuotaAlertThreshold(sanitizeNumberInput(e.target.value))}
                              onBlur={() => {
                                const normalized = normalizeNumberInput(workbuddyQuotaAlertThreshold, 0, 100);
                                if (THRESHOLD_PRESET_VALUES.includes(normalized)) {
                                  setWorkbuddyQuotaAlertThresholdCustomMode(false);
                                }
                                setWorkbuddyQuotaAlertThreshold(normalized);
                              }}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  const normalized = normalizeNumberInput(workbuddyQuotaAlertThreshold, 0, 100);
                                  setWorkbuddyQuotaAlertThresholdCustomMode(false);
                                  setWorkbuddyQuotaAlertThreshold(normalized);
                                }
                              }}
                            />
                            <span className="settings-input-unit">%</span>
                          </div>
                        ) : (
                          <SettingsThresholdPresetDropdown
                            value={workbuddyQuotaAlertThreshold}
                            isPreset={workbuddyQuotaAlertThresholdIsPreset}
                            setValue={setWorkbuddyQuotaAlertThreshold}
                            setCustomMode={setWorkbuddyQuotaAlertThresholdCustomMode}
                          />
                        )}
                      </div>
                    </div>
                  )}
                </div>
              </PlatformSettingsSection>

              <PlatformSettingsSection platformId={'minimaxcode'} order={platformSettingsOrder.minimaxcode}>
                <div className="group-title">{t('quickSettings.minimaxCode.title', 'MiniMax Code 设置')}</div>
                <div className="settings-group">
                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('settings.general.minimaxCodeAutoRefresh', 'MiniMax Code 自动刷新配额')}</div>
                      <div className="row-desc">{t('settings.general.minimaxCodeAutoRefreshDesc', '后台自动更新频率')}</div>
                    </div>
                    <div className="row-control">
                      <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                        {minimaxCodeAutoRefreshCustomMode ? (
                          <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                            <input
                              type="number"
                              min={1}
                              max={999}
                              className="settings-select settings-select--input-mode settings-select--with-unit"
                              value={minimaxCodeAutoRefresh}
                              placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                              onChange={(e) => setMinimaxCodeAutoRefresh(sanitizeNumberInput(e.target.value))}
                              onBlur={() => {
                                const normalized = normalizeNumberInput(minimaxCodeAutoRefresh, 1, 999);
                                if (REFRESH_PRESET_VALUES.includes(normalized)) {
                                  setMinimaxCodeAutoRefreshCustomMode(false);
                                }
                                setMinimaxCodeAutoRefresh(normalized);
                              }}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  const normalized = normalizeNumberInput(minimaxCodeAutoRefresh, 1, 999);
                                  setMinimaxCodeAutoRefreshCustomMode(false);
                                  setMinimaxCodeAutoRefresh(normalized);
                                }
                              }}
                            />
                            <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                          </div>
                        ) : (
                          <SettingsRefreshPresetDropdown
                            value={minimaxCodeAutoRefresh}
                            isPreset={minimaxCodeAutoRefreshIsPreset}
                            setValue={setMinimaxCodeAutoRefresh}
                            setCustomMode={setMinimaxCodeAutoRefreshCustomMode}
                          />
                        )}
                      </div>
                    </div>
                  </div>

                  {renderCurrentAccountRefreshRow('minimaxcode')}
                  {renderAccountLevelRefreshConfig('minimaxcode')}

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('settings.general.minimaxCodeAppPath', 'MiniMax Code 启动路径')}</div>
                      <div className="row-desc">{t('settings.general.minimaxCodeAppPathDesc', '留空则使用默认路径')}</div>
                    </div>
                    <div className="row-control row-control--grow">
                      <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                        <input
                          type="text"
                          className="settings-input settings-input--path"
                          value={minimaxCodeAppPath}
                          placeholder={t('settings.general.minimaxCodeAppPathPlaceholder', '默认路径')}
                          onChange={(e) => setMinimaxCodeAppPath(e.target.value)}
                        />
                        <button
                          className="btn btn-secondary"
                          onClick={() => handlePickAppPath('minimaxcode')}
                          disabled={isAppPathResetDetecting('minimaxcode')}
                        >
                          {t('settings.general.minimaxCodePathSelect', '选择')}
                        </button>
                        <button
                          className="btn btn-secondary"
                          onClick={() => handleResetAppPath('minimaxcode')}
                          disabled={isAppPathResetDetecting('minimaxcode')}
                        >
                          <RefreshCw size={16} className={isAppPathResetDetecting('minimaxcode') ? 'spin' : undefined} />
                          {isAppPathResetDetecting('minimaxcode')
                            ? t('common.loading', '加载中...')
                            : getResetLabelByTarget('minimaxcode')}
                        </button>
                      </div>
                    </div>
                  </div>

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
                      <div className="row-desc">{t('quickSettings.quotaAlert.hint', '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。')}</div>
                    </div>
                    <div className="row-control">
                      <label className="switch">
                        <input
                          type="checkbox"
                          checked={minimaxCodeQuotaAlertEnabled}
                          onChange={(e) => setMinimaxCodeQuotaAlertEnabled(e.target.checked)}
                        />
                        <span className="slider"></span>
                      </label>
                    </div>
                  </div>
                  {minimaxCodeQuotaAlertEnabled && (
                    <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                      <div className="row-label">
                        <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                        <div className="row-desc">{t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}</div>
                      </div>
                      <div className="row-control">
                        {minimaxCodeQuotaAlertThresholdCustomMode ? (
                          <div className="settings-inline-input">
                            <input
                              type="number"
                              min={0}
                              max={100}
                              className="settings-select settings-select--input-mode settings-select--with-unit"
                              value={minimaxCodeQuotaAlertThreshold}
                              placeholder={t('quickSettings.inputPercent', '输入百分比')}
                              onChange={(e) => setMinimaxCodeQuotaAlertThreshold(sanitizeNumberInput(e.target.value))}
                              onBlur={() => {
                                const normalized = normalizeNumberInput(minimaxCodeQuotaAlertThreshold, 0, 100);
                                if (THRESHOLD_PRESET_VALUES.includes(normalized)) {
                                  setMinimaxCodeQuotaAlertThresholdCustomMode(false);
                                }
                                setMinimaxCodeQuotaAlertThreshold(normalized);
                              }}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  const normalized = normalizeNumberInput(minimaxCodeQuotaAlertThreshold, 0, 100);
                                  setMinimaxCodeQuotaAlertThresholdCustomMode(false);
                                  setMinimaxCodeQuotaAlertThreshold(normalized);
                                }
                              }}
                            />
                            <span className="settings-input-unit">%</span>
                          </div>
                        ) : (
                          <SettingsThresholdPresetDropdown
                            value={minimaxCodeQuotaAlertThreshold}
                            isPreset={minimaxCodeQuotaAlertThresholdIsPreset}
                            setValue={setMinimaxCodeQuotaAlertThreshold}
                            setCustomMode={setMinimaxCodeQuotaAlertThresholdCustomMode}
                          />
                        )}
                      </div>
                    </div>
                  )}
                </div>
              </PlatformSettingsSection>

              <PlatformSettingsSection platformId={'zed'} order={platformSettingsOrder.zed}>
                <div className="group-title">{t('quickSettings.zed.title', 'Zed 设置')}</div>
                <div className="settings-group">
                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('settings.general.zedAutoRefresh', 'Zed 自动刷新配额')}</div>
                      <div className="row-desc">{t('settings.general.zedAutoRefreshDesc', '后台自动更新频率')}</div>
                    </div>
                    <div className="row-control">
                      <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                        {zedAutoRefreshCustomMode ? (
                          <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                            <input
                              type="number"
                              min={1}
                              max={999}
                              className="settings-select settings-select--input-mode settings-select--with-unit"
                              value={zedAutoRefresh}
                              placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                              onChange={(e) => setZedAutoRefresh(sanitizeNumberInput(e.target.value))}
                              onBlur={() => {
                                const normalized = normalizeNumberInput(zedAutoRefresh, 1, 999);
                                setZedAutoRefresh(normalized);
                                setZedAutoRefreshCustomMode(false);
                              }}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  const normalized = normalizeNumberInput(zedAutoRefresh, 1, 999);
                                  setZedAutoRefresh(normalized);
                                  setZedAutoRefreshCustomMode(false);
                                }
                              }}
                            />
                            <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                          </div>
                        ) : (
                          <SettingsRefreshPresetDropdown
                            value={zedAutoRefresh}
                            isPreset={zedAutoRefreshIsPreset}
                            setValue={setZedAutoRefresh}
                            setCustomMode={setZedAutoRefreshCustomMode}
                          />
                        )}
                      </div>
                    </div>
                  </div>

                  {renderCurrentAccountRefreshRow('zed')}
                  {renderAccountLevelRefreshConfig('zed')}

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('settings.general.zedAppPath', 'Zed 启动路径')}</div>
                      <div className="row-desc">{t('settings.general.zedAppPathDesc', '留空则使用默认路径')}</div>
                    </div>
                    <div className="row-control row-control--grow">
                      <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                        <input
                          type="text"
                          className="settings-input settings-input--path"
                          value={zedAppPath}
                          placeholder={t('settings.general.zedAppPathPlaceholder', '默认路径')}
                          onChange={(e) => setZedAppPath(e.target.value)}
                        />
                        <button
                          className="btn btn-secondary"
                          onClick={() => handlePickAppPath('zed')}
                          disabled={isAppPathResetDetecting('zed')}
                        >
                          {t('settings.general.zedPathSelect', '选择')}
                        </button>
                        <button
                          className="btn btn-secondary"
                          onClick={() => handleResetAppPath('zed')}
                          disabled={isAppPathResetDetecting('zed')}
                        >
                          <RefreshCw size={16} className={isAppPathResetDetecting('zed') ? 'spin' : undefined} />
                          {isAppPathResetDetecting('zed')
                            ? t('common.loading', '加载中...')
                            : getResetLabelByTarget('zed')}
                        </button>
                      </div>
                    </div>
                  </div>

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
                      <div className="row-desc">{t('quickSettings.quotaAlert.hint', '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。')}</div>
                    </div>
                    <div className="row-control">
                      <label className="switch">
                        <input
                          type="checkbox"
                          checked={zedQuotaAlertEnabled}
                          onChange={(e) => setZedQuotaAlertEnabled(e.target.checked)}
                        />
                        <span className="slider"></span>
                      </label>
                    </div>
                  </div>
                  {zedQuotaAlertEnabled && (
                    <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                      <div className="row-label">
                        <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                        <div className="row-desc">{t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}</div>
                      </div>
                      <div className="row-control">
                        {zedQuotaAlertThresholdCustomMode ? (
                          <div className="settings-inline-input">
                            <input
                              type="number"
                              min={0}
                              max={100}
                              className="settings-select settings-select--input-mode settings-select--with-unit"
                              value={zedQuotaAlertThreshold}
                              placeholder={t('quickSettings.inputPercent', '输入百分比')}
                              onChange={(e) => setZedQuotaAlertThreshold(sanitizeNumberInput(e.target.value))}
                              onBlur={() => {
                                const normalized = normalizeNumberInput(zedQuotaAlertThreshold, 0, 100);
                                setZedQuotaAlertThreshold(normalized);
                                setZedQuotaAlertThresholdCustomMode(false);
                              }}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  const normalized = normalizeNumberInput(zedQuotaAlertThreshold, 0, 100);
                                  setZedQuotaAlertThreshold(normalized);
                                  setZedQuotaAlertThresholdCustomMode(false);
                                }
                              }}
                            />
                            <span className="settings-input-unit">%</span>
                          </div>
                        ) : (
                          <SettingsThresholdPresetDropdown
                            value={zedQuotaAlertThreshold}
                            isPreset={zedQuotaAlertThresholdIsPreset}
                            setValue={setZedQuotaAlertThreshold}
                            setCustomMode={setZedQuotaAlertThresholdCustomMode}
                          />
                        )}
                      </div>
                    </div>
                  )}
                </div>
              </PlatformSettingsSection>

              <PlatformSettingsSection platformId={'cursor'} order={platformSettingsOrder.cursor}>
                <div className="group-title">{t('quickSettings.cursor.title', 'Cursor 设置')}</div>
                <div className="settings-group">
              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('quickSettings.cursorRefreshInterval', '配额自动刷新')}</div>
                  <div className="row-desc">{t('settings.general.windsurfAutoRefreshDesc', '后台自动更新频率')}</div>
                </div>
                <div className="row-control">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                    {cursorAutoRefreshCustomMode ? (
                      <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                        <input
                          type="number"
                          min={1}
                          max={999}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={cursorAutoRefresh}
                          placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                          onChange={(e) => setCursorAutoRefresh(sanitizeNumberInput(e.target.value))}
                          onBlur={() => {
                            const normalized = normalizeNumberInput(cursorAutoRefresh, 1, 999);
                            setCursorAutoRefresh(normalized);
                            setCursorAutoRefreshCustomMode(false);
                          }}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter') {
                              e.preventDefault();
                              const normalized = normalizeNumberInput(cursorAutoRefresh, 1, 999);
                              setCursorAutoRefresh(normalized);
                              setCursorAutoRefreshCustomMode(false);
                            }
                          }}
                        />
                        <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                      </div>
                    ) : (
                      <SettingsRefreshPresetDropdown
                        value={cursorAutoRefresh}
                        isPreset={cursorAutoRefreshIsPreset}
                        setValue={setCursorAutoRefresh}
                        setCustomMode={setCursorAutoRefreshCustomMode}
                      />
                    )}
                  </div>
                </div>
              </div>

              {renderCurrentAccountRefreshRow('cursor')}
              {renderAccountLevelRefreshConfig('cursor')}

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('quickSettings.cursor.appPath', 'Cursor 路径')}</div>
                  <div className="row-desc">{t('settings.general.codexAppPathDesc', '留空则使用默认路径')}</div>
                </div>
                <div className="row-control row-control--grow">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                    <input
                      type="text"
                      className="settings-input settings-input--path"
                      value={cursorAppPath}
                      placeholder={t('settings.general.codexAppPathPlaceholder', '默认路径')}
                      onChange={(e) => setCursorAppPath(e.target.value)}
                    />
                    <button
                      className="btn btn-secondary"
                      onClick={() => handlePickAppPath('cursor')}
                      disabled={isAppPathResetDetecting('cursor')}
                    >
                      {t('settings.general.codexPathSelect', '选择')}
                    </button>
                    <button
                      className="btn btn-secondary"
                      onClick={() => handleResetAppPath('cursor')}
                      disabled={isAppPathResetDetecting('cursor')}
                    >
                      <RefreshCw size={16} className={isAppPathResetDetecting('cursor') ? 'spin' : undefined} />
                      {isAppPathResetDetecting('cursor')
                        ? t('common.loading', '加载中...')
                        : getResetLabelByTarget('cursor')}
                    </button>
                  </div>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
                  <div className="row-desc">{t('quickSettings.quotaAlert.hint', '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={cursorQuotaAlertEnabled}
                      onChange={(e) => setCursorQuotaAlertEnabled(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>
              {cursorQuotaAlertEnabled && (
                <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                  <div className="row-label">
                    <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                    <div className="row-desc">{t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}</div>
                  </div>
                  <div className="row-control">
                    {cursorQuotaAlertThresholdCustomMode ? (
                      <div className="settings-inline-input">
                        <input
                          type="number"
                          min={0}
                          max={100}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={cursorQuotaAlertThreshold}
                          placeholder={t('quickSettings.inputPercent', '输入百分比')}
                          onChange={(e) => setCursorQuotaAlertThreshold(sanitizeNumberInput(e.target.value))}
                          onBlur={() => {
                            const normalized = normalizeNumberInput(cursorQuotaAlertThreshold, 0, 100);
                            setCursorQuotaAlertThreshold(normalized);
                            setCursorQuotaAlertThresholdCustomMode(false);
                          }}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter') {
                              e.preventDefault();
                              const normalized = normalizeNumberInput(cursorQuotaAlertThreshold, 0, 100);
                              setCursorQuotaAlertThreshold(normalized);
                              setCursorQuotaAlertThresholdCustomMode(false);
                            }
                          }}
                        />
                        <span className="settings-input-unit">%</span>
                      </div>
                    ) : (
                      <SettingsThresholdPresetDropdown
                        value={cursorQuotaAlertThreshold}
                        isPreset={cursorQuotaAlertThresholdIsPreset}
                        setValue={setCursorQuotaAlertThreshold}
                        setCustomMode={setCursorQuotaAlertThresholdCustomMode}
                      />
                    )}
                  </div>
                </div>
              )}
            </div>
              </PlatformSettingsSection>
<PlatformSettingsSection platformId={'grok'} order={platformSettingsOrder.grok}>
                <div className="group-title">{t('quickSettings.grok.title', 'Grok CLI 设置')}</div>
                <div className="settings-group">
                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('quickSettings.grok.cliPath', 'CLI 路径')}</div>
                      <div className="row-desc">
                        {grokCliStatus?.available
                          ? t('quickSettings.grok.cliDetected', '已检测 {{version}} · {{path}}', {
                              version: grokCliStatus.version || '--',
                              path: grokCliStatus.binaryPath || '--',
                            })
                          : t('quickSettings.grok.cliMissing', '未检测到 Grok CLI，可填写自定义路径')}
                      </div>
                    </div>
                    <div className="row-control">
                      <input
                        className="settings-input settings-input--path"
                        value={grokCliPath}
                        placeholder={grokCliStatus?.binaryPath || '~/.grok/bin/grok'}
                        onChange={(event) => {
                          setGrokCliPath(event.target.value);
                          setGrokCliStatusError(null);
                        }}
                      />
                      <button
                        type="button"
                        className="btn btn-secondary"
                        onClick={() => void saveGrokCliPath()}
                        disabled={grokCliSaving}
                      >
                        <Save size={14} />
                        {grokCliSaving ? t('common.loading', '加载中...') : t('common.save', '保存')}
                      </button>
                    </div>
                  </div>
                  {grokCliStatusError && <div className="form-error">{grokCliStatusError}</div>}

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">
                        {t('quickSettings.grok.syncOfficialAuthOnSwitch', '切号同步官方登录')}
                      </div>
                      <div className="row-desc">
                        {t(
                          'quickSettings.grok.syncOfficialAuthOnSwitchDesc',
                          '开启后，默认实例切换 OAuth 账号会写入官方 ~/.grok/auth.json；关闭时使用独立 GROK_HOME。API Key 和多开实例不改写官方登录。',
                        )}
                      </div>
                    </div>
                    <div className="row-control">
                      <label className="switch">
                        <input
                          type="checkbox"
                          checked={grokSyncOfficialAuthOnSwitch}
                          onChange={(event) =>
                            setGrokSyncOfficialAuthOnSwitch(event.target.checked)
                          }
                        />
                        <span className="slider"></span>
                      </label>
                    </div>
                  </div>

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">
                        {t(
                          'settings.general.grokOpencodeAuthOverwrite',
                          '切换 Grok 时覆盖 OpenCode 登录信息',
                        )}
                      </div>
                      <div className="row-desc">
                        {t(
                          'settings.general.grokOpencodeAuthOverwriteDesc',
                          '关闭后只切 Grok，不改 OpenCode 当前登录态',
                        )}
                      </div>
                    </div>
                    <div className="row-control">
                      <label className="switch">
                        <input
                          type="checkbox"
                          checked={grokOpencodeAuthOverwriteOnSwitch}
                          onChange={(event) => {
                            const enabled = event.target.checked;
                            setGrokOpencodeAuthOverwriteOnSwitch(enabled);
                            if (!enabled) {
                              setGrokOpencodeSyncOnSwitch(false);
                            }
                          }}
                        />
                        <span className="slider"></span>
                      </label>
                    </div>
                  </div>

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">
                        {t('settings.general.grokOpencodeRestart', '切换 Grok 时自动重启 OpenCode')}
                      </div>
                      <div className="row-desc">
                        {grokOpencodeAuthOverwriteOnSwitch
                          ? t(
                              'settings.general.grokOpencodeRestartDesc',
                              '覆盖 OpenCode 登录信息后重启，使授权立即生效',
                            )
                          : t(
                              'settings.general.grokOpencodeRestartRequiresOverwrite',
                              '请先开启“切换 Grok 时覆盖 OpenCode 登录信息”',
                            )}
                      </div>
                    </div>
                    <div className="row-control">
                      <label className="switch">
                        <input
                          type="checkbox"
                          checked={grokOpencodeSyncOnSwitch}
                          onChange={(event) => setGrokOpencodeSyncOnSwitch(event.target.checked)}
                          disabled={!grokOpencodeAuthOverwriteOnSwitch}
                        />
                        <span className="slider"></span>
                      </label>
                    </div>
                  </div>

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('quickSettings.grokRefreshInterval', '配额自动刷新')}</div>
                      <div className="row-desc">{t('settings.general.windsurfAutoRefreshDesc', '后台自动更新频率')}</div>
                    </div>
                    <div className="row-control">
                      <div className="settings-inline-input">
                        <input
                          type="number"
                          min={-1}
                          max={999}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={grokAutoRefresh}
                          onChange={(event) => {
                            if (/^-?\d*$/.test(event.target.value)) {
                              setGrokAutoRefresh(event.target.value);
                            }
                          }}
                          onBlur={() => setGrokAutoRefresh(normalizeNumberInput(grokAutoRefresh, -1, 999))}
                        />
                        <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                      </div>
                    </div>
                  </div>

                  {renderCurrentAccountRefreshRow('grok')}
                  {renderAccountLevelRefreshConfig('grok')}

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
                      <div className="row-desc">{t('grok.quotaAlert.hint', '当当前账号任意配额项低于阈值时，发送原生通知并在页面提示快捷切号。')}</div>
                    </div>
                    <div className="row-control">
                      <label className="switch">
                        <input
                          type="checkbox"
                          checked={grokQuotaAlertEnabled}
                          onChange={(event) => setGrokQuotaAlertEnabled(event.target.checked)}
                        />
                        <span className="slider"></span>
                      </label>
                    </div>
                  </div>
                  {grokQuotaAlertEnabled && (
                    <div className="settings-row">
                      <div className="row-label">
                        <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                        <div className="row-desc">{t('grok.quotaAlert.thresholdDesc', '任意配额项低于此百分比时触发预警')}</div>
                      </div>
                      <div className="row-control">
                        <div className="settings-inline-input">
                          <input
                            type="number"
                            min={0}
                            max={100}
                            className="settings-select settings-select--input-mode settings-select--with-unit"
                            value={grokQuotaAlertThreshold}
                            onChange={(event) => setGrokQuotaAlertThreshold(sanitizeNumberInput(event.target.value))}
                            onBlur={() => setGrokQuotaAlertThreshold(normalizeNumberInput(grokQuotaAlertThreshold, 0, 100))}
                          />
                          <span className="settings-input-unit">%</span>
                        </div>
                      </div>
                    </div>
                  )}
                </div>
              </PlatformSettingsSection>
            </div>

          </fieldset>
          </>
        );
}
