import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ChevronLeft, CircleAlert, FolderOpen, Save, X } from 'lucide-react';
import { confirm as confirmDialog } from '@tauri-apps/plugin-dialog';
import {
  getCodexConfigTomlPath,
  getCodexQuickConfig,
  openCodexConfigToml,
  saveCodexQuickConfig,
} from '../../services/codexService';
import { useEscClose } from '../../hooks/useEscClose';
import type { CodexExperimentalModelDefinition, CodexQuickConfig } from '../../types/codex';
import { getCodexExperimentalModelErrorMessage } from '../../utils/codexExperimentalModel';
import { resolveStoredCompactLimitInput } from '../../utils/codexModelContext';
import { CodexContextManagementControl } from './CodexContextManagementControl';
import { CodexContextOverrideEditor } from './CodexContextOverrideEditor';
import { CodexExperimentalModelEditor } from './CodexExperimentalModelEditor';

export function CodexQuickConfigCard({ onClose }: { onClose?: () => void }) {
  const { t } = useTranslation();
  useEscClose(true, onClose ?? (() => {}));
  const [configPath, setConfigPath] = useState('~/.codex/config.toml');
  const [loadedConfig, setLoadedConfig] = useState<CodexQuickConfig | null>(null);
  const [contextOverrideEnabled, setContextOverrideEnabled] = useState(false);
  const [contextWindow, setContextWindow] = useState('');
  const [compactLimit, setCompactLimit] = useState('');
  const [catalogEnabled, setCatalogEnabled] = useState(false);
  const [models, setModels] = useState<CodexExperimentalModelDefinition[]>([]);
  const [defaultModelId, setDefaultModelId] = useState<string | null>(null);
  const [modelsError, setModelsError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [opening, setOpening] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const applyLoadedConfig = useCallback((config: CodexQuickConfig) => {
    setLoadedConfig(config);
    setContextOverrideEnabled(
      config.detected_model_context_window !== undefined ||
        config.detected_auto_compact_token_limit !== undefined,
    );
    setContextWindow(config.detected_model_context_window?.toString() ?? '');
    // 存量配置里压缩阈值缺失、等于或超过上下文时按 90% 归一，避免带出非法配对。
    setCompactLimit(
      resolveStoredCompactLimitInput(
        config.detected_model_context_window,
        config.detected_auto_compact_token_limit,
      ),
    );
    setCatalogEnabled(config.experimental_model_catalog_enabled);
    setModels(config.experimental_model_catalog_models);
    setDefaultModelId(config.experimental_model_catalog_default_model_id ?? null);
    setModelsError(null);
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

  const handleSave = useCallback(async () => {
    if (catalogEnabled && modelsError) {
      setError(modelsError);
      return;
    }
    const parsedContext = Number.parseInt(contextWindow, 10);
    const parsedCompact = compactLimit.trim()
      ? Number.parseInt(compactLimit, 10)
      : undefined;
    if (contextOverrideEnabled && (!Number.isSafeInteger(parsedContext) || parsedContext <= 0)) {
      setError(t('codex.experimentalModelCatalog.models.validation.contextWindow'));
      return;
    }
    if (
      contextOverrideEnabled &&
      parsedCompact !== undefined &&
      (!Number.isSafeInteger(parsedCompact) || parsedCompact <= 0)
    ) {
      setError(t('codex.experimentalModelCatalog.models.validation.autoCompact'));
      return;
    }
    if (
      contextOverrideEnabled &&
      parsedCompact !== undefined &&
      parsedCompact >= parsedContext
    ) {
      setError(t('codex.experimentalModelCatalog.models.validation.autoCompactRange'));
      return;
    }
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      const saved = await saveCodexQuickConfig(
        contextOverrideEnabled ? parsedContext : undefined,
        contextOverrideEnabled ? parsedCompact : undefined,
        catalogEnabled,
        models,
        defaultModelId,
      );
      applyLoadedConfig(saved);
      setNotice(t('codex.modelProviders.quickConfig.saveSuccess'));
    } catch (err) {
      setError(
        getCodexExperimentalModelErrorMessage(t, err) ??
          t('codex.modelProviders.quickConfig.saveFailed', {
            error: String(err),
          }),
      );
    } finally {
      setSaving(false);
    }
  }, [
    applyLoadedConfig,
    catalogEnabled,
    compactLimit,
    contextWindow,
    contextOverrideEnabled,
    defaultModelId,
    models,
    modelsError,
    t,
  ]);

  const handleCatalogToggle = useCallback(async () => {
    if (catalogEnabled) {
      setCatalogEnabled(false);
      setError(null);
      return;
    }
    if (!loadedConfig?.experimental_model_catalog_available) return;
    const confirmed = await confirmDialog(
      t('codex.modelManagement.enableConfirmDescription'),
      {
        title: t('codex.modelManagement.enableConfirmTitle'),
        okLabel: t('codex.modelManagement.enableConfirmAction'),
        cancelLabel: t('common.cancel'),
        kind: 'warning',
      },
    );
    if (confirmed) {
      setCatalogEnabled(true);
      setError(null);
    }
  }, [catalogEnabled, loadedConfig, t]);

  const unavailableMessage =
    loadedConfig?.experimental_model_catalog_unavailable_reason === 'catalog_conflict'
      ? t('codex.experimentalModelCatalog.unavailable.catalogConflict')
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
          error: String(err),
        }),
      );
    } finally {
      setOpening(false);
    }
  }, [opening, t]);

  return (
    <div className="modal-overlay">
      <div className="modal codex-quick-config-modal">
        <div className="modal-header">
          <button
            className="btn btn-secondary icon-only"
            onClick={onClose}
            title={t('common.back')}
            aria-label={t('common.back')}
          >
            <ChevronLeft size={14} />
          </button>
          <h2>{t('codex.modelProviders.quickConfig.title')}</h2>
          <button className="modal-close" onClick={onClose} aria-label={t('common.close')}>
            <X />
          </button>
        </div>
        <div className="modal-body">
          <div className="codex-quick-config-card__path">
            <span>{t('codex.modelProviders.quickConfig.configPath')}</span>
            <code>{configPath}</code>
          </div>

          <CodexContextManagementControl variant="settings" />

          {loading ? (
            <div className="section-desc">{t('common.loading')}</div>
          ) : loadedConfig ? (
            <section className="codex-quick-config-section">
              <div className="codex-quick-config-section__heading">
                <h3>{t('codex.contextOverride.title')}</h3>
                <p>{t('codex.contextOverride.dialogDescription')}</p>
              </div>
              <CodexContextOverrideEditor
                enabled={contextOverrideEnabled}
                contextWindow={contextWindow}
                compactLimit={compactLimit}
                disabled={saving}
                onChange={(value) => {
                  setContextOverrideEnabled(value.enabled);
                  setContextWindow(value.contextWindow);
                  setCompactLimit(value.compactLimit);
                }}
              />
            </section>
          ) : null}

          {!loading && loadedConfig && (
            <div className="codex-quick-config-grid">
              <div className="codex-quick-config-field codex-quick-config-field--switch">
                <div className="codex-quick-config-field__copy">
                  <label>{t('codex.modelManagement.title')}</label>
                  <p>
                    {catalogEnabled
                      ? t('codex.modelManagement.enabledDescription')
                      : t('codex.modelManagement.disabledDescription')}
                  </p>
                  {unavailableMessage && (
                    <div className="codex-quick-config-field__error">
                      <CircleAlert size={14} />
                      <span>{unavailableMessage}</span>
                    </div>
                  )}
                </div>
                <button
                  type="button"
                  className={`btn ${catalogEnabled ? 'btn-outline' : 'btn-primary'}`}
                  onClick={() => void handleCatalogToggle()}
                  disabled={
                    saving ||
                    (!catalogEnabled && !loadedConfig.experimental_model_catalog_available)
                  }
                >
                  {catalogEnabled
                    ? t('codex.modelManagement.disable')
                    : t('codex.modelManagement.enable')}
                </button>
              </div>
              {catalogEnabled && (
                <CodexExperimentalModelEditor
                  models={models}
                  defaultModelId={defaultModelId}
                  resetModels={loadedConfig.experimental_model_catalog_reset_models}
                  resetDefaultModelId={
                    loadedConfig.experimental_model_catalog_reset_default_model_id ?? null
                  }
                  mode="summary"
                  onChange={(nextModels) => {
                    setModels(nextModels);
                    setError(null);
                  }}
                  onDefaultModelChange={setDefaultModelId}
                  onValidationChange={setModelsError}
                  disabled={saving}
                />
              )}
            </div>
          )}

          {(error || saving || notice) && (
            <div className={`add-status ${error ? 'error' : notice ? 'success' : ''}`}>
              {error ? <CircleAlert size={16} /> : <Save size={14} />}
              <span>{error || (saving ? t('common.saving') : notice)}</span>
            </div>
          )}
        </div>
        <div className="modal-footer">
          <button
            className="btn btn-secondary"
            onClick={() => void handleOpenConfig()}
            disabled={opening || loading || saving}
            type="button"
          >
            <FolderOpen size={14} />
            {opening ? t('common.loading') : t('codex.modelProviders.quickConfig.openConfig')}
          </button>
          <button
            className="btn btn-primary"
            onClick={() => void handleSave()}
            disabled={loading || saving || !loadedConfig}
            type="button"
          >
            <Save size={14} />
            {saving ? t('common.saving') : t('common.save')}
          </button>
        </div>
      </div>
    </div>
  );
}
