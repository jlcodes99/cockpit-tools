import { UnlockFireworksOverlay } from '../components/UnlockFireworksOverlay';
import { SettingsAccountTransferSection } from '../components/SettingsAccountTransferSection';
import { SettingsWebdavSyncSection } from '../components/SettingsWebdavSyncSection';
import './settings/Settings.css';
import { Github, User, Rocket, Save, AlertCircle, RefreshCw, Heart, MessageSquare, FileText, Download, X } from 'lucide-react';
import type { PlatformId } from '../types/platform';
import type { useSettingsPageController } from "./SettingsPage";
import { SettingsGeneralPanel } from "./SettingsGeneralPanel";


export type SettingsPageViewProps = ReturnType<typeof useSettingsPageController>;

/** 渲染 SettingsPage 的界面；业务状态与动作统一由 Controller 提供。 */
export function SettingsPageView(props: SettingsPageViewProps) {
  const {
    activeTab,
    actualPort,
    appVersion,
    defaultPort,
    generateReportToken,
    globalProxyEnabled,
    globalProxyNoProxy,
    globalProxyUrl,
    handleAboutAvatarTap,
    handleCheckUpdate,
    handleCloseMenuBarQuotaModal,
    handleCloseReleaseHistory,
    handleConfirmMenuBarQuotaModal,
    handleDownloadReleaseVersion,
    handleOpenReleaseHistory,
    handleSaveNetworkConfig,
    menuBarQuotaDraftPlatform,
    menuBarQuotaDraftShowPrefix,
    menuBarQuotaModalMode,
    menuBarQuotaModalOpen,
    menuBarQuotaPlatformOptions,
    needsRestart,
    networkSaving,
    openLink,
    reducedMotionEnabled,
    releaseHistoryError,
    releaseHistoryItems,
    releaseHistoryLoading,
    releaseHistoryOpen,
    releaseHistorySections,
    renderReleaseHistoryLine,
    reportActualPort,
    reportDefaultPort,
    reportEnabled,
    reportPort,
    reportRawPreviewUrl,
    reportRenderedPreviewUrl,
    reportToken,
    setActiveTab,
    setGlobalProxyEnabled,
    setGlobalProxyNoProxy,
    setGlobalProxyUrl,
    setMenuBarQuotaDraftPlatform,
    setMenuBarQuotaDraftShowPrefix,
    setReportEnabled,
    setReportPort,
    setReportToken,
    setWsEnabled,
    setWsPort,
    showUnlockFireworks,
    t,
    updateChecking,
    updateCheckMessage,
    wsEnabled,
    wsPort,
  } = props;
  return (
    <main className="main-content">
      <div className="page-tabs-row settings-page-tabs-row">
        <div className="page-tabs-label">{t('settings.title')}</div>
        <div className="page-tabs filter-tabs">
          <button 
            className={`filter-tab ${activeTab === 'general' ? 'active' : ''}`}
            onClick={() => setActiveTab('general')}
          >
            {t('settings.tabs.general')}
          </button>
          <button 
            className={`filter-tab ${activeTab === 'network' ? 'active' : ''}`}
            onClick={() => setActiveTab('network')}
          >
            {t('settings.tabs.network')}
          </button>
          <button 
            className={`filter-tab ${activeTab === 'data' ? 'active' : ''}`}
            onClick={() => setActiveTab('data')}
          >
            {t('settings.tabs.data', '数据管理')}
          </button>
          <button 
            className={`filter-tab ${activeTab === 'about' ? 'active' : ''}`}
            onClick={() => setActiveTab('about')}
          >
            {t('settings.tabs.about')}
          </button>
        </div>
      </div>

      {/* 2. Content Area */}
      <div className="settings-container">
        <div className="settings-content">
        {/* === General Tab === */}
        {activeTab === 'general' && <SettingsGeneralPanel {...props} />}

        {activeTab === 'data' && (
          <>
            <SettingsAccountTransferSection />
            <SettingsWebdavSyncSection />
          </>
        )}

        {/* === Network Tab === */}
        {activeTab === 'network' && (
          <>
            <div className="group-title">Antigravity Cockpit API</div>
            <div className="settings-group">
              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.network.wsService')}</div>
                  <div className="row-desc">{t('settings.network.wsServiceDesc')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input 
                      type="checkbox" 
                      checked={wsEnabled} 
                      onChange={(e) => setWsEnabled(e.target.checked)} 
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>

              {wsEnabled && (
                <>
                  <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                    <div className="row-label">
                      <div className="row-title">{t('settings.network.preferredPort')}</div>
                      <div className="row-desc">
                        {t('settings.network.preferredPortDesc').replace('{port}', String(defaultPort))}
                      </div>
                    </div>
                    <div className="row-control">
                      <input 
                        type="number" 
                        className="settings-input"
                        value={wsPort}
                        onChange={(e) => setWsPort(e.target.value)}
                        placeholder={String(defaultPort)}
                        min="1024"
                        max="65535"
                      />
                    </div>
                  </div>
                  
                  {actualPort && (
                    <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                      <div className="row-label">
                        <div className="row-title">{t('settings.network.currentPort')}</div>
                        <div className="row-desc">
                          {actualPort === parseInt(wsPort, 10) 
                            ? t('settings.network.portNormal')
                            : t('settings.network.portFallback')
                                .replace('{configured}', wsPort)
                                .replace('{actual}', String(actualPort))}
                        </div>
                      </div>
                      <div className="row-control">
                        <span style={{ 
                          fontFamily: 'var(--font-mono)', 
                          fontSize: '14px',
                          color: actualPort === parseInt(wsPort, 10) ? 'var(--accent)' : 'var(--warning, #f59e0b)'
                        }}>
                          ws://127.0.0.1:{actualPort}
                        </span>
                      </div>
                    </div>
                  )}
                </>
              )}
            </div>

            <div className="group-title">{t('settings.network.reportTitle')}</div>
            <div className="settings-group">
              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.network.reportService')}</div>
                  <div className="row-desc">{t('settings.network.reportServiceDesc')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={reportEnabled}
                      onChange={(e) => setReportEnabled(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>

              {reportEnabled && (
                <>
                  <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                    <div className="row-label">
                      <div className="row-title">{t('settings.network.reportPort')}</div>
                      <div className="row-desc">
                        {t('settings.network.reportPortDesc').replace('{port}', String(reportDefaultPort))}
                      </div>
                    </div>
                    <div className="row-control">
                      <input
                        type="number"
                        className="settings-input"
                        value={reportPort}
                        onChange={(e) => setReportPort(e.target.value)}
                        placeholder={String(reportDefaultPort)}
                        min="1024"
                        max="65535"
                      />
                    </div>
                  </div>

                  <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                    <div className="row-label">
                      <div className="row-title">{t('settings.network.reportToken')}</div>
                      <div className="row-desc">{t('settings.network.reportTokenDesc')}</div>
                    </div>
                    <div className="row-control" style={{ minWidth: '260px', display: 'flex', gap: '8px', alignItems: 'center' }}>
                      <input
                        type="text"
                        className="settings-input"
                        value={reportToken}
                        onChange={(e) => setReportToken(e.target.value)}
                        placeholder="change-this-token"
                      />
                      <button
                        className="btn btn-secondary"
                        onClick={() => setReportToken(generateReportToken())}
                        type="button"
                      >
                        {t('settings.network.generateToken')}
                      </button>
                    </div>
                  </div>

                  {reportActualPort && (
                    <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                      <div className="row-label">
                        <div className="row-title">{t('settings.network.currentPort')}</div>
                        <div className="row-desc">
                          {reportActualPort === parseInt(reportPort, 10)
                            ? t('settings.network.portNormal')
                            : t('settings.network.portFallback')
                                .replace('{configured}', reportPort)
                                .replace('{actual}', String(reportActualPort))}
                        </div>
                      </div>
                      <div className="row-control">
                        <span style={{
                          fontFamily: 'var(--font-mono)',
                          fontSize: '14px',
                          color: reportActualPort === parseInt(reportPort, 10) ? 'var(--accent)' : 'var(--warning, #f59e0b)',
                        }}>
                          http://0.0.0.0:{reportActualPort}
                        </span>
                      </div>
                    </div>
                  )}

                  <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                    <div className="row-label">
                      <div className="row-title">{t('settings.network.reportUrlPreview')}</div>
                      <div className="row-desc">
                        {t('settings.network.reportUrlPreviewDesc')}
                      </div>
                    </div>
                    <div className="row-control">
                      <div style={{
                        display: 'flex',
                        flexDirection: 'column',
                        gap: '6px',
                        alignItems: 'flex-start',
                        fontFamily: 'var(--font-mono)',
                        fontSize: '12px',
                        color: 'var(--text-secondary)',
                        wordBreak: 'break-all',
                      }}>
                        <span>{`${t('settings.network.reportUrlRaw')}: ${reportRawPreviewUrl}`}</span>
                        <span>{`${t('settings.network.reportUrlRendered')}: ${reportRenderedPreviewUrl}`}</span>
                      </div>
                    </div>
                  </div>

                  <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                    <div className="row-label">
                      <div className="row-title">{t('settings.network.firewallHintTitle')}</div>
                      <div className="row-desc">{t('settings.network.firewallHint')}</div>
                    </div>
                  </div>
                </>
              )}
            </div>

            <div className="group-title">{t('settings.network.proxyTitle')}</div>
            <div className="settings-group">
              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.network.proxyEnabled')}</div>
                  <div className="row-desc">{t('settings.network.proxyEnabledDesc')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={globalProxyEnabled}
                      onChange={(e) => setGlobalProxyEnabled(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>

              {globalProxyEnabled && (
                <>
                  <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                    <div className="row-label">
                      <div className="row-title">{t('settings.network.proxyUrl')}</div>
                      <div className="row-desc">{t('settings.network.proxyUrlDesc')}</div>
                    </div>
                    <div className="row-control">
                      <input
                        type="text"
                        className="settings-input"
                        value={globalProxyUrl}
                        onChange={(e) => setGlobalProxyUrl(e.target.value)}
                        placeholder={t('settings.network.proxyUrlPlaceholder')}
                      />
                    </div>
                  </div>

                  <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                    <div className="row-label">
                      <div className="row-title">{t('settings.network.proxyNoProxy')}</div>
                      <div className="row-desc">{t('settings.network.proxyNoProxyDesc')}</div>
                    </div>
                    <div className="row-control">
                      <input
                        type="text"
                        className="settings-input"
                        value={globalProxyNoProxy}
                        onChange={(e) => setGlobalProxyNoProxy(e.target.value)}
                        placeholder={t('settings.network.proxyNoProxyPlaceholder')}
                      />
                    </div>
                  </div>
                </>
              )}
            </div>
            
            {needsRestart && (
              <div style={{ 
                display: 'flex', 
                alignItems: 'center', 
                gap: '8px', 
                padding: '12px 16px',
                marginTop: '12px',
                background: 'rgba(245, 158, 11, 0.1)',
                borderRadius: '8px',
                color: 'var(--warning, #f59e0b)',
                fontSize: '14px'
              }}>
                <AlertCircle size={18} />
                {t('settings.network.restartRequired')}
              </div>
            )}

            <div style={{ display: 'flex', justifyContent: 'flex-end', marginTop: '12px' }}>
                <button 
                  className="btn btn-primary" 
                  onClick={handleSaveNetworkConfig}
                  disabled={networkSaving}
                >
                    <Save size={16} /> {networkSaving ? t('common.saving') : t('settings.saveSettings')}
                </button>
            </div>
          </>
        )}

        {/* === About Tab === */}
        {activeTab === 'about' && (
          <div className="about-container">
            <div className="about-logo-section">
              <div
                className={`app-icon-squircle${showUnlockFireworks ? ' unlock-fireworks-active' : ''}`}
                onClick={handleAboutAvatarTap}
                onMouseDown={(event) => event.preventDefault()}
              >
                <Rocket size={40} />
              </div>
              <div className="app-info">
                <h2>{t('settings.about.appName')}</h2>
                <div style={{ display: 'flex', alignItems: 'center', gap: '12px' }}>
                  <div className="version-tag">{appVersion}</div>
                  <button 
                    className="btn btn-sm btn-ghost"
                    onClick={handleCheckUpdate}
                    disabled={updateChecking}
                    style={{ 
                      fontSize: '12px', 
                      padding: '4px 10px',
                      display: 'flex',
                      alignItems: 'center',
                      gap: '4px'
                    }}
                  >
                    <>
                      <RefreshCw size={14} className={updateChecking ? 'spin' : undefined} />
                      {updateChecking ? t('settings.about.checking') : t('settings.about.checkUpdate')}
                    </>
                  </button>
                  <button
                    className="btn btn-sm btn-ghost"
                    onClick={handleOpenReleaseHistory}
                    disabled={releaseHistoryLoading}
                    style={{
                      fontSize: '12px',
                      padding: '4px 10px',
                      display: 'flex',
                      alignItems: 'center',
                      gap: '4px',
                    }}
                  >
                    <FileText size={14} />
                    {t('settings.about.viewReleaseHistory', '更新记录')}
                  </button>
                </div>
                {updateCheckMessage && (
                  <div
                    className={`action-message${updateCheckMessage.tone ? ` ${updateCheckMessage.tone}` : ''}`}
                    style={{ marginTop: '10px', marginBottom: 0 }}
                  >
                    <span className="action-message-text">{updateCheckMessage.text}</span>
                  </div>
                )}
              </div>
              <p style={{ color: 'var(--text-secondary)', fontSize: '14px' }}>
                {t('settings.about.slogan')}
              </p>
            </div>

            <div className="credits-list">
              <button className="credit-item" onClick={() => openLink('https://github.com/jlcodes99')}>
                <div className="credit-icon"><User size={24} /></div>
                <h3>{t('settings.about.author')}</h3>
                <p>jlcodes99</p>
              </button>
              
              
              <button className="credit-item" onClick={() => openLink('https://github.com/jlcodes99/cockpit-tools')}>
                <div className="credit-icon" style={{ color: '#0f172a' }}><Github size={24} /></div>
                <h3>{t('settings.about.github')}</h3>
                <p>cockpit-tools</p>
              </button>

              <button className="credit-item" onClick={() => openLink('https://github.com/jlcodes99/cockpit-tools/blob/main/docs/DONATE.md')}>
                <div className="credit-icon" style={{ color: '#ef4444' }}><Heart size={24} /></div>
                <h3>{t('settings.about.sponsor')}</h3>
                <p>{t('settings.about.sponsorDesc', 'Donate')}</p>
              </button>

              <button className="credit-item" onClick={() => openLink('https://github.com/jlcodes99/cockpit-tools/issues')}>
                <div className="credit-icon" style={{ color: '#3b82f6' }}><MessageSquare size={24} /></div>
                <h3>{t('settings.about.feedback', '意见反馈')}</h3>
                <p>{t('settings.about.feedbackDesc', 'Issues')}</p>
              </button>
            </div>
          </div>
        )}
        </div>
      </div>
      {menuBarQuotaModalOpen && (
        <div className="modal-overlay">
          <div
            className="modal settings-menu-bar-quota-modal"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <h2>
                {t('settings.general.menuBarQuotaModalTitle', '菜单栏额度')}
              </h2>
              <button
                type="button"
                className="modal-close"
                onClick={handleCloseMenuBarQuotaModal}
                aria-label={t('common.close', '关闭')}
              >
                <X size={16} />
              </button>
            </div>
            <div className="modal-body">
              <p className="settings-menu-bar-quota-modal-desc">
                {t(
                  'settings.general.menuBarQuotaModalDesc',
                  '以下为菜单栏额度的专属选项：跟随所选平台当前账号。Codex 当前为 API 服务时显示「API + 池剩余%」；API Key 账号显示「API + 剩余额度」；普通账号显示邮箱前缀与剩余%（多条取最低；低红、中橙、高绿）。'
                )}
              </p>
              <div className="settings-menu-bar-quota-modal-field">
                <label className="settings-menu-bar-quota-modal-label" htmlFor="menu-bar-quota-platform">
                  {t('settings.general.menuBarQuotaPlatform', '额度账号平台')}
                </label>
                <p className="settings-menu-bar-quota-modal-field-desc">
                  {t(
                    'settings.general.menuBarQuotaPlatformDesc',
                    '跟随该平台当前正在使用的账号，刷新或切换后自动更新'
                  )}
                </p>
                <select
                  id="menu-bar-quota-platform"
                  className="settings-select settings-menu-bar-quota-modal-select"
                  value={menuBarQuotaDraftPlatform}
                  onChange={(e) => setMenuBarQuotaDraftPlatform(e.target.value as PlatformId)}
                >
                  {menuBarQuotaPlatformOptions.map((option) => (
                    <option key={option.value} value={option.value}>
                      {option.label}
                    </option>
                  ))}
                </select>
              </div>
              <div className="settings-menu-bar-quota-modal-field">
                <label className="settings-menu-bar-quota-modal-label" htmlFor="menu-bar-quota-prefix">
                  {t('settings.general.menuBarAccountPrefix', '显示账号邮箱前 4 位')}
                </label>
                <p className="settings-menu-bar-quota-modal-field-desc">
                  {t(
                    'settings.general.menuBarAccountPrefixDesc',
                    '仅普通账号：关闭后不显示邮箱前缀。Codex API 服务 / API Key 仍会显示 API 标签'
                  )}
                </p>
                <select
                  id="menu-bar-quota-prefix"
                  className="settings-select settings-menu-bar-quota-modal-select"
                  value={menuBarQuotaDraftShowPrefix ? 'true' : 'false'}
                  onChange={(e) => setMenuBarQuotaDraftShowPrefix(e.target.value === 'true')}
                >
                  <option value="true">{t('common.enable', '启用')}</option>
                  <option value="false">{t('common.disable', '停用')}</option>
                </select>
              </div>
            </div>
            <div className="modal-footer">
              <button
                type="button"
                className="btn btn-secondary"
                onClick={handleCloseMenuBarQuotaModal}
              >
                {t('common.cancel', '取消')}
              </button>
              <button
                type="button"
                className="btn btn-primary"
                onClick={handleConfirmMenuBarQuotaModal}
              >
                {menuBarQuotaModalMode === 'enable'
                  ? t('settings.general.menuBarQuotaConfirmEnable', '启用')
                  : t('common.save', '保存')}
              </button>
            </div>
          </div>
        </div>
      )}
      {releaseHistoryOpen && (
        <div className="modal-overlay">
          <div className="modal settings-release-history-modal" onClick={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('settings.about.releaseHistoryTitle', '更新记录')}</h2>
              <button
                className="modal-close"
                onClick={handleCloseReleaseHistory}
                aria-label={t('common.close', '关闭')}
              >
                <X size={16} />
              </button>
            </div>
            <div className="modal-body settings-release-history-body">
              {releaseHistoryLoading && (
                <div className="settings-release-history-state">
                  <RefreshCw size={14} className="spin" />
                  <span>{t('settings.about.releaseHistoryLoading', '加载中...')}</span>
                </div>
              )}
              {!releaseHistoryLoading && releaseHistoryError && (
                <div className="settings-release-history-state settings-release-history-state-error">
                  {t('settings.about.releaseHistoryLoadFailed', '加载失败：{{error}}', {
                    error: releaseHistoryError,
                  })}
                </div>
              )}
              {!releaseHistoryLoading && !releaseHistoryError && releaseHistoryItems.length === 0 && (
                <div className="settings-release-history-state">
                  {t('settings.about.releaseHistoryEmpty', '暂无更新记录')}
                </div>
              )}
              {!releaseHistoryLoading &&
                !releaseHistoryError &&
                releaseHistoryItems.map((item) => (
                  <article
                    key={`${item.version}-${item.date || 'unknown'}`}
                    className="settings-release-history-item"
                  >
                    <div className="settings-release-history-item-head">
                      <span className="settings-release-history-version">v{item.version}</span>
                      <div className="settings-release-history-item-meta">
                        {item.date ? (
                          <span className="settings-release-history-date">{item.date}</span>
                        ) : null}
                        <button
                          className="settings-release-history-download-btn"
                          onClick={() => {
                            void handleDownloadReleaseVersion(item.version);
                          }}
                          type="button"
                        >
                          <Download size={12} />
                          {t('settings.about.downloadThisVersion', '下载此版本')}
                        </button>
                      </div>
                    </div>
                    <div className="settings-release-history-sections">
                      {releaseHistorySections.map((section) => {
                        const lines = item[section.key];
                        if (!Array.isArray(lines) || lines.length === 0) {
                          return null;
                        }
                        return (
                          <section key={`${item.version}-${section.key}`} className="settings-release-history-section">
                            <h3>{section.label}</h3>
                            <ul>
                              {lines.map((line, index) => (
                                <li key={`${item.version}-${section.key}-${index}`}>
                                  {renderReleaseHistoryLine(line)}
                                </li>
                              ))}
                            </ul>
                          </section>
                        );
                      })}
                    </div>
                  </article>
                ))}
            </div>
            <div className="modal-footer">
              <button className="btn btn-secondary" onClick={handleCloseReleaseHistory}>
                {t('common.close', '关闭')}
              </button>
            </div>
          </div>
        </div>
      )}
      {showUnlockFireworks && !reducedMotionEnabled && (
        <UnlockFireworksOverlay />
      )}
    </main>
  );
}
