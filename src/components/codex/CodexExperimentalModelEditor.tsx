import { ChevronDown, Plus, Star, Trash2, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import type {
  CodexExperimentalModelDefinition,
  CodexReasoningEffort,
} from "../../types/codex";
import "./CodexExperimentalModelEditor.css";

export interface CodexExperimentalModelSource {
  label: string;
  kind: "subscription" | "api" | "missing";
  managed?: boolean;
}

export interface CodexAvailableChannel {
  id: string;
  namespace: string;
  providerName: string;
  models: string[];
}

interface CodexExperimentalModelEditorProps {
  models: CodexExperimentalModelDefinition[];
  defaultModelId?: string | null;
  disabled?: boolean;
  mode?: "inline" | "summary";
  availableChannels?: CodexAvailableChannel[];
  resolveModelSource?: (modelId: string) => CodexExperimentalModelSource;
  onChange: (models: CodexExperimentalModelDefinition[]) => void;
  onDefaultModelChange?: (modelId: string | null) => void;
  onValidationChange?: (error: string | null) => void;
  onModelRemoved?: (removedModelId: string) => void;
  onModelAdded?: (addedModelId: string) => void;
}

const MODEL_ID_PATTERN = /^[A-Za-z0-9._:/-]+$/;
const REASONING_EFFORT_OPTIONS: CodexReasoningEffort[] = [
  "low",
  "medium",
  "high",
  "xhigh",
];
const CONTEXT_PRESETS = {
  preset_516k: { context_window: 516000, auto_compact_token_limit: 460000 },
  preset_1m: { context_window: 1000000, auto_compact_token_limit: 900000 },
} as const;
type ContextPresetId = "default" | keyof typeof CONTEXT_PRESETS | "custom";

interface CustomContextDraft {
  index: number;
  contextWindow: string;
  autoCompactTokenLimit: string;
}

interface CustomContextDialogProps {
  draft: CustomContextDraft;
  contextWindow: number;
  autoCompactTokenLimit: number;
  error: string | null;
  onContextWindowChange: (value: string) => void;
  onAutoCompactTokenLimitChange: (value: string) => void;
  onClose: () => void;
  onSave: () => void;
}

function CustomContextDialog({
  draft,
  contextWindow,
  autoCompactTokenLimit,
  error,
  onContextWindowChange,
  onAutoCompactTokenLimitChange,
  onClose,
  onSave,
}: CustomContextDialogProps) {
  const { t } = useTranslation();
  return createPortal(
    <div className="codex-experimental-model-custom-context-overlay">
      <div
        className="codex-experimental-model-custom-context-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby="codex-experimental-model-custom-context-title"
      >
        <div className="codex-experimental-model-custom-context-modal__header">
          <h3 id="codex-experimental-model-custom-context-title">
            {t(
              "codex.experimentalModelCatalog.models.contextCustomShort",
              "自定义",
            )}
            {" · "}
            {t(
              "codex.experimentalModelCatalog.models.contextConfig",
              "上下文与压缩",
            )}
          </h3>
          <button
            type="button"
            className="codex-experimental-model-manager-modal__close"
            onClick={onClose}
            aria-label={t("common.close", "关闭")}
          >
            <X size={16} />
          </button>
        </div>
        <div className="codex-experimental-model-custom-context-modal__body">
          <label>
            <span>
              {t(
                "codex.experimentalModelCatalog.models.contextWindow",
                "上下文窗口",
              )}
            </span>
            <input
              type="number"
              min="1"
              step="1"
              value={draft.contextWindow}
              onChange={(event) => onContextWindowChange(event.target.value)}
              className={
                !Number.isInteger(contextWindow) || contextWindow <= 0
                  ? "has-error"
                  : ""
              }
              autoFocus
            />
            {(!Number.isInteger(contextWindow) || contextWindow <= 0) && (
              <small className="codex-experimental-model-editor__error">
                {t(
                  "codex.experimentalModelCatalog.models.validation.contextWindow",
                  "上下文窗口必须是大于 0 的整数。",
                )}
              </small>
            )}
          </label>
          <label>
            <span>
              {t(
                "codex.experimentalModelCatalog.models.autoCompactLimit",
                "压缩阈值",
              )}
            </span>
            <input
              type="number"
              min="1"
              step="1"
              value={draft.autoCompactTokenLimit}
              onChange={(event) =>
                onAutoCompactTokenLimitChange(event.target.value)
              }
              className={
                !Number.isInteger(autoCompactTokenLimit) ||
                autoCompactTokenLimit <= 0 ||
                autoCompactTokenLimit >= contextWindow
                  ? "has-error"
                  : ""
              }
            />
            {(!Number.isInteger(autoCompactTokenLimit) ||
              autoCompactTokenLimit <= 0) && (
              <small className="codex-experimental-model-editor__error">
                {t(
                  "codex.experimentalModelCatalog.models.validation.autoCompact",
                  "压缩阈值必须是大于 0 的整数。",
                )}
              </small>
            )}
            {Number.isInteger(autoCompactTokenLimit) &&
              autoCompactTokenLimit > 0 &&
              autoCompactTokenLimit >= contextWindow && (
                <small className="codex-experimental-model-editor__error">
                  {t(
                    "codex.experimentalModelCatalog.models.validation.autoCompactRange",
                    "压缩阈值必须小于上下文窗口。",
                  )}
                </small>
              )}
          </label>
        </div>
        <div className="codex-experimental-model-custom-context-modal__footer">
          <button type="button" className="btn btn-secondary" onClick={onClose}>
            {t("common.cancel", "取消")}
          </button>
          <button
            type="button"
            className="btn btn-primary"
            onClick={onSave}
            disabled={Boolean(error)}
          >
            {t("common.confirm", "确认")}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}

function resolveContextPreset(
  model: CodexExperimentalModelDefinition,
): ContextPresetId {
  if (
    model.context_window === undefined &&
    model.auto_compact_token_limit === undefined
  ) {
    return "default";
  }
  if (
    model.context_window === CONTEXT_PRESETS.preset_516k.context_window &&
    model.auto_compact_token_limit ===
      CONTEXT_PRESETS.preset_516k.auto_compact_token_limit
  ) {
    return "preset_516k";
  }
  if (
    model.context_window === CONTEXT_PRESETS.preset_1m.context_window &&
    model.auto_compact_token_limit ===
      CONTEXT_PRESETS.preset_1m.auto_compact_token_limit
  ) {
    return "preset_1m";
  }
  return "custom";
}

export function validateCodexExperimentalModels(
  models: CodexExperimentalModelDefinition[],
  translate: (key: string, fallback: string) => string,
): string | null {
  if (models.length === 0) {
    return translate(
      "codex.experimentalModelCatalog.models.validation.required",
      "至少保留一个模型。",
    );
  }
  const seen = new Set<string>();
  for (const model of models) {
    const modelId = model.model_id.trim();
    if (!modelId || modelId.length > 128 || !MODEL_ID_PATTERN.test(modelId)) {
      return translate(
        "codex.experimentalModelCatalog.models.validation.modelId",
        "模型 ID 只能包含字母、数字、点、横线、下划线、斜杠和冒号。",
      );
    }
    if (!model.display_name.trim() || model.display_name.trim().length > 100) {
      return translate(
        "codex.experimentalModelCatalog.models.validation.displayName",
        "请输入不超过 100 个字符的展示名。",
      );
    }
    const key = modelId.toLowerCase();
    if (seen.has(key)) {
      return translate(
        "codex.experimentalModelCatalog.models.validation.duplicate",
        "模型 ID 不能重复。",
      );
    }
    const contextWindow = model.context_window;
    const autoCompactLimit = model.auto_compact_token_limit;
    if ((contextWindow === undefined) !== (autoCompactLimit === undefined)) {
      return translate(
        "codex.experimentalModelCatalog.models.validation.contextPair",
        "上下文窗口和压缩阈值必须同时填写。",
      );
    }
    if (
      contextWindow !== undefined &&
      (!Number.isInteger(contextWindow) || contextWindow <= 0)
    ) {
      return translate(
        "codex.experimentalModelCatalog.models.validation.contextWindow",
        "上下文窗口必须是大于 0 的整数。",
      );
    }
    if (
      autoCompactLimit !== undefined &&
      (!Number.isInteger(autoCompactLimit) || autoCompactLimit <= 0)
    ) {
      return translate(
        "codex.experimentalModelCatalog.models.validation.autoCompact",
        "压缩阈值必须是大于 0 的整数。",
      );
    }
    if (
      contextWindow !== undefined &&
      autoCompactLimit !== undefined &&
      autoCompactLimit >= contextWindow
    ) {
      return translate(
        "codex.experimentalModelCatalog.models.validation.autoCompactRange",
        "压缩阈值必须小于上下文窗口。",
      );
    }
    seen.add(key);
  }
  return null;
}

function nextModelDefinition(
  models: CodexExperimentalModelDefinition[],
): CodexExperimentalModelDefinition {
  const existing = new Set(
    models.map((model) => model.model_id.trim().toLowerCase()),
  );
  let suffix = 1;
  while (existing.has(`custom-model${suffix}`)) suffix += 1;
  return {
    model_id: `custom-model${suffix}`,
    display_name: `Custom Model ${suffix}`,
  };
}

export function CodexExperimentalModelEditor({
  models,
  defaultModelId = null,
  disabled = false,
  mode = "summary",
  availableChannels,
  resolveModelSource,
  onChange,
  onDefaultModelChange,
  onValidationChange,
  onModelRemoved,
  onModelAdded,
}: CodexExperimentalModelEditorProps) {
  const { t } = useTranslation();
  const [managerOpen, setManagerOpen] = useState(false);
  const [customContextDraft, setCustomContextDraft] =
    useState<CustomContextDraft | null>(null);
  const [openReasoningIndex, setOpenReasoningIndex] = useState<number | null>(
    null,
  );
  const [openContextIndex, setOpenContextIndex] = useState<number | null>(null);
  const [addMenuOpen, setAddMenuOpen] = useState(false);
  const [channelSearchQuery, setChannelSearchQuery] = useState("");
  const addMenuRef = useRef<HTMLDivElement | null>(null);
  const reasoningPickerRef = useRef<HTMLDivElement | null>(null);
  const contextPickerRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!addMenuOpen) return;
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target as Node;
      if (addMenuRef.current && !addMenuRef.current.contains(target)) {
        setAddMenuOpen(false);
      }
    };
    document.addEventListener("pointerdown", handlePointerDown);
    return () => document.removeEventListener("pointerdown", handlePointerDown);
  }, [addMenuOpen]);

  useEffect(() => {
    if (openReasoningIndex === null && openContextIndex === null) return;
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target as Node;
      if (
        !reasoningPickerRef.current?.contains(target) &&
        !contextPickerRef.current?.contains(target)
      ) {
        setOpenReasoningIndex(null);
        setOpenContextIndex(null);
      }
    };
    document.addEventListener("pointerdown", handlePointerDown);
    return () => document.removeEventListener("pointerdown", handlePointerDown);
  }, [openContextIndex, openReasoningIndex]);

  const existingModelIds = useMemo(
    () => new Set(models.map((m) => m.model_id.trim().toLowerCase())),
    [models],
  );

  const handleAddBlankModel = () => {
    const newModel = nextModelDefinition(models);
    onChange([...models, newModel]);
    onModelAdded?.(newModel.model_id);
    setAddMenuOpen(false);
  };

  const handleToggleChannelModel = (namespace: string, upstreamModel: string) => {
    const modelId = `${namespace}/${upstreamModel.trim()}`;
    const isAdded = existingModelIds.has(modelId.toLowerCase());

    if (isAdded) {
      if (models.length <= 1) return;
      onChange(
        models.filter(
          (m) => m.model_id.trim().toLowerCase() !== modelId.toLowerCase(),
        ),
      );
      if (defaultModelId === modelId) {
        onDefaultModelChange?.(null);
      }
      onModelRemoved?.(modelId);
    } else {
      const displayName = `${namespace} / ${upstreamModel.trim()}`;
      onChange([...models, { model_id: modelId, display_name: displayName }]);
      onModelAdded?.(modelId);
    }
  };

  const handleAddAllChannelModels = (namespace: string, channelModels: string[]) => {
    const toAdd: CodexExperimentalModelDefinition[] = [];
    const seen = new Set(existingModelIds);
    for (const upstreamModel of channelModels) {
      const modelId = `${namespace}/${upstreamModel.trim()}`;
      if (seen.has(modelId.toLowerCase())) continue;
      seen.add(modelId.toLowerCase());
      toAdd.push({
        model_id: modelId,
        display_name: `${namespace} / ${upstreamModel.trim()}`,
      });
      onModelAdded?.(modelId);
    }
    if (toAdd.length > 0) {
      onChange([...models, ...toAdd]);
    }
  };

  const handleRemoveAllChannelModels = (namespace: string, channelModels: string[]) => {
    const targetIds = new Set(
      channelModels.map((m) => `${namespace}/${m.trim()}`.toLowerCase()),
    );
    const remaining = models.filter(
      (m) => !targetIds.has(m.model_id.trim().toLowerCase()),
    );
    if (remaining.length === 0) return;

    for (const upstream of channelModels) {
      const fullId = `${namespace}/${upstream.trim()}`;
      if (existingModelIds.has(fullId.toLowerCase())) {
        onModelRemoved?.(fullId);
      }
    }
    onChange(remaining);
  };

  const rowErrors = useMemo(() => {
    const counts = new Map<string, number>();
    models.forEach((model) => {
      const key = model.model_id.trim().toLowerCase();
      if (key) counts.set(key, (counts.get(key) ?? 0) + 1);
    });
    return models.map((model) => {
      const modelId = model.model_id.trim();
      return {
        modelId:
          !modelId || modelId.length > 128 || !MODEL_ID_PATTERN.test(modelId)
            ? t(
                "codex.experimentalModelCatalog.models.validation.modelId",
                "模型 ID 只能包含字母、数字、点、横线、下划线、斜杠和冒号。",
              )
            : counts.get(modelId.toLowerCase())! > 1
              ? t(
                  "codex.experimentalModelCatalog.models.validation.duplicate",
                  "模型 ID 不能重复。",
                )
              : null,
        displayName:
          !model.display_name.trim() || model.display_name.trim().length > 100
            ? t(
                "codex.experimentalModelCatalog.models.validation.displayName",
                "请输入不超过 100 个字符的展示名。",
              )
            : null,
        context:
          (model.context_window === undefined) !==
          (model.auto_compact_token_limit === undefined)
            ? t(
                "codex.experimentalModelCatalog.models.validation.contextPair",
                "上下文窗口和压缩阈值必须同时填写。",
              )
            : model.context_window !== undefined &&
                (!Number.isInteger(model.context_window) ||
                  model.context_window <= 0)
              ? t(
                  "codex.experimentalModelCatalog.models.validation.contextWindow",
                  "上下文窗口必须是大于 0 的整数。",
                )
              : model.auto_compact_token_limit !== undefined &&
                  (!Number.isInteger(model.auto_compact_token_limit) ||
                    model.auto_compact_token_limit <= 0)
                ? t(
                    "codex.experimentalModelCatalog.models.validation.autoCompact",
                    "压缩阈值必须是大于 0 的整数。",
                  )
                : model.context_window !== undefined &&
                    model.auto_compact_token_limit !== undefined &&
                    model.auto_compact_token_limit >= model.context_window
                  ? t(
                      "codex.experimentalModelCatalog.models.validation.autoCompactRange",
                      "压缩阈值必须小于上下文窗口。",
                    )
                  : null,
      };
    });
  }, [models, t]);

  const validationError = validateCodexExperimentalModels(
    models,
    (key, fallback) => t(key, fallback),
  );
  useEffect(() => {
    onValidationChange?.(validationError);
  }, [onValidationChange, validationError]);

  useEffect(() => {
    if (!managerOpen) return;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      event.stopPropagation();
      if (customContextDraft) {
        setCustomContextDraft(null);
        return;
      }
      setManagerOpen(false);
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [customContextDraft, managerOpen]);

  const updateModel = (
    index: number,
    field: keyof CodexExperimentalModelDefinition,
    value: string,
  ) => {
    const previous = models[index];
    onChange(
      models.map((model, modelIndex) =>
        modelIndex === index ? { ...model, [field]: value } : model,
      ),
    );
    if (field === "model_id" && defaultModelId === previous?.model_id) {
      onDefaultModelChange?.(value.trim() || null);
    }
  };

  const reasoningLabel = (effort: CodexReasoningEffort) =>
    t(`codex.wakeup.reasoningEfforts.${effort}`, effort.toUpperCase());

  const updateReasoningEfforts = (
    index: number,
    effort: CodexReasoningEffort | "official",
  ) => {
    onChange(
      models.map((model, modelIndex) => {
        if (modelIndex !== index) return model;
        if (effort === "official") {
          return { ...model, reasoning_efforts: undefined };
        }
        const current = model.reasoning_efforts ?? [];
        if (current.includes(effort)) {
          if (current.length === 1) return model;
          return {
            ...model,
            reasoning_efforts: current.filter((item) => item !== effort),
          };
        }
        return { ...model, reasoning_efforts: [...current, effort] };
      }),
    );
    if (effort === "official") setOpenReasoningIndex(null);
  };

  const applyContextPreset = (
    index: number,
    preset: Exclude<ContextPresetId, "custom">,
  ) => {
    onChange(
      models.map((model, modelIndex) => {
        if (modelIndex !== index) return model;
        if (preset === "default") {
          const {
            context_window: _context,
            auto_compact_token_limit: _compact,
            ...rest
          } = model;
          return rest;
        }
        return { ...model, ...CONTEXT_PRESETS[preset] };
      }),
    );
    setOpenContextIndex(null);
  };

  const openCustomContextEditor = (index: number) => {
    const model = models[index];
    setOpenContextIndex(null);
    setCustomContextDraft({
      index,
      contextWindow: String(
        model.context_window ?? CONTEXT_PRESETS.preset_1m.context_window,
      ),
      autoCompactTokenLimit: String(
        model.auto_compact_token_limit ??
          CONTEXT_PRESETS.preset_1m.auto_compact_token_limit,
      ),
    });
  };

  const customContextWindow = customContextDraft
    ? Number(customContextDraft.contextWindow.trim())
    : Number.NaN;
  const customAutoCompactTokenLimit = customContextDraft
    ? Number(customContextDraft.autoCompactTokenLimit.trim())
    : Number.NaN;
  const customContextError = customContextDraft
    ? !Number.isInteger(customContextWindow) || customContextWindow <= 0
      ? t(
          "codex.experimentalModelCatalog.models.validation.contextWindow",
          "上下文窗口必须是大于 0 的整数。",
        )
      : !Number.isInteger(customAutoCompactTokenLimit) ||
          customAutoCompactTokenLimit <= 0
        ? t(
            "codex.experimentalModelCatalog.models.validation.autoCompact",
            "压缩阈值必须是大于 0 的整数。",
          )
        : customAutoCompactTokenLimit >= customContextWindow
          ? t(
              "codex.experimentalModelCatalog.models.validation.autoCompactRange",
              "压缩阈值必须小于上下文窗口。",
            )
          : null
    : null;

  const saveCustomContext = () => {
    if (!customContextDraft || customContextError) return;
    onChange(
      models.map((model, index) =>
        index === customContextDraft.index
          ? {
              ...model,
              context_window: customContextWindow,
              auto_compact_token_limit: customAutoCompactTokenLimit,
            }
          : model,
      ),
    );
    setCustomContextDraft(null);
  };

  const formatTokenSize = (value?: number) => {
    if (value === undefined) {
      return t(
        "codex.experimentalModelCatalog.models.contextDefault",
        "跟随模型",
      );
    }
    if (value === 1_000_000) return "1M";
    if (value % 1_000 === 0) return `${value / 1_000}K`;
    return value.toLocaleString();
  };

  const contextLabel = (model: CodexExperimentalModelDefinition) => {
    const preset = resolveContextPreset(model);
    if (preset === "default") {
      return t(
        "codex.experimentalModelCatalog.models.contextDefault",
        "跟随模型",
      );
    }
    if (preset === "preset_516k") return "516K/460K";
    if (preset === "preset_1m") return "1M/900K";
    return `${formatTokenSize(model.context_window)}/${formatTokenSize(
      model.auto_compact_token_limit,
    )}`;
  };
  const showModelSource = Boolean(resolveModelSource);

  const editorContent = (
    <div className="codex-experimental-model-editor">
      <div className="codex-experimental-model-editor__header">
        <span>
          {t("codex.experimentalModelCatalog.models.title", "模型列表")}
        </span>
        <div className="codex-experimental-model-editor__add-wrap" ref={addMenuRef}>
          <button
            type="button"
            className="codex-experimental-model-editor__add-btn"
            onClick={() => {
              if (availableChannels && availableChannels.length > 0) {
                setAddMenuOpen((prev) => !prev);
              } else {
                handleAddBlankModel();
              }
            }}
            disabled={disabled}
            title={t("codex.experimentalModelCatalog.models.add", "添加模型")}
            aria-expanded={addMenuOpen}
          >
            <Plus size={14} />
            <span>{t("codex.experimentalModelCatalog.models.add", "添加模型")}</span>
            {availableChannels && availableChannels.length > 0 && (
              <ChevronDown size={13} className={addMenuOpen ? "is-open" : undefined} />
            )}
          </button>

          {addMenuOpen && (
            <div className="codex-experimental-model-editor__add-menu">
              <div className="codex-experimental-model-editor__add-menu-search">
                <input
                  type="text"
                  placeholder={t("common.search", "搜索渠道或模型...")}
                  value={channelSearchQuery}
                  onChange={(e) => setChannelSearchQuery(e.target.value)}
                  autoFocus
                />
              </div>

              <div className="codex-experimental-model-editor__add-menu-list">
                <button
                  type="button"
                  className="codex-experimental-model-editor__add-menu-item is-blank"
                  onClick={handleAddBlankModel}
                >
                  <Plus size={14} />
                  <span>{t("codex.experimentalModelCatalog.models.addCustomBlank", "新建自定义空白模型")}</span>
                </button>

                {availableChannels?.map((channel) => {
                  const query = channelSearchQuery.trim().toLowerCase();
                  const filtered = query
                    ? channel.models.filter(
                        (m) =>
                          m.toLowerCase().includes(query) ||
                          channel.namespace.toLowerCase().includes(query) ||
                          channel.providerName.toLowerCase().includes(query),
                      )
                    : channel.models;

                  if (filtered.length === 0 && query) return null;

                  const unaddedModels = channel.models.filter(
                    (m) => !existingModelIds.has(`${channel.namespace}/${m}`.toLowerCase()),
                  );
                  const allAdded = unaddedModels.length === 0;

                  return (
                    <div className="codex-experimental-model-editor__channel-group" key={channel.id}>
                      <div className="codex-experimental-model-editor__channel-head">
                        <span className="codex-experimental-model-editor__channel-title">
                          <span className="codex-experimental-model-editor__channel-ns">{channel.namespace}/</span>
                          <span className="codex-experimental-model-editor__channel-name">{channel.providerName}</span>
                        </span>
                        {allAdded ? (
                          <button
                            type="button"
                            className="codex-experimental-model-editor__channel-add-all is-remove"
                            onClick={() => handleRemoveAllChannelModels(channel.namespace, channel.models)}
                          >
                            {t("common.clearAll", "全部取消")}
                          </button>
                        ) : (
                          <button
                            type="button"
                            className="codex-experimental-model-editor__channel-add-all"
                            onClick={() => handleAddAllChannelModels(channel.namespace, unaddedModels)}
                          >
                            {t("codex.experimentalModelCatalog.models.addAll", "+ 全部 ({{count}})", {
                              count: unaddedModels.length,
                            })}
                          </button>
                        )}
                      </div>

                      {filtered.map((upstreamModel) => {
                        const fullId = `${channel.namespace}/${upstreamModel}`;
                        const isAdded = existingModelIds.has(fullId.toLowerCase());
                        return (
                          <button
                            type="button"
                            key={fullId}
                            className={`codex-experimental-model-editor__add-menu-item${
                              isAdded ? " is-added" : ""
                            }`}
                            onClick={() => handleToggleChannelModel(channel.namespace, upstreamModel)}
                            title={isAdded ? t("common.clickToRemove", "点击取消选择") : t("common.clickToAdd", "点击添加模型")}
                          >
                            <span className="codex-experimental-model-editor__menu-item-left">
                              <span className="codex-experimental-model-editor__menu-checkbox">
                                {isAdded ? "✓" : ""}
                              </span>
                              <span className="codex-experimental-model-editor__menu-model-id">{fullId}</span>
                            </span>
                            <span className="codex-experimental-model-editor__menu-status">
                              {isAdded ? t("common.added", "已添加") : t("common.add", "+ 添加")}
                            </span>
                          </button>
                        );
                      })}
                    </div>
                  );
                })}
              </div>
            </div>
          )}
        </div>
      </div>

      <div
        className={`codex-experimental-model-editor__table-head${
          showModelSource ? " has-source" : ""
        }`}
        aria-hidden="true"
      >
        <span>
          {t("codex.experimentalModelCatalog.models.modelId", "模型 ID")}
        </span>
        <span>
          {t("codex.experimentalModelCatalog.models.displayName", "展示名")}
        </span>
        {showModelSource && (
          <span>{t("codex.experimentalModelCatalog.models.source", "来源")}</span>
        )}
        <span>
          {t("codex.experimentalModelCatalog.models.reasoning", "推理强度")}
        </span>
        <span>
          {t(
            "codex.experimentalModelCatalog.models.contextConfig",
            "上下文与压缩",
          )}
        </span>
        <span>
          {t("codex.experimentalModelCatalog.models.operation", "操作")}
        </span>
      </div>

      <div
        className={`codex-experimental-model-editor__list${
          openReasoningIndex !== null || openContextIndex !== null
            ? " has-open-menu"
            : ""
        }`}
      >
        {models.map((model, index) => {
          const source = resolveModelSource?.(model.model_id);
          return (
          <div
            className="codex-experimental-model-editor__row"
            key={`${index}:${model.model_id}`}
          >
            <div
              className={`codex-experimental-model-editor__fields${
                showModelSource ? " has-source" : ""
              }`}
            >
              <label>
                <span>
                  {t(
                    "codex.experimentalModelCatalog.models.modelId",
                    "模型 ID",
                  )}
                </span>
                <input
                  type="text"
                  value={model.model_id}
                  onChange={(event) =>
                    updateModel(index, "model_id", event.target.value)
                  }
                  disabled={disabled || Boolean(source?.managed)}
                  className={rowErrors[index]?.modelId ? "has-error" : ""}
                  placeholder="custom-model"
                />
                {rowErrors[index]?.modelId && (
                  <small className="codex-experimental-model-editor__error">
                    {rowErrors[index].modelId}
                  </small>
                )}
              </label>
              <label>
                <span>
                  {t(
                    "codex.experimentalModelCatalog.models.displayName",
                    "展示名",
                  )}
                </span>
                <input
                  type="text"
                  value={model.display_name}
                  onChange={(event) =>
                    updateModel(index, "display_name", event.target.value)
                  }
                  disabled={disabled}
                  className={rowErrors[index]?.displayName ? "has-error" : ""}
                  placeholder="Custom Model"
                />
                {rowErrors[index]?.displayName && (
                  <small className="codex-experimental-model-editor__error">
                    {rowErrors[index].displayName}
                  </small>
                )}
              </label>
              {showModelSource && (
                <div className="codex-experimental-model-editor__source">
                  <span className="codex-experimental-model-editor__field-label">
                    {t("codex.experimentalModelCatalog.models.source", "来源")}
                  </span>
                  {source ? (
                    <span
                      className="codex-experimental-model-editor__source-badge"
                      data-kind={source.kind}
                      title={source.label}
                    >
                      {source.label}
                    </span>
                  ) : (
                    <span className="codex-experimental-model-editor__source-badge" data-kind="subscription">
                      {t("instances.form.modelRouting.subscriptionSource", "订阅")}
                    </span>
                  )}
                </div>
              )}
              <div
                className="codex-experimental-model-editor__reasoning"
                ref={
                  openReasoningIndex === index ? reasoningPickerRef : undefined
                }
              >
                <span className="codex-experimental-model-editor__field-label">
                  {t(
                    "codex.experimentalModelCatalog.models.reasoning",
                    "推理强度",
                  )}
                </span>
                <div className="codex-experimental-model-editor__reasoning-picker">
                  <button
                    type="button"
                    className="codex-experimental-model-editor__reasoning-trigger"
                    onClick={() => {
                      setOpenContextIndex(null);
                      setOpenReasoningIndex((current) =>
                        current === index ? null : index,
                      );
                    }}
                    disabled={disabled}
                    aria-expanded={openReasoningIndex === index}
                    title={
                      model.reasoning_efforts?.length
                        ? model.reasoning_efforts.map(reasoningLabel).join("、")
                        : t(
                            "codex.experimentalModelCatalog.models.followOfficial",
                            "跟随官方",
                          )
                    }
                  >
                    <span>
                      {model.reasoning_efforts?.length
                        ? model.reasoning_efforts.map(reasoningLabel).join("、")
                        : t(
                            "codex.experimentalModelCatalog.models.followOfficial",
                            "跟随官方",
                          )}
                    </span>
                    <ChevronDown size={14} />
                  </button>
                  {openReasoningIndex === index && (
                    <div className="codex-experimental-model-editor__reasoning-menu">
                      <button
                        type="button"
                        className={`codex-experimental-model-editor__reasoning-option${
                          !model.reasoning_efforts?.length
                            ? " is-selected"
                            : " is-muted"
                        }`}
                        onClick={() =>
                          updateReasoningEfforts(index, "official")
                        }
                      >
                        <span
                          className="codex-experimental-model-editor__check"
                          aria-hidden="true"
                        >
                          {!model.reasoning_efforts?.length ? "✓" : ""}
                        </span>
                        {t(
                          "codex.experimentalModelCatalog.models.followOfficial",
                          "跟随官方",
                        )}
                      </button>
                      {REASONING_EFFORT_OPTIONS.map((effort) => {
                        const selected =
                          model.reasoning_efforts?.includes(effort) ?? false;
                        return (
                          <button
                            key={effort}
                            type="button"
                            className={`codex-experimental-model-editor__reasoning-option${
                              selected
                                ? " is-selected"
                                : !model.reasoning_efforts?.length
                                  ? " is-muted"
                                  : ""
                            }`}
                            onClick={() =>
                              updateReasoningEfforts(index, effort)
                            }
                          >
                            <span
                              className="codex-experimental-model-editor__check"
                              aria-hidden="true"
                            >
                              {selected ? "✓" : ""}
                            </span>
                            {reasoningLabel(effort)}
                          </button>
                        );
                      })}
                    </div>
                  )}
                </div>
              </div>
              <div className="codex-experimental-model-editor__context">
                <span className="codex-experimental-model-editor__field-label">
                  {t(
                    "codex.experimentalModelCatalog.models.contextConfig",
                    "上下文与压缩",
                  )}
                </span>
                <div
                  className="codex-experimental-model-editor__context-picker"
                  ref={
                    openContextIndex === index ? contextPickerRef : undefined
                  }
                >
                  <button
                    type="button"
                    className="codex-experimental-model-editor__context-trigger"
                    onClick={() => {
                      setOpenReasoningIndex(null);
                      setOpenContextIndex((current) =>
                        current === index ? null : index,
                      );
                    }}
                    disabled={disabled}
                    aria-expanded={openContextIndex === index}
                    title={contextLabel(model)}
                  >
                    <span>{contextLabel(model)}</span>
                    <ChevronDown size={14} />
                  </button>
                  {openContextIndex === index && (
                    <div className="codex-experimental-model-editor__context-menu">
                      {(
                        [
                          [
                            "default",
                            t(
                              "codex.experimentalModelCatalog.models.contextDefaultShort",
                              "默认",
                            ),
                          ],
                          ["preset_516k", "516K/460K"],
                          ["preset_1m", "1M/900K"],
                          [
                            "custom",
                            t(
                              "codex.experimentalModelCatalog.models.contextCustomShort",
                              "自定义",
                            ),
                          ],
                        ] as Array<[ContextPresetId, string]>
                      ).map(([preset, label]) => (
                        <button
                          key={preset}
                          type="button"
                          className={`codex-experimental-model-editor__context-option${
                            resolveContextPreset(model) === preset
                              ? " is-selected"
                              : ""
                          }`}
                          onClick={() => {
                            if (preset === "custom") {
                              openCustomContextEditor(index);
                            } else {
                              applyContextPreset(index, preset);
                            }
                          }}
                        >
                          {label}
                        </button>
                      ))}
                    </div>
                  )}
                </div>
                {rowErrors[index]?.context && (
                  <small className="codex-experimental-model-editor__error">
                    {rowErrors[index].context}
                  </small>
                )}
              </div>
              <div className="codex-experimental-model-editor__operation">
                <span className="codex-experimental-model-editor__field-label">
                  {t("codex.experimentalModelCatalog.models.operation", "操作")}
                </span>
                <div className="codex-experimental-model-editor__operation-actions">
                  <button
                    type="button"
                    className="codex-experimental-model-editor__icon-btn"
                    data-default={
                      defaultModelId === model.model_id ? "true" : undefined
                    }
                    onClick={() => onDefaultModelChange?.(model.model_id)}
                    disabled={disabled || !onDefaultModelChange}
                    title={t(
                      "codex.experimentalModelCatalog.models.setDefault",
                      "设为默认模型",
                    )}
                    aria-label={t(
                      "codex.experimentalModelCatalog.models.setDefault",
                      "设为默认模型",
                    )}
                    aria-pressed={defaultModelId === model.model_id}
                  >
                    <Star
                      size={14}
                      fill={
                        defaultModelId === model.model_id
                          ? "currentColor"
                          : "none"
                      }
                    />
                  </button>
                  <button
                    type="button"
                    className="codex-experimental-model-editor__icon-btn is-danger"
                    onClick={() => {
                      const targetModelId = model.model_id;
                      onChange(
                        models.filter((_, itemIndex) => itemIndex !== index),
                      );
                      if (defaultModelId === targetModelId)
                        onDefaultModelChange?.(null);
                      onModelRemoved?.(targetModelId);
                    }}
                    disabled={disabled || models.length === 1}
                    title={t(
                      "codex.experimentalModelCatalog.models.remove",
                      "删除模型",
                    )}
                    aria-label={t(
                      "codex.experimentalModelCatalog.models.remove",
                      "删除模型",
                    )}
                  >
                    <Trash2 size={14} />
                  </button>
                </div>
              </div>
            </div>
          </div>
          );
        })}
      </div>
      <p className="codex-experimental-model-editor__hint">
        {t(
          "codex.experimentalModelCatalog.models.inheritHint",
          "官方模型保留原有能力字段；自定义模型使用通用能力模板。可见模型可直接新增或删除。",
        )}
      </p>
    </div>
  );

  const inlineCustomContextDialog =
    mode === "inline" && customContextDraft ? (
      <CustomContextDialog
        draft={customContextDraft}
        contextWindow={customContextWindow}
        autoCompactTokenLimit={customAutoCompactTokenLimit}
        error={customContextError}
        onContextWindowChange={(value) =>
          setCustomContextDraft((current) =>
            current ? { ...current, contextWindow: value } : current,
          )
        }
        onAutoCompactTokenLimitChange={(value) =>
          setCustomContextDraft((current) =>
            current ? { ...current, autoCompactTokenLimit: value } : current,
          )
        }
        onClose={() => setCustomContextDraft(null)}
        onSave={saveCustomContext}
      />
    ) : null;

  if (mode === "summary") {
    return (
      <>
        <div className="codex-experimental-model-summary">
          <div className="codex-experimental-model-summary__header">
            <span>
              {t("codex.experimentalModelCatalog.models.title", "模型列表")}
            </span>
            <button
              type="button"
              className="codex-experimental-model-summary__manage"
              onClick={() => setManagerOpen(true)}
              disabled={disabled}
            >
              {t("codex.experimentalModelCatalog.models.manage", "管理")}
            </button>
          </div>
          <div
            className={`codex-experimental-model-summary__table-head${
              showModelSource ? " has-source" : ""
            }`}
            aria-hidden="true"
          >
            <span>
              {t("codex.experimentalModelCatalog.models.modelId", "模型 ID")}
            </span>
            <span>
              {t("codex.experimentalModelCatalog.models.displayName", "展示名")}
            </span>
            {showModelSource && (
              <span>{t("codex.experimentalModelCatalog.models.source", "来源")}</span>
            )}
            <span>
              {t("codex.experimentalModelCatalog.models.reasoning", "推理强度")}
            </span>
            <span>
              {t(
                "codex.experimentalModelCatalog.models.contextConfig",
                "上下文与压缩",
              )}
            </span>
            <span>
              {t("codex.experimentalModelCatalog.models.default", "默认")}
            </span>
          </div>
          <div className="codex-experimental-model-summary__list">
            {models.map((model) => {
              const source = resolveModelSource?.(model.model_id);
              return (
              <div
                className={`codex-experimental-model-summary__row${
                  showModelSource ? " has-source" : ""
                }`}
                key={`${model.model_id}:${model.display_name}`}
              >
                <code>{model.model_id}</code>
                <span className="codex-experimental-model-summary__name">
                  {model.display_name || model.model_id}
                </span>
                {showModelSource && (
                  <span
                    className="codex-experimental-model-summary__source"
                    data-kind={source?.kind ?? "subscription"}
                    title={source?.label ?? t("instances.form.modelRouting.subscriptionSource", "订阅")}
                  >
                    {source?.label ?? t("instances.form.modelRouting.subscriptionSource", "订阅")}
                  </span>
                )}
                <span className="codex-experimental-model-summary__reasoning">
                  {model.reasoning_efforts?.length
                    ? model.reasoning_efforts.map(reasoningLabel).join("、")
                    : t(
                        "codex.experimentalModelCatalog.models.followOfficial",
                        "跟随官方",
                      )}
                </span>
                <span className="codex-experimental-model-summary__context">
                  {contextLabel(model)}
                </span>
                <span
                  className={`codex-experimental-model-summary__default${
                    defaultModelId === model.model_id ? " is-active" : ""
                  }`}
                >
                  {defaultModelId === model.model_id
                    ? t("codex.experimentalModelCatalog.models.default", "默认")
                    : "—"}
                </span>
              </div>
              );
            })}
          </div>
        </div>
        {managerOpen && (
          <div className="codex-experimental-model-manager-overlay">
            <div
              className="codex-experimental-model-manager-modal"
              role="dialog"
              aria-modal="true"
              aria-labelledby="codex-experimental-model-manager-title"
            >
              <div className="codex-experimental-model-manager-modal__header">
                <h3 id="codex-experimental-model-manager-title">
                  {t("codex.experimentalModelCatalog.models.title", "模型列表")}
                </h3>
                <button
                  type="button"
                  className="codex-experimental-model-manager-modal__close"
                  onClick={() => setManagerOpen(false)}
                  aria-label={t("common.close", "关闭")}
                >
                  <X size={16} />
                </button>
              </div>
              {editorContent}
            </div>
          </div>
        )}
        {customContextDraft && (
          <div className="codex-experimental-model-custom-context-overlay">
            <div
              className="codex-experimental-model-custom-context-modal"
              role="dialog"
              aria-modal="true"
              aria-labelledby="codex-experimental-model-custom-context-title"
            >
              <div className="codex-experimental-model-custom-context-modal__header">
                <h3 id="codex-experimental-model-custom-context-title">
                  {t(
                    "codex.experimentalModelCatalog.models.contextCustomShort",
                    "自定义",
                  )}
                  {" · "}
                  {t(
                    "codex.experimentalModelCatalog.models.contextConfig",
                    "上下文与压缩",
                  )}
                </h3>
                <button
                  type="button"
                  className="codex-experimental-model-manager-modal__close"
                  onClick={() => setCustomContextDraft(null)}
                  aria-label={t("common.close", "关闭")}
                >
                  <X size={16} />
                </button>
              </div>
              <div className="codex-experimental-model-custom-context-modal__body">
                <label>
                  <span>
                    {t(
                      "codex.experimentalModelCatalog.models.contextWindow",
                      "上下文窗口",
                    )}
                  </span>
                  <input
                    type="number"
                    min="1"
                    step="1"
                    value={customContextDraft.contextWindow}
                    onChange={(event) =>
                      setCustomContextDraft((current) =>
                        current
                          ? { ...current, contextWindow: event.target.value }
                          : current,
                      )
                    }
                    className={
                      !Number.isInteger(customContextWindow) ||
                      customContextWindow <= 0
                        ? "has-error"
                        : ""
                    }
                    autoFocus
                  />
                  {(!Number.isInteger(customContextWindow) ||
                    customContextWindow <= 0) && (
                    <small className="codex-experimental-model-editor__error">
                      {t(
                        "codex.experimentalModelCatalog.models.validation.contextWindow",
                        "上下文窗口必须是大于 0 的整数。",
                      )}
                    </small>
                  )}
                </label>
                <label>
                  <span>
                    {t(
                      "codex.experimentalModelCatalog.models.autoCompactLimit",
                      "压缩阈值",
                    )}
                  </span>
                  <input
                    type="number"
                    min="1"
                    step="1"
                    value={customContextDraft.autoCompactTokenLimit}
                    onChange={(event) =>
                      setCustomContextDraft((current) =>
                        current
                          ? {
                              ...current,
                              autoCompactTokenLimit: event.target.value,
                            }
                          : current,
                      )
                    }
                    className={
                      !Number.isInteger(customAutoCompactTokenLimit) ||
                      customAutoCompactTokenLimit <= 0 ||
                      customAutoCompactTokenLimit >= customContextWindow
                        ? "has-error"
                        : ""
                    }
                  />
                  {(!Number.isInteger(customAutoCompactTokenLimit) ||
                    customAutoCompactTokenLimit <= 0) && (
                    <small className="codex-experimental-model-editor__error">
                      {t(
                        "codex.experimentalModelCatalog.models.validation.autoCompact",
                        "压缩阈值必须是大于 0 的整数。",
                      )}
                    </small>
                  )}
                  {Number.isInteger(customAutoCompactTokenLimit) &&
                    customAutoCompactTokenLimit > 0 &&
                    customAutoCompactTokenLimit >= customContextWindow && (
                      <small className="codex-experimental-model-editor__error">
                        {t(
                          "codex.experimentalModelCatalog.models.validation.autoCompactRange",
                          "压缩阈值必须小于上下文窗口。",
                        )}
                      </small>
                    )}
                </label>
              </div>
              <div className="codex-experimental-model-custom-context-modal__footer">
                <button
                  type="button"
                  className="btn btn-secondary"
                  onClick={() => setCustomContextDraft(null)}
                >
                  {t("common.cancel", "取消")}
                </button>
                <button
                  type="button"
                  className="btn btn-primary"
                  onClick={saveCustomContext}
                  disabled={Boolean(customContextError)}
                >
                  {t("common.confirm", "确认")}
                </button>
              </div>
            </div>
          </div>
        )}
      </>
    );
  }

  return (
    <>
      {editorContent}
      {inlineCustomContextDialog}
    </>
  );
}
