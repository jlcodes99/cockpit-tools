import { useTranslation } from "react-i18next";

interface CodexModelContextWindowTableProps {
  models: string[];
  drafts: Record<string, string>;
  onChange: (model: string, value: string) => void;
  disabled?: boolean;
}

export function CodexModelContextWindowTable({
  models,
  drafts,
  onChange,
  disabled = false,
}: CodexModelContextWindowTableProps) {
  const { t } = useTranslation();
  if (models.length === 0) return null;

  return (
    <div className="api-model-context-window-panel">
      <div className="api-model-context-window-head">
        <span>
          {t("codex.api.modelCatalog.contextWindow", "上下文窗口")}
        </span>
        <span>
          {t("codex.api.modelCatalog.contextWindowPlaceholder", "留空=默认")}
        </span>
      </div>
      <div className="api-model-context-window-rows">
        {models.map((model) => (
          <label key={model} className="api-model-context-window-row">
            <span title={model}>{model}</span>
            <input
              type="text"
              inputMode="numeric"
              className="form-input"
              value={drafts[model] ?? ""}
              onChange={(event) => onChange(model, event.target.value)}
              placeholder={t(
                "codex.api.modelCatalog.contextWindowPlaceholder",
                "留空=默认",
              )}
              disabled={disabled}
              aria-label={`${model} ${t(
                "codex.api.modelCatalog.contextWindow",
                "上下文窗口",
              )}`}
            />
          </label>
        ))}
      </div>
      <p className="api-model-catalog-hint">
        {t(
          "codex.api.modelCatalog.contextWindowHint",
          "可选。官方 / DeepSeek 默认保留厂商值；第三方留空则回落到「上下文与压缩阈值」或 128000。改完需重启 Codex。",
        )}
      </p>
    </div>
  );
}
