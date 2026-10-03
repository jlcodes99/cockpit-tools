import { useState } from "react";
import { Save, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { ModalErrorMessage } from "../ModalErrorMessage";
import { useEscClose } from "../../hooks/useEscClose";
import * as piService from "../../services/piService";
import { getPiAccountDisplayEmail, type PiAccount } from "../../types/pi";

const PI_THINKING_LEVELS = ["off", "minimal", "low", "medium", "high", "xhigh"];

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
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [errorScrollKey, setErrorScrollKey] = useState(0);

  useEscClose(!saving, onClose);

  const handleSave = async () => {
    setSaving(true);
    setError(null);
    try {
      await piService.updatePiAccountDefaults(account.id, {
        displayName,
        defaultProvider,
        defaultModel,
        defaultThinkingLevel: thinkingLevel,
      });
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
        className="modal"
        onClick={(event) => event.stopPropagation()}>
        <div className="modal-header">
          <h2>{t("pi.defaults.title", "编辑默认设置")}</h2>
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
            disabled={saving}
          >
            <Save size={16} />
            {saving ? t("common.loading", "加载中...") : t("common.save", "保存")}
          </button>
        </div>
      </div>
    </div>
  );
}
