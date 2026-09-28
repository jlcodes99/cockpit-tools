import { RefreshCw } from "lucide-react";
import { confirm as confirmDialog } from "@tauri-apps/plugin-dialog";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  listCodexModelCatalogSourceModels,
  type CodexModelCatalogSource,
  type CodexModelCatalogSourceInfo,
} from "../../services/codexModelCatalogSourceService";
import type { CodexExperimentalModelDefinition } from "../../types/codex";

type Props = {
  accountId?: string | null;
  requireAccountId?: boolean;
  instanceId?: string | null;
  scopeKey?: string | null;
  onReplace: (models: CodexExperimentalModelDefinition[], source: CodexModelCatalogSourceInfo) => void;
  sourceInfo?: CodexModelCatalogSourceInfo | null;
  onFetchUpstream?: () => Promise<CodexExperimentalModelDefinition[]>;
  hasExistingModels: boolean;
  disabled?: boolean;
};

export function CodexModelCatalogSourceControls({
  accountId,
  requireAccountId = false,
  instanceId,
  scopeKey,
  onReplace,
  sourceInfo,
  onFetchUpstream,
  hasExistingModels,
  disabled = false,
}: Props) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState<CodexModelCatalogSource | null>(null);
  const [error, setError] = useState<string | null>(null);
  const currentScope = `${instanceId ?? ""}:${accountId ?? ""}:${scopeKey ?? ""}`;
  const currentScopeRef = useRef(currentScope);
  currentScopeRef.current = currentScope;
  const disabledRef = useRef(disabled);
  disabledRef.current = disabled;

  const replaceFrom = async (nextSource: CodexModelCatalogSource) => {
    if (busy || disabled) return;
    const requestScope = currentScope;
    setBusy(nextSource);
    setError(null);
    try {
      if (nextSource === "codex" && requireAccountId && !accountId) {
        throw new Error(t("codex.api.oauthBinding.switchRequiresBinding", "请先绑定 OAuth 账号"));
      }
      const models = nextSource === "upstream"
        ? await onFetchUpstream?.()
        : await listCodexModelCatalogSourceModels({
            source: nextSource,
            accountId,
            instanceId,
          });
      if (!models?.length) {
        throw new Error(
          t("codex.modelManagement.sourceEmpty", "来源没有返回可用模型，已保留当前列表。"),
        );
      }
      if (currentScopeRef.current !== requestScope || disabledRef.current) return;
      if (hasExistingModels) {
        const confirmed = await confirmDialog(
          t("codex.modelManagement.sourceReplaceConfirm",
            "刷新将覆盖当前模型列表，以及逐模型上下文和压缩阈值。是否继续？"),
          {
            title: t("codex.modelManagement.sourceReplaceTitle", "覆盖模型配置？"),
            okLabel: t("common.confirm", "继续"),
            cancelLabel: t("common.cancel", "取消"),
            kind: "warning",
          },
        );
        if (!confirmed) return;
      }
      if (currentScopeRef.current !== requestScope || disabledRef.current) return;
      const modelMetadata = Object.fromEntries(models.flatMap((model) => {
        if (!model.context_window && !model.auto_compact_token_limit) return [];
        return [[model.model_id, {
          contextWindow: model.context_window,
          autoCompactTokenLimit: model.auto_compact_token_limit,
        }]];
      }));
      const definitions = models.map(({ context_window: _window, auto_compact_token_limit: _limit, ...model }) => model);
      onReplace(definitions, {
        source: nextSource,
        fetchedAt: Date.now(),
        modelMetadata,
        cacheInfo: nextSource === "codex"
          ? "已避开本机缓存；Codex 服务端缓存未知"
          : nextSource === "upstream" ? "上游缓存状态未知" : "随包预设",
      });
    } catch (fetchError) {
      setError(String(fetchError).replace(/^Error:\s*/, ""));
    } finally {
      setBusy(null);
    }
  };

  const sourceLabel = sourceInfo?.source === "codex"
    ? t("codex.modelManagement.sourceCodex", "Codex 端 model/list")
    : sourceInfo?.source === "upstream"
      ? t("codex.modelManagement.sourceUpstream", "上游 /v1/models")
      : sourceInfo?.source === "cockpit"
        ? t("codex.modelManagement.sourceCockpit", "Cockpit 预设")
        : t("codex.modelManagement.sourceUnknown", "来源未知");

  return (
    <div className="codex-model-catalog-source-controls">
      <div className="codex-model-catalog-source-controls__buttons">
        {(["codex", "upstream", "cockpit"] as const).map((item) => (
          <button
            key={item}
            type="button"
            className="btn btn-secondary btn-sm"
            onClick={() => void replaceFrom(item)}
            disabled={disabled || busy !== null || (item === "upstream" && !onFetchUpstream)}
          >
            <RefreshCw size={14} className={busy === item ? "loading-spinner" : undefined} />
            {item === "codex"
              ? t("codex.modelManagement.sourceCodexAction", "从 Codex 端刷新获取")
              : item === "upstream"
                ? t("codex.modelManagement.sourceUpstreamAction", "从上游获取")
                : t("codex.modelManagement.sourceCockpitAction", "使用 Cockpit 预设模型")}
          </button>
        ))}
      </div>
      <div className="codex-model-catalog-source-controls__feedback">
        <span className="codex-model-catalog-source-controls__status">
          {t("codex.modelManagement.sourceStatus", "当前来源：{{source}}", { source: sourceLabel })}
          {sourceInfo?.manuallyAdjusted ? " · 基于此来源手动调整" : ""}
          {sourceInfo?.fetchedAt ? ` · ${new Date(sourceInfo.fetchedAt).toLocaleString()}` : ""}
          {sourceInfo?.cacheInfo ? ` · ${sourceInfo.cacheInfo}` : ""}
        </span>
        {error && <span className="codex-model-catalog-source-controls__error" role="alert">{error}</span>}
      </div>
    </div>
  );
}
