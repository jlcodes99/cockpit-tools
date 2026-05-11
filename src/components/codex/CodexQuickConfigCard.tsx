import { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { CircleAlert, FolderOpen, Save, X } from 'lucide-react';
import {
  getCodexConfigTomlPath,
  getCodexQuickConfig,
  openCodexConfigToml,
  saveCodexQuickConfig,
} from '../../services/codexService';
import type { CodexQuickConfig } from '../../types/codex';

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
  const [configPath, setConfigPath] = useState('~/.codex/config.toml');
  const [loadedConfig, setLoadedConfig] = useState<CodexQuickConfig | null>(null);
  const [selectedPresetId, setSelectedPresetId] = useState<QuickConfigPresetId>('default');
  const [contextWindowInput, setContextWindowInput] = useState(String(CONTEXT_WINDOW_1M));
  const [autoCompactLimitInput, setAutoCompactLimitInput] = useState(
    String(DEFAULT_AUTO_COMPACT_TOKEN_LIMIT),
  );
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [opening, setOpening] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

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
        label: t('codex.modelProviders.quickConfig.presetDefaultShort', "Default"),
        desc: t(
          'codex.modelProviders.quickConfig.presetDefaultDesc',
          "Remove both fields and use official defaults",
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
        label: t('codex.modelProviders.quickConfig.presetCustomShort', "Custom"),
        desc: t(
          'codex.modelProviders.quickConfig.presetCustomDesc',
          "Manually set context and compact values",
        ),
      },
    ],
    [t],
  );

  const isCustomPreset = selectedPresetId === 'custom';

  const handlePresetChange = useCallback((nextPreset: QuickConfigPresetId) => {
    setNotice(null);
    setError(null);
    setSelectedPresetId(nextPreset);
    if (nextPreset !== 'custom') {
      const preset = QUICK_CONFIG_PRESETS[nextPreset];
      setContextWindowInput(String(preset.modelContextWindow ?? CONTEXT_WINDOW_1M));
      setAutoCompactLimitInput(
        String(preset.autoCompactTokenLimit ?? DEFAULT_AUTO_COMPACT_TOKEN_LIMIT),
      );
    }
  }, []);

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
      "Context window must be an integer greater than 0.",
    );
  }, [isCustomPreset, parsedContextWindow, t]);

  const compactLimitError = useMemo(() => {
    if (!isCustomPreset) return null;
    if (parsedAutoCompactLimit !== null) return null;
    return t(
      'codex.modelProviders.quickConfig.validation.autoCompactInvalid',
      "Auto-compact limit must be an integer greater than 0.",
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

  const detectedPresetId = useMemo(
    () => resolvePresetId(detectedModelContextWindow, detectedAutoCompactTokenLimit),
    [detectedModelContextWindow, detectedAutoCompactTokenLimit],
  );

  const quickConfigWarning = useMemo(() => {
    if (!loadedConfig) return null;
    if ((detectedModelContextWindow == null) !== (detectedAutoCompactTokenLimit == null)) {
      return t('codex.modelProviders.quickConfig.partialDetected', {
        defaultValue:
          '检测到当前两个字段并不完整：model_context_window={{context}}，model_auto_compact_token_limit={{compact}}。保存后会按当前方案改写。',
        context: detectedModelContextWindow ?? t('codex.modelProviders.quickConfig.notSet', "Not set"),
        compact:
          detectedAutoCompactTokenLimit ??
          t('codex.modelProviders.quickConfig.notSet', "Not set"),
      });
    }
    if (detectedPresetId === 'custom' && selectedPresetId !== 'custom') {
      return t('codex.modelProviders.quickConfig.customDetected', {
        defaultValue:
          '检测到当前 config.toml 为自定义值：model_context_window={{context}}，model_auto_compact_token_limit={{compact}}。保存后会按你选择的预设改写。',
        context: detectedModelContextWindow ?? t('codex.modelProviders.quickConfig.notSet', "Not set"),
        compact:
          detectedAutoCompactTokenLimit ??
          t('codex.modelProviders.quickConfig.notSet', "Not set"),
      });
    }
    return null;
  }, [
    detectedAutoCompactTokenLimit,
    detectedModelContextWindow,
    detectedPresetId,
    loadedConfig,
    selectedPresetId,
    t,
  ]);

  const previewText = useMemo(() => {
    const lines = [
      targetConfig.modelContextWindow == null
        ? '# remove model_context_window'
        : `model_context_window = ${targetConfig.modelContextWindow}`,
      targetConfig.autoCompactTokenLimit == null
        ? '# remove model_auto_compact_token_limit'
        : `model_auto_compact_token_limit = ${targetConfig.autoCompactTokenLimit}`,
    ];
    return lines.join('\n');
  }, [targetConfig.autoCompactTokenLimit, targetConfig.modelContextWindow]);

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

  const handleSave = useCallback(async () => {
    if (saving || loading) return;
    setNotice(null);
    setError(null);
    if (validationError) {
      setError(validationError);
      return;
    }

    setSaving(true);
    try {
      const saved = await saveCodexQuickConfig(
        targetConfig.modelContextWindow ?? undefined,
        targetConfig.autoCompactTokenLimit ?? undefined,
      );
      applyLoadedConfig(saved);
      setNotice(
        t(
          'codex.modelProviders.quickConfig.saveSuccess',
          "Current Codex config saved.",
        ),
      );
    } catch (err) {
      setError(
        t('codex.modelProviders.quickConfig.saveFailed', {
          defaultValue: '保存当前 Codex 配置失败：{{error}}',
          error: String(err),
        }),
      );
    } finally {
      setSaving(false);
    }
  }, [applyLoadedConfig, loading, saving, t, targetConfig, validationError]);

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal codex-quick-config-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-header">
          <h2>{t('codex.modelProviders.quickConfig.title', "Current Codex Config")}</h2>
          <button className="modal-close" onClick={onClose} aria-label={t('common.close', "Close")}>
            <X />
          </button>
        </div>
        <div className="modal-body">
          <p className="codex-quick-config-desc">
            {t('codex.modelProviders.quickConfig.desc', "These quick settings write directly to the active ~/.codex/config.toml and do not change the model provider store.")}
          </p>

          <div className="codex-quick-config-card__path">
            <span>{t('codex.modelProviders.quickConfig.configPath', "Config File")}</span>
            <code>{configPath}</code>
          </div>

      {loading ? (
        <div className="section-desc">{t('common.loading', "Loading...")}</div>
      ) : loadedConfig ? (
        <>
          <div className="codex-quick-config-grid">
            <div className="codex-quick-config-field codex-quick-config-field--full">
              <label id="codex-quick-config-preset-label">
                {t('codex.modelProviders.quickConfig.presetLabel', "Config Preset")}
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
                    disabled={saving}
                  >
                    <span className="codex-quick-config-preset-btn__label">{option.label}</span>
                    <span className="codex-quick-config-preset-btn__desc">{option.desc}</span>
                  </button>
                ))}
              </div>
              <p>
                {t(
                  'codex.modelProviders.quickConfig.presetHint',
                  "Choose a preset (Default / 516K / 1M), or switch to Custom and fill both fields manually.",
                )}
              </p>
            </div>

            <div className="codex-quick-config-inputs-row">
              <div className="codex-quick-config-field">
                <label htmlFor="codex-context-window">
                {t(
                  'codex.modelProviders.quickConfig.contextWindow',
                  "Context Window",
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
                  setContextWindowInput(event.target.value);
                }}
                disabled={!isCustomPreset || saving}
                placeholder={String(CONTEXT_WINDOW_1M)}
              />
              <p>
                {t(
                  'codex.modelProviders.quickConfig.contextWindowHint',
                  "Writes model_context_window. Editable only in Custom mode.",
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
                  "Auto-Compact Limit",
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
                  setAutoCompactLimitInput(event.target.value);
                }}
                disabled={!isCustomPreset || saving}
                placeholder={String(DEFAULT_AUTO_COMPACT_TOKEN_LIMIT)}
              />
              <p>
                {t(
                  'codex.modelProviders.quickConfig.autoCompactLimitHint',
                  "Writes model_auto_compact_token_limit. Editable only in Custom mode.",
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
          </div>

          {quickConfigWarning && (
            <div className="codex-quick-config-warning">
              <CircleAlert size={15} />
              <span>{quickConfigWarning}</span>
            </div>
          )}

          <div className="codex-quick-config-preview">
            <div className="codex-quick-config-preview__head">
              <span>{t('codex.modelProviders.quickConfig.preview', "Write Preview")}</span>
              <span
                className={`provider-save-preview-chip ${
                  targetConfig.modelContextWindow == null &&
                  targetConfig.autoCompactTokenLimit == null
                    ? 'muted'
                    : 'primary'
                }`}
              >
                {targetConfig.modelContextWindow == null &&
                targetConfig.autoCompactTokenLimit == null
                  ? t('codex.modelProviders.quickConfig.previewRemove', "Will Remove")
                  : t('codex.modelProviders.quickConfig.previewApply', "Will Write")}
              </span>
            </div>
            <pre>{previewText}</pre>
          </div>
        </>
      ) : null}

          {(error || notice) && (
            <div className={`add-status ${error ? 'error' : 'success'}`}>
              {error ? <CircleAlert size={16} /> : <Save size={14} />}
              <span>{error || notice}</span>
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
              ? t('common.loading', "Loading...")
              : t('codex.modelProviders.quickConfig.openConfig', "Open File")}
          </button>
          <button
            className="btn btn-primary"
            onClick={() => void handleSave()}
            disabled={saving || loading || !!validationError}
            type="button"
          >
            <Save size={14} />
            {saving ? t('common.saving', "Saving...") : t('common.save', "Save")}
          </button>
        </div>
      </div>
    </div>
  );
}
