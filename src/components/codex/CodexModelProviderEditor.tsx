import { OpenAIUsageKeyField, OpenAIUsageSettingsButton } from "./OpenAIUsageSettingsDialog";
import { isOpenAIOfficialUsageUrl } from "../../utils/openaiOfficialUsage";
import "./CodexModelProviderEditor.css";
import { SingleSelectDropdown } from "../SingleSelectDropdown";
import { CodexProviderModelFetch } from "./CodexProviderModelFetch";
import { createPortal } from 'react-dom';
import { useModalScrollLock } from '../../hooks/useModalScrollLock';
import { CodexAnthropicProviderFields } from "./CodexAnthropicProviderFields";
import { ModalErrorMessage } from "../ModalErrorMessage";
import { Check, ExternalLink, HelpCircle, KeyRound, Pencil, Star, Trash2, X, Search } from "lucide-react";
import { CodexModelContextWindowTable } from "./CodexModelContextWindowTable";
import { CODEX_API_PROVIDER_CUSTOM_ID, CODEX_API_PROVIDER_PRESETS, DEEPSEEK_API_PROVIDER_ID } from "../../utils/codexProviderPresets";
import { canConfigureCodexProviderVision } from "../../utils/codexProviderGateway";
import type { useCodexModelProviderManagerController } from './CodexModelProviderManager';

type Props = Pick<ReturnType<typeof useCodexModelProviderManagerController>,
  'accountEditor' | 'setAccountEditor' | 'accounts' | 'providers' | 'editorModelKeyId' | 'selectEditorModelKey' | 'selectAccountEditorProvider' | 'maskAccountText' | 'closeModal' | 'currentEditingProvider' | 'editingApiKey' | 'existingApiKeySearchQuery' | 'form' | 'formError' | 'handleDeleteApiKey' | 'handleSaveApiKeyEdit' | 'handleSaveProvider' | 'handleSelectPresetEndpoint' | 'handleSelectProviderPreset' | 'handleSelectSponsorTemplate' | 'maskApiKey' | 'mutateForm' | 'parseModelCatalogText' | 'previewPaths' | 'resolveEnableModePreferenceForWireApi' | 'saving' | 'selectedPreset' | 'selectedPresetId' | 'selectedSponsorTemplate' | 'selectedSponsorTemplateId' | 'setEditingApiKey' | 'setExistingApiKeySearchQuery' | 'showModal' | 'sponsorProviderTemplates' | 't'>;

export function CodexModelProviderEditor(props: Props) {
  const { accountEditor, setAccountEditor, accounts, providers, editorModelKeyId, selectEditorModelKey, selectAccountEditorProvider, maskAccountText, closeModal, currentEditingProvider, editingApiKey, existingApiKeySearchQuery, form, formError, handleDeleteApiKey, handleSaveApiKeyEdit, handleSaveProvider, handleSelectPresetEndpoint, handleSelectProviderPreset, handleSelectSponsorTemplate, maskApiKey, mutateForm, parseModelCatalogText, previewPaths, resolveEnableModePreferenceForWireApi, saving, selectedPreset, selectedPresetId, selectedSponsorTemplate, selectedSponsorTemplateId, setEditingApiKey, setExistingApiKeySearchQuery, showModal, sponsorProviderTemplates, t } = props;
  useModalScrollLock(showModal);
  const selectedKey = currentEditingProvider?.apiKeys.find(key => key.id === (editorModelKeyId ?? accountEditor?.apiKeyId));
  const queryKey = form.newApiKey || selectedKey?.apiKey || currentEditingProvider?.apiKeys[0]?.apiKey || "";
  const boundAccount = accounts.find(account => account.id === accountEditor?.accountId);
  let officialDeepSeek = false;
  try { officialDeepSeek = new URL(form.baseUrl.trim()).hostname.toLowerCase() === "api.deepseek.com"; } catch { /* incomplete URL */ }
  const effectiveWireApi = officialDeepSeek ? "responses" : form.wireApi;
  return showModal ? createPortal((
        <div className="modal-overlay">
          <div
            className="modal codex-provider-modal"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <h2>
                {form.providerId
                  ? t("codex.modelProviders.editTitle", "编辑模型供应商")
                  : t("codex.modelProviders.createTitle", "新增模型供应商")}
              </h2>
              <button
                className="modal-close"
                onClick={closeModal}
                aria-label={t("common.close", "关闭")}
                disabled={saving}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              {accountEditor && <section className="form-group">
                <label>{maskAccountText(boundAccount?.account_name || boundAccount?.email || accountEditor.accountId)}</label>
                <SingleSelectDropdown value={form.providerId ?? ""} options={providers.map(provider => ({ value: provider.id, label: provider.name }))}
                  onChange={selectAccountEditorProvider} disabled={saving} placeholder={t("codex.modelProviders.selectSavedProvider")} />
                <label>{t("codex.modelProviders.selectSavedApiKey")}</label>
                <SingleSelectDropdown value={accountEditor.apiKeyId}
                  options={[...(currentEditingProvider?.apiKeys ?? []).map(key => ({ value: key.id, label: key.name || maskApiKey(key.apiKey) })), { value: "__new__", label: t("codex.modelProviders.fields.newApiKey") }]}
                  onChange={apiKeyId => { setAccountEditor(current => current ? { ...current, apiKeyId } : null); selectEditorModelKey(apiKeyId); }} disabled={saving} />
                {effectiveWireApi === "responses" && selectedPresetId !== "openai_official" && <label className="provider-vision-toggle">
                  <span className="provider-vision-toggle-copy"><span>{t("codex.api.modelCatalog.syncToggle")}</span><small>{t("codex.api.modelCatalog.syncDescription")}</small></span>
                  <span className="provider-vision-switch"><input type="checkbox" checked={accountEditor.syncModels} onChange={event => { mutateForm({}); setAccountEditor(current => current ? { ...current, syncModels: event.target.checked } : null); }} disabled={saving} /><span className="provider-vision-switch-track" /></span>
                </label>}
              </section>}
              {currentEditingProvider && <div className="form-group">
                <label>{t("codex.modelProviders.editor.modelScope")}</label>
                <SingleSelectDropdown value={editorModelKeyId ?? ""} options={[{ value: "", label: t("codex.modelProviders.editor.providerDefault") }, ...currentEditingProvider.apiKeys.map(key => ({ value: key.id, label: key.name || maskApiKey(key.apiKey) })), { value: "__new__", label: t("codex.modelProviders.fields.newApiKey") }]}
                  onChange={selectEditorModelKey} disabled={saving} />
                <p className="api-provider-hint">{t("codex.modelProviders.editor.sharedSettings")}</p>
              </div>}
              <div className="form-group">
                <label>{t("codex.api.provider.label", "供应商")}</label>
                <div className="api-provider-chip-list">
                  <button
                    className={`api-provider-chip ${selectedPresetId === CODEX_API_PROVIDER_CUSTOM_ID && !selectedSponsorTemplateId ? "active" : ""}`}
                    onClick={() =>
                      handleSelectProviderPreset(CODEX_API_PROVIDER_CUSTOM_ID)
                    }
                    type="button"
                    disabled={saving}
                  >
                    <span>{t("codex.api.provider.custom", "自定义")}</span>
                  </button>
                  {sponsorProviderTemplates.map((template) => (
                    <button
                      key={template.id}
                      className={`api-provider-chip sponsor ${selectedSponsorTemplateId === template.id ? "active" : ""}`}
                      onClick={() => handleSelectSponsorTemplate(template)}
                      type="button"
                      disabled={saving}
                    >
                      <span>{template.name}</span>
                      <Star size={12} className="api-provider-chip-badge" />
                    </button>
                  ))}
                  {CODEX_API_PROVIDER_PRESETS.filter(
                    (preset) => !preset.isService,
                  ).map((preset) => (
                    <button
                      key={preset.id}
                      className={`api-provider-chip ${selectedPresetId === preset.id ? "active" : ""}`}
                      onClick={() => handleSelectProviderPreset(preset.id)}
                      type="button"
                      disabled={saving}
                    >
                      <span>
                        {t(
                          `codex.api.providers.${preset.id}.name`,
                          preset.name,
                        )}
                      </span>
                      {preset.isPartner && (
                        <Star size={12} className="api-provider-chip-badge" />
                      )}
                    </button>
                  ))}
                </div>
              </div>
              {selectedPreset && selectedPreset.baseUrls.length > 1 && (
                <div className="form-group">
                  <label>
                    {t("codex.api.provider.endpoint", "供应商端点")}
                  </label>
                  <div className="api-provider-endpoint-list">
                    {selectedPreset.baseUrls.map((baseUrl) => (
                      <button
                        key={baseUrl}
                        className={`api-provider-endpoint-chip ${form.baseUrl === baseUrl ? "active" : ""}`}
                        onClick={() => handleSelectPresetEndpoint(baseUrl)}
                        type="button"
                        disabled={saving}
                      >
                        {baseUrl}
                      </button>
                    ))}
                  </div>
                </div>
              )}
              {selectedPreset && (
                <div className="api-provider-hint-block">
                  <p className="api-provider-hint">
                    {t(
                      "codex.api.provider.hint",
                      "已自动填写兼容 Base URL，可继续手动调整。",
                    )}
                  </p>
                  <div className="api-provider-links">
                    {selectedPreset.website && (
                      <a
                        className="btn btn-secondary"
                        href={selectedPreset.website}
                        target="_blank"
                        rel="noreferrer"
                      >
                        <ExternalLink size={14} />
                        {t("codex.api.provider.website", "官网")}
                      </a>
                    )}
                    {selectedPreset.apiKeyUrl && (
                      <a
                        className="btn btn-secondary"
                        href={selectedPreset.apiKeyUrl}
                        target="_blank"
                        rel="noreferrer"
                      >
                        <KeyRound size={14} />
                        {t("codex.api.provider.apiKeyPage", "API Key 页面")}
                      </a>
                    )}
                  </div>
                </div>
              )}
              {selectedSponsorTemplate && (
                <div className="api-provider-hint-block sponsor">
                  <p className="api-provider-hint">
                    {t(
                      "codex.modelProviders.sponsorHint",
                      "已按专属中转站配置自动填写兼容服务地址。输入 API Key 后，卡片会自动查询余额和用量。",
                    )}
                  </p>
                  <div className="api-provider-links">
                    {selectedSponsorTemplate.website && (
                      <a
                        className="btn btn-secondary"
                        href={selectedSponsorTemplate.website}
                        target="_blank"
                        rel="noreferrer"
                      >
                        <ExternalLink size={14} />
                        {t("codex.api.provider.website", "官网")}
                      </a>
                    )}
                    {selectedSponsorTemplate.apiKeyUrl && (
                      <a
                        className="btn btn-secondary"
                        href={selectedSponsorTemplate.apiKeyUrl}
                        target="_blank"
                        rel="noreferrer"
                      >
                        <KeyRound size={14} />
                        {t("codex.api.provider.apiKeyPage", "API Key 页面")}
                      </a>
                    )}
                  </div>
                </div>
              )}
              <div className="form-group">
                <label>
                  {t("codex.modelProviders.fields.name", "供应商名称")}
                </label>
                <input
                  className="form-input"
                  type="text"
                  value={form.name}
                  onChange={(event) => mutateForm({ name: event.target.value })}
                  disabled={saving}
                />
              </div>
              <div className="form-group">
                <label>
                  {t("codex.modelProviders.fields.baseUrl", "Base URL")}
                </label>
                <input
                  className="form-input"
                  type="text"
                  value={form.baseUrl}
                  onChange={(event) =>
                    mutateForm({ baseUrl: event.target.value })
                  }
                  disabled={saving}
                />
              </div>
              {!officialDeepSeek && <div className="form-group">
                <label className="codex-provider-label-with-help">
                  <span>{t("codex.modelProviders.fields.wireApi", "协议")}</span>
                  <span
                    className="codex-provider-inline-help"
                    title={t(
                      "codex.modelProviders.wireApi.help",
                      "大多数供应商请选择 Responses；仅当供应商明确只支持 Chat Completions 时再切换。如果不确定，优先选 Responses。",
                    )}
                    aria-label={t(
                      "codex.modelProviders.wireApi.helpAria",
                      "查看协议说明",
                    )}
                  >
                    <HelpCircle size={14} />
                  </span>
                </label>
                <div className="api-provider-chip-list">
                  <button
                    type="button"
                    className={`api-provider-chip ${effectiveWireApi === "responses" ? "active" : ""}`}
                    onClick={() =>
                      mutateForm({
                        wireApi: "responses",
                        enableModePreference:
                          resolveEnableModePreferenceForWireApi(
                            "responses",
                            selectedPresetId,
                          ),
                      })
                    }
                    disabled={saving}
                  >
                    <span>
                      {t("codex.modelProviders.wireApi.responses", "Responses 原生")}
                    </span>
                  </button>
                  <button
                    type="button"
                    className={`api-provider-chip ${effectiveWireApi === "chat_completions" ? "active" : ""}`}
                    onClick={() =>
                      mutateForm({
                        wireApi: "chat_completions",
                        supportsWebsockets: false,
                        enableModePreference:
                          resolveEnableModePreferenceForWireApi(
                            "chat_completions",
                            selectedPresetId,
                          ),
                      })
                    }
                    disabled={saving}
                  >
                    <span>
                      {t(
                        "codex.modelProviders.wireApi.chatCompletions",
                        "Chat Completions 协议",
                      )}
                    </span>
                  </button>
                  <button type="button" className={`api-provider-chip ${effectiveWireApi === "anthropic" ? "active" : ""}`}
                    disabled={saving} onClick={() => mutateForm({ wireApi: "anthropic", supportsWebsockets: false, enableModePreference: "gateway" })}>
                    {t("codex.modelProviders.wireApi.anthropic")}
                  </button>
                </div>
                {selectedPresetId === DEEPSEEK_API_PROVIDER_ID && (
                  <p className="api-provider-hint">
                    {effectiveWireApi === "responses"
                      ? t(
                          "codex.modelProviders.wireApi.deepseekResponsesHint",
                          "原生 Responses 直连官方 API，写入官方 models.json（工具/shell/apply_patch），默认模型 deepseek-flash。",
                        )
                      : t(
                          "codex.modelProviders.wireApi.deepseekChatHint",
                          "DeepSeek Chat Completions 走本地网关协议转换，适合兼容旧链路；需要官方 Codex 工具形态时请选 Responses。",
                        )}
                  </p>
                )}
              </div>}
              {/* DeepSeek 官方 Responses 强制直连、不支持 WebSocket：不展示该开关。 */}
              {effectiveWireApi === "responses" &&
                selectedPresetId !== DEEPSEEK_API_PROVIDER_ID && (
                <div className="form-group">
                  <label>
                    {t(
                      "codex.modelProviders.fields.supportsWebsockets",
                      "WebSocket 传输",
                    )}
                  </label>
                  <label className="provider-vision-toggle">
                    <span className="provider-vision-toggle-copy">
                      <span className="provider-vision-toggle-title">
                        {t(
                          "codex.modelProviders.websockets.title",
                          "允许 Codex 使用 Responses WebSocket",
                        )}
                      </span>
                      <span className="provider-vision-toggle-desc">
                        {t(
                          "codex.modelProviders.websockets.help",
                          "仅在供应商明确支持 Responses WebSocket 时开启；连接方式可通过 Codex 或代理服务日志确认。",
                        )}
                      </span>
                    </span>
                    <span className="provider-vision-switch">
                      <input
                        type="checkbox"
                        checked={form.supportsWebsockets}
                        onChange={(event) =>
                          mutateForm({ supportsWebsockets: event.target.checked })
                        }
                        disabled={
                          saving ||
                          selectedPresetId === "openai_official" ||
                          selectedPresetId === DEEPSEEK_API_PROVIDER_ID
                        }
                      />
                      <span className="provider-vision-switch-track" />
                    </span>
                  </label>
                </div>
              )}
              {effectiveWireApi === "anthropic" && <CodexAnthropicProviderFields
                baseUrl={form.baseUrl} apiKey={queryKey}
                authScheme={form.authScheme} disabled={saving}
                onAuthScheme={authScheme => mutateForm({ authScheme })}
                onImport={config => mutateForm({ baseUrl: config.baseUrl, newApiKey: config.apiKey, authScheme: config.authScheme,
                  modelCatalogText: config.models.join("\n"), supportsWebsockets: false, enableModePreference: "gateway" })}
                onModels={models => mutateForm({ modelCatalogText: [...new Set([...parseModelCatalogText(form.modelCatalogText), ...models])].join("\n") })} />}
              {(
                <>
                  <div className="form-group">
                    <label>
                      {t("codex.modelProviders.fields.modelCatalog", "模型目录")}
                    </label>
                    <textarea
                      className="form-input"
                      rows={4}
                      value={form.modelCatalogText}
                      onChange={(event) =>
                        mutateForm({ modelCatalogText: event.target.value })
                      }
                      placeholder={effectiveWireApi === "anthropic" ? "claude-sonnet-5-5\nclaude-opus-5-5" : "deepseek-flash\ndeepseek-v4-pro"}
                      disabled={saving}
                    />
                    {effectiveWireApi !== "anthropic" && <CodexProviderModelFetch baseUrl={form.baseUrl} apiKey={queryKey} wireApi={effectiveWireApi} authScheme={form.authScheme} disabled={saving}
                      onModels={models => mutateForm({ modelCatalogText: models.join("\n") })} />}
                    <CodexModelContextWindowTable
                      models={parseModelCatalogText(form.modelCatalogText)}
                      drafts={form.modelContextWindowsDraft}
                      showContextWindow={true}
                      onChange={(model, value) =>
                        mutateForm({
                          modelContextWindowsDraft: {
                            ...form.modelContextWindowsDraft,
                            [model]: value,
                          },
                        })
                      }
                      visionStates={form.visionModelStates}
                      visionDefault={form.supportsVision}
                      onVisionChange={(model, value) =>
                        mutateForm({
                          visionModelStates: {
                            ...form.visionModelStates,
                            [model]: value,
                          },
                        })
                      }
                      disabled={saving}
                    />
                  </div>
                  {/* DeepSeek Responses 只保留逐模型能力；其他第三方协议均可配置默认值。 */}
                  {canConfigureCodexProviderVision({ presetId: selectedPresetId, wireApi: effectiveWireApi }) && (
                    <>
                  <div className="form-group">
                    <label>
                      {t(
                        "codex.modelProviders.fields.visionCapability",
                        "图片输入能力",
                      )}
                    </label>
                    <label className="provider-vision-toggle">
                      <span className="provider-vision-toggle-copy">
                        <span className="provider-vision-toggle-title">
                          {t(
                            "codex.modelProviders.vision.allModels",
                            "图片输入",
                          )}
                        </span>
                        <span className="provider-vision-toggle-desc">
                          {t(
                            "codex.modelProviders.vision.providerDefaultHint",
                            "未单独设置的模型使用此默认值，GPT-5.6 Sol、GPT-5.6 Luna 等模型默认支持图片。可在模型列表中单独关闭。网关无法匹配视觉模型时会省略图片并继续处理文本。",
                          )}
                        </span>
                      </span>
                      <span className="provider-vision-switch">
                        <input
                          type="checkbox"
                          checked={form.supportsVision}
                          onChange={(event) =>
                            mutateForm({ supportsVision: event.target.checked })
                          }
                          disabled={saving}
                        />
                        <span className="provider-vision-switch-track" />
                      </span>
                    </label>
                  </div>
                  <div className="form-group">
                    <label>
                      {t(
                        "codex.modelProviders.fields.visionModels",
                        "支持图片的模型",
                      )}
                    </label>
                    {parseModelCatalogText(form.modelCatalogText).length === 0 ? (
                      <>
                        <textarea
                          className="form-input"
                          rows={3}
                          value={form.visionModelText}
                          onChange={(event) =>
                            mutateForm({ visionModelText: event.target.value })
                          }
                          placeholder={"qwen-vl-plus\ngpt-6.1-sol"}
                          disabled={saving}
                        />
                        <p className="api-provider-hint">
                          {t(
                            "codex.modelProviders.vision.modelsHint",
                            "每行一个模型名。适合同一供应商里只有部分视觉模型支持粘贴图片的情况。",
                          )}
                        </p>
                      </>
                    ) : null}
                </div>
                <div className="form-group">
                  <label>
                    {t(
                      "codex.modelProviders.fields.visionRoutingModel",
                      "图片请求默认模型",
                    )}
                  </label>
                  <input
                    className="form-input"
                    value={form.visionRoutingModel}
                    onChange={(event) =>
                      mutateForm({ visionRoutingModel: event.target.value })
                    }
                    placeholder={"mimo-v2.5"}
                    disabled={saving}
                  />
                  <p className="api-provider-hint">
                    {t(
                      "codex.modelProviders.vision.routingModelHint",
                      "当前模型不支持图片时，网关会改用该模型；留空时自动使用唯一视觉模型，否则省略图片并继续处理文本。",
                    )}
                  </p>
                </div>
                    </>
                  )}
                {effectiveWireApi === "chat_completions" && (
                  <p className="api-provider-hint">
                    {t(
                      "codex.modelProviders.gatewayHint",
                      "第三方供应商启动时会使用本地网关隔离实例并完成协议转换；OpenAI 官方供应商保持直连。",
                    )}
                  </p>
                )}
                </>
              )}
              <div className="form-group">
                <label>
                  {t("codex.modelProviders.fields.website", "官网（可选）")}
                </label>
                <input
                  className="form-input"
                  type="text"
                  value={form.website}
                  onChange={(event) =>
                    mutateForm({ website: event.target.value })
                  }
                  disabled={saving}
                />
              </div>
              <div className="form-group">
                <label>
                  {t(
                    "codex.modelProviders.fields.apiKeyUrl",
                    "API Key 页面（可选）",
                  )}
                </label>
                <input
                  className="form-input"
                  type="text"
                  value={form.apiKeyUrl}
                  onChange={(event) =>
                    mutateForm({ apiKeyUrl: event.target.value })
                  }
                  disabled={saving}
                />
              </div>

              {currentEditingProvider &&
                currentEditingProvider.apiKeys.length > 0 && (
                  <div className="form-group">
                    <label>
                      {t(
                        "codex.modelProviders.existingApiKeys",
                        "现有 API Keys",
                      )}
                    </label>
                    {currentEditingProvider.apiKeys.length > 5 && (
                      <div className="search-box codex-provider-key-search">
                        <Search className="search-icon" size={16} />
                        <input
                          type="text"
                          placeholder={t(
                            "codex.modelProviders.existingApiKeysSearch",
                            "搜索已有 API Key…",
                          )}
                          value={existingApiKeySearchQuery}
                          onChange={(event) =>
                            setExistingApiKeySearchQuery(event.target.value)
                          }
                          disabled={saving}
                        />
                      </div>
                    )}
                    <div className="codex-provider-key-list inline">
                      {currentEditingProvider.apiKeys
                        .filter((item) => {
                          const query = existingApiKeySearchQuery
                            .trim()
                            .toLowerCase();
                          if (!query) return true;
                          const label = (
                            item.name ||
                            t("codex.modelProviders.unnamedKey", "未命名 Key")
                          ).toLowerCase();
                          const masked = maskApiKey(item.apiKey).toLowerCase();
                          return (
                            label.includes(query) ||
                            masked.includes(query) ||
                            item.apiKey.toLowerCase().includes(query)
                          );
                        })
                        .map((item) => {
                          const isEditing = editingApiKey?.apiKeyId === item.id;
                          if (isEditing && editingApiKey) {
                            return (
                              <div
                                className="codex-provider-key-row is-editing"
                                key={item.id}
                              >
                                <div className="codex-provider-key-edit-fields">
                                  <input
                                    className="form-input"
                                    type="text"
                                    value={editingApiKey.name}
                                    onChange={(event) =>
                                      setEditingApiKey((current) =>
                                        current
                                          ? { ...current, name: event.target.value }
                                          : current,
                                      )
                                    }
                                    placeholder={t(
                                      "codex.modelProviders.fields.newApiKeyName",
                                      "API Key name (optional)",
                                    )}
                                    disabled={saving}
                                  />
                                  <input
                                    className="form-input"
                                    type="password"
                                    value={editingApiKey.apiKey}
                                    onChange={(event) =>
                                      setEditingApiKey((current) =>
                                        current
                                          ? { ...current, apiKey: event.target.value }
                                          : current,
                                      )
                                    }
                                    placeholder={t(
                                      "codex.modelProviders.fields.apiKey",
                                      "API Key",
                                    )}
                                    autoComplete="off"
                                    disabled={saving}
                                  />
                                </div>
                                <div className="codex-provider-key-edit-actions">
                                  <button
                                    type="button"
                                    className="action-btn success"
                                    onClick={() => void handleSaveApiKeyEdit()}
                                    disabled={saving}
                                    title={t("common.save", "Save")}
                                  >
                                    <Check size={12} />
                                  </button>
                                  <button
                                    type="button"
                                    className="action-btn"
                                    onClick={() => setEditingApiKey(null)}
                                    disabled={saving}
                                    title={t("common.cancel", "Cancel")}
                                  >
                                    <X size={12} />
                                  </button>
                                </div>
                              </div>
                            );
                          }
                          return (
                            <div className="codex-provider-key-row" key={item.id}>
                          <div className="codex-provider-key-text">
                            <span className="codex-provider-key-name">
                              {item.name ||
                                t(
                                  "codex.modelProviders.unnamedKey",
                                  "未命名 Key",
                                )}
                            </span>
                            <code>{maskApiKey(item.apiKey)}</code>
                          </div>
                          <button
                            type="button"
                            className="action-btn"
                            onClick={() =>
                              setEditingApiKey({
                                providerId: currentEditingProvider.id,
                                apiKeyId: item.id,
                                originalApiKey: item.apiKey,
                                apiKey: item.apiKey,
                                name: item.name,
                              })
                            }
                            disabled={saving}
                            title={t(
                              "codex.modelProviders.editApiKey",
                              "Edit API Key",
                            )}
                          >
                            <KeyRound size={12} />
                          </button>
                          <button
                            type="button"
                            className="action-btn"
                            onClick={() =>
                              setEditingApiKey({ providerId: currentEditingProvider.id, apiKeyId: item.id,
                                originalApiKey: item.apiKey, apiKey: item.apiKey, name: item.name })
                            }
                            disabled={saving}
                            title={t("common.rename", "重命名")}
                          >
                            <Pencil size={12} />
                          </button>
                          <button
                            type="button"
                            className="action-btn danger"
                            onClick={() =>
                              void handleDeleteApiKey(
                                currentEditingProvider,
                                item,
                              )
                            }
                            disabled={saving}
                            title={t("common.delete", "删除")}
                          >
                            <Trash2 size={12} />
                          </button>
                            </div>
                          );
                        })}
                    </div>
                  </div>
                )}

              <div className="form-group">
                <label>
                  {t(
                    "codex.modelProviders.fields.newApiKeyName",
                    "新增 Key 名称（可选）",
                  )}
                </label>
                <input
                  className="form-input"
                  type="text"
                  value={form.newApiKeyName}
                  onChange={(event) =>
                    mutateForm({ newApiKeyName: event.target.value })
                  }
                  disabled={saving}
                />
              </div>
              <div className="form-group">
                <label>
                  {t(isOpenAIOfficialUsageUrl(form.baseUrl) ? "codex.openaiUsage.callKey" : "codex.modelProviders.fields.newApiKey")}
                </label>
                <input
                  className="form-input"
                  type="text"
                  value={form.newApiKey}
                  onChange={(event) =>
                    mutateForm({ newApiKey: event.target.value, newUsageApiKey: "" })
                  }
                  disabled={saving}
                />
              </div>

              {isOpenAIOfficialUsageUrl(form.baseUrl) && <details className="openai-usage-optional">
                <summary>{t('codex.openaiUsage.optional')}</summary>
                {form.newApiKey.trim() || !currentEditingProvider
                  ? <OpenAIUsageKeyField value={form.newUsageApiKey} onChange={newUsageApiKey => mutateForm({ newUsageApiKey })} disabled={saving} t={t} />
                  : <OpenAIUsageSettingsButton provider={currentEditingProvider} apiKey={queryKey} t={t} />}
              </details>}

              <div className="provider-save-preview">
                <strong>{t("codex.modelProviders.preview.providerStoreTitle")}</strong>
                <code>{previewPaths.providerStorePath}</code>
                <p className="provider-save-preview-desc">{t("codex.modelProviders.editor.saveScope")}</p>
              </div>

              <ModalErrorMessage message={formError} position="bottom" />
            </div>

            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={closeModal}
                disabled={saving}
              >
                {t("common.cancel", "取消")}
              </button>
              <button
                className="btn btn-primary codex-provider-save-button"
                onClick={() => void handleSaveProvider()}
                disabled={saving}
              >
                <span className="codex-provider-save-button-label">
                  <span aria-hidden={saving}>
                    {t("common.save", "保存")}
                  </span>
                  <span aria-hidden={!saving}>
                    {t("common.saving", "保存中...")}
                  </span>
                </span>
              </button>
            </div>
          </div>
        </div>
      ), document.body) : null;
}
