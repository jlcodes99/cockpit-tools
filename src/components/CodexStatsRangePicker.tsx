import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  buildCodexStatsTimeRange,
  parseCodexStatsTimeRange,
  type CodexStatsRangeKey,
  type CodexStatsTimeRange,
} from "../utils/codexStatsRange";
import "./CodexStatsRangePicker.css";

interface CodexStatsRangePickerProps {
  value: CodexStatsRangeKey;
  range: CodexStatsTimeRange;
  onPresetChange: (
    key: Exclude<CodexStatsRangeKey, "custom">,
    range: CodexStatsTimeRange,
  ) => void;
  onCustomApply: (range: CodexStatsTimeRange) => void;
  disabled?: boolean;
  error?: string;
  compact?: boolean;
}

export function CodexStatsRangePicker({
  value,
  range,
  onPresetChange,
  onCustomApply,
  disabled = false,
  error,
  compact = false,
}: CodexStatsRangePickerProps) {
  const { t } = useTranslation();
  const [startInput, setStartInput] = useState(range.startInput);
  const [endInput, setEndInput] = useState(range.endInput);
  const [validationError, setValidationError] = useState("");

  useEffect(() => {
    setStartInput(range.startInput);
    setEndInput(range.endInput);
    setValidationError("");
  }, [range.endInput, range.startInput]);

  useEffect(() => {
    if (value === "custom") return;
    const nextMidnight = new Date();
    nextMidnight.setHours(24, 0, 0, 50);
    const timer = window.setTimeout(() => {
      onPresetChange(value, buildCodexStatsTimeRange(value));
    }, Math.max(nextMidnight.getTime() - Date.now(), 50));
    return () => window.clearTimeout(timer);
    // The preset key is the only input that should reset the midnight timer.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value]);

  const presets = [
    { key: "daily" as const, label: t("codex.localAccess.statsRange.daily", "日") },
    { key: "rolling7d" as const, label: t("codex.sessionUsage.range.7d", "近 7 天") },
    { key: "weekly" as const, label: t("codex.localAccess.statsRange.weekly", "周") },
    { key: "monthly" as const, label: t("codex.localAccess.statsRange.monthly", "月") },
  ];

  const applyCustomRange = (nextStartInput: string, nextEndInput: string) => {
    if (!nextStartInput || !nextEndInput) return;
    const next = parseCodexStatsTimeRange(nextStartInput, nextEndInput);
    if (!next) {
      setValidationError(
        t("codex.localAccess.statsRange.invalid", "结束时间不能早于开始时间"),
      );
      return;
    }
    setValidationError("");
    onCustomApply(next);
  };

  return (
    <div className={`codex-stats-range-picker${compact ? " is-compact" : ""}`}>
      <div className="codex-stats-range-tabs" role="tablist" aria-label={t("codex.localAccess.statsRange.label", "统计范围")}>
        {presets.map((preset) => (
          <button
            key={preset.key}
            type="button"
            role="tab"
            className={value === preset.key ? "active" : ""}
            aria-selected={value === preset.key}
            disabled={disabled}
            onClick={() => onPresetChange(preset.key, buildCodexStatsTimeRange(preset.key))}
          >
            {preset.label}
          </button>
        ))}
        <button
          type="button"
          role="tab"
          className={value === "custom" ? "active" : ""}
          aria-selected={value === "custom"}
          disabled={disabled}
          onClick={() => applyCustomRange(startInput, endInput)}
        >
          {t("codex.localAccess.statsRange.custom", "自定义")}
        </button>
      </div>
      <div className="codex-stats-range-fields">
        <label>
          <span>{t("codex.localAccess.statsRange.start", "开始")}</span>
          <input
            type="date"
            value={startInput}
            disabled={disabled}
            onChange={(event) => {
              const nextValue = event.target.value;
              setStartInput(nextValue);
              setValidationError("");
              applyCustomRange(nextValue, endInput);
            }}
          />
        </label>
        <span className="codex-stats-range-separator">—</span>
        <label>
          <span>{t("codex.localAccess.statsRange.end", "结束")}</span>
          <input
            type="date"
            value={endInput}
            disabled={disabled}
            onChange={(event) => {
              const nextValue = event.target.value;
              setEndInput(nextValue);
              setValidationError("");
              applyCustomRange(startInput, nextValue);
            }}
          />
        </label>
      </div>
      {(validationError || error) && (
        <div className="codex-stats-range-error" role="alert">
          {validationError || error}
        </div>
      )}
    </div>
  );
}
