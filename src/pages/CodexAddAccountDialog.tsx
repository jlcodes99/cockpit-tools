import { createPortal } from "react-dom";
import { RefreshCw, Download, X, Globe, KeyRound, Database, Copy, Check, RotateCw, CircleAlert, Info, Star, Eye, EyeOff, FileUp, FileText, ExternalLink, FolderPlus, Terminal, ShieldCheck } from "lucide-react";
import { MfaQuickCodeSelect } from "../components/MfaQuickCodeSelect";
import { CodexModelContextWindowTable } from "../components/codex/CodexModelContextWindowTable";
import { SingleSelectDropdown } from "../components/SingleSelectDropdown";
import { CODEX_API_PROVIDER_CUSTOM_ID, CODEX_API_PROVIDER_PRESETS, COCKPIT_API_PROVIDER_ID, COCKPIT_TOOLS_API_PROVIDER_ID } from "../utils/codexProviderPresets";
import type { CodexAccountsViewProps } from "./CodexAccountsView";

/** 渲染 CodexAccountsOverviewPanel 的 expr:showAddModal && 业务面板。 */
export function CodexAddAccountDialog(props: CodexAccountsViewProps) {
  const {
    addMessage,
    addStatus,
    addTab,
    apiBaseUrlInput,
    apiKeyInput,
    apiKeyInputVisible,
    apiModelCatalogDraft,
    apiModelCatalogError,
    apiModelCatalogFetching,
    apiModelCatalogInput,
    apiModelCatalogSyncAvailable,
    apiModelContextWindowsInput,
    apiProviderPresetId,
    apiSyncModelCatalogToCodex,
    closeCodexAddModal,
    CODEX_TOKEN_BATCH_EXAMPLE,
    CODEX_TOKEN_SESSION_EXAMPLE,
    CODEX_TOKEN_SINGLE_EXAMPLE,
    codexAddTargetGroup,
    deviceAuthError,
    deviceAuthInfo,
    deviceAuthStarting,
    deviceCodeCopied,
    formatCodexManagedApiKeyOptionLabel,
    handleApiBaseUrlInputChange,
    handleApiKeyInputChange,
    handleApiKeyLogin,
    handleCopyDeviceCode,
    handleCopyOauthUrl,
    handleCopyReauthEmail,
    handleFetchApiModelCatalog,
    handleImportFromFiles,
    handleImportFromLocal,
    handleOpenCodexSecuritySettings,
    handleOpenDeviceAuthUrl,
    handleOpenOauthIncognitoWindow,
    handleOpenOauthUrl,
    handleOpenProviderLink,
    handlePendingOAuthEmailInputChange,
    handleReleaseOauthPort,
    handleRetryOauthAfterTimeout,
    handleRetryOauthTokenExchange,
    handleSavePendingOAuthAccount,
    handleSelectApiProviderPreset,
    handleSelectManagedProvider,
    handleSelectManagedProviderApiKey,
    handleStartDeviceAuth,
    handleSubmitOauthCallbackUrl,
    handleSwitchBrowserOAuth,
    handleSyncImportedToApiServiceChange,
    handleTokenImport,
    importing,
    isMacOS,
    isOauthTimeoutState,
    isOauthTokenExchangeErrorState,
    managedProviderApiKeyId,
    managedProviderId,
    managedProviders,
    managedProvidersLoading,
    newManagedProviderNameInput,
    oauthCallbackError,
    oauthCallbackInput,
    oauthCallbackSubmitting,
    oauthCompletingRef,
    oauthLoginIdRef,
    oauthMethod,
    oauthPortInUse,
    oauthPrepareError,
    oauthTimeoutInfo,
    oauthUrl,
    oauthUrlCopied,
    OPENAI_OFFICIAL_PRESET_ID,
    openCodexAddModal,
    openPendingOAuthNoteModal,
    pendingOAuthEmailInput,
    pendingOAuthFieldErrors,
    pendingOAuthHasNoteDetails,
    reauthEmailCopied,
    reauthTargetAccount,
    reauthTargetEmail,
    renderAccountNoteButton,
    savingPendingOAuthAccount,
    selectedApiProviderPreset,
    selectedManagedProvider,
    selectedSponsorApiProviderTemplate,
    setApiKeyInputVisible,
    setApiModelCatalogError,
    setApiModelCatalogInput,
    setApiModelContextWindowsInput,
    setApiSyncModelCatalogToCodex,
    setNewManagedProviderNameInput,
    setOauthCallbackInput,
    setTokenInput,
    shouldShowPendingOAuthDraftForm,
    showAddModal,
    sponsorApiProviderTemplates,
    syncImportedToApiService,
    t,
    tokenImportProgress,
    tokenInput,
  } = props;
  return showAddModal &&
            createPortal(
              <div className="modal-overlay">
                <div
                  className="modal-content codex-add-modal codex-account-add-modal platform-account-add-modal"
                  onClick={(e) => e.stopPropagation()}
                >
                  <div className="modal-header">
                    <h2>{t("codex.addModal.title", "添加 Codex 账号")}</h2>
                    <button
                      className="modal-close"
                      onClick={closeCodexAddModal}
                      disabled={importing}
                      aria-label={t("common.close", "关闭")}
                    >
                      <X />
                    </button>
                  </div>
                  <div className="modal-tabs">
                    <button
                      className={`modal-tab ${addTab === "oauth" ? "active" : ""}`}
                      onClick={() => openCodexAddModal("oauth")}
                      disabled={importing}
                    >
                      <Globe size={14} />
                      <span className="modal-tab-label">
                        {t(
                          "common.shared.addModal.oauth",
                          "OAuth Authorization",
                        )}
                      </span>
                    </button>
                    <button
                      className={`modal-tab ${addTab === "token" ? "active" : ""}`}
                      onClick={() => openCodexAddModal("token")}
                      disabled={importing}
                    >
                      <FileText size={14} />
                      <span className="modal-tab-label">
                        {t("common.shared.addModal.token", "Token / JSON")}
                      </span>
                    </button>
                    <button
                      className={`modal-tab ${addTab === "apikey" ? "active" : ""}`}
                      onClick={() => openCodexAddModal("apikey")}
                      disabled={importing}
                    >
                      <KeyRound size={14} />
                      <span className="modal-tab-label">
                        {t("codex.addModal.token", "API Key")}
                      </span>
                    </button>
                    <button
                      className={`modal-tab ${addTab === "import" ? "active" : ""}`}
                      onClick={() => openCodexAddModal("import")}
                      disabled={importing}
                    >
                      <Database size={14} />
                      <span className="modal-tab-label">
                        {t("accounts.tabs.import", "本地导入")}
                      </span>
                    </button>
                  </div>
                  <div className="modal-body">
                    {codexAddTargetGroup && !reauthTargetAccount && (
                      <div className="codex-add-target-group-hint">
                        <FolderPlus size={14} />
                        <span>
                          {t("codex.addModal.targetGroup", {
                            defaultValue: "将添加到分组：{{group}}",
                            group: codexAddTargetGroup.name,
                          })}
                        </span>
                      </div>
                    )}
                    {addTab !== "oauth" && <MfaQuickCodeSelect />}
                    {addTab === "oauth" && (
                      <div className="add-section">
                        {reauthTargetEmail && (
                          <div className="oauth-link codex-reauth-email-block">
                            <label>
                              {t(
                                "codex.oauth.reauthEmailLabel",
                                "本次重新授权账号",
                              )}
                            </label>
                            <div className="oauth-url-box">
                              <input
                                type="text"
                                value={reauthTargetEmail}
                                readOnly
                                aria-label={t(
                                  "codex.oauth.reauthEmailLabel",
                                  "本次重新授权账号",
                                )}
                              />
                              <button
                                type="button"
                                onClick={() => void handleCopyReauthEmail()}
                                title={
                                  reauthEmailCopied
                                    ? t("common.copied", "已复制")
                                    : t("common.copy", "复制")
                                }
                                aria-label={
                                  reauthEmailCopied
                                    ? t("common.copied", "已复制")
                                    : t("common.copy", "复制")
                                }
                              >
                                {reauthEmailCopied ? (
                                  <Check size={16} />
                                ) : (
                                  <Copy size={16} />
                                )}
                              </button>
                            </div>
                          </div>
                        )}
                        {reauthTargetAccount && (
                          <div className="codex-reauth-note-summary">
                            {renderAccountNoteButton(reauthTargetAccount)}
                          </div>
                        )}
                        {shouldShowPendingOAuthDraftForm && (
                          <div className="codex-pending-oauth-draft">
                            <div className="oauth-link">
                              <label>
                                {t(
                                  "codex.pendingAuth.emailLabel",
                                  "待授权账号",
                                )}
                              </label>
                              <div className="oauth-url-box oauth-manual-input">
                                <input
                                  type="email"
                                  value={pendingOAuthEmailInput}
                                  onChange={(event) => {
                                    handlePendingOAuthEmailInputChange(
                                      event.target.value,
                                    );
                                  }}
                                  placeholder={t(
                                    "codex.pendingAuth.emailPlaceholder",
                                    "输入 OpenAI 账号邮箱",
                                  )}
                                  disabled={savingPendingOAuthAccount}
                                />
                              </div>
                              {pendingOAuthFieldErrors.email && (
                                <span className="codex-account-note-field-error">
                                  {pendingOAuthFieldErrors.email}
                                </span>
                              )}
                              {pendingOAuthFieldErrors.twoFactorSecret && (
                                <span className="codex-account-note-field-error">
                                  {pendingOAuthFieldErrors.twoFactorSecret}
                                </span>
                              )}
                            </div>
                            <button
                              type="button"
                              className={`codex-account-note-chip ${pendingOAuthHasNoteDetails ? "has-note" : "empty-note"}`}
                              onClick={openPendingOAuthNoteModal}
                              disabled={savingPendingOAuthAccount}
                            >
                              <FileText size={12} />
                              <span>
                                {pendingOAuthHasNoteDetails
                                  ? t("codex.accountNote.short", "账号备注")
                                  : t("codex.accountNote.addShort", "加备注")}
                              </span>
                            </button>
                            <button
                              type="button"
                              className="btn btn-secondary btn-full"
                              onClick={() =>
                                void handleSavePendingOAuthAccount()
                              }
                              disabled={
                                savingPendingOAuthAccount ||
                                !pendingOAuthEmailInput.trim()
                              }
                            >
                              {savingPendingOAuthAccount ? (
                                <RefreshCw
                                  size={16}
                                  className="loading-spinner"
                                />
                              ) : (
                                <FileText size={16} />
                              )}
                              {t(
                                "codex.pendingAuth.saveDraft",
                                "保存待授权卡片",
                              )}
                            </button>
                          </div>
                        )}
                        <p className="section-desc">
                          {t(
                            "codex.oauth.desc",
                            "通过 OpenAI 官方 OAuth 授权您的 Codex 账号。",
                          )}
                        </p>
                        <div
                          className="codex-oauth-method-switch"
                          role="tablist"
                        >
                          <button
                            type="button"
                            className={
                              oauthMethod === "browser" ? "active" : ""
                            }
                            onClick={() => void handleSwitchBrowserOAuth()}
                            role="tab"
                            aria-selected={oauthMethod === "browser"}
                          >
                            <Globe size={15} />
                            <span>
                              {t(
                                "common.shared.oauth.browserAuth",
                                "浏览器授权",
                              )}
                            </span>
                          </button>
                          <button
                            type="button"
                            className={oauthMethod === "device" ? "active" : ""}
                            onClick={() => void handleStartDeviceAuth()}
                            disabled={
                              deviceAuthStarting || oauthCompletingRef.current
                            }
                            role="tab"
                            aria-selected={oauthMethod === "device"}
                          >
                            {deviceAuthStarting ? (
                              <RefreshCw
                                size={15}
                                className="loading-spinner"
                              />
                            ) : (
                              <Terminal size={15} />
                            )}
                            <span>
                              {t("common.shared.oauth.deviceAuth", "设备授权")}
                            </span>
                          </button>
                        </div>
                        {oauthMethod === "device" && (
                          <div className="codex-device-auth-notice">
                            <Info size={15} />
                            <span>
                              {t(
                                "common.shared.oauth.deviceAuthPrerequisite",
                                "如果提示未启用设备代码授权，请先在 ChatGPT 安全设置中为 Codex 开启设备代码授权。",
                              )}
                            </span>
                            <button
                              type="button"
                              onClick={() =>
                                void handleOpenCodexSecuritySettings()
                              }
                            >
                              {t(
                                "common.shared.oauth.openSecuritySettings",
                                "打开设置",
                              )}
                            </button>
                          </div>
                        )}
                        {deviceAuthError && (
                          <div className="add-status error">
                            <CircleAlert size={16} />
                            <span>{deviceAuthError}</span>
                          </div>
                        )}
                        {deviceAuthInfo && (
                          <div className="oauth-link codex-device-auth-panel">
                            <div className="codex-device-auth-header">
                              <div>
                                <strong>
                                  {t(
                                    "common.shared.oauth.deviceAuth",
                                    "设备授权",
                                  )}
                                </strong>
                                <span>
                                  {t(
                                    "common.shared.oauth.deviceAuthWaiting",
                                    "请完成以下步骤，完成后会自动继续",
                                  )}
                                </span>
                              </div>
                              <span className="codex-device-auth-status">
                                {t("common.shared.oauth.waiting", "等待授权")}
                              </span>
                            </div>
                            <label>
                              {t(
                                "common.shared.oauth.deviceCode",
                                "设备验证码",
                              )}
                            </label>
                            <div className="oauth-url-box">
                              <input
                                type="text"
                                value={deviceAuthInfo.userCode}
                                readOnly
                              />
                              <button
                                type="button"
                                onClick={() => void handleCopyDeviceCode()}
                                title={
                                  deviceCodeCopied
                                    ? t("common.copied", "已复制")
                                    : t("common.copy", "复制")
                                }
                                aria-label={
                                  deviceCodeCopied
                                    ? t("common.copied", "已复制")
                                    : t("common.copy", "复制")
                                }
                              >
                                {deviceCodeCopied ? (
                                  <Check size={16} />
                                ) : (
                                  <Copy size={16} />
                                )}
                              </button>
                            </div>
                            <button
                              type="button"
                              className="btn btn-primary btn-full"
                              onClick={() => void handleOpenDeviceAuthUrl()}
                            >
                              <ExternalLink size={16} />
                              {t(
                                "common.shared.oauth.openBrowser",
                                "打开授权页面",
                              )}
                            </button>
                            <p className="codex-device-auth-footnote">
                              {t(
                                "common.shared.oauth.deviceAuthFootnote",
                                "设备授权不占用本地 1455 回调端口。",
                              )}
                            </p>
                          </div>
                        )}
                        {oauthMethod === "browser" &&
                          (deviceAuthInfo ||
                          deviceAuthStarting ||
                          deviceAuthError ? null : oauthPrepareError ? (
                            <div className="add-status error">
                              <CircleAlert size={16} />
                              <span>{oauthPrepareError}</span>
                              {oauthPortInUse && (
                                <button
                                  className="btn btn-sm btn-outline"
                                  onClick={handleReleaseOauthPort}
                                >
                                  {t(
                                    "codex.oauth.portInUseAction",
                                    "Close port and retry",
                                  )}
                                </button>
                              )}
                              {!oauthPortInUse && oauthTimeoutInfo && (
                                <button
                                  className="btn btn-sm btn-outline"
                                  onClick={handleRetryOauthAfterTimeout}
                                >
                                  {t(
                                    "codex.oauth.timeoutRetry",
                                    "刷新授权链接",
                                  )}
                                </button>
                              )}
                            </div>
                          ) : oauthUrl ? (
                            <div className="oauth-url-section">
                              <div className="oauth-link">
                                <label>
                                  {t("accounts.oauth.linkLabel", "授权链接")}
                                </label>
                                <div className="oauth-url-box">
                                  <input
                                    type="text"
                                    value={oauthUrl}
                                    readOnly
                                  />
                                  <button onClick={handleCopyOauthUrl}>
                                    {oauthUrlCopied ? (
                                      <Check size={16} />
                                    ) : (
                                      <Copy size={16} />
                                    )}
                                  </button>
                                </div>
                              </div>
                              <button
                                className="btn btn-primary btn-full"
                                onClick={
                                  isOauthTimeoutState
                                    ? handleRetryOauthAfterTimeout
                                    : handleOpenOauthUrl
                                }
                              >
                                {isOauthTimeoutState ? (
                                  <RefreshCw size={16} />
                                ) : (
                                  <Globe size={16} />
                                )}
                                {isOauthTimeoutState
                                  ? t(
                                      "codex.oauth.timeoutRetry",
                                      "刷新授权链接",
                                    )
                                  : t(
                                      "common.shared.oauth.openBrowser",
                                      "Open in Browser",
                                    )}
                              </button>
                              {!isOauthTimeoutState && isMacOS && (
                                <button
                                  type="button"
                                  className="btn btn-secondary btn-full"
                                  onClick={() =>
                                    void handleOpenOauthIncognitoWindow()
                                  }
                                >
                                  <ShieldCheck size={16} />
                                  {t(
                                    "common.shared.oauth.incognitoWindow",
                                    "无痕窗口",
                                  )}
                                </button>
                              )}
                              <div className="oauth-link">
                                <label>
                                  {t(
                                    "common.shared.oauth.manualCallbackLabel",
                                    "手动输入回调地址",
                                  )}
                                </label>
                                <div className="oauth-url-box oauth-manual-input">
                                  <input
                                    type="text"
                                    value={oauthCallbackInput}
                                    onChange={(e) =>
                                      setOauthCallbackInput(e.target.value)
                                    }
                                    placeholder={t(
                                      "common.shared.oauth.manualCallbackPlaceholder",
                                      "粘贴完整回调地址，例如：http://localhost:1455/auth/callback?code=...&state=...",
                                    )}
                                  />
                                  <button
                                    className="oauth-copy-button"
                                    onClick={() =>
                                      void handleSubmitOauthCallbackUrl()
                                    }
                                    disabled={
                                      oauthCallbackSubmitting ||
                                      !oauthCallbackInput.trim()
                                    }
                                  >
                                    {oauthCallbackSubmitting ? (
                                      <RefreshCw
                                        size={16}
                                        className="loading-spinner"
                                      />
                                    ) : (
                                      <Check size={16} />
                                    )}
                                    <span className="oauth-copy-button-label">
                                      {t(
                                        "accounts.oauth.continue",
                                        "我已授权，继续",
                                      )}
                                    </span>
                                  </button>
                                </div>
                              </div>
                              {oauthCallbackError && (
                                <div className="add-status error">
                                  <CircleAlert size={16} />
                                  <span>{oauthCallbackError}</span>
                                </div>
                              )}
                              {isOauthTimeoutState && (
                                <div className="add-status error">
                                  <CircleAlert size={16} />
                                  <span>
                                    {t(
                                      "codex.oauth.timeout",
                                      '授权超时，请点击"刷新授权链接"后重试。',
                                    )}
                                  </span>
                                </div>
                              )}
                              <div className="codex-oauth-auto-update-note">
                                <Info size={14} />
                                {t(
                                  "common.shared.oauth.hint",
                                  "授权完成后，此窗口会自动更新",
                                )}
                              </div>
                            </div>
                          ) : (
                            <div className="oauth-loading">
                              <RefreshCw
                                size={24}
                                className="loading-spinner"
                              />
                              <span>
                                {t(
                                  "codex.oauth.preparing",
                                  "正在准备授权链接...",
                                )}
                              </span>
                            </div>
                          ))}
                      </div>
                    )}
                    {addTab === "apikey" && (
                      <div className="add-section">
                        <div className="oauth-link">
                          <label>
                            {t(
                              "codex.modelProviders.selectSavedProvider",
                              "已保存供应商",
                            )}
                          </label>
                          {managedProvidersLoading ? (
                            <div className="section-desc">
                              {t("common.loading", "加载中...")}
                            </div>
                          ) : managedProviders.length === 0 ? (
                            <div className="section-desc">
                              {t(
                                "codex.modelProviders.noSavedProviders",
                                "暂无已保存供应商，可直接填写后自动保存。",
                              )}
                            </div>
                          ) : (
                            <div className="api-provider-chip-list">
                              {managedProviders.map((provider) => (
                                <button
                                  key={provider.id}
                                  className={`api-provider-chip ${managedProviderId === provider.id ? "active" : ""}`}
                                  onClick={() =>
                                    handleSelectManagedProvider(provider.id)
                                  }
                                  type="button"
                                >
                                  <span>{provider.name}</span>
                                </button>
                              ))}
                            </div>
                          )}
                        </div>
                        {selectedManagedProvider &&
                          selectedManagedProvider.apiKeys.length > 0 && (
                            <div className="oauth-link">
                              <label>
                                {t(
                                  "codex.modelProviders.selectSavedApiKey",
                                  "已保存 API Key",
                                )}
                              </label>
                              <SingleSelectDropdown
                                className="codex-managed-api-key-select"
                                value={managedProviderApiKeyId}
                                options={[
                                  {
                                    value: "",
                                    label: t(
                                      "codex.modelProviders.manualApiKeyOption",
                                      "手动输入新 Key",
                                    ),
                                  },
                                  ...selectedManagedProvider.apiKeys.map(
                                    (item) => ({
                                      value: item.id,
                                      label:
                                        formatCodexManagedApiKeyOptionLabel(
                                          item,
                                          t(
                                            "codex.modelProviders.unnamedKey",
                                            "未命名 Key",
                                          ),
                                        ),
                                    }),
                                  ),
                                ]}
                                onChange={handleSelectManagedProviderApiKey}
                                placeholder={t(
                                  "codex.modelProviders.selectSavedApiKeyPlaceholder",
                                  "选择 API Key",
                                )}
                                ariaLabel={t(
                                  "codex.modelProviders.selectSavedApiKey",
                                  "已保存 API Key",
                                )}
                              />
                              {selectedManagedProvider.apiKeys.length > 1 && (
                                <p className="api-provider-hint">
                                  {t(
                                    "codex.modelProviders.selectSavedApiKeyHint",
                                    "该供应商有多个 API Key，可在此切换选择。",
                                  )}
                                </p>
                              )}
                            </div>
                          )}
                        <div className="oauth-link">
                          <label>
                            {t("codex.api.provider.label", "供应商")}
                          </label>
                          <div className="api-provider-chip-list">
                            <button
                              className={`api-provider-chip ${apiProviderPresetId === CODEX_API_PROVIDER_CUSTOM_ID ? "active" : ""}`}
                              onClick={() =>
                                handleSelectApiProviderPreset(
                                  CODEX_API_PROVIDER_CUSTOM_ID,
                                )
                              }
                              type="button"
                            >
                              <span>
                                {t("codex.api.provider.custom", "自定义")}
                              </span>
                            </button>
                            {sponsorApiProviderTemplates.map((template) => (
                              <button
                                key={template.id}
                                className={`api-provider-chip sponsor ${apiProviderPresetId === template.id ? "active" : ""}`}
                                onClick={() =>
                                  handleSelectApiProviderPreset(template.id)
                                }
                                type="button"
                              >
                                <span>{template.name}</span>
                                <Star
                                  size={12}
                                  className="api-provider-chip-badge"
                                />
                              </button>
                            ))}
                            {CODEX_API_PROVIDER_PRESETS.map((preset) => (
                              <button
                                key={preset.id}
                                className={`api-provider-chip ${apiProviderPresetId === preset.id ? "active" : ""}`}
                                onClick={() =>
                                  handleSelectApiProviderPreset(preset.id)
                                }
                                type="button"
                              >
                                <span>
                                  {t(
                                    `codex.api.providers.${preset.id}.name`,
                                    preset.name,
                                  )}
                                </span>
                                {preset.isPartner && (
                                  <Star
                                    size={12}
                                    className="api-provider-chip-badge"
                                  />
                                )}
                              </button>
                            ))}
                          </div>
                        </div>
                        {selectedSponsorApiProviderTemplate && (
                          <div className="api-provider-hint-block sponsor">
                            <p className="api-provider-hint">
                              {t(
                                "codex.modelProviders.sponsorHint",
                                "已按专属中转站配置自动填写兼容服务地址。输入 API Key 后，卡片会自动查询余额和用量。",
                              )}
                            </p>
                            <div className="api-provider-links">
                              {selectedSponsorApiProviderTemplate.website && (
                                <button
                                  className="btn btn-secondary"
                                  onClick={() =>
                                    void handleOpenProviderLink(
                                      selectedSponsorApiProviderTemplate.website,
                                    )
                                  }
                                >
                                  <ExternalLink size={14} />
                                  {t("codex.api.provider.website", "官网")}
                                </button>
                              )}
                              {selectedSponsorApiProviderTemplate.apiKeyUrl && (
                                <button
                                  className="btn btn-secondary"
                                  onClick={() =>
                                    void handleOpenProviderLink(
                                      selectedSponsorApiProviderTemplate.apiKeyUrl,
                                    )
                                  }
                                >
                                  <KeyRound size={14} />
                                  {t(
                                    "codex.api.provider.apiKeyPage",
                                    "API Key 页面",
                                  )}
                                </button>
                              )}
                            </div>
                          </div>
                        )}
                        {selectedApiProviderPreset &&
                          selectedApiProviderPreset.baseUrls.length > 1 && (
                            <div className="oauth-link">
                              <label>
                                {t("codex.api.provider.endpoint", "供应商端点")}
                              </label>
                              <div className="api-provider-endpoint-list">
                                {selectedApiProviderPreset.baseUrls.map(
                                  (baseUrl) => (
                                    <button
                                      key={baseUrl}
                                      className={`api-provider-endpoint-chip ${apiBaseUrlInput === baseUrl ? "active" : ""}`}
                                      onClick={() =>
                                        handleApiBaseUrlInputChange(baseUrl)
                                      }
                                      type="button"
                                    >
                                      {baseUrl}
                                    </button>
                                  ),
                                )}
                              </div>
                            </div>
                          )}
                        {selectedApiProviderPreset && (
                          <div className="api-provider-hint-block">
                            <p className="api-provider-hint">
                              {t(
                                "codex.api.provider.hint",
                                "已自动填写兼容 Base URL，可继续手动调整。",
                              )}
                            </p>
                            {selectedApiProviderPreset.id ===
                              COCKPIT_TOOLS_API_PROVIDER_ID && (
                              <p className="api-provider-hint">
                                {t(
                                  "codex.api.provider.cockpitToolsHint",
                                  "请把 Base URL 改成部署 Cockpit Tools 的局域网地址；例如 http://主机IP:端口/v1。保存后会自动查询 /cockpit/quota。",
                                )}
                              </p>
                            )}
                            <div className="api-provider-links">
                              {selectedApiProviderPreset.website && (
                                <button
                                  className="btn btn-secondary"
                                  onClick={() =>
                                    void handleOpenProviderLink(
                                      selectedApiProviderPreset.website || "",
                                    )
                                  }
                                >
                                  <ExternalLink size={14} />
                                  {t("codex.api.provider.website", "官网")}
                                </button>
                              )}
                              {selectedApiProviderPreset.apiKeyUrl && (
                                <button
                                  className="btn btn-secondary"
                                  onClick={() =>
                                    void handleOpenProviderLink(
                                      selectedApiProviderPreset.apiKeyUrl || "",
                                    )
                                  }
                                >
                                  <KeyRound size={14} />
                                  {selectedApiProviderPreset.id ===
                                  COCKPIT_API_PROVIDER_ID
                                    ? t(
                                        "codex.api.provider.getApiKey",
                                        "获取秘钥",
                                      )
                                    : t(
                                        "codex.api.provider.apiKeyPage",
                                        "API Key 页面",
                                      )}
                                </button>
                              )}
                            </div>
                          </div>
                        )}
                        <div className="oauth-link">
                          <label>{t("codex.addModal.token", "API Key")}</label>
                          <div className="oauth-url-box oauth-manual-input codex-secret-input">
                            <input
                              type={apiKeyInputVisible ? "text" : "password"}
                              value={apiKeyInput}
                              onChange={(e) =>
                                handleApiKeyInputChange(e.target.value)
                              }
                              autoComplete="off"
                              spellCheck={false}
                            />
                            <button
                              type="button"
                              className="codex-secret-toggle-btn"
                              onClick={() =>
                                setApiKeyInputVisible((visible) => !visible)
                              }
                              title={
                                apiKeyInputVisible
                                  ? t("codex.api.hideApiKey", "隐藏 API Key")
                                  : t("codex.api.showApiKey", "显示 API Key")
                              }
                              aria-label={
                                apiKeyInputVisible
                                  ? t("codex.api.hideApiKey", "隐藏 API Key")
                                  : t("codex.api.showApiKey", "显示 API Key")
                              }
                            >
                              {apiKeyInputVisible ? (
                                <EyeOff size={16} />
                              ) : (
                                <Eye size={16} />
                              )}
                            </button>
                          </div>
                        </div>
                        <div className="oauth-link">
                          <label>{t("codex.api.baseUrl", "Base URL")}</label>
                          <div className="oauth-url-box oauth-manual-input">
                            <input
                              type="text"
                              value={apiBaseUrlInput}
                              onChange={(e) =>
                                handleApiBaseUrlInputChange(e.target.value)
                              }
                              placeholder={t(
                                "codex.api.baseUrlPlaceholder",
                                "不填写则是官方默认",
                              )}
                            />
                          </div>
                        </div>
                        {apiProviderPresetId !== COCKPIT_API_PROVIDER_ID && (
                          <div className="oauth-link">
                            <label>
                              {t(
                                "codex.modelProviders.newProviderName",
                                "供应商名称（自动保存时使用，可选）",
                              )}
                            </label>
                            <div className="oauth-url-box oauth-manual-input">
                              <input
                                type="text"
                                value={newManagedProviderNameInput}
                                onChange={(e) =>
                                  setNewManagedProviderNameInput(e.target.value)
                                }
                                placeholder={t(
                                  "codex.modelProviders.newProviderNamePlaceholder",
                                  "不填则按域名自动生成",
                                )}
                              />
                            </div>
                          </div>
                        )}
                        {apiProviderPresetId !== OPENAI_OFFICIAL_PRESET_ID && (
                          <>
                            <div className="api-model-catalog-panel">
                              <div className="api-model-catalog-header">
                                <label htmlFor="codex-api-model-catalog-add">
                                  {t(
                                    "codex.api.modelCatalog.label",
                                    "模型列表",
                                  )}
                                </label>
                                <span className="api-model-catalog-count">
                                  {t("codex.api.modelCatalog.count", {
                                    defaultValue: "{{count}} 个模型",
                                    count: apiModelCatalogDraft.length,
                                  })}
                                </span>
                              </div>
                              <textarea
                                id="codex-api-model-catalog-add"
                                className="form-input api-model-catalog-input"
                                rows={6}
                                value={apiModelCatalogInput}
                                onChange={(event) => {
                                  setApiModelCatalogInput(event.target.value);
                                  setApiModelCatalogError(null);
                                }}
                                placeholder={t(
                                  "codex.api.modelCatalog.placeholder",
                                  "每行填写一个模型 ID，也可以使用逗号分隔。",
                                )}
                                disabled={addStatus === "loading"}
                                aria-describedby="codex-api-model-catalog-add-hint"
                              />
                              <CodexModelContextWindowTable
                                models={apiModelCatalogDraft}
                                drafts={apiModelContextWindowsInput}
                                onChange={(model, value) => {
                                  setApiModelContextWindowsInput((current) => ({
                                    ...current,
                                    [model]: value,
                                  }));
                                  setApiModelCatalogError(null);
                                }}
                                disabled={addStatus === "loading"}
                              />
                              <div className="api-model-catalog-toolbar">
                                <p
                                  id="codex-api-model-catalog-add-hint"
                                  className="api-model-catalog-hint"
                                >
                                  {t(
                                    "codex.api.modelCatalog.editHint",
                                    "上游结果仅填入当前草稿，可在保存前删除、补充或调整模型。",
                                  )}
                                </p>
                                <button
                                  type="button"
                                  className="btn btn-secondary api-model-catalog-fetch"
                                  onClick={() =>
                                    void handleFetchApiModelCatalog()
                                  }
                                  disabled={
                                    apiModelCatalogFetching ||
                                    addStatus === "loading" ||
                                    !apiKeyInput.trim()
                                  }
                                >
                                  <RefreshCw
                                    size={14}
                                    className={
                                      apiModelCatalogFetching
                                        ? "loading-spinner"
                                        : undefined
                                    }
                                  />
                                  {apiModelCatalogFetching
                                    ? t(
                                        "codex.api.modelCatalog.fetching",
                                        "获取中...",
                                      )
                                    : t(
                                        "codex.api.modelCatalog.fetch",
                                        "从上游获取",
                                      )}
                                </button>
                              </div>
                              {apiModelCatalogError && (
                                <div className="add-status error api-model-catalog-error">
                                  <CircleAlert size={16} />
                                  <span>{apiModelCatalogError}</span>
                                </div>
                              )}
                            </div>
                            {apiModelCatalogSyncAvailable && (
                              <label className="codex-import-api-service-toggle api-model-catalog-sync-toggle">
                                <span className="codex-import-api-service-toggle-copy">
                                  <strong>
                                    {t(
                                      "codex.api.modelCatalog.syncToggle",
                                      "同步供应商模型到 Codex",
                                    )}
                                  </strong>
                                  <small>
                                    {t(
                                      "codex.api.modelCatalog.syncDescription",
                                      "保存后使用当前模型列表生成 Cockpit 受管的 Codex 模型目录，不覆盖用户自定义目录。",
                                    )}
                                  </small>
                                </span>
                                <input
                                  type="checkbox"
                                  checked={apiSyncModelCatalogToCodex}
                                  disabled={addStatus === "loading"}
                                  onChange={(event) => {
                                    setApiSyncModelCatalogToCodex(
                                      event.target.checked,
                                    );
                                    setApiModelCatalogError(null);
                                  }}
                                />
                                <span className="codex-import-api-service-switch" />
                              </label>
                            )}
                          </>
                        )}
                        <div className="api-key-add-actions">
                          <button
                            className="btn btn-primary"
                            onClick={() => void handleApiKeyLogin()}
                            disabled={
                              importing ||
                              addStatus === "loading" ||
                              apiModelCatalogFetching ||
                              !apiKeyInput.trim()
                            }
                          >
                            {addStatus === "loading" ? (
                              <RefreshCw
                                size={16}
                                className="loading-spinner"
                              />
                            ) : (
                              <KeyRound size={16} />
                            )}
                            {t("common.shared.addAccount", "添加账号")}
                          </button>
                        </div>
                      </div>
                    )}
                    {addTab === "token" && (
                      <div className="add-section">
                        <p className="section-desc">
                          {t(
                            "codex.token.desc",
                            "粘贴 auth.json、账号 JSON、Sub2API JSON、accessToken、个人访问令牌 at-… 或 refresh_token。",
                          )}
                        </p>
                        <details className="token-format-collapse">
                          <summary className="token-format-collapse-summary">
                            {t(
                              "codex.token.formatSummary",
                              "必填字段与示例（点击展开）",
                            )}
                          </summary>
                          <div className="token-format">
                            <p className="token-format-required">
                              {t(
                                "codex.token.formatRequired",
                                "支持 session JSON、完整 tokens（id_token + access_token）、Sub2API 导出 JSON、仅 accessToken、个人访问令牌 at-… / personal_access_token，或仅 refresh_token。仅 refresh_token 会先联网换取完整凭据；无 refresh 的 at-… 按 personal_access_token 形态落盘。",
                              )}
                            </p>
                            <div className="token-format-group">
                              <div className="token-format-label">
                                {t(
                                  "codex.token.formatSingleLabel",
                                  "完整 tokens 示例",
                                )}
                              </div>
                              <pre className="token-format-code">
                                {CODEX_TOKEN_SINGLE_EXAMPLE}
                              </pre>
                            </div>
                            <div className="token-format-group">
                              <div className="token-format-label">
                                {t(
                                  "codex.token.formatRefreshOnlyLabel",
                                  "session / accessToken / at- / refresh_token 示例",
                                )}
                              </div>
                              <pre className="token-format-code">
                                {CODEX_TOKEN_SESSION_EXAMPLE}
                              </pre>
                            </div>
                            <div className="token-format-group">
                              <div className="token-format-label">
                                {t("codex.token.formatBatchLabel", "批量示例")}
                              </div>
                              <pre className="token-format-code">
                                {CODEX_TOKEN_BATCH_EXAMPLE}
                              </pre>
                            </div>
                          </div>
                        </details>
                        <textarea
                          className="token-input"
                          value={tokenInput}
                          onChange={(e) => setTokenInput(e.target.value)}
                          disabled={importing}
                          placeholder={t(
                            "codex.token.placeholder",
                            '示例：session JSON、accessToken、at-… 个人访问令牌、Sub2API JSON，或 {"personal_access_token":"at-..."}',
                          )}
                        />
                        <label className="codex-import-api-service-toggle">
                          <span className="codex-import-api-service-toggle-copy">
                            <strong>
                              {t(
                                "codex.importApiService.toggle",
                                "同步加入 API 服务",
                              )}
                            </strong>
                            <small>
                              {t(
                                "codex.importApiService.description",
                                "导入成功后，将符合条件的账号加入 API 服务账号池。",
                              )}
                            </small>
                          </span>
                          <input
                            type="checkbox"
                            checked={syncImportedToApiService}
                            disabled={importing}
                            onChange={(event) =>
                              handleSyncImportedToApiServiceChange(
                                event.target.checked,
                              )
                            }
                          />
                          <span className="codex-import-api-service-switch" />
                        </label>
                        <button
                          className="btn btn-primary btn-full"
                          onClick={handleTokenImport}
                          disabled={importing || !tokenInput.trim()}
                        >
                          {importing ? (
                            <RefreshCw size={16} className="loading-spinner" />
                          ) : (
                            <Download size={16} />
                          )}
                          {tokenImportProgress
                            ? `${tokenImportProgress.current}/${tokenImportProgress.total}`
                            : t("common.shared.token.import", "Import")}
                        </button>
                      </div>
                    )}
                    {addTab === "import" && (
                      <div className="add-section">
                        <p className="section-desc">
                          {t(
                            "codex.import.localDesc",
                            "从本地已登录的会话中导入 Codex 账号。",
                          )}
                        </p>
                        <button
                          className="btn btn-primary btn-full"
                          onClick={handleImportFromLocal}
                          disabled={importing}
                        >
                          {importing ? (
                            <RefreshCw size={16} className="loading-spinner" />
                          ) : (
                            <Database size={16} />
                          )}
                          {t("codex.local.import", "Get Local Account")}
                        </button>
                        <div style={{ height: 12 }} />
                        <p className="section-desc">
                          {t("modals.import.fromFilesDesc")}
                        </p>
                        <label className="codex-import-api-service-toggle">
                          <span className="codex-import-api-service-toggle-copy">
                            <strong>
                              {t(
                                "codex.importApiService.toggle",
                                "同步加入 API 服务",
                              )}
                            </strong>
                            <small>
                              {t(
                                "codex.importApiService.description",
                                "导入成功后，将符合条件的账号加入 API 服务账号池。",
                              )}
                            </small>
                          </span>
                          <input
                            type="checkbox"
                            checked={syncImportedToApiService}
                            disabled={importing}
                            onChange={(event) =>
                              handleSyncImportedToApiServiceChange(
                                event.target.checked,
                              )
                            }
                          />
                          <span className="codex-import-api-service-switch" />
                        </label>
                        <button
                          className="btn btn-secondary btn-full"
                          onClick={handleImportFromFiles}
                          disabled={importing}
                        >
                          {importing ? (
                            <RefreshCw size={16} className="loading-spinner" />
                          ) : (
                            <FileUp size={16} />
                          )}
                          {t("modals.import.fromFiles")}
                        </button>
                      </div>
                    )}
                    {addStatus !== "idle" && (
                      <div className={`add-status ${addStatus}`}>
                        {addStatus === "success" ? (
                          <Check size={16} />
                        ) : addStatus === "loading" ? (
                          <RefreshCw size={16} className="loading-spinner" />
                        ) : (
                          <CircleAlert size={16} />
                        )}
                        <span>{addMessage}</span>
                        {addTab === "oauth" &&
                          addStatus === "error" &&
                          isOauthTokenExchangeErrorState &&
                          oauthLoginIdRef.current && (
                            <button
                              className="btn btn-sm btn-outline"
                              onClick={() =>
                                void handleRetryOauthTokenExchange()
                              }
                              disabled={oauthCallbackSubmitting}
                            >
                              {oauthCallbackSubmitting ? (
                                <RefreshCw
                                  size={14}
                                  className="loading-spinner"
                                />
                              ) : (
                                <RotateCw size={14} />
                              )}
                              {t("accounts.oauth.continue")}
                            </button>
                          )}
                      </div>
                    )}
                  </div>
                </div>
              </div>,
              document.body,
            );
}
