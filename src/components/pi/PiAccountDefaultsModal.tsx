import { useState } from "react";
import {
  PiGatewayFields,
  createPiGatewayState,
  isPiGatewayStateReady,
  type PiGatewayState,
} from "./PiGatewayFields";
import {
  PI_GATEWAY_CUSTOM_ID,
  PI_GATEWAY_PRESETS,
  type PiGatewayApi,
} from "../../utils/piProviderPresets";
import { Save, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { ModalErrorMessage } from "../ModalErrorMessage";
import { useEscClose } from "../../hooks/useEscClose";
import * as piService from "../../services/piService";
import { getPiAccountDisplayEmail, type PiAccount } from "../../types/pi";

const PI_THINKING_LEVELS = ["off", "minimal", "low", "medium", "high", "xhigh"];

function gatewayStateFrom(account: PiAccount): PiGatewayState | null {
  const providers = account.providers ?? [];
  const gw =
    providers.find((p) => p.base_url && p.provider === account.default_provider) ??
    providers.find((p) => p.base_url);
  if (!gw?.base_url) return null;
  const api = (gw.api || "openai-completions") as PiGatewayApi;
  const base = gw.base_url.replace(/\/+$/, "");
  const preset = PI_GATEWAY_PRESETS.find((item) =>
    Object.values(item.endpoints).some((urls) =>
      urls?.some((url) => url.replace(/\/+$/, "") === base),
    ),
  );
  const models = gw.models ?? [];
  return {
    ...createPiGatewayState(),
    presetId: preset?.id ?? PI_GATEWAY_CUSTOM_ID,
    providerId: gw.provider,
    api,
    baseUrl: gw.base_url,
    authHeader: !!gw.auth_header,
    models,
    displayName: getPiAccountDisplayEmail(account),
    defaultModel:
      account.default_provider === gw.provider && account.default_model
        ? account.default_model
        : models[0] ?? "",
  };
}

interface PiAccountDefaultsModalProps {
  account: PiAccount;
  onClose: () => void;
  onSaved: () => void | Promise<void>;
}

export function PiAccountDefaultsModal({
  account,
  onClose,
  onSaved,
}: PiAccountDefaultsModalProps) {
  const { t } = useTranslation();
  const providers = (account.providers ?? []).map((item) => item.provider);
  const [displayName, setDisplayName] = useState(
    getPiAccountDisplayEmail(account),
  );
  const [defaultProvider, setDefaultProvider] = useState(
    account.default_provider ?? "",
  );
  const [defaultModel, setDefaultModel] = useState(account.default_model ?? "");
  const [thinkingLevel, setThinkingLevel] = useState(
    account.default_thinking_level ?? "",
  );
  const [gateway, setGateway] = useState<PiGatewayState | null>(() =>
    gatewayStateFrom(account),
  );
  const [gatewayKey, setGatewayKey] = useState("");
  const gatewayKeyHint =
    account.providers?.find((p) => p.provider === gateway?.providerId)?.key_hint ?? "";
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [errorScrollKey, setErrorScrollKey] = useState(0);

  useEscClose(!saving, onClose);

  const handleSave = async () => {
    setSaving(true);
    setError(null);
    try {
      if (gateway) {
        await piService.updatePiAccountGateway(account.id, {
          provider: gateway.providerId,
          name: gateway.displayName,
          baseUrl: gateway.baseUrl,
          api: gateway.api,
          authHeader: gateway.authHeader,
          apiKey: gatewayKey,
          models: gateway.models,
          displayName: gateway.displayName,
          defaultModel: gateway.defaultModel,
        });
        await piService.updatePiAccountDefaults(account.id, {
          displayName: gateway.displayName,
          defaultProvider: gateway.providerId,
          defaultModel: gateway.defaultModel || gateway.models[0] || "",
          defaultThinkingLevel: thinkingLevel,
        });
      } else {
        await piService.updatePiAccountDefaults(account.id, {
          displayName,
          defaultProvider,
          defaultModel,
          defaultThinkingLevel: thinkingLevel,
        });
      }
      await onSaved();
      onClose();
    } catch (err) {
      setError(String(err));
      setErrorScrollKey((value) => value + 1);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="modal-overlay">
      <div
        className={
          gateway
            ? "modal modal-lg ghcp-add-modal platform-account-add-modal pi-accounts-page-add-modal"
            : "modal"
}
        onClick={(event) => event.stopPropagation()}>
        <div className="modal-header">
          <h2>
            {gateway
              ? t("pi.gateway.editTitle", "编辑网关")
              : t("pi.defaults.title", "编辑默认设置")}
          </h2>
          <button
            className="modal-close"
            onClick={onClose}
            disabled={saving}
            aria-label={t("common.close", "关闭")}
          >
            <X />
          </button>
        </div>
        <div className="modal-body pi-defaults-form">
          {gateway ? (
            <>
              <PiGatewayFields
                state={gateway}
                apiKey={gatewayKey}
                edit={{ accountId: account.id }}
                onChange={(patch) =>
                  setGateway((prev) => (prev ? { ...prev, ...patch } : prev))
                }
              />
              <div className="form-group">
                <label htmlFor="pi-gw-edit-key">API Key</label>
                <input
                  id="pi-gw-edit-key"
                  className="form-input"
                  type="password"
                  value={gatewayKey}
                  autoComplete="off"
                  placeholder={t("pi.gateway.keepKey", {
                    defaultValue: "留空则保留当前 Key {{hint}}",
                    hint: gatewayKeyHint,
                  })}
                  onChange={(event) => setGatewayKey(event.target.value)}
                />
              </div>
            </>
          ) : (
          <>
          <p className="pi-defaults-hint">
            {t(
              "pi.defaults.hint",
              "切换到此账号时，会把默认提供商、模型和思考级别写入 pi 的 settings.json。留空表示不修改。",
            )}
          </p>
          <label htmlFor="pi-defaults-name">
            {t("pi.import.displayName", "账号名称（可选）")}
          </label>
          <input
            id="pi-defaults-name"
            className="form-input"
            value={displayName}
            autoComplete="off"
            onChange={(event) => setDisplayName(event.target.value)}
          />
          <label htmlFor="pi-defaults-provider">
            {t("pi.defaults.provider", "默认提供商")}
          </label>
          <select
            id="pi-defaults-provider"
            className="form-input"
            value={defaultProvider}
            onChange={(event) => setDefaultProvider(event.target.value)}
          >
            <option value="">{t("pi.defaults.unset", "不设置")}</option>
            {providers.map((provider) => (
              <option key={provider} value={provider}>
                {provider}
              </option>
            ))}
            {defaultProvider && !providers.includes(defaultProvider) && (
              <option value={defaultProvider}>{defaultProvider}</option>
            )}
          </select>
          <label htmlFor="pi-defaults-model">
            {t("pi.providers.defaultModel", "默认模型")}
          </label>
          <input
            id="pi-defaults-model"
            className="form-input"
            value={defaultModel}
            autoComplete="off"
            placeholder={t(
              "pi.import.defaultModelPlaceholder",
              "例如 claude-sonnet-4-5",
            )}
            onChange={(event) => setDefaultModel(event.target.value)}
          />
          </>
          )}
          <label htmlFor="pi-defaults-thinking">
            {t("pi.defaults.thinkingLevel", "默认思考级别")}
          </label>
          <select
            id="pi-defaults-thinking"
            className="form-input"
            value={thinkingLevel}
            onChange={(event) => setThinkingLevel(event.target.value)}
          >
            <option value="">{t("pi.defaults.unset", "不设置")}</option>
            {PI_THINKING_LEVELS.map((level) => (
              <option key={level} value={level}>
                {level}
              </option>
            ))}
          </select>
          <ModalErrorMessage message={error} scrollKey={errorScrollKey} />
        </div>
        <div className="modal-footer">
          <button
            className="btn btn-secondary"
            onClick={onClose}
            disabled={saving}
          >
            {t("common.cancel", "取消")}
          </button>
          <button
            className="btn btn-primary"
            onClick={() => void handleSave()}
            disabled={saving || (!!gateway && !isPiGatewayStateReady(gateway))}
          >
            <Save size={16} />
            {saving ? t("common.loading", "加载中...") : t("common.save", "保存")}
          </button>
        </div>
      </div>
    </div>
  );
}
