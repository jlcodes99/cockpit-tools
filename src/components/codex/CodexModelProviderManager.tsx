import { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { confirm as confirmDialog } from '@tauri-apps/plugin-dialog';
import { homeDir, join } from '@tauri-apps/api/path';
import {
  CircleAlert,
  ExternalLink,
  KeyRound,
  Pencil,
  Plus,
  Star,
  Trash2,
  X,
  Search,
  Settings,
} from 'lucide-react';
import type { CodexAccount } from '../../types/codex';
import {
  addApiKeyToCodexModelProvider,
  countCodexModelProviderReferences,
  createCodexModelProvider,
  deleteCodexModelProvider,
  listCodexModelProviders,
  normalizeCodexModelProviderBaseUrl,
  removeApiKeyFromCodexModelProvider,
  type CodexModelProvider,
  type CodexModelProviderApiKey,
  updateCodexModelProvider,
} from '../../services/codexModelProviderService';
import {
  CODEX_API_PROVIDER_CUSTOM_ID,
  CODEX_API_PROVIDER_PRESETS,
  findCodexApiProviderPresetById,
  resolveCodexApiProviderPresetId,
} from '../../utils/codexProviderPresets';
import { CodexQuickConfigCard } from './CodexQuickConfigCard';

interface CodexModelProviderManagerProps {
  accounts: CodexAccount[];
  onProvidersChanged?: (providers: CodexModelProvider[]) => void;
  onManageModelPresets?: () => void;
}

function maskApiKey(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return '';
  if (trimmed.length <= 8) return `${trimmed.slice(0, 2)}****`;
  return `${trimmed.slice(0, 4)}****${trimmed.slice(-4)}`;
}

interface ProviderFormState {
  providerId: string | null;
  name: string;
  baseUrl: string;
  website: string;
  apiKeyUrl: string;
  newApiKeyName: string;
  newApiKey: string;
}

const EMPTY_FORM: ProviderFormState = {
  providerId: null,
  name: '',
  baseUrl: '',
  website: '',
  apiKeyUrl: '',
  newApiKeyName: '',
  newApiKey: '',
};

interface ProviderPreviewPaths {
  providerStorePath: string;
  codexConfigPath: string;
  codexAuthPath: string;
}

const DEFAULT_PROVIDER_PREVIEW_PATHS: ProviderPreviewPaths = {
  providerStorePath: '~/.antigravity_cockpit/codex_model_providers.json',
  codexConfigPath: '~/.codex/config.toml',
  codexAuthPath: '~/.codex/auth.json',
};

export function CodexModelProviderManager({
  accounts,
  onProvidersChanged,
  onManageModelPresets,
}: CodexModelProviderManagerProps) {
  const { t } = useTranslation();
  const [providers, setProviders] = useState<CodexModelProvider[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<{ text: string; tone: 'success' | 'error' } | null>(null);
  const [showModal, setShowModal] = useState(false);
  const [showQuickConfigModal, setShowQuickConfigModal] = useState(false);
  const [saving, setSaving] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);
  const [form, setForm] = useState<ProviderFormState>(EMPTY_FORM);
  const [previewPaths, setPreviewPaths] = useState<ProviderPreviewPaths>(
    DEFAULT_PROVIDER_PREVIEW_PATHS,
  );
  const [selectedPresetId, setSelectedPresetId] = useState<string>(CODEX_API_PROVIDER_CUSTOM_ID);
  const [searchQuery, setSearchQuery] = useState('');

  const filteredProviders = useMemo(() => {
    const query = searchQuery.trim().toLowerCase();
    if (!query) return providers;
    return providers.filter((provider) => {
      const haystack = [
        provider.name,
        provider.baseUrl,
        provider.website || '',
        provider.apiKeyUrl || '',
      ]
        .join(' ')
        .toLowerCase();
      return haystack.includes(query);
    });
  }, [providers, searchQuery]);

  const reloadProviders = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const next = await listCodexModelProviders();
      setProviders(next);
      onProvidersChanged?.(next);
    } catch (err) {
      setError(
        t('codex.modelProviders.loadFailed', {
          defaultValue: '加载模型供应商失败：{{error}}',
          error: String(err),
        }),
      );
    } finally {
      setLoading(false);
    }
  }, [onProvidersChanged, t]);

  useEffect(() => {
    void reloadProviders();
  }, [reloadProviders]);

  useEffect(() => {
    let cancelled = false;

    void (async () => {
      try {
        const home = await homeDir();
        const [providerStorePath, codexConfigPath, codexAuthPath] = await Promise.all([
          join(home, '.antigravity_cockpit', 'codex_model_providers.json'),
          join(home, '.codex', 'config.toml'),
          join(home, '.codex', 'auth.json'),
        ]);
        if (cancelled) return;
        setPreviewPaths({
          providerStorePath,
          codexConfigPath,
          codexAuthPath,
        });
      } catch {
        // ignore path resolution failures and keep fallback preview paths
      }
    })();

    return () => {
      cancelled = true;
    };
  }, []);

  const providerReferenceMap = useMemo(() => {
    const map = new Map<string, number>();
    providers.forEach((provider) => {
      map.set(provider.id, countCodexModelProviderReferences(provider, accounts));
    });
    return map;
  }, [accounts, providers]);

  const currentEditingProvider = useMemo(
    () => (form.providerId ? providers.find((item) => item.id === form.providerId) ?? null : null),
    [form.providerId, providers],
  );
  const selectedPreset = useMemo(
    () => findCodexApiProviderPresetById(selectedPresetId),
    [selectedPresetId],
  );

  const openCreateModal = useCallback(() => {
    setNotice(null);
    setFormError(null);
    setForm(EMPTY_FORM);
    setSelectedPresetId(CODEX_API_PROVIDER_CUSTOM_ID);
    setShowModal(true);
  }, []);

  const openEditModal = useCallback((provider: CodexModelProvider) => {
    setNotice(null);
    setFormError(null);
    setForm({
      providerId: provider.id,
      name: provider.name,
      baseUrl: provider.baseUrl,
      website: provider.website ?? '',
      apiKeyUrl: provider.apiKeyUrl ?? '',
      newApiKeyName: '',
      newApiKey: '',
    });
    setSelectedPresetId(resolveCodexApiProviderPresetId(provider.baseUrl));
    setShowModal(true);
  }, []);

  const closeModal = useCallback(() => {
    if (saving) return;
    setShowModal(false);
    setFormError(null);
  }, [saving]);

  const mutateForm = useCallback((patch: Partial<ProviderFormState>) => {
    setForm((prev) => ({ ...prev, ...patch }));
  }, []);

  useEffect(() => {
    const resolved = resolveCodexApiProviderPresetId(form.baseUrl);
    setSelectedPresetId((prev) => (prev === resolved ? prev : resolved));
  }, [form.baseUrl]);

  const handleSelectProviderPreset = useCallback(
    (presetId: string) => {
      setSelectedPresetId(presetId);
      if (presetId === CODEX_API_PROVIDER_CUSTOM_ID) return;
      const preset = findCodexApiProviderPresetById(presetId);
      if (!preset) return;
      mutateForm({
        name: preset.name,
        baseUrl: preset.baseUrls[0] ?? '',
        website: preset.website ?? '',
        apiKeyUrl: preset.apiKeyUrl ?? '',
      });
    },
    [mutateForm],
  );

  const handleSelectPresetEndpoint = useCallback(
    (baseUrl: string) => {
      mutateForm({ baseUrl });
    },
    [mutateForm],
  );

  const parseServiceError = useCallback(
    (err: unknown): string => {
      const raw = String(err ?? '');
      if (raw.includes('PROVIDER_NAME_REQUIRED')) {
        return t('codex.modelProviders.validation.nameRequired', "Provider name is required.");
      }
      if (raw.includes('PROVIDER_BASE_URL_INVALID')) {
        return t('codex.modelProviders.validation.baseUrlInvalid', "Invalid Base URL.");
      }
      if (raw.includes('PROVIDER_BASE_URL_EXISTS')) {
        return t('codex.modelProviders.validation.baseUrlExists', "This Base URL already exists.");
      }
      if (raw.includes('PROVIDER_NOT_FOUND')) {
        return t('codex.modelProviders.validation.providerNotFound', "Provider not found.");
      }
      return raw.replace(/^Error:\s*/, '');
    },
    [t],
  );

  const handleSaveProvider = useCallback(async () => {
    if (saving) return;
    setFormError(null);
    setNotice(null);

    const name = form.name.trim();
    const baseUrl = form.baseUrl.trim();
    const normalizedBaseUrl = normalizeCodexModelProviderBaseUrl(baseUrl);
    const newApiKey = form.newApiKey.trim();
    const isCreate = !form.providerId;
    const existingKeyCount = currentEditingProvider?.apiKeys.length ?? 0;

    if (!name) {
      setFormError(t('codex.modelProviders.validation.nameRequired', "Provider name is required."));
      return;
    }
    if (!normalizedBaseUrl) {
      setFormError(t('codex.modelProviders.validation.baseUrlInvalid', "Invalid Base URL."));
      return;
    }
    if (isCreate && !newApiKey) {
      setFormError(t('codex.modelProviders.validation.apiKeyRequiredOnCreate', "At least one API Key is required when creating a provider."));
      return;
    }
    if (!isCreate && existingKeyCount === 0 && !newApiKey) {
      setFormError(t('codex.modelProviders.validation.apiKeyRequiredWhenEmpty', "This provider has no API key yet. Please add one first."));
      return;
    }

    setSaving(true);
    try {
      if (!form.providerId) {
        await createCodexModelProvider({
          name,
          baseUrl,
          website: form.website,
          apiKeyUrl: form.apiKeyUrl,
          initialApiKey: newApiKey || undefined,
          initialApiKeyName: form.newApiKeyName,
        });
      } else {
        await updateCodexModelProvider(form.providerId, {
          name,
          baseUrl,
          website: form.website,
          apiKeyUrl: form.apiKeyUrl,
        });
        if (newApiKey) {
          await addApiKeyToCodexModelProvider(form.providerId, newApiKey, form.newApiKeyName);
        }
      }
      await reloadProviders();
      setShowModal(false);
      setForm(EMPTY_FORM);
      setFormError(null);
      setNotice({
        tone: 'success',
        text: t('codex.modelProviders.saveSuccess', "Model provider saved."),
      });
    } catch (err) {
      setFormError(parseServiceError(err));
    } finally {
      setSaving(false);
    }
  }, [currentEditingProvider?.apiKeys.length, form, parseServiceError, reloadProviders, saving, t]);

  const handleDeleteProvider = useCallback(
    async (provider: CodexModelProvider) => {
      const referenceCount = providerReferenceMap.get(provider.id) ?? 0;
      if (referenceCount > 0) {
        setNotice({
          tone: 'error',
          text: t('codex.modelProviders.deleteBlocked', {
            defaultValue: '该供应商已被 {{count}} 个账号引用，禁止删除。',
            count: referenceCount,
          }),
        });
        return;
      }
      const confirmed = await confirmDialog(
        t('codex.modelProviders.confirmDelete', {
          defaultValue: '确认删除供应商「{{name}}」吗？',
          name: provider.name,
        }),
        {
          title: t('common.confirm', "Confirm"),
          kind: 'warning',
          okLabel: t('common.delete', "Delete"),
          cancelLabel: t('common.cancel', "Cancel"),
        },
      );
      if (!confirmed) return;
      try {
        await deleteCodexModelProvider(provider.id);
        await reloadProviders();
      } catch (err) {
        setNotice({
          tone: 'error',
          text: t('codex.modelProviders.deleteFailed', {
            defaultValue: '删除供应商失败：{{error}}',
            error: parseServiceError(err),
          }),
        });
      }
    },
    [parseServiceError, providerReferenceMap, reloadProviders, t],
  );

  const handleDeleteApiKey = useCallback(
    async (provider: CodexModelProvider, apiKey: CodexModelProviderApiKey) => {
      try {
        await removeApiKeyFromCodexModelProvider(provider.id, apiKey.id);
        await reloadProviders();
      } catch (err) {
        setNotice({
          tone: 'error',
          text: t('codex.modelProviders.deleteApiKeyFailed', {
            defaultValue: '删除 API Key 失败：{{error}}',
            error: parseServiceError(err),
          }),
        });
      }
    },
    [parseServiceError, reloadProviders, t],
  );

  return (
    <div className="codex-provider-manager-page">
      {notice && (
        <div className={`message-bar ${notice.tone === 'error' ? 'error' : 'success'}`}>
          {notice.text}
          <button onClick={() => setNotice(null)} aria-label={t('common.close', "Close")}>
            <X size={14} />
          </button>
        </div>
      )}

      {showQuickConfigModal && (
        <CodexQuickConfigCard onClose={() => setShowQuickConfigModal(false)} />
      )}

      <div className="toolbar">
        <div className="toolbar-left">
          <div className="search-box">
            <Search className="search-icon" size={16} />
            <input
              type="text"
              placeholder={t('common.search', 'Search...')}
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
            />
          </div>
        </div>
        <div className="toolbar-right">
          {onManageModelPresets && (
            <button className="btn btn-secondary" onClick={onManageModelPresets}>
              <Settings size={14} />
              {t('codex.modelProviders.managePresets', "Model Presets")}
            </button>
          )}
          <button className="btn btn-secondary" onClick={() => setShowQuickConfigModal(true)}>
            <Settings size={14} />
            {t('codex.modelProviders.quickConfig.title', "Current Codex Config")}
          </button>
          <button className="btn btn-primary" onClick={openCreateModal}>
            <Plus size={14} />
            {t('codex.modelProviders.add', "Add Provider")}
          </button>
        </div>
      </div>

      {error && <div className="add-status error"><CircleAlert size={16} /><span>{error}</span></div>}

      {loading ? (
        <div className="section-desc">{t('common.loading', "Loading...")}</div>
      ) : providers.length === 0 ? (
        <div className="empty-state">
          <h3>{t('codex.modelProviders.emptyTitle', "No model providers yet")}</h3>
          <p>{t('codex.modelProviders.emptyDesc', "Click \"Add Provider\" in the top right to start.")}</p>
        </div>
      ) : filteredProviders.length === 0 ? (
        <div className="empty-state">
          <h3>{t('codex.modelProviders.noMatchTitle', 'No matching providers')}</h3>
          <p>{t('common.shared.noMatch.desc', "Try adjusting your search or filters")}</p>
        </div>
      ) : (
        <div className="codex-provider-grid">
          {filteredProviders.map((provider) => {
            const referenceCount = providerReferenceMap.get(provider.id) ?? 0;
            return (
              <div className="codex-provider-card" key={provider.id}>
                <div className="codex-provider-card-header">
                  <div className="codex-provider-title">{provider.name}</div>
                  <div className="codex-provider-actions">
                    <button
                      className="action-btn"
                      onClick={() => openEditModal(provider)}
                      title={t('instances.actions.edit', "Edit")}
                    >
                      <Pencil size={14} />
                    </button>
                    <button
                      className="action-btn danger"
                      onClick={() => void handleDeleteProvider(provider)}
                      title={t('common.delete', "Delete")}
                    >
                      <Trash2 size={14} />
                    </button>
                  </div>
                </div>
                <div className="codex-provider-meta">
                  <span>{t('codex.modelProviders.baseUrl', 'Base URL')}</span>
                  <code>{provider.baseUrl}</code>
                </div>
                {(provider.website || provider.apiKeyUrl) && (
                  <div className="codex-provider-links">
                    {provider.website && (
                      <a href={provider.website} target="_blank" rel="noreferrer">
                        <ExternalLink size={12} />
                        {t('codex.modelProviders.website', "Website")}
                      </a>
                    )}
                    {provider.apiKeyUrl && (
                      <a href={provider.apiKeyUrl} target="_blank" rel="noreferrer">
                        <KeyRound size={12} />
                        {t('codex.modelProviders.apiKeyPage', "API Key Page")}
                      </a>
                    )}
                  </div>
                )}
                <div className="codex-provider-badges">
                  <span className={`provider-badge ${provider.apiKeys.length > 0 ? 'primary' : ''}`}>
                    {t('codex.modelProviders.apiKeysCount', {
                      defaultValue: 'API Key {{count}} 个',
                      count: provider.apiKeys.length,
                    })}
                  </span>
                  <span className={`provider-badge ${referenceCount > 0 ? 'danger' : ''}`}>
                    {t('codex.modelProviders.referencesCount', {
                      defaultValue: '引用账号 {{count}} 个',
                      count: referenceCount,
                    })}
                  </span>
                </div>
                {provider.apiKeys.length > 0 && (
                  <div className="codex-provider-key-list">
                    {provider.apiKeys.map((item) => (
                      <div className="codex-provider-key-row" key={item.id}>
                        <div className="codex-provider-key-text">
                          <span className="codex-provider-key-name">
                            {item.name || t('codex.modelProviders.unnamedKey', "Unnamed Key")}
                          </span>
                          <code>{maskApiKey(item.apiKey)}</code>
                        </div>
                        <button
                          className="action-btn danger"
                          onClick={() => void handleDeleteApiKey(provider, item)}
                          title={t('common.delete', "Delete")}
                        >
                          <Trash2 size={12} />
                        </button>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}

      {showModal && (
        <div className="modal-overlay" onClick={closeModal}>
          <div className="modal codex-provider-modal" onClick={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <h2>
                {form.providerId
                  ? t('codex.modelProviders.editTitle', "Edit Model Provider")
                  : t('codex.modelProviders.createTitle', "Add Model Provider")}
              </h2>
              <button
                className="modal-close"
                onClick={closeModal}
                aria-label={t('common.close', "Close")}
                disabled={saving}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <div className="form-group">
                <label>{t('codex.api.provider.label', "Provider")}</label>
                <div className="api-provider-chip-list">
                  <button
                    className={`api-provider-chip ${selectedPresetId === CODEX_API_PROVIDER_CUSTOM_ID ? 'active' : ''}`}
                    onClick={() => handleSelectProviderPreset(CODEX_API_PROVIDER_CUSTOM_ID)}
                    type="button"
                    disabled={saving}
                  >
                    <span>{t('codex.api.provider.custom', "Custom")}</span>
                  </button>
                  {CODEX_API_PROVIDER_PRESETS.map((preset) => (
                    <button
                      key={preset.id}
                      className={`api-provider-chip ${selectedPresetId === preset.id ? 'active' : ''}`}
                      onClick={() => handleSelectProviderPreset(preset.id)}
                      type="button"
                      disabled={saving}
                    >
                      <span>{t(`codex.api.providers.${preset.id}.name`, preset.name)}</span>
                      {preset.isPartner && <Star size={12} className="api-provider-chip-badge" />}
                    </button>
                  ))}
                </div>
              </div>
              {selectedPreset && selectedPreset.baseUrls.length > 1 && (
                <div className="form-group">
                  <label>{t('codex.api.provider.endpoint', "Provider Endpoint")}</label>
                  <div className="api-provider-endpoint-list">
                    {selectedPreset.baseUrls.map((baseUrl) => (
                      <button
                        key={baseUrl}
                        className={`api-provider-endpoint-chip ${form.baseUrl === baseUrl ? 'active' : ''}`}
                        onClick={() => handleSelectPresetEndpoint(baseUrl)}
                        type="button"
                        disabled={saving}
                      >
                        {baseUrl}
                      </button>
                    ))}
                  </div>
                </div>
              )}
              {selectedPreset && (
                <div className="api-provider-hint-block">
                  <p className="api-provider-hint">
                    {t('codex.api.provider.hint', "A compatible Base URL has been filled in automatically. You can still edit it manually.")}
                  </p>
                  <div className="api-provider-links">
                    {selectedPreset.website && (
                      <a className="btn btn-secondary" href={selectedPreset.website} target="_blank" rel="noreferrer">
                        <ExternalLink size={14} />
                        {t('codex.api.provider.website', "Website")}
                      </a>
                    )}
                    {selectedPreset.apiKeyUrl && (
                      <a className="btn btn-secondary" href={selectedPreset.apiKeyUrl} target="_blank" rel="noreferrer">
                        <KeyRound size={14} />
                        {t('codex.api.provider.apiKeyPage', "API Key Page")}
                      </a>
                    )}
                  </div>
                </div>
              )}
              <div className="form-group">
                <label>{t('codex.modelProviders.fields.name', "Provider Name")}</label>
                <input
                  className="form-input"
                  type="text"
                  value={form.name}
                  onChange={(event) => mutateForm({ name: event.target.value })}
                  disabled={saving}
                />
              </div>
              <div className="form-group">
                <label>{t('codex.modelProviders.fields.baseUrl', 'Base URL')}</label>
                <input
                  className="form-input"
                  type="text"
                  value={form.baseUrl}
                  onChange={(event) => mutateForm({ baseUrl: event.target.value })}
                  disabled={saving}
                />
              </div>
              <div className="form-group">
                <label>{t('codex.modelProviders.fields.website', "Website (Optional)")}</label>
                <input
                  className="form-input"
                  type="text"
                  value={form.website}
                  onChange={(event) => mutateForm({ website: event.target.value })}
                  disabled={saving}
                />
              </div>
              <div className="form-group">
                <label>{t('codex.modelProviders.fields.apiKeyUrl', "API Key Page (Optional)")}</label>
                <input
                  className="form-input"
                  type="text"
                  value={form.apiKeyUrl}
                  onChange={(event) => mutateForm({ apiKeyUrl: event.target.value })}
                  disabled={saving}
                />
              </div>

              {currentEditingProvider && currentEditingProvider.apiKeys.length > 0 && (
                <div className="form-group">
                  <label>{t('codex.modelProviders.existingApiKeys', "Existing API Keys")}</label>
                  <div className="codex-provider-key-list inline">
                    {currentEditingProvider.apiKeys.map((item) => (
                      <div className="codex-provider-key-row" key={item.id}>
                        <div className="codex-provider-key-text">
                          <span className="codex-provider-key-name">
                            {item.name || t('codex.modelProviders.unnamedKey', "Unnamed Key")}
                          </span>
                          <code>{maskApiKey(item.apiKey)}</code>
                        </div>
                        <button
                          className="action-btn danger"
                          onClick={() => void handleDeleteApiKey(currentEditingProvider, item)}
                          disabled={saving}
                          title={t('common.delete', "Delete")}
                        >
                          <Trash2 size={12} />
                        </button>
                      </div>
                    ))}
                  </div>
                </div>
              )}

              <div className="form-group">
                <label>{t('codex.modelProviders.fields.newApiKeyName', "New Key Name (Optional)")}</label>
                <input
                  className="form-input"
                  type="text"
                  value={form.newApiKeyName}
                  onChange={(event) => mutateForm({ newApiKeyName: event.target.value })}
                  disabled={saving}
                />
              </div>
              <div className="form-group">
                <label>{t('codex.modelProviders.fields.newApiKey', "New API Key")}</label>
                <input
                  className="form-input"
                  type="text"
                  value={form.newApiKey}
                  onChange={(event) => mutateForm({ newApiKey: event.target.value })}
                  disabled={saving}
                />
              </div>

              <div className="provider-save-preview">
                <div className="provider-save-preview-header">
                  <div className="provider-save-preview-title">
                    {t('codex.modelProviders.preview.title', "Save Preview")}
                  </div>
                  <span className="provider-save-preview-chip primary">
                    {t('codex.modelProviders.preview.writeNow', "Will Write")}
                  </span>
                </div>
                <p className="provider-save-preview-desc">
                  {t(
                    'codex.modelProviders.preview.desc',
                    "Saving a provider updates the provider store first. It does not switch the current official Codex config immediately.",
                  )}
                </p>
                <div className="provider-save-preview-list">
                  <div className="provider-save-preview-item primary">
                    <div className="provider-save-preview-item-head">
                      <span className="provider-save-preview-item-title">
                        {t(
                          'codex.modelProviders.preview.providerStoreTitle',
                          "Provider Store",
                        )}
                      </span>
                      <span className="provider-save-preview-chip primary">
                        {t('codex.modelProviders.preview.writeNow', "Will Write")}
                      </span>
                    </div>
                    <code>{previewPaths.providerStorePath}</code>
                    <p>
                      {t(
                        'codex.modelProviders.preview.providerStoreDesc',
                        "Saves the provider name, Base URL, website/API Key page links, and any new API key added in this modal.",
                      )}
                    </p>
                  </div>

                  <div className="provider-save-preview-item muted">
                    <div className="provider-save-preview-item-head">
                      <span className="provider-save-preview-item-title">
                        {t(
                          'codex.modelProviders.preview.codexConfigTitle',
                          "Current Codex Config",
                        )}
                      </span>
                      <span className="provider-save-preview-chip muted">
                        {t(
                          'codex.modelProviders.preview.noImmediateChange',
                          "No Immediate Change",
                        )}
                      </span>
                    </div>
                    <code>{previewPaths.codexConfigPath}</code>
                    <p>
                      {t(
                        'codex.modelProviders.preview.codexConfigDesc',
                        "The current provider or Base URL will not change right away. That file updates only when a Codex API-key account is saved or switched.",
                      )}
                    </p>
                  </div>

                  <div className="provider-save-preview-item muted">
                    <div className="provider-save-preview-item-head">
                      <span className="provider-save-preview-item-title">
                        {t(
                          'codex.modelProviders.preview.authFileTitle',
                          "Current Codex Credentials",
                        )}
                      </span>
                      <span className="provider-save-preview-chip muted">
                        {t(
                          'codex.modelProviders.preview.noImmediateChange',
                          "No Immediate Change",
                        )}
                      </span>
                    </div>
                    <code>{previewPaths.codexAuthPath}</code>
                    <p>
                      {t(
                        'codex.modelProviders.preview.authFileDesc',
                        "Saving a provider does not overwrite the current OPENAI_API_KEY in auth.json.",
                      )}
                    </p>
                  </div>
                </div>
              </div>

              {formError && (
                <div className="add-status error">
                  <CircleAlert size={16} />
                  <span>{formError}</span>
                </div>
              )}
            </div>
            
            <div className="modal-footer">
              <button className="btn btn-secondary" onClick={closeModal} disabled={saving}>
                {t('common.cancel', "Cancel")}
              </button>
              <button
                className="btn btn-primary"
                onClick={() => void handleSaveProvider()}
                disabled={saving}
              >
                {saving ? t('common.saving', "Saving...") : t('common.save', "Save")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
