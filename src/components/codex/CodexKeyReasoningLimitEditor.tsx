import type { TFunction } from "i18next";
import type { CodexApiKeyReasoningCeiling } from "../../types/codexLocalAccess";
import { SingleSelectDropdown } from "../SingleSelectDropdown";

type Ceiling = CodexApiKeyReasoningCeiling | "";

interface Props {
  value: Ceiling;
  onChange: (value: Ceiling) => void;
  disabled: boolean;
  t: TFunction;
}

export function CodexKeyReasoningLimitEditor({ value, onChange, disabled, t }: Props) {
  const label = t("codex.apiService.keys.maxReasoningEffort");
  const options = [
    { value: "unlimited", label: t("common.shared.quota.unlimited") },
    { value: "none", label: t("codex.apiService.keys.reasoningNone") },
    { value: "minimal", label: t("codex.apiService.keys.reasoningMinimal") },
    ...(["low", "medium", "high", "xhigh", "max", "ultra"] as const).map((effort) => ({
      value: effort,
      label: t(`codex.wakeup.reasoningEfforts.${effort}`),
    })),
  ];

  return (
    <label title={t("codex.apiService.keys.maxReasoningHint")}>
      <span>{label}</span>
      <SingleSelectDropdown
        value={value || "unlimited"}
        options={options}
        onChange={(next) => onChange(next === "unlimited" ? "" : next as CodexApiKeyReasoningCeiling)}
        disabled={disabled}
        ariaLabel={label}
      />
    </label>
  );
}
