import type { TFunction } from "i18next";
import { modelsOutsideCodexApiKeyAllowlist, parseModelRuleText } from "../../utils/codexApiKeyModelRules";
import { SingleSelectDropdown } from "../SingleSelectDropdown";

interface Props {
  value: string;
  modelIds: string[];
  onChange: (value: string) => void;
  disabled: boolean;
  t: TFunction;
}

export function CodexKeyModelAllowlistEditor({ value, modelIds, onChange, disabled, t }: Props) {
  const rules = parseModelRuleText(value);
  const chooserLabel = t("codex.apiService.keys.modelsOutsideAllowlist");
  const options = [
    { value: "all", label: t("codex.apiService.keys.allowCurrentAndFutureModels") },
    ...modelsOutsideCodexApiKeyAllowlist(modelIds, rules).map((id) => ({ value: `model:${id}`, label: id })),
  ];

  return (
    <div className="codex-key-model-allowlist-editor">
      <label>
        <span>{t("codex.apiService.keys.allowedModels")}</span>
        <textarea
          value={value}
          onChange={(event) => onChange(event.target.value)}
          placeholder={t("codex.apiService.keys.allowedModelsPlaceholder")}
          disabled={disabled}
        />
      </label>
      {rules.length > 0 && (
        <SingleSelectDropdown
          value=""
          options={options}
          onChange={(selection) => onChange(selection === "all" ? "" : [...rules, selection.slice("model:".length)].join("\n"))}
          disabled={disabled}
          ariaLabel={chooserLabel}
          placeholder={chooserLabel}
        />
      )}
    </div>
  );
}
