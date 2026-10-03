import { useTranslation } from "react-i18next";
import { PI_BUILTIN_PROVIDERS } from "../../utils/piProviderPresets";

export interface PiGatewayState {
  /** pi built-in provider id. */
  builtinProvider: string;
  displayName: string;
  defaultModel: string;
}

export function createPiGatewayState(): PiGatewayState {
  return {
    builtinProvider: "",
    displayName: "",
    defaultModel: "",
  };
}

export function isPiGatewayStateReady(state: PiGatewayState): boolean {
  return !!state.builtinProvider.trim();
}

interface Props {
  state: PiGatewayState;
  onChange: (patch: Partial<PiGatewayState>) => void;
}

export function PiGatewayFields({ state, onChange }: Props) {
  const { t } = useTranslation();

  return (
    <div className="pi-gateway-fields">
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
