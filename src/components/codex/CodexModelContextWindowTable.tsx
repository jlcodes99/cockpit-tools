import { useState } from "react";
import { useTranslation } from "react-i18next";
import { SingleSelectDropdown } from "../SingleSelectDropdown";
import { resolveProviderModelVisionState } from "../../utils/codexModelProviderVision";

interface CodexModelContextWindowTableProps {
  models: string[];
  drafts: Record<string, string>;
  onChange: (model: string, value: string) => void;
  /** 逐模型上下文预设；不传或 false 则不渲染该列（统一走启动预览的实例级上下文管理）。 */
  showContextWindow?: boolean;
  /** 逐模型识图开关；不传则不渲染该列。 */
  visionStates?: Record<string, boolean>;
  visionDefault?: boolean;
  onVisionChange?: (model: string, value: boolean) => void;
  disabled?: boolean;
}

/** 与 Codex「上下文与压缩阈值」保持一致的上下文预设。 */
const CONTEXT_PRESETS = {
  preset_516k: "516000",
  preset_1m: "1000000",
} as const;

function resolveContextPreset(value: string): string {
  const normalized = value.trim();
  if (!normalized) return "official";
  if (normalized === CONTEXT_PRESETS.preset_516k) return "preset_516k";
  if (normalized === CONTEXT_PRESETS.preset_1m) return "preset_1m";
  return "custom";
}

export function CodexModelContextWindowTable({
  models,
  drafts,
  onChange,
  showContextWindow = true,
  visionStates,
  visionDefault = false,
  onVisionChange,
  disabled = false,
}: CodexModelContextWindowTableProps) {
  const { t } = useTranslation();
  // 「自定义」是显式选择：空值时无法从值本身推断，需要单独记一份，否则会立刻回落到「跟随官方」。
  const [customModels, setCustomModels] = useState<Record<string, boolean>>({});
  if (models.length === 0) return null;
  const showContext = showContextWindow;
  const showVision = typeof onVisionChange === "function";
  const visionLabel = t("codex.modelProviders.vision.allModels", "图片输入");
  const presetOptions = [
    {
      value: "official",
      label: t("codex.contextOverride.followOfficial", "跟随官方"),
    },
    { value: "preset_516k", label: "516K" },
    { value: "preset_1m", label: "1M" },
    { value: "custom", label: t("codex.api.modelCatalog.contextCustom", "自定义") },
  ];

  return (
    <div
      className={`api-model-context-window-panel ${
        showVision ? "with-vision" : ""
      } ${showContext ? "" : "no-context"}`}
    >
      <div className="api-model-context-window-head">
        <span>{t("codex.api.modelCatalog.modelColumn", "模型")}</span>
        {showContext ? (
          <span>
            {t("codex.api.modelCatalog.contextWindow", "上下文")}
            <em className="api-model-context-window-optional">
              {t("codex.api.modelCatalog.contextWindowOptional", "可选")}
            </em>
          </span>
        ) : null}
        {showVision ? <span>{visionLabel}</span> : null}
      </div>
      <div className="api-model-context-window-rows">
        {models.map((model) => {
          const value = drafts[model] ?? "";
          const preset = customModels[model] ? "custom" : resolveContextPreset(value);
          return (
            <div key={model} className="api-model-context-window-row">
              <span title={model}>{model}</span>
              {showContext ? (
                <div className="api-model-context-window-preset">
                  <SingleSelectDropdown
                    value={preset}
                    options={presetOptions}
                    onChange={(next) => {
                      if (next === "official") {
                        setCustomModels((current) => ({
                          ...current,
                          [model]: false,
                        }));
                        return onChange(model, "");
                      }
                      if (next === "preset_516k") {
                        setCustomModels((current) => ({
                          ...current,
                          [model]: false,
                        }));
                        return onChange(model, CONTEXT_PRESETS.preset_516k);
                      }
                      if (next === "preset_1m") {
                        setCustomModels((current) => ({
                          ...current,
                          [model]: false,
                        }));
                        return onChange(model, CONTEXT_PRESETS.preset_1m);
                      }
                      setCustomModels((current) => ({
                        ...current,
                        [model]: true,
                      }));
                      return onChange(model, value.trim());
                    }}
                    className="api-model-context-window-select"
                    disabled={disabled}
                    ariaLabel={`${model} ${t(
                      "codex.api.modelCatalog.contextWindow",
                      "上下文",
                    )}`}
                    menuPlacement="up"
                  />
                  {preset === "custom" ? (
                    <input
                      type="text"
                      inputMode="numeric"
                      className="form-input"
                      value={value}
                      onChange={(event) => onChange(model, event.target.value)}
                      disabled={disabled}
                      aria-label={`${model} ${t(
                        "codex.api.modelCatalog.contextWindow",
                        "上下文",
                      )}`}
                    />
                  ) : null}
                </div>
              ) : null}
              {showVision ? (
                <label className="api-model-vision-toggle">
                  <input
                    type="checkbox"
                    checked={resolveProviderModelVisionState(model, visionStates, visionDefault)}
                    onChange={(event) =>
                      onVisionChange?.(model.trim().toLowerCase(), event.target.checked)
                    }
                    disabled={disabled}
                    aria-label={`${model} ${visionLabel}`}
                  />
                  <span className="api-model-vision-switch" />
                </label>
              ) : null}
            </div>
          );
        })}
      </div>
    </div>
  );
}
