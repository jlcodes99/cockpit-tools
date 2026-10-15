import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { SingleSelectDropdown } from "../SingleSelectDropdown";
import { deriveAutoCompactTokenLimitInput } from "../../utils/codexModelContext";

export type CodexContextOverridePreset =
  | "official"
  | "preset_516k"
  | "preset_1m"
  | "custom";

interface CodexContextOverrideEditorProps {
  enabled: boolean;
  contextWindow: string;
  compactLimit: string;
  disabled?: boolean;
  onChange: (value: {
    enabled: boolean;
    contextWindow: string;
    compactLimit: string;
  }) => void;
}

const CONTEXT_PRESETS = {
  preset_516k: {
    contextWindow: "516000",
    compactLimit: "464400",
  },
  preset_1m: {
    contextWindow: "1000000",
    compactLimit: "900000",
  },
} as const;

export function resolveCodexContextOverridePreset(
  enabled: boolean,
  contextWindow: string,
  compactLimit: string,
): CodexContextOverridePreset {
  if (!enabled) return "official";
  const normalizedContext = contextWindow.trim();
  const normalizedCompact = compactLimit.trim();
  if (
    normalizedContext === CONTEXT_PRESETS.preset_516k.contextWindow &&
    normalizedCompact === CONTEXT_PRESETS.preset_516k.compactLimit
  ) {
    return "preset_516k";
  }
  if (
    normalizedContext === CONTEXT_PRESETS.preset_1m.contextWindow &&
    normalizedCompact === CONTEXT_PRESETS.preset_1m.compactLimit
  ) {
    return "preset_1m";
  }
  return "custom";
}

export function CodexContextOverrideEditor({
  enabled,
  contextWindow,
  compactLimit,
  disabled = false,
  onChange,
}: CodexContextOverrideEditorProps) {
  const { t } = useTranslation();
  // Editing intent must not be inferred from numbers that may still match a preset.
  const [customEditing, setCustomEditing] = useState(false);
  useEffect(() => {
    if (!enabled) setCustomEditing(false);
  }, [enabled]);
  const preset = enabled && customEditing ? "custom" : resolveCodexContextOverridePreset(
    enabled,
    contextWindow,
    compactLimit,
  );
  const options = useMemo(
    () => [
      {
        value: "official",
        label: t("codex.contextOverride.followOfficial", "跟随官方"),
      },
      { value: "preset_516k", label: "516K / 464K" },
      { value: "preset_1m", label: "1M / 900K" },
      {
        value: "custom",
        label: t("codex.contextOverride.custom", "自定义上下文"),
      },
    ],
    [t],
  );

  const handlePresetChange = (nextPreset: string) => {
    setCustomEditing(nextPreset === "custom");
    if (nextPreset === "official") {
      onChange({ enabled: false, contextWindow, compactLimit });
      return;
    }
    if (nextPreset === "preset_516k" || nextPreset === "preset_1m") {
      onChange({ enabled: true, ...CONTEXT_PRESETS[nextPreset] });
      return;
    }
    onChange({ enabled: true, contextWindow, compactLimit });
  };

  return (
    <div className="codex-context-override-editor">
      <SingleSelectDropdown
        value={preset}
        options={options}
        onChange={handlePresetChange}
        className="codex-context-override-preset"
        menuClassName="codex-context-override-preset-menu"
        disabled={disabled}
        ariaLabel={t("codex.contextOverride.title", "上下文管理")}
      />
      {preset === "custom" && (
        <div className="codex-context-override-fields">
          <label>
            <span>
              {t("codex.experimentalModelCatalog.models.contextWindow")}
            </span>
            <input
              type="number"
              className="form-input"
              min={1}
              step={1}
              value={contextWindow}
              onChange={(event) => {
                setCustomEditing(true);
                const nextContextWindow = event.target.value;
                // 自定义模式不允许压缩阈值留空：只填上下文时按 90% 派生，
                // 用户显式填过的值（不等于旧派生值）保持不变。
                const nextCompactLimit =
                  compactLimit.trim() === "" ||
                  compactLimit ===
                    deriveAutoCompactTokenLimitInput(contextWindow)
                    ? deriveAutoCompactTokenLimitInput(nextContextWindow)
                    : compactLimit;
                onChange({
                  enabled: true,
                  contextWindow: nextContextWindow,
                  compactLimit: nextCompactLimit,
                });
              }}
              disabled={disabled}
            />
          </label>
          <label>
            <span>
              {t("codex.experimentalModelCatalog.models.autoCompactLimit")}
            </span>
            <input
              type="number"
              className="form-input"
              min={1}
              step={1}
              value={compactLimit}
              placeholder={t("codex.contextOverride.automatic", "自动")}
              onChange={(event) => {
                setCustomEditing(true);
                // 留空时回落到 90% 派生值，避免向下游透传空压缩阈值。
                const rawCompactLimit = event.target.value;
                onChange({
                  enabled: true,
                  contextWindow,
                  compactLimit:
                    rawCompactLimit.trim() === ""
                      ? deriveAutoCompactTokenLimitInput(contextWindow)
                      : rawCompactLimit,
                });
              }}
              disabled={disabled}
            />
          </label>
        </div>
      )}
    </div>
  );
}
