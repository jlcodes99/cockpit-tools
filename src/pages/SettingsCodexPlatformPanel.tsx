import { AutoSwitchAccountScopeSelector } from '../components/AutoSwitchAccountScopeSelector';
import { CodexSshSyncSettingsControl } from '../components/codex/CodexSshSyncSettingsControl';
import './settings/Settings.css';
import { RefreshCw } from 'lucide-react';
import type { SettingsPageViewProps } from "./SettingsPageView";
import { useRemoteConfigStore } from '../stores/useRemoteConfigStore';

/** 渲染 SettingsGeneralPanel 的 jsx:platformSettingsOrder.codex 业务面板。 */
export function SettingsCodexPlatformPanel(props: SettingsPageViewProps) {
  const {
    codexAppPath,
    codexAppScanError,
    codexAppUiInjectionEnabled,
    codexOAuthAppVersion,
    codexAutoRefresh,
    codexAutoRefreshCustomMode,
    codexAutoRefreshIsPreset,
    codexAutoSwitchAccountScopeMode,
    codexAutoSwitchEnabled,
    codexAutoSwitchSelectedAccountIds,
    codexGroups,
    codexHideRelayQuota,
    codexLaunchCandidates,
    codexLaunchOnSwitch,
    codexLocalAccessEntryVisible,
    codexQuotaAlertEnabled,
    codexQuotaAlertThreshold,
    codexQuotaAlertThresholdCustomMode,
    codexQuotaAlertThresholdIsPreset,
    codexRestartSpecifiedAppOnSwitch,
    codexScopeAccounts,
    codexSpecifiedAppPath,
    codexSyncWsl,
    codexWslConfigDir,
    getResetLabelByTarget,
    handlePickAppPath,
    handlePickCodexSpecifiedAppPath,
    handleResetAppPath,
    handleSelectCodexLaunchCandidate,
    hermesAuthOverwriteOnSwitch,
    isAppPathResetDetecting,
    isWindows,
    normalizeNumberInput,
    openclawAuthOverwriteOnSwitch,
    opencodeAppPath,
    opencodeAuthOverwriteOnSwitch,
    opencodeSyncOnSwitch,
    platformSettingsOrder,
    renderAccountLevelRefreshConfig,
    renderCurrentAccountRefreshRow,
    sanitizeNumberInput,
    setCodexAppPath,
    setCodexAppScanError,
    setCodexAppUiInjectionEnabled,
    setCodexOAuthAppVersion,
    setCodexAutoRefresh,
    setCodexAutoRefreshCustomMode,
    setCodexAutoSwitchAccountScopeMode,
    setCodexAutoSwitchSelectedAccountIds,
    setCodexHideRelayQuota,
    setCodexLaunchCandidates,
    setCodexLaunchOnSwitch,
    setCodexLocalAccessEntryVisible,
    setCodexQuotaAlertEnabled,
    setCodexQuotaAlertThreshold,
    setCodexQuotaAlertThresholdCustomMode,
    setCodexRestartSpecifiedAppOnSwitch,
    setCodexSpecifiedAppPath,
    setCodexSyncWsl,
    setCodexWslConfigDir,
    setHermesAuthOverwriteOnSwitch,
    setOpenclawAuthOverwriteOnSwitch,
    setOpencodeAppPath,
    setOpencodeAuthOverwriteOnSwitch,
    setOpencodeSyncOnSwitch,
    t,
  } = props;
  const remoteCodexOAuthAppVersion = useRemoteConfigStore(
    (state) => state.state.codexOAuthAppVersion,
  );
  return <div style={{ order: platformSettingsOrder.codex }}>
                <div className="group-title">{t('settings.general.codexSettingsTitle', 'Codex 设置')}</div>
                <div className="settings-group">
              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">
                    {t('settings.general.codexLocalAccessEntryVisible', '显示 API 服务入口')}
                  </div>
                  <div className="row-desc">
                    {t(
                      'settings.general.codexLocalAccessEntryVisibleDesc',
                      '仅控制 Codex 总览中的 API 服务入口显示，不会停止本地 API 服务；关闭后可在这里重新打开。',
                    )}
                  </div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={codexLocalAccessEntryVisible}
                      onChange={(e) => setCodexLocalAccessEntryVisible(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>
              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.codexAutoRefresh')}</div>
                  <div className="row-desc">{t('settings.general.codexAutoRefreshDesc')}</div>
                </div>
                <div className="row-control">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                    {codexAutoRefreshCustomMode ? (
                      <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                        <input
                          type="number"
                          min={1}
                          max={999}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={codexAutoRefresh}
                          placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                          onChange={(e) => setCodexAutoRefresh(sanitizeNumberInput(e.target.value))}
                        onBlur={() => {
                          const normalized = normalizeNumberInput(codexAutoRefresh, 1, 999);
                          setCodexAutoRefresh(normalized);
                          setCodexAutoRefreshCustomMode(false);
                        }}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') {
                            e.preventDefault();
                            const normalized = normalizeNumberInput(codexAutoRefresh, 1, 999);
                            setCodexAutoRefresh(normalized);
                            setCodexAutoRefreshCustomMode(false);
                          }
                        }}
                      />
                        <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                      </div>
                    ) : (
                      <select
                        className="settings-select"
                        style={{ minWidth: '120px', width: 'auto' }}
                        value={codexAutoRefresh}
                        onChange={(e) => {
                          const val = e.target.value;
                          if (val === 'custom') {
                            setCodexAutoRefreshCustomMode(true);
                            setCodexAutoRefresh(codexAutoRefresh !== '-1' ? codexAutoRefresh : '1');
                            return;
                          }
                          setCodexAutoRefreshCustomMode(false);
                          setCodexAutoRefresh(val);
                        }}
                      >
                        {!codexAutoRefreshIsPreset && (
                          <option value={codexAutoRefresh}>
                            {codexAutoRefresh} {t('settings.general.minutes')}
                          </option>
                        )}
                        <option value="-1">{t('settings.general.autoRefreshDisabled')}</option>
                        <option value="2">2 {t('settings.general.minutes')}</option>
                        <option value="5">5 {t('settings.general.minutes')}</option>
                        <option value="10">10 {t('settings.general.minutes')}</option>
                        <option value="15">15 {t('settings.general.minutes')}</option>
                        <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                      </select>
                    )}
                  </div>
                </div>
              </div>

              {renderCurrentAccountRefreshRow('codex')}
              {renderAccountLevelRefreshConfig('codex')}

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">
                    {t('settings.general.codexOAuthAppVersion', 'OAuth 客户端版本')}
                  </div>
                  <div className="row-desc">
                    {t('settings.general.codexOAuthAppVersionDesc', {
                      defaultValue: '留空跟随远端默认值 {{version}}；填写后仅覆盖 OAuth 授权链接中的版本字段。',
                      version: remoteCodexOAuthAppVersion || '26.820.60940',
                    })}
                  </div>
                </div>
                <div className="row-control row-control--grow">
                  <input
                    type="text"
                    className="settings-input"
                    value={codexOAuthAppVersion}
                    placeholder={t('settings.general.codexOAuthAppVersionPlaceholder', {
                      defaultValue: '留空使用 {{version}}',
                      version: remoteCodexOAuthAppVersion || '26.820.60940',
                    })}
                    onChange={(event) => setCodexOAuthAppVersion(event.target.value)}
                  />
                </div>
              </div>

              {isWindows && (
                <>
                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('settings.general.codexSyncWsl')}</div>
                      <div className="row-desc">{t('settings.general.codexSyncWslDesc')}</div>
                    </div>
                    <div className="row-control">
                      <label className="switch">
                        <input
                          type="checkbox"
                          checked={codexSyncWsl}
                          onChange={(e) => setCodexSyncWsl(e.target.checked)}
                        />
                        <span className="slider"></span>
                      </label>
                    </div>
                  </div>

                  {codexSyncWsl && (
                    <div className="settings-row">
                      <div className="row-label">
                        <div className="row-title">{t('settings.general.codexWslConfigDir')}</div>
                        <div className="row-desc">{t('settings.general.codexWslConfigDirDesc')}</div>
                      </div>
                      <div className="row-control row-control--grow">
                        <input
                          type="text"
                          className="settings-input settings-input--path"
                          value={codexWslConfigDir}
                          placeholder={t('settings.general.codexWslConfigDirPlaceholder')}
                          onChange={(e) => setCodexWslConfigDir(e.target.value)}
                        />
                      </div>
                    </div>
                  )}
                </>
              )}

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.codexAppUiInjection')}</div>
                  <div className="row-desc">{t('settings.general.codexAppUiInjectionDesc')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={codexAppUiInjectionEnabled}
                      onChange={(event) => setCodexAppUiInjectionEnabled(event.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>

              <CodexSshSyncSettingsControl variant="settings" />

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.codexAppPath', 'Codex 启动路径')}</div>
                  <div className="row-desc">{t('settings.general.codexAppPathDesc', '留空则使用默认路径')}</div>
                </div>
                <div className="row-control row-control--grow settings-claude-launch-control">
                  <div className="settings-claude-launch-row">
                    <input
                      type="text"
                      className="settings-input settings-input--path"
                      value={codexAppPath}
                      placeholder={t('settings.general.codexAppPathPlaceholder', '默认路径')}
                      onChange={(e) => {
                        setCodexLaunchCandidates([]);
                        setCodexAppScanError('');
                        setCodexAppPath(e.target.value);
                      }}
                    />
                    <button
                      className="btn btn-secondary"
                      onClick={() => handlePickAppPath('codex')}
                      disabled={isAppPathResetDetecting('codex')}
                    >
                      {t('settings.general.codexPathSelect', '选择')}
                    </button>
                    <button
                      className="btn btn-secondary"
                      onClick={() => handleResetAppPath('codex')}
                      disabled={isAppPathResetDetecting('codex')}
                    >
                      <RefreshCw size={16} className={isAppPathResetDetecting('codex') ? 'spin' : undefined} />
                      {isAppPathResetDetecting('codex')
                        ? t('common.loading', '加载中...')
                        : getResetLabelByTarget('codex')}
                    </button>
                  </div>
                  {isWindows && codexLaunchCandidates.length > 0 ? (
                    <div className="settings-claude-candidate-list">
                      {codexLaunchCandidates.map((candidate) => (
                        <button
                          key={`${candidate.target_type}:${candidate.target}`}
                          type="button"
                          className={`settings-claude-candidate-item${
                            codexAppPath.trim() === candidate.target ? ' selected' : ''
                          }`}
                          onClick={() => handleSelectCodexLaunchCandidate(candidate)}
                        >
                          <div className="settings-claude-candidate-main">
                            <span>{candidate.label || t('nav.codex', 'Codex')}</span>
                            <span className="settings-claude-candidate-badge">EXE</span>
                          </div>
                          <div className="settings-claude-candidate-target">
                            {candidate.target}
                          </div>
                        </button>
                      ))}
                    </div>
                  ) : null}
                  {codexAppScanError ? (
                    <p className="settings-app-path-error" role="alert">
                      {codexAppScanError}
                    </p>
                  ) : null}
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.codexLaunchOnSwitch', '切换 Codex 时自动启动 Codex App')}</div>
                  <div className="row-desc">{t('settings.general.codexLaunchOnSwitchDesc', '切换账号后自动启动或重启 Codex App')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={codexLaunchOnSwitch}
                      onChange={(e) => setCodexLaunchOnSwitch(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">
                    {t(
                      'settings.general.codexRestartSpecifiedAppOnSwitch',
                      '切换 Codex 时重启指定应用',
                    )}
                  </div>
                  <div className="row-desc">
                    {t(
                      'settings.general.codexRestartSpecifiedAppOnSwitchDesc',
                      '开启后按下方路径重启指定应用（适用于依赖插件宿主的场景）',
                    )}
                  </div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={codexRestartSpecifiedAppOnSwitch}
                      onChange={(e) =>
                        setCodexRestartSpecifiedAppOnSwitch(e.target.checked)
                      }
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">
                    {t('settings.general.codexSpecifiedAppPath', '指定应用启动路径')}
                  </div>
                  <div className="row-desc">
                    {t(
                      'settings.general.codexSpecifiedAppPathDesc',
                      '填写需联动重启的应用路径',
                    )}
                  </div>
                </div>
                <div className="row-control row-control--grow">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                    <input
                      type="text"
                      className="settings-input settings-input--path"
                      value={codexSpecifiedAppPath}
                      placeholder={t(
                        'settings.general.codexSpecifiedAppPathPlaceholder',
                        '例如 /Applications/Host.app',
                      )}
                      onChange={(e) => setCodexSpecifiedAppPath(e.target.value)}
                    />
                    <button className="btn btn-secondary" onClick={handlePickCodexSpecifiedAppPath}>
                      {t('settings.general.codexPathSelect', '选择')}
                    </button>
                    <button
                      className="btn btn-secondary"
                      onClick={() => setCodexSpecifiedAppPath('')}
                    >
                      {t('settings.general.codexSpecifiedAppPathClear', '清空')}
                    </button>
                  </div>
                </div>
              </div>
              <div className="settings-row settings-row--align-start">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.codexAutoSwitchAccountScope')}</div>
                  <div className="row-desc">
                    {t('settings.general.codexAutoSwitchAccountScopeDesc', {
                      status: codexAutoSwitchEnabled ? t('common.enabled') : t('common.disabled'),
                    })}
                  </div>
                </div>
                <div className="row-control row-control--grow">
                  <AutoSwitchAccountScopeSelector
                    mode={codexAutoSwitchAccountScopeMode}
                    onModeChange={setCodexAutoSwitchAccountScopeMode}
                    selectedAccountIds={codexAutoSwitchSelectedAccountIds}
                    onSelectedAccountIdsChange={setCodexAutoSwitchSelectedAccountIds}
                    accounts={codexScopeAccounts}
                    groups={codexGroups}
                    useDialog
                  />
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.opencodeAuthOverwrite')}</div>
                  <div className="row-desc">{t('settings.general.opencodeAuthOverwriteDesc')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={opencodeAuthOverwriteOnSwitch}
                      onChange={(e) => {
                        const enabled = e.target.checked;
                        setOpencodeAuthOverwriteOnSwitch(enabled);
                        if (!enabled) {
                          setOpencodeSyncOnSwitch(false);
                        }
                      }}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.openclawAuthOverwrite')}</div>
                  <div className="row-desc">{t('settings.general.openclawAuthOverwriteDesc')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={openclawAuthOverwriteOnSwitch}
                      onChange={(e) => setOpenclawAuthOverwriteOnSwitch(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">
                    {t('settings.general.hermesAuthOverwrite', '切换 Codex 时同步 Hermes')}
                  </div>
                  <div className="row-desc">
                    {t(
                      'settings.general.hermesAuthOverwriteDesc',
                      '仅 OAuth 账号：切号后写入 ~/.hermes/auth.json 的 openai-codex 凭据（默认关闭）'
                    )}
                  </div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={hermesAuthOverwriteOnSwitch}
                      onChange={(e) => setHermesAuthOverwriteOnSwitch(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.opencodeRestart')}</div>
                  <div className="row-desc">
                    {opencodeAuthOverwriteOnSwitch
                      ? t('settings.general.opencodeRestartDesc')
                      : t('settings.general.opencodeRestartRequiresOverwrite')}
                  </div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={opencodeSyncOnSwitch}
                      onChange={(e) => setOpencodeSyncOnSwitch(e.target.checked)}
                      disabled={!opencodeAuthOverwriteOnSwitch}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.opencodeAppPath')}</div>
                  <div className="row-desc">
                    {t('settings.general.opencodeAppPathDesc')}
                  </div>
                </div>
                <div className="row-control row-control--grow">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                    <input
                      type="text"
                      className="settings-input settings-input--path"
                      value={opencodeAppPath}
                      placeholder={t('settings.general.opencodeAppPathPlaceholder')}
                      onChange={(e) => setOpencodeAppPath(e.target.value)}
                    />
                    <button
                      className="btn btn-secondary"
                      onClick={() => handlePickAppPath('opencode')}
                      disabled={isAppPathResetDetecting('opencode')}
                    >
                      {t('settings.general.opencodePathSelect', '选择')}
                    </button>
                    <button
                      className="btn btn-secondary"
                      onClick={() => handleResetAppPath('opencode')}
                      disabled={isAppPathResetDetecting('opencode')}
                    >
                      <RefreshCw size={16} className={isAppPathResetDetecting('opencode') ? 'spin' : undefined} />
                      {isAppPathResetDetecting('opencode')
                        ? t('common.loading', '加载中...')
                        : getResetLabelByTarget('opencode')}
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
                      checked={codexQuotaAlertEnabled}
                      onChange={(e) => setCodexQuotaAlertEnabled(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>
              {codexQuotaAlertEnabled && (
                <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                  <div className="row-label">
                    <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                    <div className="row-desc">{t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}</div>
                  </div>
                  <div className="row-control">
                    {codexQuotaAlertThresholdCustomMode ? (
                      <div className="settings-inline-input">
                        <input
                          type="number"
                          min={0}
                          max={100}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={codexQuotaAlertThreshold}
                          placeholder={t('quickSettings.inputPercent', '输入百分比')}
                          onChange={(e) => setCodexQuotaAlertThreshold(sanitizeNumberInput(e.target.value))}
                          onBlur={() => {
                            const normalized = normalizeNumberInput(codexQuotaAlertThreshold, 0, 100);
                            setCodexQuotaAlertThreshold(normalized);
                            setCodexQuotaAlertThresholdCustomMode(false);
                          }}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter') {
                              e.preventDefault();
                              const normalized = normalizeNumberInput(codexQuotaAlertThreshold, 0, 100);
                              setCodexQuotaAlertThreshold(normalized);
                              setCodexQuotaAlertThresholdCustomMode(false);
                            }
                          }}
                        />
                        <span className="settings-input-unit">%</span>
                      </div>
                    ) : (
                      <select
                        className="settings-select"
                        value={codexQuotaAlertThreshold}
                        onChange={(e) => {
                          const val = e.target.value;
                          if (val === 'custom') {
                            setCodexQuotaAlertThresholdCustomMode(true);
                            setCodexQuotaAlertThreshold(codexQuotaAlertThreshold || '20');
                            return;
                          }
                          setCodexQuotaAlertThresholdCustomMode(false);
                          setCodexQuotaAlertThreshold(val);
                        }}
                      >
                        {!codexQuotaAlertThresholdIsPreset && (
                          <option value={codexQuotaAlertThreshold}>{codexQuotaAlertThreshold}%</option>
                        )}
                        <option value="0">0%</option>
                        <option value="20">20%</option>
                        <option value="40">40%</option>
                        <option value="60">60%</option>
                        <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                      </select>
                    )}
                  </div>
                </div>
              )}

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">
                    {t('settings.general.codexHideRelayQuota', '隐藏中转站额度')}
                  </div>
                  <div className="row-desc">
                    {t(
                      'settings.general.codexHideRelayQuotaDesc',
                      '开启后，Codex 账号总览隐藏中转 / New API 类额度面板，减轻列表重叠与视觉干扰。',
                    )}
                  </div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={codexHideRelayQuota}
                      onChange={(e) => setCodexHideRelayQuota(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>
            </div>

              </div>;
}
