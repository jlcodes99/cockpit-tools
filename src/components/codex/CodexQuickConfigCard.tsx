import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ChevronLeft, CircleAlert, FolderOpen, Save, X } from 'lucide-react';
import {
  getCodexConfigTomlPath,
  getCodexQuickConfig,
  openCodexConfigToml,
  saveCodexQuickConfig,
} from '../../services/codexService';
import { useEscClose } from '../../hooks/useEscClose';
import type {
  CodexExperimentalModelDefinition,
  CodexQuickConfig,
} from '../../types/codex';
import { getCodexExperimentalModelErrorMessage } from '../../utils/codexExperimentalModel';
import { CodexExperimentalModelEditor } from './CodexExperimentalModelEditor';

const DEFAULT_AUTO_COMPACT_TOKEN_LIMIT = 900000;
const CONTEXT_WINDOW_516K = 516000;
const AUTO_COMPACT_TOKEN_LIMIT_516K = 460000;
const CONTEXT_WINDOW_1M = 1000000;
const AUTO_COMPACT_TOKEN_LIMIT_1M = 900000;

type BuiltInPresetId = 'default' | 'preset_516k' | 'preset_1m';
type QuickConfigPresetId = BuiltInPresetId | 'custom';

interface QuickConfigTarget {
  modelContextWindow: number | null;
  autoCompactTokenLimit: number | null;
}

const QUICK_CONFIG_PRESETS: Record<BuiltInPresetId, QuickConfigTarget> = {
  default: {
    modelContextWindow: null,
    autoCompactTokenLimit: null,
  },
  preset_516k: {
    modelContextWindow: CONTEXT_WINDOW_516K,
    autoCompactTokenLimit: AUTO_COMPACT_TOKEN_LIMIT_516K,
  },
  preset_1m: {
    modelContextWindow: CONTEXT_WINDOW_1M,
    autoCompactTokenLimit: AUTO_COMPACT_TOKEN_LIMIT_1M,
  },
};

function parsePositiveInteger(value: string): number | null {
  const parsed = Number.parseInt(value.trim(), 10);
  if (!Number.isFinite(parsed) || parsed <= 0) return null;
  return parsed;
}

function resolvePresetId(
  modelContextWindow: number | null,
  autoCompactTokenLimit: number | null,
): QuickConfigPresetId {
  if (modelContextWindow === null && autoCompactTokenLimit === null) {
    return 'default';
  }
  if (
    modelContextWindow === QUICK_CONFIG_PRESETS.preset_516k.modelContextWindow &&
    autoCompactTokenLimit === QUICK_CONFIG_PRESETS.preset_516k.autoCompactTokenLimit
  ) {
    return 'preset_516k';
  }
  if (
    modelContextWindow === QUICK_CONFIG_PRESETS.preset_1m.modelContextWindow &&
    autoCompactTokenLimit === QUICK_CONFIG_PRESETS.preset_1m.autoCompactTokenLimit
  ) {
    return 'preset_1m';
  }
  return 'custom';
}

export function CodexQuickConfigCard({ onClose }: { onClose?: () => void }) {
  const { t } = useTranslation();
  useEscClose(true, onClose ?? (() => {}));
  const [configPath, setConfigPath] = useState('~/.codex/config.toml');
  const [loadedConfig, setLoadedConfig] = useState<CodexQuickConfig | null>(null);
  const [selectedPresetId, setSelectedPresetId] = useState<QuickConfigPresetId>('default');
  const [contextWindowInput, setContextWindowInput] = useState(String(CONTEXT_WINDOW_1M));
  const [autoCompactLimitInput, setAutoCompactLimitInput] = useState(
    String(DEFAULT_AUTO_COMPACT_TOKEN_LIMIT),
  );
  const [customEdited, setCustomEdited] = useState(false);
  const [experimentalModelCatalogEnabled, setExperimentalModelCatalogEnabled] =
    useState(false);
  const [experimentalModels, setExperimentalModels] = useState<
    CodexExperimentalModelDefinition[]
  >([]);
  const [experimentalDefaultModelId, setExperimentalDefaultModelId] = useState<string | null>(null);
  const [experimentalModelsEdited, setExperimentalModelsEdited] = useState(false);
  const [experimentalModelsError, setExperimentalModelsError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [opening, setOpening] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const saveQueueRef = useRef<Promise<void>>(Promise.resolve());
  const saveVersionRef = useRef(0);

  const applyLoadedConfig = useCallback((config: CodexQuickConfig) => {
    const detectedModelContextWindow = config.detected_model_context_window ?? null;
    const detectedAutoCompactTokenLimit = config.detected_auto_compact_token_limit ?? null;
    const presetId = resolvePresetId(detectedModelContextWindow, detectedAutoCompactTokenLimit);

    setLoadedConfig(config);
    setSelectedPresetId(presetId);
    setContextWindowInput(
      String(detectedModelContextWindow ?? QUICK_CONFIG_PRESETS.preset_1m.modelContextWindow),
    );
    setAutoCompactLimitInput(
      String(
        detectedAutoCompactTokenLimit ?? QUICK_CONFIG_PRESETS.preset_1m.autoCompactTokenLimit,
      ),
    );
    setCustomEdited(false);
    setExperimentalModelCatalogEnabled(config.experimental_model_catalog_enabled);
    setExperimentalModels(config.experimental_model_catalog_models);
    setExperimentalDefaultModelId(
      config.experimental_model_catalog_default_model_id ?? null,
    );
    setExperimentalModelsEdited(false);
  }, []);

  const reload = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [path, config] = await Promise.all([
        getCodexConfigTomlPath(),
        getCodexQuickConfig(),
      ]);
      setConfigPath(path);
      applyLoadedConfig(config);
    } catch (err) {
      setError(
        t('codex.modelProviders.quickConfig.loadFailed', {
          defaultValue: '加载当前 Codex 配置失败：{{error}}',
          error: String(err),
        }),
      );
    } finally {
      setLoading(false);
    }
  }, [applyLoadedConfig, t]);

  useEffect(() => {
    void reload();
  }, [reload]);

  const presetOptions = useMemo(
    () => [
      {
        id: 'default' as QuickConfigPresetId,
        label: t('codex.modelProviders.quickConfig.presetDefaultShort', '默认'),
        desc: t(
          'codex.modelProviders.quickConfig.presetDefaultDesc',
          '移除两个字段，回到官方默认',
        ),
      },
      {
        id: 'preset_516k' as QuickConfigPresetId,
        label: t('codex.modelProviders.quickConfig.preset516kShort', '516K'),
        desc: t(
          'codex.modelProviders.quickConfig.preset516kDesc',
          'context=516000 / compact=460000',
        ),
      },
      {
        id: 'preset_1m' as QuickConfigPresetId,
        label: t('codex.modelProviders.quickConfig.preset1mShort', '1M'),
        desc: t(
          'codex.modelProviders.quickConfig.preset1mDesc',
          'context=1000000 / compact=900000',
        ),
      },
      {
        id: 'custom' as QuickConfigPresetId,
        label: t('codex.modelProviders.quickConfig.presetCustomShort', '自定义'),
        desc: t(
          'codex.modelProviders.quickConfig.presetCustomDesc',
          '手动填写上下文与压缩阈值',
        ),
      },
    ],
    [t],
  );

  const isCustomPreset = selectedPresetId === 'custom';

  const detectedModelContextWindow = loadedConfig?.detected_model_context_window ?? null;
  const detectedAutoCompactTokenLimit = loadedConfig?.detected_auto_compact_token_limit ?? null;

  const parsedContextWindow = useMemo(
    () => parsePositiveInteger(contextWindowInput),
    [contextWindowInput],
  );
  const parsedAutoCompactLimit = useMemo(
    () => parsePositiveInteger(autoCompactLimitInput),
    [autoCompactLimitInput],
  );

  const contextWindowError = useMemo(() => {
    if (!isCustomPreset) return null;
    if (parsedContextWindow !== null) return null;
    return t(
      'codex.modelProviders.quickConfig.validation.contextWindowInvalid',
      '上下文窗口必须是大于 0 的整数',
    );
  }, [isCustomPreset, parsedContextWindow, t]);

  const compactLimitError = useMemo(() => {
    if (!isCustomPreset) return null;
    if (parsedAutoCompactLimit !== null) return null;
    return t(
      'codex.modelProviders.quickConfig.validation.autoCompactInvalid',
      '自动压缩阈值必须是大于 0 的整数',
    );
  }, [isCustomPreset, parsedAutoCompactLimit, t]);

  const validationError = contextWindowError ?? compactLimitError;

  const targetConfig = useMemo<QuickConfigTarget>(() => {
    if (selectedPresetId === 'custom') {
      return {
        modelContextWindow: parsedContextWindow,
        autoCompactTokenLimit: parsedAutoCompactLimit,
      };
    }
    return QUICK_CONFIG_PRESETS[selectedPresetId];
  }, [selectedPresetId, parsedContextWindow, parsedAutoCompactLimit]);

  const experimentalModelUnavailableMessage = useMemo(() => {
    const reason = loadedConfig?.experimental_model_catalog_unavailable_reason;
    if (!reason) return null;
    if (reason === 'catalog_conflict') {
      return t('codex.experimentalModelCatalog.unavailable.catalogConflict', {
        defaultValue: '已有其他 model_catalog_json，禁止覆盖。',
      });
    }
    return null;
  }, [loadedConfig, t]);

  const handleOpenConfig = useCallback(async () => {
    if (opening) return;
    setOpening(true);
    setError(null);
    try {
      await openCodexConfigToml();
    } catch (err) {
      setError(
        t('codex.modelProviders.quickConfig.openFailed', {
          defaultValue: '打开 config.toml 失败：{{error}}',
          error: String(err),
        }),
      );
    } finally {
      setOpening(false);
    }
  }, [opening, t]);

  const persistConfig = useCallback(
    (
      target: QuickConfigTarget,
      experimentalEnabled: boolean,
      models: CodexExperimentalModelDefinition[],
      defaultModelId: string | null,
    ) => {
      if (loading) return;

      const saveVersion = saveVersionRef.current + 1;
      saveVersionRef.current = saveVersion;
      setNotice(null);
      setError(null);
      setSaving(true);

      const save = async () => {
        try {
          const saved = await saveCodexQuickConfig(
            target.modelContextWindow ?? undefined,
            target.autoCompactTokenLimit ?? undefined,
            experimentalEnabled,
            models,
            defaultModelId,
          );
          if (saveVersion === saveVersionRef.current) {
            applyLoadedConfig(saved);
            setNotice(
              t(
                'codex.modelProviders.quickConfig.saveSuccess',
                '当前 Codex 配置已保存',
              ),
            );
          }
        } catch (err) {
          if (saveVersion === saveVersionRef.current) {
            setError(
              getCodexExperimentalModelErrorMessage(t, err) ??
                t('codex.modelProviders.quickConfig.saveFailed', {
                  defaultValue: '保存当前 Codex 配置失败：{{error}}',
                  error: String(err),
                }),
            );
          }
        } finally {
          if (saveVersion === saveVersionRef.current) {
            setSaving(false);
          }
        }
      };

      saveQueueRef.current = saveQueueRef.current.catch(() => undefined).then(save);
    },
    [applyLoadedConfig, loading, t],
  );

  const handlePresetChange = useCallback(
    (nextPreset: QuickConfigPresetId) => {
      setNotice(null);
      setError(null);
      setSelectedPresetId(nextPreset);
      if (nextPreset !== 'custom') {
        const preset = QUICK_CONFIG_PRESETS[nextPreset];
        setContextWindowInput(String(preset.modelContextWindow ?? CONTEXT_WINDOW_1M));
        setAutoCompactLimitInput(
          String(preset.autoCompactTokenLimit ?? DEFAULT_AUTO_COMPACT_TOKEN_LIMIT),
        );
        persistConfig(
          preset,
          experimentalModelCatalogEnabled,
          experimentalModels,
          experimentalDefaultModelId,
        );
      }
    },
    [
      experimentalDefaultModelId,
      experimentalModelCatalogEnabled,
      experimentalModels,
      persistConfig,
    ],
  );

  useEffect(() => {
    if (
      loading ||
      !loadedConfig ||
      !isCustomPreset ||
      !customEdited ||
      validationError ||
      (detectedModelContextWindow === parsedContextWindow &&
        detectedAutoCompactTokenLimit === parsedAutoCompactLimit)
    ) {
      return;
    }

    const timer = window.setTimeout(() => {
      persistConfig(
        {
          modelContextWindow: parsedContextWindow,
          autoCompactTokenLimit: parsedAutoCompactLimit,
        },
        experimentalModelCatalogEnabled,
        experimentalModels,
        experimentalDefaultModelId,
      );
    }, 500);
    return () => window.clearTimeout(timer);
  }, [
    customEdited,
    detectedAutoCompactTokenLimit,
    detectedModelContextWindow,
    experimentalDefaultModelId,
    experimentalModelCatalogEnabled,
    experimentalModels,
    isCustomPreset,
    loadedConfig,
    loading,
    parsedAutoCompactLimit,
    parsedContextWindow,
    persistConfig,
    validationError,
  ]);

  useEffect(() => {
    if (
      loading ||
      !loadedConfig ||
      !experimentalModelsEdited ||
      experimentalModelsError ||
      JSON.stringify(loadedConfig.experimental_model_catalog_models) ===
        JSON.stringify(experimentalModels) &&
      (loadedConfig.experimental_model_catalog_default_model_id ?? null) ===
        experimentalDefaultModelId
    ) {
      return;
    }
    const timer = window.setTimeout(() => {
      persistConfig(
        validationError
          ? {
              modelContextWindow: detectedModelContextWindow,
              autoCompactTokenLimit: detectedAutoCompactTokenLimit,
            }
          : targetConfig,
        experimentalModelCatalogEnabled,
        experimentalModels,
        experimentalDefaultModelId,
      );
    }, 500);
    return () => window.clearTimeout(timer);
  }, [
    detectedAutoCompactTokenLimit,
    detectedModelContextWindow,
    experimentalModelCatalogEnabled,
    experimentalDefaultModelId,
    experimentalModels,
    experimentalModelsEdited,
    experimentalModelsError,
    loadedConfig,
    loading,
    persistConfig,
    targetConfig,
    validationError,
  ]);

  return (
    <div className="modal-overlay">
      <div className="modal codex-quick-config-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <button className="btn btn-secondary icon-only" onClick={onClose} title={t('common.back', '返回')} aria-label={t('common.back', '返回')}><ChevronLeft size={14} /></button>
          <h2>{t('codex.modelProviders.quickConfig.title', '当前 Codex 配置')}</h2>
          <button className="modal-close" onClick={onClose} aria-label={t('common.close', '关闭')}>
            <X />
          </button>
        </div>
        <div className="modal-body">
          <p className="codex-quick-config-desc">
            {t('codex.modelProviders.quickConfig.desc', '这里的快捷项直接写入当前生效的 ~/.codex/config.toml，不会改动模型供应商仓库。')}
          </p>

          <div className="codex-quick-config-card__path">
            <span>{t('codex.modelProviders.quickConfig.configPath', '配置文件')}</span>
            <code>{configPath}</code>
          </div>

      {loading ? (
        <div className="section-desc">{t('common.loading', '加载中...')}</div>
      ) : loadedConfig ? (
        <>
          <div className="codex-quick-config-grid">
            <div className="codex-quick-config-field codex-quick-config-field--full">
              <label id="codex-quick-config-preset-label">
                {t('codex.modelProviders.quickConfig.presetLabel', '配置预设')}
              </label>
              <div
                className="codex-quick-config-presets"
                role="radiogroup"
                aria-labelledby="codex-quick-config-preset-label"
              >
                {presetOptions.map((option) => (
                  <button
                    key={option.id}
                    type="button"
                    role="radio"
                    aria-checked={selectedPresetId === option.id}
                    className={`codex-quick-config-preset-btn ${
                      selectedPresetId === option.id ? 'active' : ''
                    }`}
                    onClick={() => handlePresetChange(option.id)}
                    disabled={loading}
                  >
                    <span className="codex-quick-config-preset-btn__label">{option.label}</span>
                    <span className="codex-quick-config-preset-btn__desc">{option.desc}</span>
                  </button>
                ))}
              </div>
              <p>
                {t(
                  'codex.modelProviders.quickConfig.presetHint',
                  '可直接选择预设（默认 / 516K / 1M），或切到自定义手动填写两个字段。',
                )}
              </p>
            </div>

            <div className="codex-quick-config-inputs-row">
              <div className="codex-quick-config-field">
                <label htmlFor="codex-context-window">
                {t(
                  'codex.modelProviders.quickConfig.contextWindow',
                  '上下文窗口',
                )}
              </label>
              <input
                id="codex-context-window"
                className="form-input"
                type="text"
                inputMode="numeric"
                value={contextWindowInput}
                onChange={(event) => {
                  setNotice(null);
                  setError(null);
                  setCustomEdited(true);
                  setContextWindowInput(event.target.value);
                }}
                disabled={!isCustomPreset || loading}
                placeholder={String(CONTEXT_WINDOW_1M)}
              />
              <p>
                {t(
                  'codex.modelProviders.quickConfig.contextWindowHint',
                  '写入 model_context_window。仅在“自定义”模式可编辑。',
                )}
              </p>
              {contextWindowError && (
                <div className="codex-quick-config-field__error">
                  <CircleAlert size={14} />
                  <span>{contextWindowError}</span>
                </div>
              )}
            </div>

            <div className="codex-quick-config-field">
              <label htmlFor="codex-auto-compact-limit">
                {t(
                  'codex.modelProviders.quickConfig.autoCompactLimit',
                  '自动压缩阈值',
                )}
              </label>
              <input
                id="codex-auto-compact-limit"
                className="form-input"
                type="text"
                inputMode="numeric"
                value={autoCompactLimitInput}
                onChange={(event) => {
                  setNotice(null);
                  setError(null);
                  setCustomEdited(true);
                  setAutoCompactLimitInput(event.target.value);
                }}
                disabled={!isCustomPreset || loading}
                placeholder={String(DEFAULT_AUTO_COMPACT_TOKEN_LIMIT)}
              />
              <p>
                {t(
                  'codex.modelProviders.quickConfig.autoCompactLimitHint',
                  '写入 model_auto_compact_token_limit。仅在“自定义”模式可编辑。',
                )}
              </p>
              {compactLimitError && (
                <div className="codex-quick-config-field__error">
                  <CircleAlert size={14} />
                  <span>{compactLimitError}</span>
                </div>
              )}
            </div>
            </div>

            <div className="codex-quick-config-field codex-quick-config-field--switch">
              <div className="codex-quick-config-field__copy">
                <label htmlFor="codex-experimental-model-catalog">
                  {t('codex.experimentalModelCatalog.title', '可见模型')}
                </label>
                <p>
                  {t(
                    'codex.experimentalModelCatalog.description',
                    '根据当前可用官方模型和自定义条目生成 Cockpit 受管目录；不会覆盖用户自定义目录。',
                  )}
                </p>
                {experimentalModelCatalogEnabled && (
                  <p>
                    {t(
                      'codex.experimentalModelCatalog.enabledHint',
                      '启用后使用当前可见模型列表，重启 Codex 生效。',
                    )}
                  </p>
                )}
                {experimentalModelUnavailableMessage && (
                  <div className="codex-quick-config-field__error">
                    <CircleAlert size={14} />
                    <span>{experimentalModelUnavailableMessage}</span>
                  </div>
                )}
              </div>
              <label className="codex-quick-config-switch">
                <input
                  id="codex-experimental-model-catalog"
                  type="checkbox"
                  checked={experimentalModelCatalogEnabled}
                  onChange={(event) => {
                    const enabled = event.target.checked;
                    setError(null);
                    setNotice(null);
                    setExperimentalModelCatalogEnabled(enabled);
                    const target =
                      (isCustomPreset && !customEdited) || validationError
                      ? {
                          modelContextWindow: detectedModelContextWindow,
                          autoCompactTokenLimit: detectedAutoCompactTokenLimit,
                        }
                      : targetConfig;
                    persistConfig(
                      target,
                      enabled,
                      experimentalModelsError
                        ? (loadedConfig?.experimental_model_catalog_models ?? [])
                        : experimentalModels,
                      experimentalDefaultModelId,
                    );
                  }}
                  disabled={
                    loading ||
                    (!experimentalModelCatalogEnabled &&
                      !loadedConfig?.experimental_model_catalog_available)
                  }
                />
                <span className="codex-quick-config-switch__slider" />
              </label>
            </div>
            {experimentalModelCatalogEnabled && (
              <CodexExperimentalModelEditor
                models={experimentalModels}
                defaultModelId={experimentalDefaultModelId}
                mode="summary"
                onChange={(models) => {
                  setExperimentalModels(models);
                  setExperimentalModelsEdited(true);
                  setError(null);
                }}
                onDefaultModelChange={(modelId) => {
                  setExperimentalDefaultModelId(modelId);
                  setExperimentalModelsEdited(true);
                  setError(null);
                }}
                onValidationChange={setExperimentalModelsError}
                disabled={loading}
              />
            )}
          </div>

        </>
      ) : null}

          {(error || saving || notice) && (
            <div className={`add-status ${error ? 'error' : notice ? 'success' : ''}`}>
              {error ? <CircleAlert size={16} /> : <Save size={14} />}
              <span>
                {error || (saving ? t('common.saving', '保存中...') : notice)}
              </span>
            </div>
          )}
        </div>

        <div className="modal-footer">
          <button
            className="btn btn-secondary"
            onClick={() => void handleOpenConfig()}
            disabled={opening || loading}
            type="button"
          >
            <FolderOpen size={14} />
            {opening
              ? t('common.loading', '加载中...')
              : t('codex.modelProviders.quickConfig.openConfig', '打开文件')}
          </button>
        </div>
      </div>
    </div>
  );
}
