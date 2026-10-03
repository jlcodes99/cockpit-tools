import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { ExternalLink, RefreshCw, Star } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";
import * as piService from "../../services/piService";
import {
  PI_BUILTIN_PROVIDERS,
  PI_GATEWAY_CUSTOM_ID,
  PI_GATEWAY_PRESETS,
  findPiGatewayPreset,
  getPresetApis,
  toGatewayProviderId,
  type PiGatewayApi,
} from "../../utils/piProviderPresets";

export type PiApiMode = "builtin" | "gateway";

export interface PiGatewayState {
  mode: PiApiMode;
  /** builtin mode: pi provider id. */
  builtinProvider: string;
  presetId: string;
  providerId: string;
  api: PiGatewayApi;
  baseUrl: string;
  authHeader: boolean;
  models: string[];
  displayName: string;
  defaultModel: string;
}

export function createPiGatewayState(): PiGatewayState {
  return {
    mode: "gateway",
    builtinProvider: "",
    presetId: "",
    providerId: "",
    api: "openai-completions",
    baseUrl: "",
    authHeader: false,
    models: [],
    displayName: "",
    defaultModel: "",
  };
}

export function isPiGatewayStateReady(state: PiGatewayState): boolean {
  if (state.mode === "builtin") return !!state.builtinProvider.trim();
  return (
    !!state.providerId.trim() &&
    /^https?:\/\//i.test(state.baseUrl.trim()) &&
    state.models.length > 0
  );
}

const API_LABELS: Record<PiGatewayApi, string> = {
  "openai-completions": "OpenAI Chat",
  "openai-responses": "OpenAI Responses",
  "anthropic-messages": "Anthropic",
};

interface Props {
  state: PiGatewayState;
  apiKey: string;
  onChange: (patch: Partial<PiGatewayState>) => void;
  /** Editing an existing gateway: provider id is locked, stored key is reused. */
  edit?: { accountId: string };
}

export function PiGatewayFields({ state, apiKey, onChange, edit }: Props) {
  const canFetch = !!apiKey.trim() || !!edit;
  const { t } = useTranslation();
  const [modelDraft, setModelDraft] = useState("");
  const [fetching, setFetching] = useState(false);
  const [fetchError, setFetchError] = useState<string | null>(null);
  const [fetchedModels, setFetchedModels] = useState<string[]>([]);

  const preset = useMemo(() => findPiGatewayPreset(state.presetId), [state.presetId]);
  const apis = getPresetApis(preset);
  const endpoints = preset?.endpoints[state.api] ?? [];

  const resetDiscovery = () => {
    setFetchedModels([]);
    setFetchError(null);
  };

  const selectPreset = (presetId: string) => {
    resetDiscovery();
    const next = findPiGatewayPreset(presetId);
    if (!next) {
      onChange({
        presetId: PI_GATEWAY_CUSTOM_ID,
        providerId: state.presetId === PI_GATEWAY_CUSTOM_ID ? state.providerId : "",
        baseUrl: "",
        models: [],
        defaultModel: "",
      });
      return;
    }
    const nextApis = getPresetApis(next);
    const api = nextApis.includes(state.api) ? state.api : nextApis[0];
    const catalog = next.modelCatalog ?? [];
    onChange({
      presetId,
      providerId: toGatewayProviderId(next.id),
      api,
      baseUrl: next.endpoints[api]?.[0] ?? "",
      // Most Claude-compatible relays expect a bearer token.
      authHeader: api === "anthropic-messages",
      models: catalog,
      defaultModel: catalog[0] ?? "",
      displayName: state.displayName || next.name,
    });
  };

  const selectApi = (api: PiGatewayApi) => {
    resetDiscovery();
    onChange({
      api,
      baseUrl: preset?.endpoints[api]?.[0] ?? state.baseUrl,
      authHeader: api === "anthropic-messages" ? state.authHeader || !!preset : false,
    });
  };

  const addModels = (ids: string[]) => {
    const next = Array.from(new Set([...state.models, ...ids.map((id) => id.trim()).filter(Boolean)]));
    onChange({ models: next, defaultModel: state.defaultModel || next[0] || "" });
  };

  const removeModel = (id: string) => {
    const next = state.models.filter((item) => item !== id);
    onChange({
      models: next,
      defaultModel: state.defaultModel === id ? next[0] ?? "" : state.defaultModel,
    });
  };

  const fetchModels = async () => {
    setFetching(true);
    setFetchError(null);
    try {
      const models = await piService.listPiGatewayModels({
        baseUrl: state.baseUrl,
        api: state.api,
        apiKey,
        authHeader: state.authHeader,
        accountId: edit?.accountId,
        provider: edit ? state.providerId : undefined,
      });
      setFetchedModels(models);
      if (models.length === 0) {
        setFetchError(t("pi.gateway.noModels", "接口未返回模型，请手动添加"));
      }
    } catch (error) {
      setFetchError(String(error).replace(/^Error:\s*/, ""));
    } finally {
      setFetching(false);
    }
  };

  const modeSwitch = (
    <div className="form-group">
      <div className="pi-segmented pi-segmented-2">
        {(["gateway", "builtin"] as const).map((mode) => (
          <button
            key={mode}
            type="button"
            className={`claude-provider-endpoint-chip ${state.mode === mode ? "active" : ""}`}
            onClick={() => onChange({ mode })}
          >
            {mode === "gateway"
              ? t("pi.gateway.modeGateway", "三方网关")
              : t("pi.gateway.modeBuiltin", "pi 内置提供商")}
          </button>
        ))}
      </div>
    </div>
  );

  const nameField = (
    <div className="form-group">
      <label htmlFor="pi-gw-display-name">
        {t("pi.import.displayName", "账号名称（可选）")}
      </label>
      <input
        id="pi-gw-display-name"
        className="form-input"
        value={state.displayName}
        autoComplete="off"
        onChange={(event) => onChange({ displayName: event.target.value })}
      />
    </div>
  );

  if (state.mode === "builtin") {
    return (
      <div className="pi-gateway-fields">
        {modeSwitch}
        <div className="form-group">
          <label>{t("pi.import.provider", "提供商 ID")}</label>
          <div className="claude-provider-chip-list">
            {PI_BUILTIN_PROVIDERS.map((id) => (
              <button
                key={id}
                type="button"
                className={`claude-provider-chip ${state.builtinProvider === id ? "active" : ""}`}
                onClick={() => onChange({ builtinProvider: id })}
              >
                {id}
              </button>
            ))}
          </div>
          <input
            className="form-input pi-gateway-gap"
            value={state.builtinProvider}
            autoComplete="off"
            placeholder="anthropic"
            onChange={(event) => onChange({ builtinProvider: event.target.value })}
          />
        </div>
        {nameField}
        <div className="form-group">
          <label htmlFor="pi-builtin-model">
            {t("pi.import.defaultModel", "默认模型（可选）")}
          </label>
          <input
            id="pi-builtin-model"
            className="form-input"
            value={state.defaultModel}
            autoComplete="off"
            placeholder={t("pi.import.defaultModelPlaceholder", "例如 claude-sonnet-4-5")}
            onChange={(event) => onChange({ defaultModel: event.target.value })}
          />
        </div>
      </div>
    );
  }

  const candidates = fetchedModels.filter((id) => !state.models.includes(id));

  return (
    <div className="pi-gateway-fields">
      {!edit && modeSwitch}
      <div className="form-group">
        <label>{t("pi.gateway.provider", "供应商")}</label>
        <div className="claude-provider-chip-list">
          {PI_GATEWAY_PRESETS.map((item) => (
            <button
              key={item.id}
              type="button"
              className={`claude-provider-chip ${item.isPartner ? "sponsor" : ""} ${state.presetId === item.id ? "active" : ""}`}
              onClick={() => selectPreset(item.id)}
            >
              <span>{item.name}</span>
              {item.isPartner && <Star size={12} className="api-provider-chip-badge" />}
            </button>
          ))}
          <button
            type="button"
            className={`claude-provider-chip ${state.presetId === PI_GATEWAY_CUSTOM_ID ? "active" : ""}`}
            onClick={() => selectPreset(PI_GATEWAY_CUSTOM_ID)}
          >
            <span>{t("pi.gateway.custom", "自定义")}</span>
          </button>
        </div>
        {preset?.apiKeyUrl && (
          <button
            type="button"
            className="pi-gateway-link"
            onClick={() => void openUrl(preset.apiKeyUrl!)}
          >
            <ExternalLink size={12} />
            {t("pi.gateway.getKey", "获取 API Key")}
          </button>
        )}
      </div>

      {state.presetId && (
        <>
          <div className="form-group">
            <label>{t("pi.gateway.protocol", "接口协议")}</label>
            <div className={`pi-segmented pi-segmented-${apis.length}`}>
              {apis.map((api) => (
                <button
                  key={api}
                  type="button"
                  className={`claude-provider-endpoint-chip ${state.api === api ? "active" : ""}`}
                  onClick={() => selectApi(api)}
                >
                  {API_LABELS[api]}
                </button>
              ))}
            </div>
          </div>

          {endpoints.length > 1 && (
            <div className="form-group">
              <label>{t("pi.gateway.endpoint", "供应商端点")}</label>
              <div className="claude-provider-endpoint-list">
                {endpoints.map((url) => (
                  <button
                    key={url}
                    type="button"
                    className={`claude-provider-endpoint-chip ${state.baseUrl === url ? "active" : ""}`}
                    onClick={() => {
                      resetDiscovery();
                      onChange({ baseUrl: url });
                    }}
                  >
                    {url}
                  </button>
                ))}
              </div>
            </div>
          )}

          <div className="pi-gateway-row">
            <div className="form-group">
              <label htmlFor="pi-gw-base-url">{t("pi.gateway.baseUrl", "基础 URL")}</label>
              <input
                id="pi-gw-base-url"
                className="form-input"
                value={state.baseUrl}
                autoComplete="off"
                placeholder={
                  state.api === "anthropic-messages"
                    ? "https://example.com/anthropic"
                    : "https://example.com/v1"
                }
                onChange={(event) => {
                  resetDiscovery();
                  onChange({ baseUrl: event.target.value });
                }}
              />
            </div>
            <div className="form-group">
              <label htmlFor="pi-gw-provider-id">
                {t("pi.gateway.providerId", "pi 提供商 ID")}
              </label>
              <input
                id="pi-gw-provider-id"
                className="form-input"
                value={state.providerId}
                autoComplete="off"
                placeholder="gw-my-relay"
                disabled={!!edit}
                onChange={(event) => onChange({ providerId: event.target.value })}
              />
            </div>
          </div>

          {state.api === "anthropic-messages" && (
            <div className="form-group">
              <label>{t("pi.gateway.authScheme", "认证方式")}</label>
              <div className="pi-segmented pi-segmented-2">
                {[false, true].map((bearer) => (
                  <button
                    key={String(bearer)}
                    type="button"
                    className={`claude-provider-endpoint-chip ${state.authHeader === bearer ? "active" : ""}`}
                    onClick={() => onChange({ authHeader: bearer })}
                  >
                    {bearer ? "Bearer + x-api-key" : "x-api-key"}
                  </button>
                ))}
              </div>
            </div>
          )}

          <div className="form-group">
            <div className="pi-gateway-models-head">
              <label>{t("pi.gateway.models", "模型")}</label>
              <button
                type="button"
                className="btn btn-secondary btn-sm"
                disabled={fetching || !canFetch || !state.baseUrl.trim()}
                title={
                  canFetch
                    ? undefined
                    : t("pi.gateway.fetchNeedsKey", "先在下方填写 API Key")
                }
                onClick={() => void fetchModels()}
              >
                <RefreshCw size={12} className={fetching ? "spin" : undefined} />
                {t("pi.gateway.fetchModels", "获取模型列表")}
              </button>
            </div>
            <div className="pi-gateway-model-list">
              {state.models.length === 0 && (
                <span className="pi-defaults-hint">
                  {t("pi.gateway.modelsEmpty", "至少添加一个模型")}
                </span>
              )}
              {state.models.map((id) => (
                <span
                  key={id}
                  className={`pi-gateway-model ${state.defaultModel === id ? "is-default" : ""}`}
                >
                  <button
                    type="button"
                    className="pi-gateway-model-name"
                    title={t("pi.gateway.setDefault", "设为默认模型")}
                    onClick={() => onChange({ defaultModel: id })}
                  >
                    {state.defaultModel === id && <Star size={11} />}
                    {id}
                  </button>
                  <button
                    type="button"
                    className="pi-gateway-model-remove"
                    aria-label={t("common.delete", "删除")}
                    onClick={() => removeModel(id)}
                  >
                    ×
                  </button>
                </span>
              ))}
            </div>
            {candidates.length > 0 && (
              <div className="pi-gateway-candidates">
                <div className="pi-gateway-models-head">
                  <span className="pi-defaults-hint">
                    {t("pi.gateway.fetched", "接口返回 {{count}} 个模型，点击添加", {
                      count: candidates.length,
                    })}
                  </span>
                  <button
                    type="button"
                    className="btn btn-secondary btn-sm"
                    onClick={() => addModels(candidates)}
                  >
                    {t("pi.gateway.addAll", "全部添加")}
                  </button>
                </div>
                <div className="claude-provider-endpoint-list">
                  {candidates.map((id) => (
                    <button
                      key={id}
                      type="button"
                      className="claude-provider-endpoint-chip"
                      onClick={() => addModels([id])}
                    >
                      + {id}
                    </button>
                  ))}
                </div>
              </div>
            )}
            {fetchError && <div className="pi-gateway-error">{fetchError}</div>}
            <div className="pi-gateway-model-add">
              <input
                className="form-input"
                value={modelDraft}
                autoComplete="off"
                placeholder={t("pi.gateway.modelPlaceholder", "手动输入模型 ID，回车添加")}
                onChange={(event) => setModelDraft(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" && modelDraft.trim()) {
                    event.preventDefault();
                    addModels([modelDraft]);
                    setModelDraft("");
                  }
                }}
              />
            </div>
          </div>
          {nameField}
        </>
      )}
    </div>
  );
}
