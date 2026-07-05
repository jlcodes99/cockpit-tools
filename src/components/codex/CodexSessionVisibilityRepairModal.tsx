import type { ReactNode } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Check, RefreshCw, Search, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { ModalErrorMessage, useModalErrorState } from "../ModalErrorMessage";
import { SingleSelectDropdown, type SingleSelectOption } from "../SingleSelectDropdown";
import { useEscClose } from "../../hooks/useEscClose";
import { useCodexInstanceStore } from "../../stores/useCodexInstanceStore";
import type {
  CodexSessionVisibilityRepairMode,
  CodexSessionVisibilityRepairInstanceList,
  CodexSessionVisibilityRepairProgress,
  CodexSessionVisibilityRepairSummary,
} from "../../types/codex";
import { formatCodexSessionVisibilityRepairMessage } from "../../utils/codexSessionVisibility";

type RepairStatus = "idle" | "previewing" | "previewed" | "running" | "success";
type RepairScope = "all" | "selected";
type InstanceRepairScope = "target" | "all";

const SESSION_VISIBILITY_REPAIR_PROGRESS_EVENT =
  "codex:session_visibility_repair_progress";

interface CodexSessionVisibilityRepairModalProps {
  open: boolean;
  description?: ReactNode;
  selectedSessionIds?: string[];
  totalSessionCount?: number;
  onClose: () => void;
  onRepaired?: (summary: CodexSessionVisibilityRepairSummary) => void | Promise<void>;
  onRunningChange?: (running: boolean) => void;
}

interface CodexSessionVisibilityRepairProgressViewProps {
  progress: CodexSessionVisibilityRepairProgress | null;
}

function createRepairRunId() {
  return `repair-${Date.now().toString(36)}-${Math.random()
    .toString(36)
    .slice(2, 8)}`;
}

function buildInitialProgress(
  runId: string,
  mode: CodexSessionVisibilityRepairMode = "quick",
): CodexSessionVisibilityRepairProgress {
  return {
    runId,
    mode,
    stage: "queued",
    percent: 8,
    current: 0,
    total: 0,
  };
}

export function buildCodexSessionVisibilityInitialProgress(
  runId: string,
  mode: CodexSessionVisibilityRepairMode = "quick",
): CodexSessionVisibilityRepairProgress {
  return buildInitialProgress(runId, mode);
}

export function createCodexSessionVisibilityRepairRunId() {
  return createRepairRunId();
}

function clampRepairProgressPercent(value?: number | null): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return 0;
  }
  return Math.max(0, Math.min(100, Math.round(value)));
}

export function CodexSessionVisibilityRepairProgressView({
  progress,
}: CodexSessionVisibilityRepairProgressViewProps) {
  const { t } = useTranslation();

  const resolveProgressMessage = useCallback(
    (value: CodexSessionVisibilityRepairProgress) => {
      const instanceName = value.instanceName ?? "";
      const values = {
        current: value.current,
        total: value.total,
        instance: instanceName,
      };
      switch (value.stage) {
        case "collect_instances":
          return t(
            "codex.sessionManager.repairModal.progress.collectInstances",
            "正在收集 Codex 实例...",
          );
        case "scan_instances":
          return t(
            "codex.sessionManager.repairModal.progress.scanInstances",
            "正在准备扫描实例...",
          );
        case "scan_instance":
          return t(
            "codex.sessionManager.repairModal.progress.scanInstance",
            "正在扫描 {{instance}}（{{current}}/{{total}}）...",
            values,
          );
        case "backup_instance":
          return t(
            "codex.sessionManager.repairModal.progress.backupInstance",
            "正在备份 {{instance}}（{{current}}/{{total}}）...",
            values,
          );
        case "write_instance":
          return t(
            "codex.sessionManager.repairModal.progress.writeInstance",
            "正在写入 {{instance}}（{{current}}/{{total}}）...",
            values,
          );
        case "write_sqlite_provider":
          return t(
            "codex.sessionManager.repairModal.progress.writeSqliteProvider",
            "正在写入 {{instance}} 的 SQLite 可见性...",
            values,
          );
        case "write_rollout_files":
          return t(
            "codex.sessionManager.repairModal.progress.writeRolloutFiles",
            "正在跳过旧会话文件写入...",
            values,
          );
        case "write_sqlite_timestamps":
          return t(
            "codex.sessionManager.repairModal.progress.writeSqliteTimestamps",
            "正在校正 {{instance}} 的 SQLite 时间...",
            values,
          );
        case "write_session_index":
          return t(
            "codex.sessionManager.repairModal.progress.writeSessionIndex",
            "正在跳过旧索引写入...",
            values,
          );
        case "rebuild_metadata":
          return t(
            "codex.sessionManager.repairModal.progress.rebuildMetadata",
            "正在刷新 {{instance}} 的官方会话状态...",
            values,
          );
        case "prune_backups":
          return t(
            "codex.sessionManager.repairModal.progress.pruneBackups",
            "正在清理旧备份...",
          );
        case "done":
          return t(
            "codex.sessionManager.repairModal.progress.done",
            "修复已完成",
          );
        default:
          return t(
            "codex.sessionManager.repairModal.progress.queued",
            "正在启动修复任务...",
          );
      }
    },
    [t],
  );

  const percent = clampRepairProgressPercent(progress?.percent);

  return (
    <div className="codex-visibility-repair-progress" role="status">
      <div className="codex-visibility-repair-progress__head">
        <strong>
          {t("codex.sessionManager.repairModal.progressTitle", "修复进度")}
        </strong>
        <span>{percent}%</span>
      </div>
      <div className="codex-visibility-repair-progress__bar">
        <span style={{ width: `${percent}%` }} />
      </div>
      <div className="codex-api-switch-notice-repair-status is-loading">
        <RefreshCw size={14} className="loading-spinner" />
        <span>
          {progress
            ? resolveProgressMessage(progress)
            : t(
                "codex.sessionManager.repairModal.progress.queued",
                "正在启动修复任务...",
              )}
        </span>
      </div>
    </div>
  );
}

export function CodexSessionVisibilityRepairModal({
  open,
  description,
  selectedSessionIds = [],
  totalSessionCount = 0,
  onClose,
  onRepaired,
  onRunningChange,
}: CodexSessionVisibilityRepairModalProps) {
  const { t } = useTranslation();
  const repairSessionVisibilityAcrossInstances = useCodexInstanceStore(
    (state) => state.repairSessionVisibilityAcrossInstances,
  );
  const listSessionVisibilityRepairInstances = useCodexInstanceStore(
    (state) => state.listSessionVisibilityRepairInstances,
  );
  const runIdRef = useRef<string | null>(null);
  const [status, setStatus] = useState<RepairStatus>("idle");
  const [selectedMode, setSelectedMode] =
    useState<CodexSessionVisibilityRepairMode>("quick");
  const [selectedInstanceScope, setSelectedInstanceScope] =
    useState<InstanceRepairScope>("target");
  const [selectedScope, setSelectedScope] = useState<RepairScope>("all");
  const [instanceList, setInstanceList] =
    useState<CodexSessionVisibilityRepairInstanceList | null>(null);
  const [selectedInstanceId, setSelectedInstanceId] = useState("");
  const [loadingInstances, setLoadingInstances] = useState(false);
  const [progress, setProgress] =
    useState<CodexSessionVisibilityRepairProgress | null>(null);
  const [previewSummary, setPreviewSummary] =
    useState<CodexSessionVisibilityRepairSummary | null>(null);
  const [result, setResult] = useState<string | null>(null);
  const {
    message: error,
    scrollKey: errorScrollKey,
    set: setError,
  } = useModalErrorState();

  const running = status === "running" || status === "previewing";
  const repairing = status === "running";
  const previewing = status === "previewing";
  const uniqueSelectedSessionIds = useMemo(
    () => Array.from(new Set(selectedSessionIds.filter(Boolean))),
    [selectedSessionIds],
  );
  const repairInstances = instanceList?.instances ?? [];
  const instanceOptions = useMemo<SingleSelectOption[]>(
    () =>
      repairInstances.map((instance) => {
        const name = instance.isDefault
          ? t("codex.sessionManager.repairModal.defaultInstance", "默认实例")
          : instance.name || instance.id;
        const runningLabel = instance.running
          ? ` · ${t("codex.sessionManager.repairModal.runningInstance", "运行中")}`
          : "";
        return {
          value: instance.id,
          label: `${name}${runningLabel}`,
        };
      }),
    [repairInstances, t],
  );
  const canUseSelectedScope = uniqueSelectedSessionIds.length > 0;
  const effectiveScope = canUseSelectedScope ? selectedScope : "all";
  const startDisabled =
    running ||
    loadingInstances ||
    !selectedInstanceId ||
    (effectiveScope === "selected" && uniqueSelectedSessionIds.length === 0);
  const hasPreview = previewSummary !== null;

  useEffect(() => {
    if (!open) {
      setStatus("idle");
      setSelectedMode("quick");
      setSelectedInstanceScope("target");
      setSelectedScope("all");
      setInstanceList(null);
      setSelectedInstanceId("");
      setLoadingInstances(false);
      setProgress(null);
      setPreviewSummary(null);
      setResult(null);
      setError(null);
      runIdRef.current = null;
      onRunningChange?.(false);
    }
  }, [onRunningChange, open, setError]);

  useEffect(() => {
    if (!open) return;
    setSelectedScope(uniqueSelectedSessionIds.length > 0 ? "selected" : "all");
  }, [open, uniqueSelectedSessionIds.length]);

  useEffect(() => {
    if (!open) return;

    let cancelled = false;
    setLoadingInstances(true);
    void listSessionVisibilityRepairInstances()
      .then((nextInstanceList) => {
        if (cancelled) return;
        setInstanceList(nextInstanceList);
        const instances = nextInstanceList.instances ?? [];
        const preferred =
          instances.find((instance) => instance.id === nextInstanceList.defaultInstanceId)?.id ||
          nextInstanceList.defaultInstanceId ||
          instances[0]?.id ||
          "";
        setSelectedInstanceId((current) =>
          current && instances.some((instance) => instance.id === current) ? current : preferred,
        );
      })
      .catch((err) => {
        if (cancelled) return;
        setError(
          t("codex.sessionManager.repairModal.instanceLoadFailed", {
            defaultValue: "读取实例失败：{{error}}",
            error: String(err).replace(/^Error:\s*/, ""),
          }),
        );
      })
      .finally(() => {
        if (!cancelled) {
          setLoadingInstances(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [listSessionVisibilityRepairInstances, open, setError, t]);

  useEffect(() => {
    if (!open) return;

    let disposed = false;
    let unlisten: UnlistenFn | null = null;
    void listen<CodexSessionVisibilityRepairProgress>(
      SESSION_VISIBILITY_REPAIR_PROGRESS_EVENT,
      (event) => {
        const payload = event.payload;
        if (!payload) return;
        if (payload.runId && payload.runId !== runIdRef.current) return;
        setProgress(payload);
      },
    ).then((nextUnlisten) => {
      if (disposed) {
        nextUnlisten();
      } else {
        unlisten = nextUnlisten;
      }
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [open]);

  useEscClose(open && !running, onClose);

  const closeModal = useCallback(() => {
    if (running) return;
    onClose();
  }, [onClose, running]);

  const clearPreview = useCallback(() => {
    if (running) return;
    setPreviewSummary(null);
    if (status === "previewed" || status === "success") {
      setStatus("idle");
      setResult(null);
      setProgress(null);
    }
  }, [running, status]);

  const buildRepairOptions = useCallback(
    (dryRun: boolean) => {
      const sessionIds =
        effectiveScope === "selected" ? uniqueSelectedSessionIds : null;
      const repairInstanceIds =
        selectedInstanceScope === "target" ? [selectedInstanceId] : null;
      return {
        mode: selectedMode,
        dryRun,
        targetInstanceId: selectedInstanceId,
        repairInstanceIds,
        sessionIds,
      };
    },
    [
      effectiveScope,
      selectedInstanceScope,
      selectedInstanceId,
      selectedMode,
      uniqueSelectedSessionIds,
    ],
  );

  const handlePreview = useCallback(async () => {
    if (running) return;
    const runId = createRepairRunId();
    runIdRef.current = runId;
    setStatus("previewing");
    setProgress(buildInitialProgress(runId, selectedMode));
    setPreviewSummary(null);
    setResult(null);
    setError(null);
    onRunningChange?.(true);
    try {
      const summary = await repairSessionVisibilityAcrossInstances(
        runId,
        buildRepairOptions(true),
      );
      setPreviewSummary(summary);
      setResult(formatCodexSessionVisibilityRepairMessage(summary, t));
      setStatus("previewed");
      setProgress((current) =>
        current
          ? {
              ...current,
              stage: "done",
              percent: 100,
            }
          : null,
      );
    } catch (err) {
      setStatus("idle");
      setPreviewSummary(null);
      setError(
        t("codex.sessionManager.repairModal.previewFailedWithError", {
          defaultValue: "预览会话修复失败：{{error}}",
          error: String(err).replace(/^Error:\s*/, ""),
        }),
      );
    } finally {
      onRunningChange?.(false);
    }
  }, [
    buildRepairOptions,
    onRunningChange,
    repairSessionVisibilityAcrossInstances,
    running,
    selectedMode,
    setError,
    t,
  ]);

  const handleRepair = useCallback(async () => {
    if (running) return;
    const runId = createRepairRunId();
    runIdRef.current = runId;
    setStatus("running");
    setProgress(buildInitialProgress(runId, selectedMode));
    setPreviewSummary(null);
    setResult(null);
    setError(null);
    onRunningChange?.(true);
    try {
      const summary = await repairSessionVisibilityAcrossInstances(
        runId,
        buildRepairOptions(false),
      );
      setResult(formatCodexSessionVisibilityRepairMessage(summary, t));
      setStatus("success");
      setProgress((current) =>
        current
          ? {
              ...current,
              stage: "done",
              percent: 100,
            }
          : null,
      );
      await onRepaired?.(summary);
    } catch (err) {
      setStatus("idle");
      setError(
        t("codex.apiSwitchNotice.repairFailedWithError", {
          defaultValue: "修复可见性失败：{{error}}",
          error: String(err).replace(/^Error:\s*/, ""),
        }),
      );
    } finally {
      onRunningChange?.(false);
    }
  }, [
    buildRepairOptions,
    onRepaired,
    onRunningChange,
    repairSessionVisibilityAcrossInstances,
    running,
    selectedMode,
    setError,
    t,
  ]);

  if (!open) return null;

  return (
    <div
      className="modal-overlay codex-local-access-hide-confirm-overlay"
    >
      <div
        className="modal codex-local-access-hide-confirm-modal codex-api-switch-notice-modal"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="modal-header">
          <h2>{t("codex.apiSwitchNotice.title", "Codex 会话不可见")}</h2>
          <button
            className="modal-close"
            onClick={closeModal}
            disabled={running}
            aria-label={t("common.close", "关闭")}
          >
            <X />
          </button>
        </div>
        <div className="modal-body">
          <ModalErrorMessage message={error} scrollKey={errorScrollKey} />
          <p className="codex-local-access-hide-confirm-desc">
            {description ??
              t(
                "codex.apiSwitchNotice.manualMessage",
                "修复可见性会校正官方 Codex state DB 中影响侧边栏显示的会话记录，适合账号与 API Key 切换后的会话恢复。",
              )}
          </p>
          <div className="codex-visibility-repair-options">
            <div className="codex-visibility-repair-scope">
              <span className="codex-visibility-repair-scope__title">
                {t("codex.sessionManager.repairModal.modeTitle", "修复方式")}
              </span>
              <div className="codex-visibility-repair-scope__grid">
                <button
                  type="button"
                  className={`codex-visibility-repair-scope-card${
                    selectedMode === "quick" ? " is-selected" : ""
                  }`}
                  onClick={() => {
                    setSelectedMode("quick");
                    clearPreview();
                  }}
                  disabled={running}
                  aria-pressed={selectedMode === "quick"}
                >
                  <strong>
                    {t("codex.sessionManager.repairModal.modeQuick", "快速修复")}
                  </strong>
                  <small>
                    {t(
                      "codex.sessionManager.repairModal.modeQuickDesc",
                      "只校正官方 state DB 和会话文件首条元数据，适合日常切号后恢复。",
                    )}
                  </small>
                </button>
                <button
                  type="button"
                  className={`codex-visibility-repair-scope-card${
                    selectedMode === "deep" ? " is-selected" : ""
                  }`}
                  onClick={() => {
                    setSelectedMode("deep");
                    clearPreview();
                  }}
                  disabled={running}
                  aria-pressed={selectedMode === "deep"}
                >
                  <strong>
                    {t("codex.sessionManager.repairModal.modeDeep", "深度修复")}
                  </strong>
                  <small>
                    {t(
                      "codex.sessionManager.repairModal.modeDeepDesc",
                      "保留当前完整元数据修复，适合快速修复后仍不可见时测试。",
                    )}
                  </small>
                </button>
              </div>
            </div>
            <div className="codex-visibility-repair-field">
              <span>
                {t("codex.sessionManager.repairModal.targetInstance", "目标实例")}
              </span>
              <SingleSelectDropdown
                value={selectedInstanceId}
                options={instanceOptions}
                onChange={(value) => {
                  setSelectedInstanceId(value);
                  setError(null);
                  clearPreview();
                }}
                disabled={running || loadingInstances || instanceOptions.length === 0}
                placeholder={
                  loadingInstances
                    ? t("codex.sessionManager.repairModal.loadingInstances", "正在读取实例...")
                    : t("codex.sessionManager.repairModal.noInstance", "未发现实例")
                }
                ariaLabel={t("codex.sessionManager.repairModal.targetInstance", "目标实例")}
                className="codex-visibility-repair-instance-select"
                menuClassName="codex-visibility-repair-instance-menu"
              />
            </div>
            <div className="codex-visibility-repair-scope">
              <span className="codex-visibility-repair-scope__title">
                {t("codex.sessionManager.repairModal.instanceScopeTitle", "实例范围")}
              </span>
              <div className="codex-visibility-repair-scope__grid">
                <button
                  type="button"
                  className={`codex-visibility-repair-scope-card${
                    selectedInstanceScope === "target" ? " is-selected" : ""
                  }`}
                  onClick={() => {
                    setSelectedInstanceScope("target");
                    clearPreview();
                  }}
                  disabled={running}
                  aria-pressed={selectedInstanceScope === "target"}
                >
                  <strong>
                    {t("codex.sessionManager.repairModal.instanceScopeTarget", "仅目标实例")}
                  </strong>
                  <small>
                    {t(
                      "codex.sessionManager.repairModal.instanceScopeTargetDesc",
                      "只修复上方选中的实例，通常更快。",
                    )}
                  </small>
                </button>
                <button
                  type="button"
                  className={`codex-visibility-repair-scope-card${
                    selectedInstanceScope === "all" ? " is-selected" : ""
                  }`}
                  onClick={() => {
                    setSelectedInstanceScope("all");
                    clearPreview();
                  }}
                  disabled={running}
                  aria-pressed={selectedInstanceScope === "all"}
                >
                  <strong>
                    {t("codex.sessionManager.repairModal.instanceScopeAll", "全部实例")}
                  </strong>
                  <small>
                    {t(
                      "codex.sessionManager.repairModal.instanceScopeAllDesc",
                      "修复所有 Codex 实例，适合多开实例一起恢复。",
                    )}
                  </small>
                </button>
              </div>
            </div>
            <div className="codex-visibility-repair-scope">
              <span className="codex-visibility-repair-scope__title">
                {t("codex.sessionManager.repairModal.scopeTitle", "会话范围")}
              </span>
              <div className="codex-visibility-repair-scope__grid">
                <button
                  type="button"
                  className={`codex-visibility-repair-scope-card${
                    effectiveScope === "all" ? " is-selected" : ""
                  }`}
                  onClick={() => {
                    setSelectedScope("all");
                    clearPreview();
                  }}
                  disabled={running}
                  aria-pressed={effectiveScope === "all"}
                >
                  <strong>
                    {t("codex.sessionManager.repairModal.scopeAll", "全部会话")}
                  </strong>
                  <small>
                    {t("codex.sessionManager.repairModal.scopeAllDesc", {
                      defaultValue: "修复当前管理列表中的 {{count}} 条会话。",
                      count: totalSessionCount,
                    })}
                  </small>
                </button>
                <button
                  type="button"
                  className={`codex-visibility-repair-scope-card${
                    effectiveScope === "selected" ? " is-selected" : ""
                  }`}
                  onClick={() => {
                    setSelectedScope("selected");
                    clearPreview();
                  }}
                  disabled={running || !canUseSelectedScope}
                  aria-pressed={effectiveScope === "selected"}
                >
                  <strong>
                    {t("codex.sessionManager.repairModal.scopeSelected", "所选会话")}
                  </strong>
                  <small>
                    {canUseSelectedScope
                      ? t("codex.sessionManager.repairModal.scopeSelectedDesc", {
                          defaultValue: "只修复已勾选的 {{count}} 条会话。",
                          count: uniqueSelectedSessionIds.length,
                        })
                      : t(
                          "codex.sessionManager.repairModal.scopeSelectedEmpty",
                          "先在列表中勾选会话。",
                        )}
                  </small>
                </button>
              </div>
            </div>
          </div>
          {running && (
            <CodexSessionVisibilityRepairProgressView
              progress={progress}
            />
          )}
          {status === "success" && result && (
            <div className="codex-api-switch-notice-repair-status is-success">
              <Check size={14} />
              <span>{result}</span>
            </div>
          )}
          {status === "previewed" && result && (
            <div className="codex-api-switch-notice-repair-status is-preview">
              <Search size={14} />
              <span>{result}</span>
            </div>
          )}
        </div>
        <div className="modal-footer codex-api-switch-notice-footer">
          <button className="btn btn-secondary" onClick={closeModal} disabled={running}>
            {t("common.close", "关闭")}
          </button>
          <button
            className="btn btn-secondary"
            onClick={() => void handlePreview()}
            disabled={startDisabled}
          >
            <Search size={14} className={previewing ? "icon-spin" : undefined} />
            {previewing
              ? t("codex.sessionManager.repairModal.previewing", "正在预览...")
              : hasPreview
                ? t("codex.sessionManager.repairModal.previewAgain", "重新预览")
                : t("codex.sessionManager.repairModal.preview", "预览变更")}
          </button>
          <button
            className="btn btn-primary"
            onClick={() => void handleRepair()}
            disabled={startDisabled}
          >
            <RefreshCw size={14} className={repairing ? "icon-spin" : undefined} />
            {repairing
              ? t("codex.sessionManager.repairModal.running", "正在修复...")
              : hasPreview
                ? t("codex.sessionManager.repairModal.confirmRepair", "确认修复")
                : t("codex.sessionManager.repairModal.start", "开始修复")}
          </button>
        </div>
      </div>
    </div>
  );
}
