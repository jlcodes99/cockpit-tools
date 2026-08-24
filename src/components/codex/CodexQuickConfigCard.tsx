import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ChevronLeft, CircleAlert, FolderOpen, Save, X } from 'lucide-react';
import {
  getCodexConfigTomlPath,
  getCodexQuickConfig,
  openCodexConfigToml,
  saveCodexQuickConfig,
} from '../../services/codexService';
import { useEscClose } from '../../hooks/useEscClose';
import type { CodexExperimentalModelDefinition, CodexQuickConfig } from '../../types/codex';
import { getCodexExperimentalModelErrorMessage } from '../../utils/codexExperimentalModel';
import { CodexExperimentalModelEditor } from './CodexExperimentalModelEditor';

export function CodexQuickConfigCard({ onClose }: { onClose?: () => void }) {
  const { t } = useTranslation();
  useEscClose(true, onClose ?? (() => {}));
  const [configPath, setConfigPath] = useState('~/.codex/config.toml');
  const [loadedConfig, setLoadedConfig] = useState<CodexQuickConfig | null>(null);
  const [catalogEnabled, setCatalogEnabled] = useState(false);
  const [models, setModels] = useState<CodexExperimentalModelDefinition[]>([]);
  const [defaultModelId, setDefaultModelId] = useState<string | null>(null);
  const [modelsEdited, setModelsEdited] = useState(false);
  const [modelsError, setModelsError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [opening, setOpening] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const saveQueueRef = useRef<Promise<void>>(Promise.resolve());
  const saveVersionRef = useRef(0);

  const applyLoadedConfig = useCallback((config: CodexQuickConfig) => {
    setLoadedConfig(config);
    setCatalogEnabled(config.experimental_model_catalog_enabled);
    setModels(config.experimental_model_catalog_models);
    setDefaultModelId(config.experimental_model_catalog_default_model_id ?? null);
    setModelsEdited(false);
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

  const persistConfig = useCallback(
    (
      enabled: boolean,
      nextModels: CodexExperimentalModelDefinition[],
      nextDefaultModelId: string | null,
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
            undefined,
            undefined,
            enabled,
            nextModels,
            nextDefaultModelId,
          );
          if (saveVersion === saveVersionRef.current) {
            applyLoadedConfig(saved);
            setNotice(
              t('codex.modelProviders.quickConfig.saveSuccess', '当前 Codex 配置已保存'),
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
          if (saveVersion === saveVersionRef.current) setSaving(false);
        }
      };
      saveQueueRef.current = saveQueueRef.current.catch(() => undefined).then(save);
    },
    [applyLoadedConfig, loading, t],
  );

  useEffect(() => {
    if (!loadedConfig || loading || !modelsEdited || modelsError) return;
    if (
      JSON.stringify(loadedConfig.experimental_model_catalog_models) === JSON.stringify(models) &&
      (loadedConfig.experimental_model_catalog_default_model_id ?? null) === defaultModelId
    ) {
      return;
    }
    const timer = window.setTimeout(() => {
      persistConfig(catalogEnabled, models, defaultModelId);
    }, 500);
    return () => window.clearTimeout(timer);
  }, [
    catalogEnabled,
    defaultModelId,
    loadedConfig,
    loading,
    models,
    modelsEdited,
    modelsError,
    persistConfig,
  ]);

  const unavailableMessage =
    loadedConfig?.experimental_model_catalog_unavailable_reason === 'catalog_conflict'
      ? t('codex.experimentalModelCatalog.unavailable.catalogConflict', {
          defaultValue: '已有其他 model_catalog_json，禁止覆盖。',
        })
      : null;

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

  return (
    <div className="modal-overlay">
      <div className="modal codex-quick-config-modal" onClick={(event) => event.stopPropagation()}>
        <div className="modal-header">
          <button
            className="btn btn-secondary icon-only"
            onClick={onClose}
            title={t('common.back', '返回')}
            aria-label={t('common.back', '返回')}
          >
            <ChevronLeft size={14} />
          </button>
          <h2>{t('codex.modelProviders.quickConfig.title', '当前 Codex 配置')}</h2>
          <button className="modal-close" onClick={onClose} aria-label={t('common.close', '关闭')}>
            <X />
          </button>
        </div>
        <div className="modal-body">
          <p className="codex-quick-config-desc">
            {t(
              'codex.modelProviders.quickConfig.modelCatalogDesc',
              '可见模型及每个模型的上下文配置统一写入当前 Codex 的 Cockpit 受管模型目录。',
            )}
          </p>
          <div className="codex-quick-config-card__path">
            <span>{t('codex.modelProviders.quickConfig.configPath', '配置文件')}</span>
            <code>{configPath}</code>
          </div>

          {loading ? (
            <div className="section-desc">{t('common.loading', '加载中...')}</div>
          ) : loadedConfig ? (
            <div className="codex-quick-config-grid">
              <div className="codex-quick-config-field codex-quick-config-field--switch">
                <div className="codex-quick-config-field__copy">
                  <label htmlFor="codex-experimental-model-catalog">
                    {t('codex.experimentalModelCatalog.title', '可见模型')}
                  </label>
                  <p>
                    {t(
                      'codex.experimentalModelCatalog.description',
                      '统一管理可见模型、推理强度、上下文窗口和压缩阈值。',
                    )}
                  </p>
                  {catalogEnabled && (
                    <p>
                      {t(
                        'codex.experimentalModelCatalog.enabledHint',
                        '启用后使用当前可见模型列表，重启 Codex 生效。',
                      )}
                    </p>
                  )}
                  {unavailableMessage && (
                    <div className="codex-quick-config-field__error">
                      <CircleAlert size={14} />
                      <span>{unavailableMessage}</span>
                    </div>
                  )}
                </div>
                <label className="codex-quick-config-switch">
                  <input
                    id="codex-experimental-model-catalog"
                    type="checkbox"
                    checked={catalogEnabled}
                    onChange={(event) => {
                      const enabled = event.target.checked;
                      setCatalogEnabled(enabled);
                      persistConfig(
                        enabled,
                        modelsError ? loadedConfig.experimental_model_catalog_models : models,
                        defaultModelId,
                      );
                    }}
                    disabled={
                      !catalogEnabled && !loadedConfig.experimental_model_catalog_available
                    }
                  />
                  <span className="codex-quick-config-switch__slider" />
                </label>
              </div>
              {catalogEnabled && (
                <CodexExperimentalModelEditor
                  models={models}
                  defaultModelId={defaultModelId}
                  mode="summary"
                  onChange={(nextModels) => {
                    setModels(nextModels);
                    setModelsEdited(true);
                    setError(null);
                  }}
                  onDefaultModelChange={(modelId) => {
                    setDefaultModelId(modelId);
                    setModelsEdited(true);
                    setError(null);
                  }}
                  onValidationChange={setModelsError}
                />
              )}
            </div>
          ) : null}

          {(error || saving || notice) && (
            <div className={`add-status ${error ? 'error' : notice ? 'success' : ''}`}>
              {error ? <CircleAlert size={16} /> : <Save size={14} />}
              <span>{error || (saving ? t('common.saving', '保存中...') : notice)}</span>
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
