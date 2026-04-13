import { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { openPath } from '@tauri-apps/plugin-opener';
import { CircleAlert, FolderOpen, Save } from 'lucide-react';
import {
  getCodexConfigTomlPath,
  getCodexQuickConfig,
  saveCodexQuickConfig,
} from '../../services/codexService';
import type { CodexQuickConfig } from '../../types/codex';

const DEFAULT_AUTO_COMPACT_TOKEN_LIMIT = 900000;
const CONTEXT_WINDOW_1M = 1000000;

function parsePositiveInteger(value: string): number | null {
  const parsed = Number.parseInt(value.trim(), 10);
  if (!Number.isFinite(parsed) || parsed <= 0) return null;
  return parsed;
}

export function CodexQuickConfigCard() {
  const { t } = useTranslation();
  const [configPath, setConfigPath] = useState('~/.codex/config.toml');
  const [loadedConfig, setLoadedConfig] = useState<CodexQuickConfig | null>(null);
  const [contextWindow1m, setContextWindow1m] = useState(false);
  const [autoCompactLimitInput, setAutoCompactLimitInput] = useState(
    String(DEFAULT_AUTO_COMPACT_TOKEN_LIMIT),
  );
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [opening, setOpening] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const reload = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [path, config] = await Promise.all([
        getCodexConfigTomlPath(),
        getCodexQuickConfig(),
      ]);
      setConfigPath(path);
      setLoadedConfig(config);
      setContextWindow1m(config.context_window_1m);
      setAutoCompactLimitInput(String(config.auto_compact_token_limit));
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
  }, [t]);

  useEffect(() => {
    void reload();
  }, [reload]);

  const parsedAutoCompactLimit = useMemo(
    () => parsePositiveInteger(autoCompactLimitInput),
    [autoCompactLimitInput],
  );
  const compactLimitError = useMemo(() => {
    if (!contextWindow1m) return null;
    if (parsedAutoCompactLimit !== null) return null;
    return t(
      'codex.modelProviders.quickConfig.validation.autoCompactInvalid',
      '自动压缩阈值必须是大于 0 的整数',
    );
  }, [contextWindow1m, parsedAutoCompactLimit, t]);

  const customContextDetected = useMemo(() => {
    const detected = loadedConfig?.detected_model_context_window;
    return typeof detected === 'number' && detected !== CONTEXT_WINDOW_1M;
  }, [loadedConfig?.detected_model_context_window]);

  const quickConfigWarning = useMemo(() => {
    if (!loadedConfig) return null;
    if (customContextDetected) {
      return t('codex.modelProviders.quickConfig.customDetected', {
        defaultValue:
          '检测到当前 config.toml 存在自定义 model_context_window = {{context}}。保存后会按下方快捷项改写。',
        context: loadedConfig.detected_model_context_window,
      });
    }
    if (
      contextWindow1m &&
      loadedConfig.detected_model_context_window === CONTEXT_WINDOW_1M &&
      loadedConfig.detected_auto_compact_token_limit == null
    ) {
      return t('codex.modelProviders.quickConfig.compactMissingDetected', {
        defaultValue:
          '检测到当前已启用 1M 上下文，但缺少自动压缩阈值。保存后会补写默认值 {{limit}}。',
        limit: DEFAULT_AUTO_COMPACT_TOKEN_LIMIT,
      });
    }
    return null;
  }, [contextWindow1m, customContextDetected, loadedConfig, t]);

  const isDirty = useMemo(() => {
    if (!loadedConfig) return false;
    if (contextWindow1m) {
      if (parsedAutoCompactLimit === null) return false;
      return (
        loadedConfig.detected_model_context_window !== CONTEXT_WINDOW_1M ||
        loadedConfig.detected_auto_compact_token_limit !== parsedAutoCompactLimit
      );
    }
    return (
      loadedConfig.detected_model_context_window != null ||
      loadedConfig.detected_auto_compact_token_limit != null
    );
  }, [contextWindow1m, loadedConfig, parsedAutoCompactLimit]);

  const previewText = useMemo(() => {
    if (contextWindow1m) {
      return [
        `model_context_window = ${CONTEXT_WINDOW_1M}`,
        `model_auto_compact_token_limit = ${parsedAutoCompactLimit ?? DEFAULT_AUTO_COMPACT_TOKEN_LIMIT}`,
      ].join('\n');
    }
    return [
      '# remove model_context_window',
      '# remove model_auto_compact_token_limit',
    ].join('\n');
  }, [contextWindow1m, parsedAutoCompactLimit]);

  const handleOpenConfig = useCallback(async () => {
    if (opening) return;
    setOpening(true);
    setError(null);
    try {
      await openPath(configPath);
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
  }, [configPath, opening, t]);

  const handleSave = useCallback(async () => {
    if (saving || loading) return;
    setNotice(null);
    setError(null);
    if (contextWindow1m && parsedAutoCompactLimit === null) {
      setError(
        t(
          'codex.modelProviders.quickConfig.validation.autoCompactInvalid',
          '自动压缩阈值必须是大于 0 的整数',
        ),
      );
      return;
    }

    setSaving(true);
    try {
      const saved = await saveCodexQuickConfig(
        contextWindow1m,
        contextWindow1m ? parsedAutoCompactLimit ?? DEFAULT_AUTO_COMPACT_TOKEN_LIMIT : undefined,
      );
      setLoadedConfig(saved);
      setContextWindow1m(saved.context_window_1m);
      setAutoCompactLimitInput(String(saved.auto_compact_token_limit));
      setNotice(
        t(
          'codex.modelProviders.quickConfig.saveSuccess',
          '当前 Codex 配置已保存',
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
  }, [contextWindow1m, loading, parsedAutoCompactLimit, saving, t]);

  return (
    <section className="codex-quick-config-card">
      <div className="codex-quick-config-card__header">
        <div>
          <h3>{t('codex.modelProviders.quickConfig.title', '当前 Codex 配置')}</h3>
          <p>{t('codex.modelProviders.quickConfig.desc', '这里的快捷项直接写入当前生效的 ~/.codex/config.toml，不会改动模型供应商仓库。')}</p>
        </div>
        <div className="codex-quick-config-card__actions">
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
          <button
            className="btn btn-primary"
            onClick={() => void handleSave()}
            disabled={saving || loading || !isDirty || !!compactLimitError}
            type="button"
          >
            <Save size={14} />
            {saving ? t('common.saving', '保存中...') : t('common.save', '保存')}
          </button>
        </div>
      </div>

      <div className="codex-quick-config-card__path">
        <span>{t('codex.modelProviders.quickConfig.configPath', '配置文件')}</span>
        <code>{configPath}</code>
      </div>

      {loading ? (
        <div className="section-desc">{t('common.loading', '加载中...')}</div>
      ) : loadedConfig ? (
        <>
          <div className="codex-quick-config-grid">
            <div className="codex-quick-config-field codex-quick-config-field--switch">
              <div className="codex-quick-config-field__copy">
                <label htmlFor="codex-context-window-1m">
                  {t('codex.modelProviders.quickConfig.contextWindow1m', '1M 上下文窗口')}
                </label>
                <p>
                  {t(
                    'codex.modelProviders.quickConfig.contextWindow1mHint',
                    '启用后写入 model_context_window = 1000000，并联动管理自动压缩阈值。',
                  )}
                </p>
              </div>
              <label className="codex-quick-config-switch">
                <input
                  id="codex-context-window-1m"
                  type="checkbox"
                  checked={contextWindow1m}
                  onChange={(event) => {
                    setNotice(null);
                    setError(null);
                    setContextWindow1m(event.target.checked);
                  }}
                  disabled={saving}
                />
                <span className="codex-quick-config-switch__slider" />
              </label>
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
                  setAutoCompactLimitInput(event.target.value);
                }}
                disabled={!contextWindow1m || saving}
                placeholder={String(DEFAULT_AUTO_COMPACT_TOKEN_LIMIT)}
              />
              <p>
                {t(
                  'codex.modelProviders.quickConfig.autoCompactLimitHint',
                  '写入 model_auto_compact_token_limit。关闭 1M 开关后，这个字段会一起移除。',
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

          {quickConfigWarning && (
            <div className="codex-quick-config-warning">
              <CircleAlert size={15} />
              <span>{quickConfigWarning}</span>
            </div>
          )}

          <div className="codex-quick-config-preview">
            <div className="codex-quick-config-preview__head">
              <span>{t('codex.modelProviders.quickConfig.preview', '写入预览')}</span>
              <span className={`provider-save-preview-chip ${contextWindow1m ? 'primary' : 'muted'}`}>
                {contextWindow1m
                  ? t('codex.modelProviders.quickConfig.previewApply', '将写入')
                  : t('codex.modelProviders.quickConfig.previewRemove', '将移除')}
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
    </section>
  );
}
