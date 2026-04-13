import { useState, useEffect, useRef, useMemo, useCallback, Fragment } from 'react';
import {
  Plus,
  RefreshCw,
  Download,
  Upload,
  Trash2,
  X,
  Globe,
  KeyRound,
  Database,
  Copy,
  Check,
  Play,
  RotateCw,
  Repeat,
  CircleAlert,
  Rows3,
  LayoutGrid,
  List,
  Search,
  ArrowDownWideNarrow,
  Clock,
  Calendar,
  Tag,
  Star,
  Eye,
  EyeOff,
  BookOpen,
  FileUp,
  FileText,
  ExternalLink,
  Pencil,
  FolderOpen,
  FolderPlus,
} from 'lucide-react';
import { useCodexAccountStore } from '../stores/useCodexAccountStore';
import * as codexService from '../services/codexService';
import { TagEditModal } from '../components/TagEditModal';
import { ExportJsonModal } from '../components/ExportJsonModal';
import { ModalErrorMessage } from '../components/ModalErrorMessage';
import { PaginationControls } from '../components/PaginationControls';
import { CodexAccountGroupModal, CodexAddToGroupModal } from '../components/CodexAccountGroupModal';
import {
  type CodexAccountGroup,
  getCodexAccountGroups,
} from '../services/codexAccountGroupService';
import {
  hasCodexAccountStructure,
  formatCodexLoginProvider,
  getCodexAuthMetadata,
  hasCodexAccountName,
  isCodexApiKeyAccount,
  isCodexTeamLikePlan,
  type CodexApiProviderMode,
  type CodexQuotaErrorInfo,
} from '../types/codex';
import { buildCodexAccountPresentation } from '../presentation/platformAccountPresentation';

import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { confirm as confirmDialog, open as openFileDialog } from '@tauri-apps/plugin-dialog';
import { openUrl } from '@tauri-apps/plugin-opener';
import { CodexOverviewTabsHeader, CodexTab } from '../components/CodexOverviewTabsHeader';
import { CodexInstancesContent } from './CodexInstancesPage';
import { CodexSessionManager } from '../components/codex/CodexSessionManager';
import { CodexWakeupContent } from '../components/codex/CodexWakeupContent';
import { CodexModelProviderManager } from '../components/codex/CodexModelProviderManager';
import { QuickSettingsPopover } from '../components/QuickSettingsPopover';
import { useProviderAccountsPage } from '../hooks/useProviderAccountsPage';
import { MultiSelectFilterDropdown, type MultiSelectFilterOption } from '../components/MultiSelectFilterDropdown';
import { SingleSelectFilterDropdown } from '../components/SingleSelectFilterDropdown';
import type { CodexAccount } from '../types/codex';
import {
  CODEX_CODE_REVIEW_QUOTA_VISIBILITY_CHANGED_EVENT,
  isCodexCodeReviewQuotaVisibleByDefault,
} from '../utils/codexPreferences';
import { emitAccountsChanged } from '../utils/accountSyncEvents';
import { compareCurrentAccountFirst } from '../utils/currentAccountSort';
import {
  CODEX_API_PROVIDER_CUSTOM_ID,
  CODEX_API_PROVIDER_PRESETS,
  findCodexApiProviderPresetById,
  resolveCodexApiProviderPresetId,
} from '../utils/codexProviderPresets';
import {
  findCodexModelProviderById,
  findCodexModelProviderByBaseUrl,
  listCodexModelProviders,
  type CodexModelProvider,
  upsertCodexModelProviderFromCredential,
} from '../services/codexModelProviderService';
import {
  buildValidAccountsFilterOption,
  splitValidityFilterValues,
} from '../utils/accountValidityFilter';
import {
  buildPaginatedGroups,
  buildPaginationPageSizeStorageKey,
  isEveryIdSelected,
  usePagination,
} from '../hooks/usePagination';

const CODEX_TOKEN_SINGLE_EXAMPLE = `{
  "tokens": {
    "id_token": "eyJ...",
    "access_token": "eyJ...",
    "refresh_token": "rt_..."
  }
}`;
const CODEX_TOKEN_BATCH_EXAMPLE = `[
  {
    "id": "codex_demo_1",
    "email": "user@example.com",
    "tokens": {
      "id_token": "eyJ...",
      "access_token": "eyJ...",
      "refresh_token": "rt_..."
    },
    "created_at": 1730000000,
    "last_used": 1730000000
  }
]`;
const OPENAI_OFFICIAL_PRESET_ID = 'openai_official';

function normalizeCodexApiBaseUrl(rawValue?: string | null): string {
  return normalizeHttpBaseUrl(rawValue ?? '') ?? '';
}

function inferCodexAccountProviderMode(account: CodexAccount): CodexApiProviderMode {
  if (account.api_provider_mode === 'custom' || account.api_provider_mode === 'openai_builtin') {
    return account.api_provider_mode;
  }
  const normalizedBaseUrl = normalizeCodexApiBaseUrl(account.api_base_url);
  if (!normalizedBaseUrl || normalizedBaseUrl === 'https://api.openai.com/v1') {
    return 'openai_builtin';
  }
  return 'custom';
}
const CODEX_USAGE_URL = 'https://platform.openai.com/usage';
const CODEX_OVERVIEW_LAYOUT_MODE_KEY = 'agtools.codex.accounts.overview_layout_mode';
const DEFAULT_CODEX_API_PROVIDER_ID = CODEX_API_PROVIDER_CUSTOM_ID;

type CodexOverviewLayoutMode = 'compact' | 'list' | 'grid';

function normalizeCodexOverviewLayoutMode(
  value: string | null,
): CodexOverviewLayoutMode | null {
  if (value === 'compact' || value === 'list' || value === 'grid') return value;
  return null;
}

function isHttpLikeUrl(value: string): boolean {
  const trimmed = value.trim();
  if (!trimmed) return false;
  try {
    const parsed = new URL(trimmed);
    return parsed.protocol === 'http:' || parsed.protocol === 'https:';
  } catch {
    const lower = trimmed.toLowerCase();
    return lower.startsWith('http://') || lower.startsWith('https://');
  }
}

function normalizeHttpBaseUrl(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return null;
    return trimmed.replace(/\/+$/, '');
  } catch {
    return null;
  }
}

export function CodexAccountsPage() {
  const [activeTab, setActiveTab] = useState<CodexTab>('overview');
  const untaggedKey = '__untagged__';
  const [filterTypes, setFilterTypes] = useState<string[]>([]);

  // ─── Codex 账号分组 ────────────────────────────────────────────
  const [codexGroups, setCodexGroups] = useState<CodexAccountGroup[]>([]);
  const [groupFilter, setGroupFilter] = useState<string[]>([]);
  const [showCodexGroupModal, setShowCodexGroupModal] = useState(false);
  const [showAddToCodexGroupModal, setShowAddToCodexGroupModal] = useState(false);

  const reloadCodexGroups = useCallback(async () => {
    setCodexGroups(await getCodexAccountGroups());
  }, []);

  useEffect(() => {
    reloadCodexGroups();
  }, [reloadCodexGroups]);

  const toggleGroupFilterValue = useCallback((groupId: string) => {
    setGroupFilter((prev) => {
      if (prev.includes(groupId)) return prev.filter((id) => id !== groupId);
      return [...prev, groupId];
    });
  }, []);

  const clearGroupFilter = useCallback(() => {
    setGroupFilter([]);
  }, []);

  const [overviewLayoutMode, setOverviewLayoutMode] = useState<CodexOverviewLayoutMode>(() => {
    try {
      const saved = normalizeCodexOverviewLayoutMode(localStorage.getItem(CODEX_OVERVIEW_LAYOUT_MODE_KEY));
      if (saved) return saved;
      const legacy = normalizeCodexOverviewLayoutMode(localStorage.getItem('agtools.codex.accounts_view_mode'));
      if (legacy === 'list' || legacy === 'grid') return legacy;
    } catch {
      // ignore persistence failures
    }
    return 'grid';
  });

  const store = useCodexAccountStore();

  // Use the common hook WITHOUT oauthService since Codex uses Tauri event-based OAuth
  const page = useProviderAccountsPage<CodexAccount>({
    platformKey: 'Codex',
    oauthLogPrefix: 'CodexOAuth',
    exportFilePrefix: 'codex_accounts',
    store: {
      accounts: store.accounts,
      loading: store.loading,
      error: store.error,
      fetchAccounts: store.fetchAccounts,
      deleteAccounts: store.deleteAccounts,
      refreshToken: (id) => store.refreshQuota(id).then(() => { }),
      refreshAllTokens: () => store.refreshAllQuotas().then(() => { }),
      updateAccountTags: store.updateAccountTags,
    },
    dataService: {
      importFromJson: codexService.importCodexFromJson,
      exportAccounts: codexService.exportCodexAccounts,
    },
    getDisplayEmail: (account) => account.email ?? account.id,
  });

  const {
    t, maskAccountText, privacyModeEnabled, togglePrivacyMode,
    viewMode, setViewMode, searchQuery, setSearchQuery,
    sortBy, setSortBy, sortDirection, setSortDirection,
    selected, toggleSelect, toggleSelectAll,
    tagFilter, groupByTag, setGroupByTag, showTagFilter, setShowTagFilter,
    showTagModal, setShowTagModal, tagFilterRef, availableTags,
    toggleTagFilterValue, clearTagFilter, tagDeleteConfirm, tagDeleteConfirmError, tagDeleteConfirmErrorScrollKey, setTagDeleteConfirm,
    deletingTag, requestDeleteTag, confirmDeleteTag, openTagModal, handleSaveTags,
    refreshing, refreshingAll,
    handleRefresh, handleRefreshAll, handleDelete, handleBatchDelete,
    deleteConfirm, deleteConfirmError, deleteConfirmErrorScrollKey, setDeleteConfirm, deleting, confirmDelete,
    message, setMessage,
    exporting, handleExport, handleExportByIds, getScopedSelectedCount,
    showExportModal, closeExportModal, exportJsonContent, exportJsonHidden,
    toggleExportJsonHidden, exportJsonCopied, copyExportJson,
    savingExportJson, saveExportJson, exportSavedPath,
    canOpenExportSavedDirectory, openExportSavedDirectory, copyExportSavedPath, exportPathCopied,
    showAddModal, addTab, addStatus, addMessage, tokenInput, setTokenInput,
    importing, openAddModal, closeAddModal,
    formatDate, normalizeTag,
  } = page;

  useEffect(() => {
    try {
      localStorage.setItem(CODEX_OVERVIEW_LAYOUT_MODE_KEY, overviewLayoutMode);
    } catch {
      // ignore persistence failures
    }
  }, [overviewLayoutMode]);

  const handleChangeOverviewLayoutMode = useCallback(
    (mode: CodexOverviewLayoutMode) => {
      setOverviewLayoutMode(mode);
      if (mode === 'list' || mode === 'grid') {
        setViewMode(mode);
      }
    },
    [setViewMode],
  );

  useEffect(() => {
    if (overviewLayoutMode !== 'compact' && viewMode !== overviewLayoutMode) {
      setViewMode(overviewLayoutMode);
    }
  }, [overviewLayoutMode, setViewMode, viewMode]);

  const toggleFilterTypeValue = useCallback((value: string) => {
    setFilterTypes((prev) => {
      if (prev.includes(value)) {
        return prev.filter((item) => item !== value);
      }
      return [...prev, value];
    });
  }, []);

  const clearFilterTypes = useCallback(() => {
    setFilterTypes([]);
  }, []);

  const validateApiKeyCredentialInputs = useCallback(
    (
      apiKeyRaw: string,
      apiBaseUrlRaw: string,
    ): { ok: true; apiKey: string; apiBaseUrl?: string } | { ok: false; message: string } => {
      const apiKey = apiKeyRaw.trim();
      if (!apiKey) {
        return { ok: false, message: t('common.shared.token.empty', '请输入 Token 或 JSON') };
      }
      if (isHttpLikeUrl(apiKey)) {
        return {
          ok: false,
          message: t('codex.api.validation.apiKeyCannotBeUrl', 'API Key 不能是 URL，请检查是否填反'),
        };
      }

      const rawBaseUrl = apiBaseUrlRaw.trim();
      if (!rawBaseUrl) {
        return { ok: true, apiKey };
      }
      const normalizedBaseUrl = normalizeHttpBaseUrl(rawBaseUrl);
      if (!normalizedBaseUrl) {
        return {
          ok: false,
          message: t('codex.api.validation.baseUrlInvalid', 'Base URL 格式无效，请输入完整的 http:// 或 https:// 地址'),
        };
      }
      if (normalizedBaseUrl === apiKey) {
        return {
          ok: false,
          message: t('codex.api.validation.apiKeyEqualsBaseUrl', 'API Key 不能与 Base URL 相同'),
        };
      }
      return {
        ok: true,
        apiKey,
        apiBaseUrl: normalizedBaseUrl,
      };
    },
    [t],
  );

  const {
    accounts,
    loading,
    currentAccount,
    fetchAccounts,
    fetchCurrentAccount,
    switchAccount,
    refreshQuota,
    hydrateAccountProfilesIfNeeded,
    updateAccountName,
    updateApiKeyCredentials,
  } = store;

  // ─── Codex-specific: OAuth via Tauri events ──────────────────────────

  const [oauthUrl, setOauthUrl] = useState<string | null>(null);
  const [oauthUrlCopied, setOauthUrlCopied] = useState(false);
  const [oauthPrepareError, setOauthPrepareError] = useState<string | null>(null);
  const [oauthPortInUse, setOauthPortInUse] = useState<number | null>(null);
  const [oauthTimeoutInfo, setOauthTimeoutInfo] = useState<{ loginId?: string; callbackUrl?: string; timeoutSeconds?: number } | null>(null);
  const [oauthCallbackInput, setOauthCallbackInput] = useState('');
  const [oauthCallbackSubmitting, setOauthCallbackSubmitting] = useState(false);
  const [oauthCallbackError, setOauthCallbackError] = useState<string | null>(null);
  const [oauthTokenExchangeRetryVisible, setOauthTokenExchangeRetryVisible] = useState(false);
  const [switching, setSwitching] = useState<string | null>(null);
  const [apiKeyInput, setApiKeyInput] = useState('');
  const [apiBaseUrlInput, setApiBaseUrlInput] = useState('');
  const [apiProviderPresetId, setApiProviderPresetId] = useState(
    DEFAULT_CODEX_API_PROVIDER_ID,
  );
  const [managedProviders, setManagedProviders] = useState<CodexModelProvider[]>([]);
  const [managedProvidersLoading, setManagedProvidersLoading] = useState(false);
  const [managedProviderId, setManagedProviderId] = useState<string>('');
  const [managedProviderApiKeyId, setManagedProviderApiKeyId] = useState<string>('');
  const [newManagedProviderNameInput, setNewManagedProviderNameInput] = useState('');
  const [editingApiKeyNameId, setEditingApiKeyNameId] = useState<string | null>(null);
  const [editingApiKeyNameValue, setEditingApiKeyNameValue] = useState('');
  const [savingApiKeyNameId, setSavingApiKeyNameId] = useState<string | null>(null);
  const [editingApiKeyCredentialsId, setEditingApiKeyCredentialsId] = useState<string | null>(null);
  const [editingApiKeyCredentialsValue, setEditingApiKeyCredentialsValue] = useState('');
  const [editingApiBaseUrlCredentialsValue, setEditingApiBaseUrlCredentialsValue] = useState('');
  const [editingApiProviderPresetId, setEditingApiProviderPresetId] = useState(
    DEFAULT_CODEX_API_PROVIDER_ID,
  );
  const [editingManagedProviderId, setEditingManagedProviderId] = useState<string>('');
  const [editingManagedProviderApiKeyId, setEditingManagedProviderApiKeyId] = useState<string>('');
  const [editingNewManagedProviderNameInput, setEditingNewManagedProviderNameInput] = useState('');
  const [savingApiKeyCredentials, setSavingApiKeyCredentials] = useState(false);
  const [quickSwitchAccountId, setQuickSwitchAccountId] = useState<string | null>(null);
  const [quickSwitchProviderId, setQuickSwitchProviderId] = useState<string>('');
  const [quickSwitchApiKeyId, setQuickSwitchApiKeyId] = useState<string>('');
  const [quickSwitchSubmitting, setQuickSwitchSubmitting] = useState(false);
  const [quickSwitchError, setQuickSwitchError] = useState<string | null>(null);
  const [showCodeReviewQuota, setShowCodeReviewQuota] = useState<boolean>(
    isCodexCodeReviewQuotaVisibleByDefault,
  );

  const showAddModalRef = useRef(showAddModal);
  const addTabRef = useRef(addTab);
  const addStatusRef = useRef(addStatus);
  const oauthActiveRef = useRef(false);
  const oauthLoginIdRef = useRef<string | null>(null);
  const oauthCompletingRef = useRef(false);
  const oauthEventSeqRef = useRef(0);
  const oauthAttemptSeqRef = useRef(0);
  const inlineRenameDiscardRef = useRef(false);

  const selectedApiProviderPreset = useMemo(
    () => findCodexApiProviderPresetById(apiProviderPresetId),
    [apiProviderPresetId],
  );
  const selectedEditingApiProviderPreset = useMemo(
    () => findCodexApiProviderPresetById(editingApiProviderPresetId),
    [editingApiProviderPresetId],
  );
  const selectedManagedProvider = useMemo(
    () => managedProviders.find((item) => item.id === managedProviderId) ?? null,
    [managedProviderId, managedProviders],
  );
  const selectedManagedProviderApiKey = useMemo(
    () =>
      selectedManagedProvider?.apiKeys.find((item) => item.id === managedProviderApiKeyId) ?? null,
    [managedProviderApiKeyId, selectedManagedProvider],
  );
  const selectedEditingManagedProvider = useMemo(
    () => managedProviders.find((item) => item.id === editingManagedProviderId) ?? null,
    [editingManagedProviderId, managedProviders],
  );
  const selectedEditingManagedProviderApiKey = useMemo(
    () =>
      selectedEditingManagedProvider?.apiKeys.find(
        (item) => item.id === editingManagedProviderApiKeyId,
      ) ?? null,
    [editingManagedProviderApiKeyId, selectedEditingManagedProvider],
  );
  const quickSwitchAccount = useMemo(
    () =>
      quickSwitchAccountId
        ? accounts.find((item) => item.id === quickSwitchAccountId) ?? null
        : null,
    [accounts, quickSwitchAccountId],
  );
  const selectedQuickSwitchProvider = useMemo(
    () => managedProviders.find((item) => item.id === quickSwitchProviderId) ?? null,
    [managedProviders, quickSwitchProviderId],
  );
  const selectedQuickSwitchApiKey = useMemo(
    () =>
      selectedQuickSwitchProvider?.apiKeys.find((item) => item.id === quickSwitchApiKeyId) ?? null,
    [quickSwitchApiKeyId, selectedQuickSwitchProvider],
  );

  const oauthLog = useCallback((...args: unknown[]) => {
    console.info('[CodexOAuth]', ...args);
  }, []);

  const reloadManagedProviders = useCallback(async () => {
    setManagedProvidersLoading(true);
    try {
      const items = await listCodexModelProviders();
      setManagedProviders(items);
    } catch (err) {
      console.error('[CodexModelProviders] 加载失败', err);
    } finally {
      setManagedProvidersLoading(false);
    }
  }, []);

  const buildApiProviderPayload = useCallback(
    (
      apiBaseUrl: string,
      providerPresetId: string,
      providerId: string,
      customProviderName: string,
    ): {
      apiProviderMode: CodexApiProviderMode;
      apiProviderId?: string;
      apiProviderName?: string;
    } => {
      const normalizedBaseUrl = normalizeHttpBaseUrl(apiBaseUrl);
      if (providerPresetId === OPENAI_OFFICIAL_PRESET_ID || !normalizedBaseUrl) {
        return { apiProviderMode: 'openai_builtin' };
      }

      const managedProvider = findCodexModelProviderById(managedProviders, providerId);
      if (managedProvider) {
        return {
          apiProviderMode: 'custom',
          apiProviderId: managedProvider.id,
          apiProviderName: managedProvider.name,
        };
      }

      const preset = findCodexApiProviderPresetById(providerPresetId);
      if (preset && providerPresetId !== CODEX_API_PROVIDER_CUSTOM_ID) {
        return {
          apiProviderMode: 'custom',
          apiProviderId: preset.id,
          apiProviderName: preset.name,
        };
      }

      const trimmedName = customProviderName.trim();
      return {
        apiProviderMode: 'custom',
        apiProviderName: trimmedName || undefined,
      };
    },
    [managedProviders],
  );

  useEffect(() => {
    showAddModalRef.current = showAddModal;
    addTabRef.current = addTab;
    addStatusRef.current = addStatus;
  }, [showAddModal, addTab, addStatus]);

  useEffect(() => {
    fetchAccounts();
    fetchCurrentAccount();
  }, [fetchAccounts, fetchCurrentAccount]);

  useEffect(() => {
    void reloadManagedProviders();
  }, [reloadManagedProviders]);

  useEffect(() => {
    if (!showAddModal) {
      setApiKeyInput('');
      setApiBaseUrlInput('');
      setApiProviderPresetId(DEFAULT_CODEX_API_PROVIDER_ID);
      setManagedProviderId('');
      setManagedProviderApiKeyId('');
      setNewManagedProviderNameInput('');
    }
  }, [showAddModal]);

  useEffect(() => {
    if (apiProviderPresetId === OPENAI_OFFICIAL_PRESET_ID) {
      setManagedProviderId('');
      setManagedProviderApiKeyId('');
      return;
    }
    const matched = findCodexModelProviderByBaseUrl(managedProviders, apiBaseUrlInput);
    setManagedProviderId((prev) => (prev === (matched?.id ?? '') ? prev : matched?.id ?? ''));
    if (!matched || matched.apiKeys.length === 0) {
      setManagedProviderApiKeyId('');
      return;
    }
    setManagedProviderApiKeyId((prev) => {
      if (matched.apiKeys.some((item) => item.id === prev)) return prev;
      return matched.apiKeys[0]?.id ?? '';
    });
  }, [apiBaseUrlInput, apiProviderPresetId, managedProviders]);

  useEffect(() => {
    if (!selectedManagedProviderApiKey) return;
    setApiKeyInput(selectedManagedProviderApiKey.apiKey);
  }, [managedProviderApiKeyId, selectedManagedProviderApiKey]);

  useEffect(() => {
    if (editingApiProviderPresetId === OPENAI_OFFICIAL_PRESET_ID) {
      setEditingManagedProviderId('');
      setEditingManagedProviderApiKeyId('');
      return;
    }
    const matched = findCodexModelProviderByBaseUrl(
      managedProviders,
      editingApiBaseUrlCredentialsValue,
    );
    setEditingManagedProviderId((prev) => (prev === (matched?.id ?? '') ? prev : matched?.id ?? ''));
    if (!matched || matched.apiKeys.length === 0) {
      setEditingManagedProviderApiKeyId('');
      return;
    }
    setEditingManagedProviderApiKeyId((prev) => {
      if (matched.apiKeys.some((item) => item.id === prev)) return prev;
      return matched.apiKeys[0]?.id ?? '';
    });
  }, [editingApiBaseUrlCredentialsValue, editingApiProviderPresetId, managedProviders]);

  useEffect(() => {
    if (!selectedEditingManagedProviderApiKey) return;
    setEditingApiKeyCredentialsValue(selectedEditingManagedProviderApiKey.apiKey);
  }, [editingManagedProviderApiKeyId, selectedEditingManagedProviderApiKey]);

  useEffect(() => {
    if (!quickSwitchAccountId) return;
    if (accounts.some((item) => item.id === quickSwitchAccountId)) return;
    setQuickSwitchAccountId(null);
    setQuickSwitchProviderId('');
    setQuickSwitchApiKeyId('');
    setQuickSwitchError(null);
  }, [accounts, quickSwitchAccountId]);

  useEffect(() => {
    if (!selectedQuickSwitchProvider) {
      setQuickSwitchApiKeyId('');
      return;
    }
    setQuickSwitchApiKeyId((prev) => {
      if (selectedQuickSwitchProvider.apiKeys.some((item) => item.id === prev)) {
        return prev;
      }
      return selectedQuickSwitchProvider.apiKeys[0]?.id ?? '';
    });
  }, [selectedQuickSwitchProvider]);

  useEffect(() => {
    const syncCodeReviewVisibility = () => {
      setShowCodeReviewQuota(isCodexCodeReviewQuotaVisibleByDefault());
    };

    window.addEventListener(
      CODEX_CODE_REVIEW_QUOTA_VISIBILITY_CHANGED_EVENT,
      syncCodeReviewVisibility as EventListener,
    );
    return () => {
      window.removeEventListener(
        CODEX_CODE_REVIEW_QUOTA_VISIBILITY_CHANGED_EVENT,
        syncCodeReviewVisibility as EventListener,
      );
    };
  }, []);

  // Hook provides setAddStatus/setAddMessage but we need refs to page's versions
  const { setAddStatus, setAddMessage, resetAddModalState, setShowAddModal } = page;

  const handleOauthPrepareError = useCallback((e: unknown) => {
    console.error('[CodexOAuth] 准备授权链接失败', { error: String(e) });
    oauthActiveRef.current = false;
    setOauthTimeoutInfo(null);
    setOauthCallbackSubmitting(false);
    setOauthCallbackError(null);
    setOauthTokenExchangeRetryVisible(false);
    const match = String(e).match(/CODEX_OAUTH_PORT_IN_USE:(\d+)/);
    if (match) {
      const port = Number(match[1]);
      setOauthPortInUse(Number.isNaN(port) ? null : port);
      setOauthPrepareError(t('codex.oauth.portInUse', { port: match[1] }));
      return;
    }
    setOauthPrepareError(t('common.shared.oauth.failed', '授权失败') + ': ' + String(e));
  }, [t]);

  const completeOauthSuccess = useCallback(async () => {
    oauthLog('授权完成并保存成功', { loginId: oauthLoginIdRef.current });
    await fetchAccounts();
    await fetchCurrentAccount();
    await emitAccountsChanged({
      platformId: 'codex',
      reason: 'oauth',
    });
    setAddStatus('success');
    setAddMessage(t('common.shared.oauth.success', '授权成功'));
    oauthActiveRef.current = false;
    oauthCompletingRef.current = false;
    oauthLoginIdRef.current = null;
    setOauthUrl('');
    setOauthUrlCopied(false);
    setOauthPrepareError(null);
    setOauthPortInUse(null);
    setOauthTimeoutInfo(null);
    setOauthCallbackInput('');
    setOauthCallbackSubmitting(false);
    setOauthCallbackError(null);
    setOauthTokenExchangeRetryVisible(false);
    setTimeout(() => {
      setShowAddModal(false);
      resetAddModalState();
    }, 1200);
  }, [fetchAccounts, fetchCurrentAccount, t, oauthLog, setAddStatus, setAddMessage, setShowAddModal, resetAddModalState]);

  const completeOauthError = useCallback((e: unknown, allowTokenExchangeRetry = false) => {
    setAddStatus('error');
    setAddMessage(t('common.shared.oauth.failed', '授权失败') + ': ' + String(e));
    setOauthTokenExchangeRetryVisible(allowTokenExchangeRetry);
  }, [t, setAddStatus, setAddMessage]);

  const isOauthTimeoutState = useMemo(() => !!oauthTimeoutInfo, [oauthTimeoutInfo]);
  const isOauthTokenExchangeErrorState = useMemo(() => {
    return addStatus === 'error' && oauthTokenExchangeRetryVisible;
  }, [addStatus, oauthTokenExchangeRetryVisible]);

  useEffect(() => {
    let unlistenExtension: UnlistenFn | undefined;
    let unlistenTimeout: UnlistenFn | undefined;
    let disposed = false;

    listen<{ loginId?: string }>('codex-oauth-login-completed', async (event) => {
      ++oauthEventSeqRef.current;
      if (!showAddModalRef.current || addTabRef.current !== 'oauth' || addStatusRef.current === 'loading' || oauthCompletingRef.current) return;
      const loginId = event.payload?.loginId;
      if (!loginId) return;
      if (oauthLoginIdRef.current && oauthLoginIdRef.current !== loginId) return;
      ++oauthAttemptSeqRef.current;
      setAddStatus('loading');
      setAddMessage(t('codex.oauth.exchanging', '正在交换令牌...'));
      oauthCompletingRef.current = true;
      try {
        await codexService.completeCodexOAuthLogin(loginId);
        await completeOauthSuccess();
      } catch (e) {
        completeOauthError(e, true);
      } finally {
        oauthCompletingRef.current = false;
      }
    }).then((fn) => { if (disposed) fn(); else unlistenExtension = fn; });

    listen<{ loginId?: string; callbackUrl?: string; timeoutSeconds?: number }>('codex-oauth-login-timeout', async (event) => {
      if (!showAddModalRef.current || addTabRef.current !== 'oauth') return;
      const payload = event.payload ?? {};
      const loginId = payload.loginId;
      if (oauthLoginIdRef.current && loginId && oauthLoginIdRef.current !== loginId) return;
      oauthActiveRef.current = false;
      setOauthUrlCopied(false);
      setOauthPortInUse(null);
      setOauthTimeoutInfo(payload);
      setOauthPrepareError(null);
      setOauthCallbackSubmitting(false);
      setOauthCallbackError(null);
      setOauthTokenExchangeRetryVisible(false);
      setAddStatus('idle');
      setAddMessage('');
    }).then((fn) => { if (disposed) fn(); else unlistenTimeout = fn; });

    return () => { disposed = true; unlistenExtension?.(); unlistenTimeout?.(); };
  }, [completeOauthError, completeOauthSuccess, t, setAddStatus, setAddMessage]);

  const prepareOauthUrl = useCallback(() => {
    if (!showAddModalRef.current || addTabRef.current !== 'oauth') return;
    if (oauthActiveRef.current) return;
    const attemptSeq = ++oauthAttemptSeqRef.current;
    oauthActiveRef.current = true;
    setOauthPrepareError(null);
    setOauthPortInUse(null);
    setOauthTimeoutInfo(null);
    setOauthCallbackInput('');
    setOauthCallbackSubmitting(false);
    setOauthCallbackError(null);
    setOauthTokenExchangeRetryVisible(false);

    codexService.startCodexOAuthLogin()
      .then(({ loginId, authUrl }) => {
        if (attemptSeq !== oauthAttemptSeqRef.current) {
          if (loginId) {
            codexService.cancelCodexOAuthLogin(loginId).catch(() => { });
          }
          oauthLog('忽略过期 OAuth start 响应', { loginId, attemptSeq });
          return;
        }
        oauthLoginIdRef.current = loginId ?? null;
        if (typeof authUrl === 'string' && authUrl.length > 0 && showAddModalRef.current && addTabRef.current === 'oauth') {
          setOauthUrl(authUrl);
        } else {
          oauthActiveRef.current = false;
        }
      })
      .catch((e) => {
        if (attemptSeq !== oauthAttemptSeqRef.current) {
          oauthLog('忽略过期 OAuth start 异常回调', {
            attemptSeq,
            error: String(e),
          });
          return;
        }
        handleOauthPrepareError(e);
      });
  }, [handleOauthPrepareError, oauthLog]);

  useEffect(() => {
    if (!showAddModal || addTab !== 'oauth' || oauthUrl || oauthTimeoutInfo) return;
    prepareOauthUrl();
  }, [showAddModal, addTab, oauthUrl, oauthTimeoutInfo, prepareOauthUrl]);

  useEffect(() => {
    if (showAddModal && addTab === 'oauth') return;
    const loginId = oauthLoginIdRef.current ?? undefined;
    const hasOauthUiResidue = Boolean(oauthUrl)
      || Boolean(oauthTimeoutInfo)
      || oauthCallbackInput.length > 0
      || oauthCallbackSubmitting
      || Boolean(oauthCallbackError)
      || Boolean(oauthPrepareError)
      || oauthPortInUse !== null
      || oauthUrlCopied;
    if (!loginId && !oauthActiveRef.current && !oauthCompletingRef.current && !hasOauthUiResidue) return;
    oauthAttemptSeqRef.current += 1;
    if (loginId) {
      codexService.cancelCodexOAuthLogin(loginId).catch(() => { });
    }
    oauthActiveRef.current = false;
    oauthCompletingRef.current = false;
    oauthLoginIdRef.current = null;
    setOauthUrl('');
    setOauthUrlCopied(false);
    setOauthTimeoutInfo(null);
    setOauthCallbackInput('');
    setOauthCallbackSubmitting(false);
    setOauthCallbackError(null);
    setOauthTokenExchangeRetryVisible(false);
  }, [
    showAddModal,
    addTab,
    oauthUrl,
    oauthTimeoutInfo,
    oauthCallbackInput,
    oauthCallbackSubmitting,
    oauthCallbackError,
    oauthPrepareError,
    oauthPortInUse,
    oauthUrlCopied,
    oauthTokenExchangeRetryVisible,
  ]);

  useEffect(
    () => () => {
      oauthAttemptSeqRef.current += 1;
      const loginId = oauthLoginIdRef.current ?? undefined;
      if (loginId) {
        oauthLog('页面卸载，准备取消授权流程', { loginId });
        codexService.cancelCodexOAuthLogin(loginId).catch(() => { });
      }
      oauthActiveRef.current = false;
      oauthCompletingRef.current = false;
      oauthLoginIdRef.current = null;
    },
    [oauthLog],
  );

  const handleCopyOauthUrl = async () => {
    if (!oauthUrl) return;
    try { await navigator.clipboard.writeText(oauthUrl); setOauthUrlCopied(true); setTimeout(() => setOauthUrlCopied(false), 1200); } catch { }
  };

  const handleReleaseOauthPort = async () => {
    const port = oauthPortInUse;
    if (!port) return;
    const confirmed = await confirmDialog(t('codex.oauth.portInUseConfirm', { port }), { title: t('codex.oauth.portInUseTitle'), kind: 'warning', okLabel: t('common.confirm'), cancelLabel: t('common.cancel') });
    if (!confirmed) return;
    setOauthPrepareError(null);
    try { await codexService.closeCodexOAuthPort(); } catch (e) { setOauthPrepareError(t('codex.oauth.portCloseFailed', { error: String(e) })); setOauthPortInUse(port); return; }
    prepareOauthUrl();
  };

  const handleRetryOauthAfterTimeout = () => {
    oauthActiveRef.current = false;
    oauthLoginIdRef.current = null;
    setOauthTimeoutInfo(null);
    setOauthPrepareError(null);
    setOauthPortInUse(null);
    setOauthUrl('');
    setOauthUrlCopied(false);
    setOauthCallbackInput('');
    setOauthCallbackSubmitting(false);
    setOauthCallbackError(null);
    setOauthTokenExchangeRetryVisible(false);
    prepareOauthUrl();
  };

  const handleOpenOauthUrl = async () => {
    if (!oauthUrl) return;
    try { await openUrl(oauthUrl); } catch { await navigator.clipboard.writeText(oauthUrl).catch(() => { }); setOauthUrlCopied(true); setTimeout(() => setOauthUrlCopied(false), 1200); }
  };

  const handleSubmitOauthCallbackUrl = async () => {
    const callbackUrl = oauthCallbackInput.trim();
    if (!callbackUrl) return;
    const loginId = oauthLoginIdRef.current;
    if (!loginId) {
      setOauthCallbackError(t('common.shared.oauth.failed', '授权失败'));
      return;
    }

    setOauthCallbackSubmitting(true);
    setOauthCallbackError(null);
    setOauthTokenExchangeRetryVisible(false);
    oauthCompletingRef.current = true;
    let tokenExchangeStarted = false;
    try {
      await codexService.submitCodexOAuthCallbackUrl(loginId, callbackUrl);
      setAddStatus('loading');
      setAddMessage(t('codex.oauth.exchanging', '正在交换令牌...'));
      tokenExchangeStarted = true;
      await codexService.completeCodexOAuthLogin(loginId);
      await completeOauthSuccess();
    } catch (e) {
      completeOauthError(e, tokenExchangeStarted);
      setOauthCallbackError(String(e).replace(/^Error:\s*/, ''));
    } finally {
      oauthCompletingRef.current = false;
      setOauthCallbackSubmitting(false);
    }
  };

  const handleRetryOauthTokenExchange = async () => {
    const loginId = oauthLoginIdRef.current;
    if (!loginId || oauthCompletingRef.current) return;
    setOauthCallbackSubmitting(true);
    setOauthCallbackError(null);
    setOauthTokenExchangeRetryVisible(false);
    setAddStatus('loading');
    setAddMessage(t('codex.oauth.exchanging', '正在交换令牌...'));
    oauthCompletingRef.current = true;
    try {
      await codexService.completeCodexOAuthLogin(loginId);
      await completeOauthSuccess();
    } catch (e) {
      completeOauthError(e, true);
      setOauthCallbackError(String(e).replace(/^Error:\s*/, ''));
    } finally {
      oauthCompletingRef.current = false;
      setOauthCallbackSubmitting(false);
    }
  };

  // ─── Codex-specific: Switch / Import ─────────────────────────────────

  const handleSwitch = async (accountId: string) => {
    setMessage(null);
    setSwitching(accountId);
    try {
      const account = await switchAccount(accountId);
      setMessage({ text: t('codex.switched', { email: maskAccountText(account.email) }) });
    } catch (e) {
      setMessage({ text: t('codex.switchFailed', { error: String(e) }), tone: 'error' });
    }
    setSwitching(null);
  };

  const handleImportFromLocal = async () => {
    page.setAddStatus('loading');
    page.setAddMessage(t('codex.import.importing', '正在导入本地账号...'));
    try {
      const account = await codexService.importCodexFromLocal();
      await fetchAccounts();
      await new Promise((resolve) => setTimeout(resolve, 180));
      await fetchAccounts();
      await emitAccountsChanged({
        platformId: 'codex',
        reason: 'import',
      });
      try { await refreshQuota(account.id); await fetchAccounts(); } catch { }
      page.setAddStatus('success');
      page.setAddMessage(t('codex.import.successMsg', '导入成功: {{email}}').replace('{{email}}', maskAccountText(account.email)));
      setTimeout(() => { closeAddModal(); }, 1200);
    } catch (e) {
      page.setAddStatus('error');
      page.setAddMessage(t('common.shared.import.failedMsg', '导入失败: {{error}}').replace('{{error}}', String(e).replace(/^Error:\s*/, '')));
    }
  };

  const handleImportFromFiles = async () => {
    let unlistenProgress: UnlistenFn | undefined;
    try {
      const selected = await openFileDialog({
        multiple: true,
        filters: [{ name: 'JSON', extensions: ['json'] }],
      });
      if (!selected || (Array.isArray(selected) && selected.length === 0)) return;
      const paths = Array.isArray(selected) ? selected : [selected];
      page.setAddStatus('loading');
      page.setAddMessage(t('modals.import.importingFiles', { count: paths.length }));

      unlistenProgress = await listen<{ current: number; total: number; email: string }>(
        'codex:file-import-progress',
        (event) => {
          const { current, total, email } = event.payload ?? {};
          if (current > 0 && total > 0) {
            const label = email ? ` ${email}` : '';
            page.setAddMessage(`${t('modals.import.importingFiles', { count: total })} ${current}/${total}${label}`);
          }
        }
      );

      const result = await codexService.importCodexFromFiles(paths);
      const { imported, failed } = result;
      await fetchAccounts();
      if (imported.length > 0) {
        await emitAccountsChanged({
          platformId: 'codex',
          reason: 'import',
        });
      }
      if (imported.length === 0 && failed.length === 0) {
        page.setAddStatus('error');
        page.setAddMessage(t('modals.import.noAccountsFound'));
      } else if (failed.length > 0) {
        const failedList = failed.map((f) => f.email).join(', ');
        page.setAddStatus(imported.length > 0 ? 'success' : 'error');
        page.setAddMessage(
          `${t('messages.importSuccess', { count: imported.length })}，${t('messages.importPartialFailed', { failCount: failed.length, failList: failedList })}`
        );
      } else {
        page.setAddStatus('success');
        page.setAddMessage(t('messages.importSuccess', { count: imported.length }));
      }
      // 后台刷新配额，带进度显示，可关闭弹窗
      if (imported.length > 0) {
        const total = imported.length;
        let done = 0;
        const refreshAll = async () => {
          for (const acc of imported) {
            await refreshQuota(acc.id).catch(() => { });
            done++;
            page.setAddStatus('loading');
            page.setAddMessage(t('messages.refreshingQuota', { done, total }));
            // 每 5 个刷新一次列表，让 UI 实时更新
            if (done % 5 === 0) await fetchAccounts();
          }
          await fetchAccounts();
          page.setAddStatus('success');
          page.setAddMessage(`${t('messages.importSuccess', { count: total })}，${t('messages.quotaRefreshDone')}`);
        };
        refreshAll();
      }
    } catch (e) {
      page.setAddStatus('error');
      page.setAddMessage(t('messages.importFailed', { error: String(e) }));
    } finally {
      if (unlistenProgress) unlistenProgress();
    }
  };

  const handleSelectApiProviderPreset = useCallback((providerId: string) => {
    setApiProviderPresetId(providerId);
    const preset = findCodexApiProviderPresetById(providerId);
    if (!preset || preset.baseUrls.length === 0) return;
    setApiBaseUrlInput(preset.baseUrls[0]);
  }, []);

  const handleSelectManagedProvider = useCallback(
    (providerId: string) => {
      setApiProviderPresetId(CODEX_API_PROVIDER_CUSTOM_ID);
      setManagedProviderId(providerId);
      const provider = managedProviders.find((item) => item.id === providerId);
      if (!provider) return;
      setApiBaseUrlInput(provider.baseUrl);
      const firstKey = provider.apiKeys[0];
      if (firstKey) {
        setManagedProviderApiKeyId(firstKey.id);
        setApiKeyInput(firstKey.apiKey);
      } else {
        setManagedProviderApiKeyId('');
      }
      setNewManagedProviderNameInput(provider.name);
    },
    [managedProviders],
  );

  const handleSelectManagedProviderApiKey = useCallback(
    (apiKeyId: string) => {
      setManagedProviderApiKeyId(apiKeyId);
      const key = selectedManagedProvider?.apiKeys.find((item) => item.id === apiKeyId);
      if (key) {
        setApiKeyInput(key.apiKey);
      }
    },
    [selectedManagedProvider],
  );

  const handleSelectEditingApiProviderPreset = useCallback((providerId: string) => {
    setEditingApiProviderPresetId(providerId);
    const preset = findCodexApiProviderPresetById(providerId);
    if (!preset || preset.baseUrls.length === 0) return;
    setEditingApiBaseUrlCredentialsValue(preset.baseUrls[0]);
  }, []);

  const handleSelectEditingManagedProvider = useCallback(
    (providerId: string) => {
      setEditingApiProviderPresetId(CODEX_API_PROVIDER_CUSTOM_ID);
      setEditingManagedProviderId(providerId);
      const provider = managedProviders.find((item) => item.id === providerId);
      if (!provider) return;
      setEditingApiBaseUrlCredentialsValue(provider.baseUrl);
      const firstKey = provider.apiKeys[0];
      if (firstKey) {
        setEditingManagedProviderApiKeyId(firstKey.id);
        setEditingApiKeyCredentialsValue(firstKey.apiKey);
      } else {
        setEditingManagedProviderApiKeyId('');
      }
      setEditingNewManagedProviderNameInput(provider.name);
    },
    [managedProviders],
  );

  const handleSelectEditingManagedProviderApiKey = useCallback(
    (apiKeyId: string) => {
      setEditingManagedProviderApiKeyId(apiKeyId);
      const key = selectedEditingManagedProvider?.apiKeys.find((item) => item.id === apiKeyId);
      if (key) {
        setEditingApiKeyCredentialsValue(key.apiKey);
      }
    },
    [selectedEditingManagedProvider],
  );

  const closeQuickSwitchModal = useCallback(() => {
    if (quickSwitchSubmitting) return;
    setQuickSwitchAccountId(null);
    setQuickSwitchProviderId('');
    setQuickSwitchApiKeyId('');
    setQuickSwitchError(null);
  }, [quickSwitchSubmitting]);

  const openQuickSwitchProviderModal = useCallback(
    (account: CodexAccount) => {
      if (!isCodexApiKeyAccount(account)) return;
      const baseUrl = (account.api_base_url || '').trim();
      const apiKey = (account.openai_api_key || '').trim();
      const matchedProvider =
        findCodexModelProviderById(managedProviders, account.api_provider_id) ??
        findCodexModelProviderByBaseUrl(managedProviders, baseUrl);
      const fallbackProvider = matchedProvider ?? managedProviders[0] ?? null;
      const matchedApiKey = matchedProvider?.apiKeys.find((item) => item.apiKey.trim() === apiKey);
      const fallbackApiKey = matchedApiKey ?? fallbackProvider?.apiKeys[0] ?? null;

      setQuickSwitchAccountId(account.id);
      setQuickSwitchProviderId(fallbackProvider?.id ?? '');
      setQuickSwitchApiKeyId(fallbackApiKey?.id ?? '');
      setQuickSwitchError(null);
    },
    [managedProviders],
  );

  const handleSelectQuickSwitchProvider = useCallback(
    (providerId: string) => {
      setQuickSwitchProviderId(providerId);
      const provider = managedProviders.find((item) => item.id === providerId);
      setQuickSwitchApiKeyId(provider?.apiKeys[0]?.id ?? '');
      setQuickSwitchError(null);
    },
    [managedProviders],
  );

  const handleSelectQuickSwitchApiKey = useCallback((apiKeyId: string) => {
    setQuickSwitchApiKeyId(apiKeyId);
    setQuickSwitchError(null);
  }, []);

  const handleSubmitQuickSwitch = useCallback(async () => {
    if (!quickSwitchAccount) return;
    if (!selectedQuickSwitchProvider) {
      setQuickSwitchError(t('codex.quickSwitch.validation.providerRequired', '请选择供应商'));
      return;
    }
    if (!selectedQuickSwitchApiKey) {
      setQuickSwitchError(t('codex.quickSwitch.validation.apiKeyRequired', '请选择 API Key'));
      return;
    }

    setQuickSwitchSubmitting(true);
    setQuickSwitchError(null);
    try {
      await updateApiKeyCredentials(
        quickSwitchAccount.id,
        selectedQuickSwitchApiKey.apiKey,
        selectedQuickSwitchProvider.baseUrl,
        'custom',
        selectedQuickSwitchProvider.id,
        selectedQuickSwitchProvider.name,
      );
      setMessage({
        text: t('codex.quickSwitch.success', {
          defaultValue: '已切换到供应商：{{provider}}',
          provider: selectedQuickSwitchProvider.name,
        }),
      });
      setQuickSwitchAccountId(null);
      setQuickSwitchProviderId('');
      setQuickSwitchApiKeyId('');
      setQuickSwitchError(null);
    } catch (err) {
      setQuickSwitchError(
        t('codex.quickSwitch.failed', {
          defaultValue: '切换供应商失败：{{error}}',
          error: String(err).replace(/^Error:\s*/, ''),
        }),
      );
    } finally {
      setQuickSwitchSubmitting(false);
    }
  }, [
    quickSwitchAccount,
    selectedQuickSwitchApiKey,
    selectedQuickSwitchProvider,
    setMessage,
    t,
    updateApiKeyCredentials,
  ]);

  const handleOpenProviderLink = useCallback(async (url: string) => {
    try {
      await openUrl(url);
    } catch {
      await navigator.clipboard.writeText(url).catch(() => { });
    }
  }, []);

  const handleApiKeyLogin = async (switchAfterAdd = false) => {
    const validation = validateApiKeyCredentialInputs(apiKeyInput, apiBaseUrlInput);
    if (!validation.ok) {
      page.setAddStatus('error');
      page.setAddMessage(validation.message);
      return;
    }
    const providerPayload = buildApiProviderPayload(
      apiBaseUrlInput,
      apiProviderPresetId,
      managedProviderId,
      newManagedProviderNameInput,
    );

    page.setAddStatus('loading');
    page.setAddMessage(t('common.shared.token.importing', '正在导入...'));
    try {
      const account = await codexService.addCodexAccountWithApiKey(
        validation.apiKey,
        validation.apiBaseUrl,
        providerPayload.apiProviderMode,
        providerPayload.apiProviderId,
        providerPayload.apiProviderName,
      );
      if (validation.apiBaseUrl && providerPayload.apiProviderMode === 'custom') {
        try {
          await upsertCodexModelProviderFromCredential({
            providerId: providerPayload.apiProviderId ?? null,
            providerName: providerPayload.apiProviderName ?? null,
            apiBaseUrl: validation.apiBaseUrl,
            apiKey: validation.apiKey,
          });
          await reloadManagedProviders();
        } catch (providerErr) {
          console.warn('[CodexModelProviders] 添加账号后写入供应商失败', providerErr);
        }
      }
      if (switchAfterAdd) {
        await switchAccount(account.id);
      } else {
        await fetchAccounts();
        await fetchCurrentAccount();
      }
      await emitAccountsChanged({
        platformId: 'codex',
        reason: 'import',
      });
      page.setAddStatus('success');
      page.setAddMessage(
        switchAfterAdd
          ? t('codex.api.addAndSwitchSuccess', '添加并切换成功: {{email}}').replace(
            '{{email}}',
            maskAccountText(account.email),
          )
          : t('codex.import.successMsg', '导入成功: {{email}}').replace(
            '{{email}}',
            maskAccountText(account.email),
          ),
      );
      setApiKeyInput('');
      setApiBaseUrlInput('');
      setApiProviderPresetId(DEFAULT_CODEX_API_PROVIDER_ID);
      setManagedProviderId('');
      setManagedProviderApiKeyId('');
      setNewManagedProviderNameInput('');
      setTimeout(() => {
        closeAddModal();
      }, 1200);
    } catch (e) {
      page.setAddStatus('error');
      page.setAddMessage(
        t('common.shared.token.importFailedMsg', '导入失败: {{error}}').replace(
          '{{error}}',
          String(e).replace(/^Error:\s*/, ''),
        ),
      );
    }
  };

  const handleTokenImport = async () => {
    const trimmed = tokenInput.trim();
    if (!trimmed) { page.setAddStatus('error'); page.setAddMessage(t('common.shared.token.empty', '请输入 Token 或 JSON')); return; }
    page.setAddStatus('loading');
    page.setAddMessage(t('common.shared.token.importing', '正在导入...'));
    try {
      const imported = await codexService.importCodexFromJson(trimmed);
      await fetchAccounts();
      if (imported.length > 0) {
        await emitAccountsChanged({
          platformId: 'codex',
          reason: 'import',
        });
      }
      for (const acc of imported) { await refreshQuota(acc.id).catch(() => { }); }
      await fetchAccounts();
      page.setAddStatus('success');
      page.setAddMessage(t('common.shared.token.importSuccessMsg', '成功导入 {{count}} 个账号').replace('{{count}}', String(imported.length)));
      setTimeout(() => { closeAddModal(); }, 1200);
    } catch (e) {
      page.setAddStatus('error');
      page.setAddMessage(t('common.shared.token.importFailedMsg', '导入失败: {{error}}').replace('{{error}}', String(e).replace(/^Error:\s*/, '')));
    }
  };

  const handleOpenCodexUsage = useCallback(async () => {
    try {
      await openUrl(CODEX_USAGE_URL);
    } catch (e) {
      setMessage({
        text: t('codex.usage.openFailed', { error: String(e) }),
        tone: 'error',
      });
    }
  }, [setMessage, t]);

  const clearInlineRename = useCallback(() => {
    setEditingApiKeyNameId(null);
    setEditingApiKeyNameValue('');
  }, []);

  const handleAccountNameDoubleClick = useCallback((account: CodexAccount) => {
    if (!isCodexApiKeyAccount(account)) return;
    inlineRenameDiscardRef.current = false;
    setEditingApiKeyNameId(account.id);
    setEditingApiKeyNameValue((account.account_name || account.email || '').trim());
  }, []);

  const handleSubmitInlineRename = useCallback(
    async (account: CodexAccount) => {
      if (inlineRenameDiscardRef.current) {
        inlineRenameDiscardRef.current = false;
        return;
      }
      if (!isCodexApiKeyAccount(account)) return;
      if (editingApiKeyNameId !== account.id) return;

      const nextName = editingApiKeyNameValue.trim();
      const currentName = (account.account_name || '').trim();
      const fallbackName = (account.email || '').trim();
      const unchanged = nextName === currentName || (!currentName && nextName === fallbackName);
      if (unchanged) {
        clearInlineRename();
        return;
      }

      setSavingApiKeyNameId(account.id);
      try {
        await updateAccountName(account.id, nextName);
        setMessage({ text: t('fingerprints.messages.renamed', '已重命名') });
      } catch (e) {
        setMessage({
          text: `${t('fingerprints.messages.renameFailed', '重命名失败')}: ${String(e)}`,
          tone: 'error',
        });
      } finally {
        setSavingApiKeyNameId(null);
        clearInlineRename();
      }
    },
    [
      clearInlineRename,
      editingApiKeyNameId,
      editingApiKeyNameValue,
      setMessage,
      t,
      updateAccountName,
    ],
  );

  const resolveApiKeyDisplayText = useCallback(
    (account: CodexAccount) => (account.openai_api_key || '').trim() || t('common.none', '暂无'),
    [t],
  );

  const resolveApiProviderDisplayName = useCallback(
    (account: CodexAccount): string => {
      const providerMode = inferCodexAccountProviderMode(account);
      if (providerMode === 'openai_builtin') {
        const fallback = findCodexApiProviderPresetById(OPENAI_OFFICIAL_PRESET_ID);
        return fallback
          ? t(`codex.api.providers.${fallback.id}.name`, fallback.name)
          : t('common.none', '暂无');
      }
      if (account.api_provider_name?.trim()) {
        return account.api_provider_name.trim();
      }
      const baseUrl = (account.api_base_url || '').trim();
      const matchedProvider = findCodexModelProviderByBaseUrl(managedProviders, baseUrl);
      if (matchedProvider) return matchedProvider.name;
      const preset = findCodexApiProviderPresetById(resolveCodexApiProviderPresetId(baseUrl));
      if (preset) return t(`codex.api.providers.${preset.id}.name`, preset.name);
      return t('codex.api.provider.custom', '自定义');
    },
    [managedProviders, t],
  );

  const closeApiKeyCredentialsModal = useCallback(() => {
    if (savingApiKeyCredentials) return;
    setEditingApiKeyCredentialsId(null);
    setEditingApiKeyCredentialsValue('');
    setEditingApiBaseUrlCredentialsValue('');
    setEditingApiProviderPresetId(DEFAULT_CODEX_API_PROVIDER_ID);
    setEditingManagedProviderId('');
    setEditingManagedProviderApiKeyId('');
    setEditingNewManagedProviderNameInput('');
  }, [savingApiKeyCredentials]);

  const openApiKeyCredentialsModal = useCallback((account: CodexAccount) => {
    if (!isCodexApiKeyAccount(account)) return;
    const initialBaseUrl = (account.api_base_url || '').trim();
    const initialApiKey = (account.openai_api_key || '').trim();
    const providerMode = inferCodexAccountProviderMode(account);
    const matchedProvider =
      findCodexModelProviderById(managedProviders, account.api_provider_id) ??
      findCodexModelProviderByBaseUrl(managedProviders, initialBaseUrl);
    const matchedProviderKey = matchedProvider?.apiKeys.find(
      (item) => item.apiKey.trim() === initialApiKey,
    );

    setEditingApiKeyCredentialsId(account.id);
    setEditingApiKeyCredentialsValue(initialApiKey);
    setEditingApiBaseUrlCredentialsValue(initialBaseUrl);
    setEditingApiProviderPresetId(
      providerMode === 'openai_builtin'
        ? OPENAI_OFFICIAL_PRESET_ID
        : resolveCodexApiProviderPresetId(initialBaseUrl),
    );
    setEditingManagedProviderId(matchedProvider?.id ?? '');
    setEditingManagedProviderApiKeyId(matchedProviderKey?.id ?? '');
    setEditingNewManagedProviderNameInput(
      matchedProvider?.name ?? account.api_provider_name ?? '',
    );
  }, [managedProviders]);

  const handleSubmitApiKeyCredentials = useCallback(async () => {
    const accountId = editingApiKeyCredentialsId;
    if (!accountId) return;

    const validation = validateApiKeyCredentialInputs(
      editingApiKeyCredentialsValue,
      editingApiBaseUrlCredentialsValue,
    );
    if (!validation.ok) {
      setMessage({
        text: validation.message,
        tone: 'error',
      });
      return;
    }
    const providerPayload = buildApiProviderPayload(
      editingApiBaseUrlCredentialsValue,
      editingApiProviderPresetId,
      editingManagedProviderId,
      editingNewManagedProviderNameInput,
    );

    setSavingApiKeyCredentials(true);
    try {
      await updateApiKeyCredentials(
        accountId,
        validation.apiKey,
        validation.apiBaseUrl,
        providerPayload.apiProviderMode,
        providerPayload.apiProviderId,
        providerPayload.apiProviderName,
      );
      if (validation.apiBaseUrl && providerPayload.apiProviderMode === 'custom') {
        try {
          await upsertCodexModelProviderFromCredential({
            providerId: providerPayload.apiProviderId ?? null,
            providerName: providerPayload.apiProviderName ?? null,
            apiBaseUrl: validation.apiBaseUrl,
            apiKey: validation.apiKey,
          });
          await reloadManagedProviders();
        } catch (providerErr) {
          console.warn('[CodexModelProviders] 更新凭据后写入供应商失败', providerErr);
        }
      }
      setMessage({ text: t('instances.messages.updated', '实例已更新') });
      setEditingApiKeyCredentialsId(null);
      setEditingApiKeyCredentialsValue('');
      setEditingApiBaseUrlCredentialsValue('');
      setEditingApiProviderPresetId(DEFAULT_CODEX_API_PROVIDER_ID);
      setEditingManagedProviderId('');
      setEditingManagedProviderApiKeyId('');
      setEditingNewManagedProviderNameInput('');
    } catch (e) {
      setMessage({
        text: `${t('common.failed', '失败')}: ${String(e)}`,
        tone: 'error',
      });
    } finally {
      setSavingApiKeyCredentials(false);
    }
  }, [
    buildApiProviderPayload,
    editingApiBaseUrlCredentialsValue,
    editingApiKeyCredentialsId,
    editingApiKeyCredentialsValue,
    editingApiProviderPresetId,
    editingManagedProviderId,
    editingNewManagedProviderNameInput,
    reloadManagedProviders,
    setMessage,
    t,
    upsertCodexModelProviderFromCredential,
    updateApiKeyCredentials,
    validateApiKeyCredentialInputs,
  ]);

  // ─── Platform-specific: Presentation ─────────────────────────────────

  const resolveQuotaErrorMeta = useCallback((quotaError?: CodexQuotaErrorInfo) => {
    if (!quotaError?.message) return { statusCode: '', errorCode: '', displayText: '', rawMessage: '' };
    const rawMessage = quotaError.message;
    const normalizedRawMessage = rawMessage.trim();
    const lowerRawMessage = normalizedRawMessage.toLowerCase();
    const requestErrorIndex = lowerRawMessage.indexOf('error sending request');
    const requestErrorMessage =
      requestErrorIndex >= 0
        ? normalizedRawMessage.slice(requestErrorIndex).trim()
        : normalizedRawMessage;
    const statusCode = rawMessage.match(/API 返回错误\s+(\d{3})/i)?.[1] || rawMessage.match(/status[=: ]+(\d{3})/i)?.[1] || '';
    const errorCode = quotaError.code || rawMessage.match(/\[error_code:([^\]]+)\]/)?.[1] || '';
    const displayText = errorCode
      || (requestErrorIndex >= 0
        ? t('codex.quotaError.requestFailedManualRetry', { error: requestErrorMessage })
        : normalizedRawMessage);
    return { statusCode, errorCode, displayText, rawMessage };
  }, [t]);

  const shouldOfferReauthorizeAction = useCallback(
    (quotaErrorMeta: { statusCode: string; errorCode: string; rawMessage: string }) => {
      const statusCode = quotaErrorMeta.statusCode.trim();
      const errorCode = quotaErrorMeta.errorCode.trim().toLowerCase();
      const rawMessage = quotaErrorMeta.rawMessage.trim().toLowerCase();
      if (!statusCode && !errorCode && !rawMessage) return false;

      return statusCode === '401'
        || errorCode === 'token_invalidated'
        || errorCode === 'invalid_grant'
        || errorCode === 'invalid_token'
        || rawMessage.includes('token_invalidated')
        || rawMessage.includes('your authentication token has been invalidated')
        || rawMessage.includes('401 unauthorized')
        || rawMessage.includes('invalid_grant')
        || rawMessage.includes('token 已过期且无 refresh_token')
        || rawMessage.includes('token 已过期且刷新失败')
        || rawMessage.includes('刷新 token 失败');
    },
    [],
  );

  const accountPresentations = useMemo(() => {
    const map = new Map<string, ReturnType<typeof buildCodexAccountPresentation>>();
    accounts.forEach((a) => map.set(a.id, buildCodexAccountPresentation(a, t)));
    return map;
  }, [accounts, t]);

  const resolvePresentation = useCallback(
    (account: CodexAccount) => accountPresentations.get(account.id) ?? buildCodexAccountPresentation(account, t),
    [accountPresentations, t],
  );

  const resolveSingleExportBaseName = useCallback(
    (account: CodexAccount) => {
      const display = (resolvePresentation(account).displayName || account.id).trim();
      const atIndex = display.indexOf('@');
      return atIndex > 0 ? display.slice(0, atIndex) : display;
    },
    [resolvePresentation],
  );

  const resolvePlanKey = useCallback(
    (account: CodexAccount) => resolvePresentation(account).planClass.toUpperCase(),
    [resolvePresentation],
  );

  const accountIdLabel = t('kiro.account.userId', 'User ID');

  const accountMetaMap = useMemo(() => {
    const map = new Map<
      string,
      {
        chatgptAccountId: string;
        signedInWithText: string;
        userId: string;
        accountContextText: string;
      }
    >();
    const noneText = t('common.none', '暂无');

    accounts.forEach((account) => {
      if (isCodexApiKeyAccount(account)) {
        map.set(account.id, {
          chatgptAccountId: t('common.none', '暂无'),
          signedInWithText: '',
          userId: '',
          accountContextText: '',
        });
        return;
      }

      const metadata = getCodexAuthMetadata(account);
      const organizationId = (account.organization_id || '').trim();
      const matchedWorkspace = organizationId
        ? metadata.workspaces.find((workspace) => (workspace.id || '').trim() === organizationId)
        : null;
      const defaultWorkspace = metadata.workspaces.find((workspace) => workspace.is_default);
      const fallbackWorkspace = matchedWorkspace || defaultWorkspace || metadata.workspaces[0] || null;
      const workspaceTitle = fallbackWorkspace?.title?.trim() || '';
      const accountName = (account.account_name || '').trim();
      const structure = (account.account_structure || '').trim().toLowerCase();
      const isTeamLikePlan = isCodexTeamLikePlan(account.plan_type);
      const isPersonalStructure = structure.includes('personal');
      const accountContextText =
        isPersonalStructure
          ? t('codex.account.personal', '个人账户')
          : !structure && !isTeamLikePlan
            ? t('codex.account.personal', '个人账户')
            : accountName || workspaceTitle || '';
      const loginProvider =
        formatCodexLoginProvider(metadata.authProvider) ||
        t('kiro.account.providerUnknown', 'Unknown');
      const userId = (metadata.userId || account.user_id || '').trim() || noneText;
      const signedInWithText = t('kiro.account.signedInWith', {
        provider: loginProvider,
        defaultValue: 'Signed in with {{provider}}',
      });
      map.set(account.id, {
        chatgptAccountId: (metadata.chatgptAccountId || account.account_id || '').trim() || noneText,
        signedInWithText,
        userId,
        accountContextText,
      });
    });

    return map;
  }, [accounts, t]);

  const resolveAccountMeta = useCallback(
    (account: CodexAccount) =>
      accountMetaMap.get(account.id) ?? {
        chatgptAccountId: t('common.none', '暂无'),
        signedInWithText: t('kiro.account.signedInWith', {
          provider: t('kiro.account.providerUnknown', 'Unknown'),
          defaultValue: 'Signed in with {{provider}}',
        }),
        userId: t('common.none', '暂无'),
        accountContextText: '',
      },
    [accountMetaMap, t],
  );

  const isAbnormalAccount = useCallback(
    (account: CodexAccount) => Boolean(account.quota_error),
    [],
  );

  const tierCounts = useMemo(() => {
    const counts = { all: accounts.length, VALID: 0, FREE: 0, PLUS: 0, PRO: 0, TEAM: 0, ENTERPRISE: 0, ERROR: 0 };
    accounts.forEach((a) => {
      if (!isAbnormalAccount(a)) {
        counts.VALID += 1;
      }
      const tier = resolvePlanKey(a);
      if (tier in counts) counts[tier as keyof typeof counts] += 1;
      if (a.quota_error) counts.ERROR += 1;
    });
    return counts;
  }, [accounts, isAbnormalAccount, resolvePlanKey]);

  const tierFilterOptions = useMemo<MultiSelectFilterOption[]>(() => [
    { value: 'FREE', label: `FREE (${tierCounts.FREE})` },
    { value: 'PLUS', label: `PLUS (${tierCounts.PLUS})` },
    { value: 'PRO', label: `PRO (${tierCounts.PRO})` },
    { value: 'TEAM', label: `TEAM (${tierCounts.TEAM})` },
    { value: 'ENTERPRISE', label: `ENTERPRISE (${tierCounts.ENTERPRISE})` },
    { value: 'ERROR', label: `ERROR (${tierCounts.ERROR})` },
    buildValidAccountsFilterOption(t, tierCounts.VALID),
  ], [t, tierCounts]);

  // ─── Filtering & Sorting ────────────────────────────────────────────
  const compareAccountsBySort = useCallback((a: CodexAccount, b: CodexAccount) => {
    const currentFirstDiff = compareCurrentAccountFirst(a.id, b.id, currentAccount?.id);
    if (currentFirstDiff !== 0) {
      return currentFirstDiff;
    }

    if (sortBy === 'created_at') {
      const diff = b.created_at - a.created_at;
      return sortDirection === 'desc' ? diff : -diff;
    }
    if (sortBy === 'weekly_reset' || sortBy === 'hourly_reset') {
      const aR = sortBy === 'weekly_reset' ? a.quota?.weekly_reset_time ?? null : a.quota?.hourly_reset_time ?? null;
      const bR = sortBy === 'weekly_reset' ? b.quota?.weekly_reset_time ?? null : b.quota?.hourly_reset_time ?? null;
      if (aR == null && bR == null) return 0;
      if (aR == null) return 1;
      if (bR == null) return -1;
      return sortDirection === 'desc' ? bR - aR : aR - bR;
    }
    const aV = sortBy === 'weekly' ? a.quota?.weekly_percentage ?? -1 : a.quota?.hourly_percentage ?? -1;
    const bV = sortBy === 'weekly' ? b.quota?.weekly_percentage ?? -1 : b.quota?.hourly_percentage ?? -1;
    return sortDirection === 'desc' ? bV - aV : aV - bV;
  }, [currentAccount?.id, sortBy, sortDirection]);

  const sortedAccountsForInstances = useMemo(
    () => [...accounts].sort(compareAccountsBySort),
    [accounts, compareAccountsBySort],
  );

  const filteredAccounts = useMemo(() => {
    let result = [...accounts];
    if (searchQuery.trim()) {
      const query = searchQuery.toLowerCase();
      result = result.filter((a) => resolvePresentation(a).displayName.toLowerCase().includes(query));
    }
    if (filterTypes.length > 0) {
      const { requireValidAccounts, selectedTypes } = splitValidityFilterValues(filterTypes);
      if (requireValidAccounts) {
        result = result.filter((account) => !isAbnormalAccount(account));
      }
      if (selectedTypes.size > 0) {
        result = result.filter((a) => {
          if (selectedTypes.has('ERROR') && a.quota_error) {
            return true;
          }
          return selectedTypes.has(resolvePlanKey(a));
        });
      }
    }
    if (tagFilter.length > 0) {
      const selectedTags = new Set(tagFilter.map(normalizeTag));
      result = result.filter((a) => (a.tags || []).map(normalizeTag).some((tag) => selectedTags.has(tag)));
    }
    // 分组筛选 — 仅保留仍存在于 codexGroups 中的 ID，防止已删除分组导致空筛选
    if (groupFilter.length > 0) {
      const existingGroupIds = new Set(codexGroups.map((g) => g.id));
      const activeFilter = groupFilter.filter((id) => existingGroupIds.has(id));
      if (activeFilter.length > 0) {
        const groupAccountIds = new Set<string>();
        const selectedGroupIds = new Set(activeFilter);
        for (const group of codexGroups) {
          if (selectedGroupIds.has(group.id)) {
            for (const aid of group.accountIds) groupAccountIds.add(aid);
          }
        }
        result = result.filter((a) => groupAccountIds.has(a.id));
      }
    }
    result.sort(compareAccountsBySort);
    return result;
  }, [accounts, codexGroups, compareAccountsBySort, filterTypes, groupFilter, isAbnormalAccount, normalizeTag, resolvePlanKey, resolvePresentation, searchQuery, tagFilter]);

  const filteredIds = useMemo(() => filteredAccounts.map((account) => account.id), [filteredAccounts]);
  const exportSelectionCount = getScopedSelectedCount(filteredIds);
  const pagination = usePagination({
    items: filteredAccounts,
    storageKey: buildPaginationPageSizeStorageKey('Codex'),
  });
  const paginatedAccounts = pagination.pageItems;
  const paginatedIds = useMemo(() => paginatedAccounts.map((account) => account.id), [paginatedAccounts]);
  const isAllPaginatedSelected = useMemo(
    () => isEveryIdSelected(selected, paginatedIds),
    [paginatedIds, selected],
  );

  const groupedAccounts = useMemo(() => {
    if (!groupByTag) return [] as Array<[string, typeof filteredAccounts]>;
    const groups = new Map<string, typeof filteredAccounts>();
    const selectedTags = new Set(tagFilter.map(normalizeTag));
    filteredAccounts.forEach((a) => {
      const tags = (a.tags || []).map(normalizeTag).filter(Boolean);
      const matchedTags = selectedTags.size > 0 ? tags.filter((tag) => selectedTags.has(tag)) : tags;
      if (matchedTags.length === 0) { if (!groups.has(untaggedKey)) groups.set(untaggedKey, []); groups.get(untaggedKey)?.push(a); return; }
      matchedTags.forEach((tag) => { if (!groups.has(tag)) groups.set(tag, []); groups.get(tag)?.push(a); });
    });
    return Array.from(groups.entries()).sort(([a], [b]) => { if (a === untaggedKey) return 1; if (b === untaggedKey) return -1; return a.localeCompare(b); });
  }, [filteredAccounts, groupByTag, normalizeTag, tagFilter, untaggedKey]);

  const paginatedGroupedAccounts = useMemo(
    () => buildPaginatedGroups(groupedAccounts, paginatedAccounts),
    [groupedAccounts, paginatedAccounts],
  );

  useEffect(() => {
    const teamAccountIds = filteredAccounts
      .filter(
        (account) =>
          !hasCodexAccountStructure(account) ||
          (isCodexTeamLikePlan(account.plan_type) && !hasCodexAccountName(account)),
      )
      .map((account) => account.id);
    if (teamAccountIds.length === 0) return;
    void hydrateAccountProfilesIfNeeded(teamAccountIds);
  }, [filteredAccounts, hydrateAccountProfilesIfNeeded]);

  const resolveGroupLabel = (groupKey: string) => groupKey === untaggedKey ? t('accounts.defaultGroup', '默认分组') : groupKey;

  const resolveCompactQuotaItems = useCallback(
    (presentation: ReturnType<typeof buildCodexAccountPresentation>) => {
      const standardQuotaItems = presentation.quotaItems.filter((item) => item.key !== 'code_review');
      const first = standardQuotaItems[0];
      const primary = standardQuotaItems.find((item) => item.key === 'primary') ?? first;
      const secondary =
        standardQuotaItems.find((item) => item.key === 'secondary') ??
        standardQuotaItems.find((item) => item.key !== primary?.key);

      return [
        {
          key: 'primary',
          valueText: primary?.valueText ?? '--',
          quotaClass: primary?.quotaClass ?? 'unknown',
        },
        {
          key: 'secondary',
          valueText: secondary?.valueText ?? '--',
          quotaClass: secondary?.quotaClass ?? 'unknown',
        },
      ];
    },
    [],
  );

  // ─── Render helpers ──────────────────────────────────────────────────

  const renderCompactRows = (items: typeof filteredAccounts, groupKey?: string) =>
    items.map((account) => {
      const presentation = resolvePresentation(account);
      const isCurrent = currentAccount?.id === account.id;
      const isSelected = selected.has(account.id);
      const compactQuotaItems = resolveCompactQuotaItems(presentation);
      return (
        <div
          key={groupKey ? `${groupKey}-${account.id}` : account.id}
          className={`codex-compact-row ${isCurrent ? 'current' : ''} ${isSelected ? 'selected' : ''}`}
        >
          <div className="codex-compact-select">
            <input
              type="checkbox"
              checked={isSelected}
              onChange={() => toggleSelect(account.id)}
            />
          </div>
          <span
            className="codex-compact-email"
            title={maskAccountText(presentation.displayName)}
          >
            {maskAccountText(presentation.displayName)}
          </span>
          <div className="codex-compact-quotas">
            {compactQuotaItems.map((item) => (
              <span
                key={`${account.id}-${item.key}`}
                className={`codex-compact-quota codex-compact-quota-${item.key}`}
              >
                <span className="codex-compact-dot" />
                <span className={`codex-compact-quota-value ${item.quotaClass}`}>{item.valueText}</span>
              </span>
            ))}
          </div>
          <button
            className={`codex-compact-switch-btn ${!isCurrent ? 'success' : ''}`}
            onClick={() => handleSwitch(account.id)}
            disabled={!!switching}
            title={t('codex.switch', '切换')}
          >
            {switching === account.id ? (
              <RefreshCw size={14} className="loading-spinner" />
            ) : (
              <Play size={14} />
            )}
          </button>
        </div>
      );
    });

  const renderGridCards = (items: typeof filteredAccounts, groupKey?: string) =>
    items.map((account) => {
      const presentation = resolvePresentation(account);
      const meta = resolveAccountMeta(account);
      const isCurrent = currentAccount?.id === account.id;
      const isApiKeyAccount = isCodexApiKeyAccount(account);
      const isEditingApiKeyName = isApiKeyAccount && editingApiKeyNameId === account.id;
      const isSavingApiKeyName = savingApiKeyNameId === account.id;
      const planClass = presentation.planClass || 'unknown';
      const isSelected = selected.has(account.id);
      const quotaItems = isApiKeyAccount
        ? []
        : showCodeReviewQuota
          ? presentation.quotaItems
          : presentation.quotaItems.filter((item) => item.key !== 'code_review');
      const quotaErrorMeta = resolveQuotaErrorMeta(account.quota_error);
      const hasQuotaError = Boolean(quotaErrorMeta.rawMessage);
      const showReauthorizeAction =
        !isApiKeyAccount && hasQuotaError && shouldOfferReauthorizeAction(quotaErrorMeta);
      const accountIdText =
        meta.chatgptAccountId && meta.chatgptAccountId !== t('common.none', '暂无')
          ? meta.chatgptAccountId
          : meta.userId;
      const signInLine = `${meta.signedInWithText} | ${accountIdLabel}: ${accountIdText}`;
      const apiKeyText = resolveApiKeyDisplayText(account);
      const apiKeyLine = `${t('codex.addModal.token', 'API Key')}：${apiKeyText}`;
      const apiProviderName = resolveApiProviderDisplayName(account);
      const apiProviderLine = `${t('codex.api.provider.label', '供应商')}：${apiProviderName}`;
      const apiBaseUrlText = (account.api_base_url || '').trim() || '-';
      const apiBaseUrlLine = `${t('codex.api.baseUrl', 'Base URL')}：${apiBaseUrlText}`;
      const accountTags = (account.tags || []).map((tag) => tag.trim()).filter(Boolean);
      const visibleTags = accountTags.slice(0, 2);
      const moreTagCount = Math.max(0, accountTags.length - visibleTags.length);
      return (
        <div key={groupKey ? `${groupKey}-${account.id}` : account.id} className={`codex-account-card ${isCurrent ? 'current' : ''} ${isSelected ? 'selected' : ''}`}>
          <div className="card-top">
            <div className="card-select"><input type="checkbox" checked={isSelected} onChange={() => toggleSelect(account.id)} /></div>
            {isEditingApiKeyName ? (
              <input
                className="account-email inline-name-editor"
                value={editingApiKeyNameValue}
                onChange={(event) => setEditingApiKeyNameValue(event.target.value)}
                onBlur={() => void handleSubmitInlineRename(account)}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') {
                    event.preventDefault();
                    void handleSubmitInlineRename(account);
                  } else if (event.key === 'Escape') {
                    event.preventDefault();
                    inlineRenameDiscardRef.current = true;
                    clearInlineRename();
                  }
                }}
                disabled={isSavingApiKeyName}
                autoFocus
              />
            ) : (
              <span
                className={`account-email ${isApiKeyAccount ? 'editable' : ''}`}
                title={maskAccountText(presentation.displayName)}
                onDoubleClick={() => handleAccountNameDoubleClick(account)}
              >
                {maskAccountText(presentation.displayName)}
              </span>
            )}
            {isCurrent && <span className="current-tag">{t('codex.current', '当前')}</span>}
            {hasQuotaError && (<span className="codex-status-pill quota-error" title={quotaErrorMeta.rawMessage}><CircleAlert size={12} />{quotaErrorMeta.statusCode || t('codex.quotaError.badge', '配额异常')}</span>)}
            <span className={`tier-badge ${planClass}`}>{presentation.planLabel}</span>
          </div>
          {meta.accountContextText && (
            <div className="account-sub-line">
              <span className="codex-login-subline" title={meta.accountContextText}>
                Team Name：{meta.accountContextText}
              </span>
            </div>
          )}
          {!isApiKeyAccount && (
            <div className="account-sub-line">
              <span className="codex-login-subline" title={signInLine}>
                {meta.signedInWithText} | {accountIdLabel}: {maskAccountText(accountIdText)}
              </span>
            </div>
          )}
          {isApiKeyAccount && (
            <>
              <div className="account-sub-line">
                <span className="codex-login-subline" title={apiKeyLine}>
                  {apiKeyLine}
                </span>
              </div>
              <div className="account-sub-line codex-provider-inline-line">
                <span className="codex-login-subline codex-provider-inline-text" title={apiProviderLine}>
                  {apiProviderLine}
                </span>
                <button
                  type="button"
                  className="codex-provider-inline-switch"
                  onClick={() => openQuickSwitchProviderModal(account)}
                  title={t('codex.quickSwitch.action', '快速切换供应商')}
                >
                  {t('codex.quickSwitch.inlineAction', '切换')}
                </button>
              </div>
              <div className="account-sub-line">
                <span className="codex-login-subline" title={apiBaseUrlLine}>
                  {apiBaseUrlLine}
                </span>
              </div>
            </>
          )}
          {accountTags.length > 0 && (<div className="card-tags">{visibleTags.map((tag, idx) => (<span key={`${account.id}-${tag}-${idx}`} className="tag-pill">{tag}</span>))}{moreTagCount > 0 && <span className="tag-pill more">+{moreTagCount}</span>}</div>)}
          <div className="codex-quota-section">
            {isApiKeyAccount ? (
              <div className="quota-empty">
                <button className="btn btn-secondary btn-sm" onClick={() => void handleOpenCodexUsage()}>
                  <ExternalLink size={14} />
                  {t('codex.usage.open', '查看 OpenAI 用量')}
                </button>
              </div>
            ) : (
              <>
                {hasQuotaError && (
                  <div className="quota-error-inline" title={quotaErrorMeta.rawMessage}>
                    <CircleAlert size={14} />
                    <span>{quotaErrorMeta.displayText}</span>
                    {showReauthorizeAction && (
                      <button
                        className="btn btn-sm btn-outline"
                        onClick={() => openAddModal('oauth')}
                        title={t('common.shared.addModal.oauth', 'OAuth 授权')}
                      >
                        {t('common.shared.addModal.oauth', 'OAuth 授权')}
                      </button>
                    )}
                  </div>
                )}
                {quotaItems.map((item) => {
                  const QuotaIcon = item.key === 'secondary' ? Calendar : item.key === 'code_review' ? BookOpen : Clock;
                  return (<div key={item.key} className="quota-item"><div className="quota-header"><QuotaIcon size={14} /><span className="quota-label">{item.label}</span><span className={`quota-pct ${item.quotaClass}`}>{item.valueText}</span></div>
                    <div className="quota-bar-track"><div className={`quota-bar ${item.quotaClass}`} style={{ width: `${item.percentage}%` }} /></div>
                    {item.resetText && <span className="quota-reset">{item.resetText}</span>}</div>);
                })}
                {quotaItems.length === 0 && (<div className="quota-empty">{t('common.shared.quota.noData', '暂无配额数据')}</div>)}
              </>
            )}
          </div>
          <div className="card-footer">
            <span className="card-date">{formatDate(account.created_at)}</span>
            <div className="card-actions">
              <button className="card-action-btn" onClick={() => openTagModal(account.id)} title={t('accounts.editTags', '编辑标签')}><Tag size={14} /></button>
              {isApiKeyAccount && (
                <button
                  className="card-action-btn"
                  onClick={() => openQuickSwitchProviderModal(account)}
                  title={t('codex.quickSwitch.action', '快速切换供应商')}
                >
                  <Repeat size={14} />
                </button>
              )}
              {isApiKeyAccount && (
                <button
                  className="card-action-btn"
                  onClick={() => openApiKeyCredentialsModal(account)}
                  title={t('instances.actions.edit', '编辑')}
                >
                  <Pencil size={14} />
                </button>
              )}
              <button className={`card-action-btn ${!isCurrent ? 'success' : ''}`} onClick={() => handleSwitch(account.id)} disabled={!!switching} title={t('codex.switch', '切换')}>
                {switching === account.id ? <RefreshCw size={14} className="loading-spinner" /> : <Play size={14} />}
              </button>
              {!isApiKeyAccount && (
                <button className="card-action-btn" onClick={() => handleRefresh(account.id)} disabled={refreshing === account.id} title={t('common.shared.refreshQuota', '刷新配额')}>
                  <RotateCw size={14} className={refreshing === account.id ? 'loading-spinner' : ''} />
                </button>
              )}
              <button
                className="card-action-btn export-btn"
                onClick={() => handleExportByIds([account.id], resolveSingleExportBaseName(account))}
                title={t('common.shared.export.title', '导出')}
              >
                <Upload size={14} />
              </button>
              <button className="card-action-btn danger" onClick={() => handleDelete(account.id)} title={t('common.delete', '删除')}><Trash2 size={14} /></button>
            </div>
          </div>
        </div>
      );
    });

  const renderTableRows = (items: typeof filteredAccounts, groupKey?: string) =>
    items.map((account) => {
      const presentation = resolvePresentation(account);
      const meta = resolveAccountMeta(account);
      const isCurrent = currentAccount?.id === account.id;
      const isApiKeyAccount = isCodexApiKeyAccount(account);
      const isEditingApiKeyName = isApiKeyAccount && editingApiKeyNameId === account.id;
      const isSavingApiKeyName = savingApiKeyNameId === account.id;
      const planClass = presentation.planClass || 'unknown';
      const quotaItems = isApiKeyAccount
        ? []
        : showCodeReviewQuota
          ? presentation.quotaItems
          : presentation.quotaItems.filter((item) => item.key !== 'code_review');
      const quotaErrorMeta = resolveQuotaErrorMeta(account.quota_error);
      const hasQuotaError = Boolean(quotaErrorMeta.rawMessage);
      const showReauthorizeAction =
        !isApiKeyAccount && hasQuotaError && shouldOfferReauthorizeAction(quotaErrorMeta);
      const accountIdText =
        meta.chatgptAccountId && meta.chatgptAccountId !== t('common.none', '暂无')
          ? meta.chatgptAccountId
          : meta.userId;
      const signInLine = `${meta.signedInWithText} | ${accountIdLabel}: ${accountIdText}`;
      const apiKeyText = resolveApiKeyDisplayText(account);
      const apiKeyLine = `${t('codex.addModal.token', 'API Key')}：${apiKeyText}`;
      const apiProviderName = resolveApiProviderDisplayName(account);
      const apiProviderLine = `${t('codex.api.provider.label', '供应商')}：${apiProviderName}`;
      const apiBaseUrlText = (account.api_base_url || '').trim() || '-';
      const apiBaseUrlLine = `${t('codex.api.baseUrl', 'Base URL')}：${apiBaseUrlText}`;
      return (
        <tr key={groupKey ? `${groupKey}-${account.id}` : account.id} className={isCurrent ? 'current' : ''}>
          <td><input type="checkbox" checked={selected.has(account.id)} onChange={() => toggleSelect(account.id)} /></td>
          <td><div className="account-cell"><div className="account-main-line">{isEditingApiKeyName ? (
            <input
              className="account-email-text inline-name-editor"
              value={editingApiKeyNameValue}
              onChange={(event) => setEditingApiKeyNameValue(event.target.value)}
              onBlur={() => void handleSubmitInlineRename(account)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') {
                  event.preventDefault();
                  void handleSubmitInlineRename(account);
                } else if (event.key === 'Escape') {
                  event.preventDefault();
                  inlineRenameDiscardRef.current = true;
                  clearInlineRename();
                }
              }}
              disabled={isSavingApiKeyName}
              autoFocus
            />
          ) : (
            <span
              className={`account-email-text ${isApiKeyAccount ? 'editable' : ''}`}
              title={maskAccountText(presentation.displayName)}
              onDoubleClick={() => handleAccountNameDoubleClick(account)}
            >
              {maskAccountText(presentation.displayName)}
            </span>
          )}
            {isCurrent && <span className="mini-tag current">{t('codex.current', '当前')}</span>}</div>
            {meta.accountContextText && (
              <div className="account-sub-line codex-account-meta-inline">
                <span className="codex-login-subline" title={meta.accountContextText}>
                  Team Name：{meta.accountContextText}
                </span>
              </div>
            )}
            {!isApiKeyAccount && (
              <div className="account-sub-line codex-account-meta-inline">
                <span className="codex-login-subline" title={signInLine}>
                  {meta.signedInWithText} | {accountIdLabel}: {maskAccountText(accountIdText)}
                </span>
              </div>
            )}
            {isApiKeyAccount && (
              <>
                <div className="account-sub-line codex-account-meta-inline">
                  <span className="codex-login-subline" title={apiKeyLine}>
                    {apiKeyLine}
                  </span>
                </div>
                <div className="account-sub-line codex-account-meta-inline codex-provider-inline-line">
                  <span className="codex-login-subline codex-provider-inline-text" title={apiProviderLine}>
                    {apiProviderLine}
                  </span>
                  <button
                    type="button"
                    className="codex-provider-inline-switch"
                    onClick={() => openQuickSwitchProviderModal(account)}
                    title={t('codex.quickSwitch.action', '快速切换供应商')}
                  >
                    {t('codex.quickSwitch.inlineAction', '切换')}
                  </button>
                </div>
                <div className="account-sub-line codex-account-meta-inline">
                  <span className="codex-login-subline" title={apiBaseUrlLine}>
                    {apiBaseUrlLine}
                  </span>
                </div>
              </>
            )}
            {hasQuotaError && (<div className="account-sub-line"><span className="codex-status-pill quota-error" title={quotaErrorMeta.rawMessage}><CircleAlert size={12} />{quotaErrorMeta.statusCode || t('codex.quotaError.badge', '配额异常')}</span></div>)}</div></td>
          <td><span className={`tier-badge ${planClass}`}>{presentation.planLabel}</span></td>
          <td>
            {isApiKeyAccount ? (
              <button className="btn btn-secondary btn-sm" onClick={() => void handleOpenCodexUsage()}>
                <ExternalLink size={14} />
                {t('codex.usage.open', '查看 OpenAI 用量')}
              </button>
            ) : (
              <>
                <div className="quota-grid">
                  {quotaItems.map((item) => (
                    <div key={item.key} className="quota-item">
                      <div className="quota-header"><span className="quota-name">{item.label}</span><span className={`quota-value ${item.quotaClass}`}>{item.valueText}</span></div>
                      <div className="quota-progress-track"><div className={`quota-progress-bar ${item.quotaClass}`} style={{ width: `${item.percentage}%` }} /></div>
                      {item.resetText && (<div className="quota-footer"><span className="quota-reset">{item.resetText}</span></div>)}
                    </div>
                  ))}
                  {quotaItems.length === 0 && (
                    <span style={{ color: 'var(--text-muted)', fontSize: 13 }}>
                      {t('common.shared.quota.noData', '暂无配额数据')}
                    </span>
                  )}
                </div>
                {hasQuotaError && (
                  <div className="quota-error-inline table" title={quotaErrorMeta.rawMessage}>
                    <CircleAlert size={12} />
                    <span>{quotaErrorMeta.displayText}</span>
                    {showReauthorizeAction && (
                      <button
                        className="btn btn-sm btn-outline"
                        onClick={() => openAddModal('oauth')}
                        title={t('common.shared.addModal.oauth', 'OAuth 授权')}
                      >
                        {t('common.shared.addModal.oauth', 'OAuth 授权')}
                      </button>
                    )}
                  </div>
                )}
              </>
            )}
          </td>
          <td className="sticky-action-cell table-action-cell"><div className="action-buttons">
            <button className="action-btn" onClick={() => openTagModal(account.id)} title={t('accounts.editTags', '编辑标签')}><Tag size={14} /></button>
            {isApiKeyAccount && (
              <button
                className="action-btn"
                onClick={() => openQuickSwitchProviderModal(account)}
                title={t('codex.quickSwitch.action', '快速切换供应商')}
              >
                <Repeat size={14} />
              </button>
            )}
            {isApiKeyAccount && (
              <button
                className="action-btn"
                onClick={() => openApiKeyCredentialsModal(account)}
                title={t('instances.actions.edit', '编辑')}
              >
                <Pencil size={14} />
              </button>
            )}
            <button className={`action-btn ${!isCurrent ? 'success' : ''}`} onClick={() => handleSwitch(account.id)} disabled={!!switching} title={t('codex.switch', '切换')}>
              {switching === account.id ? <RefreshCw size={14} className="loading-spinner" /> : <Play size={14} />}
            </button>
            {!isApiKeyAccount && (
              <button className="action-btn" onClick={() => handleRefresh(account.id)} disabled={refreshing === account.id} title={t('common.shared.refreshQuota', '刷新配额')}>
                <RotateCw size={14} className={refreshing === account.id ? 'loading-spinner' : ''} />
              </button>
            )}
            <button
              className="action-btn"
              onClick={() => handleExportByIds([account.id], resolveSingleExportBaseName(account))}
              title={t('common.shared.export.title', '导出')}
            >
              <Upload size={14} />
            </button>
            <button className="action-btn danger" onClick={() => handleDelete(account.id)} title={t('common.delete', '删除')}><Trash2 size={14} /></button>
          </div></td>
        </tr>
      );
    });

  return (
    <div className="codex-accounts-page">
      <CodexOverviewTabsHeader
        active={activeTab}
        onTabChange={setActiveTab}
        tabs={['overview', 'providers', 'wakeup', 'instances', 'sessions']}
      />

      {activeTab === 'overview' && (<>
        {message && (<div className={`message-bar ${message.tone === 'error' ? 'error' : 'success'}`}>{message.text}<button onClick={() => setMessage(null)}><X size={14} /></button></div>)}

        <div className="toolbar">
          <div className="toolbar-left">
            <div className="search-box"><Search size={16} className="search-icon" /><input type="text" placeholder={t('common.shared.search', '搜索账号...')} value={searchQuery} onChange={(e) => setSearchQuery(e.target.value)} /></div>
            <div className="view-switcher">
              <button className={`view-btn ${overviewLayoutMode === 'compact' ? 'active' : ''}`} onClick={() => handleChangeOverviewLayoutMode('compact')} title={t('accounts.view.compact', '紧凑视图')}><Rows3 size={16} /></button>
              <button className={`view-btn ${overviewLayoutMode === 'list' ? 'active' : ''}`} onClick={() => handleChangeOverviewLayoutMode('list')} title={t('common.shared.view.list', '列表视图')}><List size={16} /></button>
              <button className={`view-btn ${overviewLayoutMode === 'grid' ? 'active' : ''}`} onClick={() => handleChangeOverviewLayoutMode('grid')} title={t('common.shared.view.grid', '卡片视图')}><LayoutGrid size={16} /></button>
            </div>
            <MultiSelectFilterDropdown
              options={tierFilterOptions}
              selectedValues={filterTypes}
              allLabel={t('common.shared.filter.all', { count: tierCounts.all })}
              filterLabel={t('common.shared.filterLabel', '筛选')}
              clearLabel={t('accounts.clearFilter', '清空筛选')}
              emptyLabel={t('common.none', '暂无')}
              ariaLabel={t('common.shared.filterLabel', '筛选')}
              onToggleValue={toggleFilterTypeValue}
              onClear={clearFilterTypes}
            />
            <div className="tag-filter" ref={tagFilterRef}>
              <button type="button" className={`tag-filter-btn ${tagFilter.length > 0 ? 'active' : ''}`} onClick={() => setShowTagFilter((prev) => !prev)} aria-label={t('accounts.filterTags', '标签筛选')}>
                <Tag size={14} />{tagFilter.length > 0 ? `${t('accounts.filterTagsCount', '标签')}(${tagFilter.length})` : t('accounts.filterTags', '标签筛选')}
              </button>
              {showTagFilter && (<div
                ref={page.tagFilterPanelRef}
                className={`tag-filter-panel ${page.tagFilterPanelPlacement === 'top' ? 'open-top' : ''}`}
              >
                {availableTags.length === 0 ? (<div className="tag-filter-empty">{t('accounts.noAvailableTags', '暂无可用标签')}</div>) : (
                  <div className="tag-filter-options" style={page.tagFilterScrollContainerStyle}>{availableTags.map((tag) => (
                    <label key={tag} className={`tag-filter-option ${tagFilter.includes(tag) ? 'selected' : ''}`}>
                      <input type="checkbox" checked={tagFilter.includes(tag)} onChange={() => toggleTagFilterValue(tag)} /><span className="tag-filter-name">{tag}</span>
                      <button type="button" className="tag-filter-delete" onClick={(e) => { e.preventDefault(); e.stopPropagation(); requestDeleteTag(tag); }} aria-label={t('accounts.deleteTagAria', { tag, defaultValue: '删除标签 {{tag}}' })}><X size={12} /></button>
                    </label>))}</div>)}
                <div className="tag-filter-divider" /><label className="tag-filter-group-toggle"><input type="checkbox" checked={groupByTag} onChange={(e) => setGroupByTag(e.target.checked)} /><span>{t('accounts.groupByTag', '按标签分组展示')}</span></label>
                {tagFilter.length > 0 && (<button type="button" className="tag-filter-clear" onClick={clearTagFilter}>{t('accounts.clearFilter', '清空筛选')}</button>)}
              </div>)}
            </div>

            <SingleSelectFilterDropdown
              value={sortBy}
              options={[
                { value: 'created_at', label: t('common.shared.sort.createdAt', '按创建时间') },
                { value: 'weekly', label: t('codex.sort.weekly', '按周配额') },
                { value: 'hourly', label: t('codex.sort.hourly', '按5小时配额') },
                { value: 'weekly_reset', label: t('codex.sort.weeklyReset', '按周配额重置时间') },
                { value: 'hourly_reset', label: t('codex.sort.hourlyReset', '按5小时配额重置时间') },
              ]}
              ariaLabel={t('common.shared.sortLabel', '排序')}
              icon={<ArrowDownWideNarrow size={14} />}
              onChange={setSortBy}
            />
            <button className="sort-direction-btn" onClick={() => setSortDirection((prev) => (prev === 'desc' ? 'asc' : 'desc'))}
              title={sortDirection === 'desc' ? t('common.shared.sort.descTooltip', '当前：降序，点击切换为升序') : t('common.shared.sort.ascTooltip', '当前：升序，点击切换为降序')}
              aria-label={t('common.shared.sort.toggleDirection', '切换排序方向')}>{sortDirection === 'desc' ? '⬇' : '⬆'}</button>
          </div>
          <div className="toolbar-right">
            <button className="btn btn-primary icon-only" onClick={() => openAddModal('oauth')} title={t('common.shared.addAccount', '添加账号')}><Plus size={14} /></button>
            <button className="btn btn-secondary icon-only" onClick={handleRefreshAll} disabled={refreshingAll || accounts.length === 0} title={t('common.shared.refreshAll', '刷新全部')}>
              <RefreshCw size={14} className={refreshingAll ? 'loading-spinner' : ''} /></button>
            <button className="btn btn-secondary icon-only" onClick={togglePrivacyMode} title={privacyModeEnabled ? t('privacy.showSensitive', '显示邮箱') : t('privacy.hideSensitive', '隐藏邮箱')}>
              {privacyModeEnabled ? <EyeOff size={14} /> : <Eye size={14} />}</button>
            <button className="btn btn-secondary export-btn icon-only" onClick={() => void handleExport(filteredIds)} disabled={exporting || filteredIds.length === 0}
              title={exportSelectionCount > 0 ? `${t('common.shared.export.title', '导出')} (${exportSelectionCount})` : t('common.shared.export.title', '导出')}><Upload size={14} /></button>
            {selected.size > 0 && (<>
              <button className="btn btn-secondary icon-only" onClick={() => setShowAddToCodexGroupModal(true)} title={t('codex.groups.addToGroup', '添加至分组')}><FolderPlus size={14} /></button>
              <button className="btn btn-danger icon-only" onClick={handleBatchDelete} title={`${t('common.delete', '删除')} (${selected.size})`}><Trash2 size={14} /></button>
            </>)}
            <button className={`btn btn-secondary icon-only ${groupFilter.length > 0 ? 'btn-filter-active' : ''}`} onClick={() => setShowCodexGroupModal(true)} title={groupFilter.length > 0 ? `${t('accounts.groups.manageTitle', '分组管理')} (${groupFilter.length})` : t('accounts.groups.manageTitle', '分组管理')}><FolderOpen size={14} /></button>
            <QuickSettingsPopover type="codex" />
          </div>
        </div>

        {loading && accounts.length === 0 ? (
          <div className="loading-container"><RefreshCw size={24} className="loading-spinner" /><p>{t('common.loading', '加载中...')}</p></div>
        ) : accounts.length === 0 ? (
          <div className="empty-state"><Globe size={48} /><h3>{t('common.shared.empty.title', '暂无账号')}</h3><p>{t('codex.empty.description', '点击"添加账号"开始管理您的 Codex 账号')}</p>
            <div style={{ display: 'flex', gap: '12px', justifyContent: 'center', marginTop: '16px' }}>
              <button className="btn btn-primary" onClick={() => openAddModal('oauth')}><Plus size={16} />{t('common.shared.addAccount', '添加账号')}</button>
              <button className="btn btn-secondary" onClick={() => window.dispatchEvent(new CustomEvent('app-request-navigate', { detail: 'manual' }))}><BookOpen size={16} />{t('manual.navTitle', '功能使用手册')}</button>
            </div>
          </div>
        ) : filteredAccounts.length === 0 ? (
          <div className="empty-state"><h3>{t('common.shared.noMatch.title', '没有匹配的账号')}</h3><p>{t('common.shared.noMatch.desc', '请尝试调整搜索或筛选条件')}</p></div>
        ) : overviewLayoutMode === 'compact' ? (
          groupByTag ? (
            <div className="tag-group-list">
              {paginatedGroupedAccounts.map(({ groupKey, items, totalCount }) => (
                <div key={groupKey} className="tag-group-section">
                  <div className="tag-group-header">
                    <span className="tag-group-title">{resolveGroupLabel(groupKey)}</span>
                    <span className="tag-group-count">{totalCount}</span>
                  </div>
                  <div className="codex-compact-list">{renderCompactRows(items, groupKey)}</div>
                </div>
              ))}
            </div>
          ) : (
            <div className="codex-compact-list">{renderCompactRows(paginatedAccounts)}</div>
          )
        ) : viewMode === 'grid' ? (
        <div className="grid-view-container">
          {paginatedAccounts.length > 0 && (
            <div className="grid-view-header" style={{ marginBottom: '12px', paddingLeft: '4px' }}>
              <label style={{ display: 'inline-flex', alignItems: 'center', gap: '8px', cursor: 'pointer', fontSize: '13px', color: 'var(--text-color)' }}>
                <input type="checkbox" checked={isAllPaginatedSelected} onChange={() => toggleSelectAll(paginatedIds)} />
                {t('common.selectAll', '全选')}
              </label>
            </div>
          )}
          {groupByTag ? (<div className="tag-group-list">{paginatedGroupedAccounts.map(({ groupKey, items, totalCount }) => (<div key={groupKey} className="tag-group-section"><div className="tag-group-header"><span className="tag-group-title">{resolveGroupLabel(groupKey)}</span><span className="tag-group-count">{totalCount}</span></div>
            <div className="tag-group-grid codex-accounts-grid">{renderGridCards(items, groupKey)}</div></div>))}</div>
          ) : (<div className="codex-accounts-grid">{renderGridCards(paginatedAccounts)}</div>)}
        </div>
      ) : groupByTag ? (
          <div className="account-table-container grouped"><table className="account-table"><thead><tr>
            <th style={{ width: 40 }}><input type="checkbox" checked={isAllPaginatedSelected} onChange={() => toggleSelectAll(paginatedIds)} /></th>
            <th style={{ width: 260 }}>{t('common.shared.columns.email', '账号')}</th><th style={{ width: 140 }}>{t('common.shared.columns.plan', '订阅')}</th>
            <th>{t('accounts.columns.quota', '配额状态')}</th><th className="sticky-action-header table-action-header">{t('common.shared.columns.actions', '操作')}</th></tr></thead>
            <tbody>{paginatedGroupedAccounts.map(({ groupKey, items, totalCount }) => (<Fragment key={groupKey}><tr className="tag-group-row"><td colSpan={5}><div className="tag-group-header"><span className="tag-group-title">{resolveGroupLabel(groupKey)}</span><span className="tag-group-count">{totalCount}</span></div></td></tr>
              {renderTableRows(items, groupKey)}</Fragment>))}</tbody></table></div>
        ) : (
          <div className="account-table-container"><table className="account-table"><thead><tr>
            <th style={{ width: 40 }}><input type="checkbox" checked={isAllPaginatedSelected} onChange={() => toggleSelectAll(paginatedIds)} /></th>
            <th style={{ width: 260 }}>{t('common.shared.columns.email', '账号')}</th><th style={{ width: 140 }}>{t('common.shared.columns.plan', '订阅')}</th>
            <th>{t('accounts.columns.quota', '配额状态')}</th><th className="sticky-action-header table-action-header">{t('common.shared.columns.actions', '操作')}</th></tr></thead>
            <tbody>{renderTableRows(paginatedAccounts)}</tbody></table></div>
        )}

        <PaginationControls
          totalItems={pagination.totalItems}
          currentPage={pagination.currentPage}
          totalPages={pagination.totalPages}
          pageSize={pagination.pageSize}
          pageSizeOptions={pagination.pageSizeOptions}
          rangeStart={pagination.rangeStart}
          rangeEnd={pagination.rangeEnd}
          canGoPrevious={pagination.canGoPrevious}
          canGoNext={pagination.canGoNext}
          onPageSizeChange={pagination.setPageSize}
          onPreviousPage={pagination.goToPreviousPage}
          onNextPage={pagination.goToNextPage}
        />

        {showAddModal && (<div className="modal-overlay" onClick={closeAddModal}><div className="modal-content codex-add-modal" onClick={(e) => e.stopPropagation()}>
          <div className="modal-header"><h2>{t('codex.addModal.title', '添加 Codex 账号')}</h2><button className="modal-close" onClick={closeAddModal} aria-label={t('common.close', '关闭')}><X /></button></div>
          <div className="modal-tabs">
            <button className={`modal-tab ${addTab === 'oauth' ? 'active' : ''}`} onClick={() => openAddModal('oauth')}>
              <Globe size={14} />
              <span className="modal-tab-label">{t('common.shared.addModal.oauth', 'OAuth Authorization')}</span>
            </button>
            <button className={`modal-tab ${addTab === 'token' ? 'active' : ''}`} onClick={() => openAddModal('token')}>
              <FileText size={14} />
              <span className="modal-tab-label">{t('common.shared.addModal.token', 'Token / JSON')}</span>
            </button>
            <button className={`modal-tab ${addTab === 'apikey' ? 'active' : ''}`} onClick={() => openAddModal('apikey')}>
              <KeyRound size={14} />
              <span className="modal-tab-label">{t('codex.addModal.token', 'API Key')}</span>
            </button>
            <button className={`modal-tab ${addTab === 'import' ? 'active' : ''}`} onClick={() => openAddModal('import')}>
              <Database size={14} />
              <span className="modal-tab-label">{t('accounts.tabs.import', '本地导入')}</span>
            </button>
          </div>
          <div className="modal-body">
            {addTab === 'oauth' && (<div className="add-section">
              <p className="section-desc">{t('codex.oauth.desc', '通过 OpenAI 官方 OAuth 授权您的 Codex 账号。')}</p>
              {oauthPrepareError ? (<div className="add-status error"><CircleAlert size={16} /><span>{oauthPrepareError}</span>
              {oauthPortInUse && (<button className="btn btn-sm btn-outline" onClick={handleReleaseOauthPort}>{t('codex.oauth.portInUseAction', 'Close port and retry')}</button>)}
                {!oauthPortInUse && oauthTimeoutInfo && (<button className="btn btn-sm btn-outline" onClick={handleRetryOauthAfterTimeout}>{t('codex.oauth.timeoutRetry', '刷新授权链接')}</button>)}</div>
              ) : oauthUrl ? (<div className="oauth-url-section">
                <div className="oauth-link">
                  <label>{t('accounts.oauth.linkLabel', '授权链接')}</label>
                  <div className="oauth-url-box"><input type="text" value={oauthUrl} readOnly /><button onClick={handleCopyOauthUrl}>{oauthUrlCopied ? <Check size={16} /> : <Copy size={16} />}</button></div>
                </div>
                <button className="btn btn-primary btn-full" onClick={isOauthTimeoutState ? handleRetryOauthAfterTimeout : handleOpenOauthUrl}>
                  {isOauthTimeoutState ? <RefreshCw size={16} /> : <Globe size={16} />}{isOauthTimeoutState ? t('codex.oauth.timeoutRetry', '刷新授权链接') : t('common.shared.oauth.openBrowser', 'Open in Browser')}</button>
                <div className="oauth-link">
                  <label>{t('common.shared.oauth.manualCallbackLabel', '手动输入回调地址')}</label>
                  <div className="oauth-url-box oauth-manual-input">
                    <input
                      type="text"
                      value={oauthCallbackInput}
                      onChange={(e) => setOauthCallbackInput(e.target.value)}
                      placeholder={t('common.shared.oauth.manualCallbackPlaceholder', '粘贴完整回调地址，例如：http://localhost:1455/auth/callback?code=...&state=...')}
                    />
                    <button
                      className="oauth-copy-button"
                      onClick={() => void handleSubmitOauthCallbackUrl()}
                      disabled={oauthCallbackSubmitting || !oauthCallbackInput.trim()}
                    >
                      {oauthCallbackSubmitting ? <RefreshCw size={16} className="loading-spinner" /> : <Check size={16} />}
                      <span className="oauth-copy-button-label">{t('accounts.oauth.continue', '我已授权，继续')}</span>
                    </button>
                  </div>
                </div>
                {oauthCallbackError && (<div className="add-status error"><CircleAlert size={16} /><span>{oauthCallbackError}</span></div>)}
                {isOauthTimeoutState && (<div className="add-status error"><CircleAlert size={16} /><span>{t('codex.oauth.timeout', '授权超时，请点击"刷新授权链接"后重试。')}</span></div>)}
                <p className="oauth-hint">{t('common.shared.oauth.hint', 'Once authorized, this window will update automatically')}</p></div>
              ) : (<div className="oauth-loading"><RefreshCw size={24} className="loading-spinner" /><span>{t('codex.oauth.preparing', '正在准备授权链接...')}</span></div>)}</div>)}
            {addTab === 'apikey' && (<div className="add-section">
              <div className="oauth-link">
                <label>{t('codex.modelProviders.selectSavedProvider', '已保存供应商')}</label>
                {managedProvidersLoading ? (
                  <div className="section-desc">{t('common.loading', '加载中...')}</div>
                ) : managedProviders.length === 0 ? (
                  <div className="section-desc">
                    {t('codex.modelProviders.noSavedProviders', '暂无已保存供应商，可直接填写后自动保存。')}
                  </div>
                ) : (
                  <div className="api-provider-chip-list">
                    {managedProviders.map((provider) => (
                      <button
                        key={provider.id}
                        className={`api-provider-chip ${managedProviderId === provider.id ? 'active' : ''}`}
                        onClick={() => handleSelectManagedProvider(provider.id)}
                        type="button"
                      >
                        <span>{provider.name}</span>
                      </button>
                    ))}
                  </div>
                )}
              </div>
              {selectedManagedProvider && selectedManagedProvider.apiKeys.length > 0 && (
                <div className="oauth-link">
                  <label>{t('codex.modelProviders.selectSavedApiKey', '已保存 API Key')}</label>
                  <div className="api-provider-endpoint-list">
                    {selectedManagedProvider.apiKeys.map((item) => (
                      <button
                        key={item.id}
                        className={`api-provider-endpoint-chip ${managedProviderApiKeyId === item.id ? 'active' : ''}`}
                        onClick={() => handleSelectManagedProviderApiKey(item.id)}
                        type="button"
                      >
                        {item.name || t('codex.modelProviders.unnamedKey', '未命名 Key')}
                      </button>
                    ))}
                  </div>
                </div>
              )}
              <div className="oauth-link">
                <label>{t('codex.api.provider.label', '供应商')}</label>
                <div className="api-provider-chip-list">
                  <button
                    className={`api-provider-chip ${apiProviderPresetId === CODEX_API_PROVIDER_CUSTOM_ID ? 'active' : ''}`}
                    onClick={() => handleSelectApiProviderPreset(CODEX_API_PROVIDER_CUSTOM_ID)}
                    type="button"
                  >
                    <span>{t('codex.api.provider.custom', '自定义')}</span>
                  </button>
                  {CODEX_API_PROVIDER_PRESETS.map((preset) => (
                    <button
                      key={preset.id}
                      className={`api-provider-chip ${apiProviderPresetId === preset.id ? 'active' : ''}`}
                      onClick={() => handleSelectApiProviderPreset(preset.id)}
                      type="button"
                    >
                      <span>{t(`codex.api.providers.${preset.id}.name`, preset.name)}</span>
                      {preset.isPartner && <Star size={12} className="api-provider-chip-badge" />}
                    </button>
                  ))}
                </div>
              </div>
              {selectedApiProviderPreset && selectedApiProviderPreset.baseUrls.length > 1 && (
                <div className="oauth-link">
                  <label>{t('codex.api.provider.endpoint', '供应商端点')}</label>
                  <div className="api-provider-endpoint-list">
                    {selectedApiProviderPreset.baseUrls.map((baseUrl) => (
                      <button
                        key={baseUrl}
                        className={`api-provider-endpoint-chip ${apiBaseUrlInput === baseUrl ? 'active' : ''}`}
                        onClick={() => setApiBaseUrlInput(baseUrl)}
                        type="button"
                      >
                        {baseUrl}
                      </button>
                    ))}
                  </div>
                </div>
              )}
              {selectedApiProviderPreset && (
                <div className="api-provider-hint-block">
                  <p className="api-provider-hint">
                    {t('codex.api.provider.hint', '已自动填写兼容 Base URL，可继续手动调整。')}
                  </p>
                  <div className="api-provider-links">
                    {selectedApiProviderPreset.website && (
                      <button
                        className="btn btn-secondary"
                        onClick={() => void handleOpenProviderLink(selectedApiProviderPreset.website || '')}
                      >
                        <ExternalLink size={14} />
                        {t('codex.api.provider.website', '官网')}
                      </button>
                    )}
                    {selectedApiProviderPreset.apiKeyUrl && (
                      <button
                        className="btn btn-secondary"
                        onClick={() => void handleOpenProviderLink(selectedApiProviderPreset.apiKeyUrl || '')}
                      >
                        <KeyRound size={14} />
                        {t('codex.api.provider.apiKeyPage', 'API Key 页面')}
                      </button>
                    )}
                  </div>
                </div>
              )}
              <div className="oauth-link">
                <label>{t('codex.addModal.token', 'API Key')}</label>
                <div className="oauth-url-box oauth-manual-input">
                  <input
                    type="text"
                    value={apiKeyInput}
                    onChange={(e) => setApiKeyInput(e.target.value)}
                  />
                </div>
              </div>
              <div className="oauth-link">
                <label>{t('codex.api.baseUrl', 'Base URL')}</label>
                <div className="oauth-url-box oauth-manual-input">
                  <input
                    type="text"
                    value={apiBaseUrlInput}
                    onChange={(e) => setApiBaseUrlInput(e.target.value)}
                    placeholder={t('codex.api.baseUrlPlaceholder', '不填写则是官方默认')}
                  />
                </div>
              </div>
              <div className="oauth-link">
                <label>{t('codex.modelProviders.newProviderName', '供应商名称（自动保存时使用，可选）')}</label>
                <div className="oauth-url-box oauth-manual-input">
                  <input
                    type="text"
                    value={newManagedProviderNameInput}
                    onChange={(e) => setNewManagedProviderNameInput(e.target.value)}
                    placeholder={t('codex.modelProviders.newProviderNamePlaceholder', '不填则按域名自动生成')}
                  />
                </div>
              </div>
              <div className="api-key-add-actions">
                <button
                  className="btn btn-secondary"
                  onClick={() => void handleApiKeyLogin(false)}
                  disabled={importing || addStatus === 'loading' || !apiKeyInput.trim()}
                >
                  {addStatus === 'loading' ? <RefreshCw size={16} className="loading-spinner" /> : <KeyRound size={16} />}
                  {t('common.shared.addAccount', '添加账号')}
                </button>
                <button
                  className="btn btn-primary"
                  onClick={() => void handleApiKeyLogin(true)}
                  disabled={importing || addStatus === 'loading' || !apiKeyInput.trim()}
                >
                  {addStatus === 'loading' ? <RefreshCw size={16} className="loading-spinner" /> : <Play size={16} />}
                  {t('codex.api.actions.addAndSwitch', '添加并切换')}
                </button>
              </div>
            </div>)}
            {addTab === 'token' && (<div className="add-section">
              <p className="section-desc">{t('codex.token.desc', '粘贴您的 Codex Access Token 或导出的 JSON 数据。')}</p>
              <details className="token-format-collapse"><summary className="token-format-collapse-summary">必填字段与示例（点击展开）</summary>
                <div className="token-format"><p className="token-format-required">必填字段：auth.json 需包含 tokens.id_token 与 tokens.access_token；账号数组需包含 id、email、tokens、created_at、last_used</p>
                  <div className="token-format-group"><div className="token-format-label">单条示例（auth.json）</div><pre className="token-format-code">{CODEX_TOKEN_SINGLE_EXAMPLE}</pre></div>
                  <div className="token-format-group"><div className="token-format-label">批量示例（账号数组）</div><pre className="token-format-code">{CODEX_TOKEN_BATCH_EXAMPLE}</pre></div></div></details>
              <textarea className="token-input" value={tokenInput} onChange={(e) => setTokenInput(e.target.value)} placeholder={t('codex.token.placeholder', '粘贴 Token 或 JSON...')} />
              <button className="btn btn-primary btn-full" onClick={handleTokenImport} disabled={importing || !tokenInput.trim()}>
                {importing ? <RefreshCw size={16} className="loading-spinner" /> : <Download size={16} />}{t('common.shared.token.import', 'Import')}</button></div>)}
            {addTab === 'import' && (<div className="add-section">
              <p className="section-desc">{t('codex.import.localDesc', '从本地已登录的会话中导入 Codex 账号。')}</p>
              <button className="btn btn-primary btn-full" onClick={handleImportFromLocal} disabled={importing}>
                {importing ? <RefreshCw size={16} className="loading-spinner" /> : <Database size={16} />}{t('codex.local.import', 'Get Local Account')}</button>
              <div style={{ height: 12 }} />
              <p className="section-desc">{t('modals.import.fromFilesDesc')}</p>
              <button className="btn btn-secondary btn-full" onClick={handleImportFromFiles} disabled={importing}>
                {importing ? <RefreshCw size={16} className="loading-spinner" /> : <FileUp size={16} />}{t('modals.import.fromFiles')}</button></div>)}
            {addStatus !== 'idle' && (
              <div className={`add-status ${addStatus}`}>
                {addStatus === 'success'
                  ? <Check size={16} />
                  : addStatus === 'loading'
                    ? <RefreshCw size={16} className="loading-spinner" />
                    : <CircleAlert size={16} />}
                <span>{addMessage}</span>
                {addTab === 'oauth' && addStatus === 'error' && isOauthTokenExchangeErrorState && oauthLoginIdRef.current && (
                  <button
                    className="btn btn-sm btn-outline"
                    onClick={() => void handleRetryOauthTokenExchange()}
                    disabled={oauthCallbackSubmitting}
                  >
                    {oauthCallbackSubmitting
                      ? <RefreshCw size={14} className="loading-spinner" />
                      : <RotateCw size={14} />}
                    {t('accounts.oauth.continue')}
                  </button>
                )}
              </div>
            )}
          </div>
        </div></div>)}

        {quickSwitchAccountId && (
          <div className="modal-overlay" onClick={closeQuickSwitchModal}>
            <div className="modal-content codex-add-modal" onClick={(e) => e.stopPropagation()}>
              <div className="modal-header">
                <h2>{t('codex.quickSwitch.title', '快速切换供应商')}</h2>
                <button
                  className="modal-close"
                  onClick={closeQuickSwitchModal}
                  aria-label={t('common.close', '关闭')}
                  disabled={quickSwitchSubmitting}
                >
                  <X />
                </button>
              </div>
              <div className="modal-body">
                <div className="add-section">
                  <p className="section-desc">
                    {t('codex.quickSwitch.desc', '为当前 API Key 账号快速切换到已保存的供应商与 API Key。')}
                  </p>
                  {quickSwitchAccount && (
                    <div className="section-desc">
                      {t('codex.quickSwitch.currentAccount', {
                        defaultValue: '当前账号：{{name}}',
                        name: maskAccountText(resolvePresentation(quickSwitchAccount).displayName),
                      })}
                    </div>
                  )}
                  <div className="oauth-link">
                    <label>{t('codex.modelProviders.selectSavedProvider', '已保存供应商')}</label>
                    {managedProvidersLoading ? (
                      <div className="section-desc">{t('common.loading', '加载中...')}</div>
                    ) : managedProviders.length === 0 ? (
                      <div className="add-status error">
                        <CircleAlert size={16} />
                        <span>{t('codex.quickSwitch.noProviders', '暂无已保存供应商，请先在“模型供应商”中添加。')}</span>
                      </div>
                    ) : (
                      <div className="api-provider-chip-list">
                        {managedProviders.map((provider) => (
                          <button
                            key={provider.id}
                            className={`api-provider-chip ${quickSwitchProviderId === provider.id ? 'active' : ''}`}
                            onClick={() => handleSelectQuickSwitchProvider(provider.id)}
                            type="button"
                            disabled={quickSwitchSubmitting}
                          >
                            <span>{provider.name}</span>
                          </button>
                        ))}
                      </div>
                    )}
                  </div>

                  {selectedQuickSwitchProvider && selectedQuickSwitchProvider.apiKeys.length > 0 && (
                    <div className="oauth-link">
                      <label>{t('codex.modelProviders.selectSavedApiKey', '已保存 API Key')}</label>
                      <div className="api-provider-endpoint-list">
                        {selectedQuickSwitchProvider.apiKeys.map((item) => (
                          <button
                            key={item.id}
                            className={`api-provider-endpoint-chip ${quickSwitchApiKeyId === item.id ? 'active' : ''}`}
                            onClick={() => handleSelectQuickSwitchApiKey(item.id)}
                            type="button"
                            disabled={quickSwitchSubmitting}
                          >
                            {item.name || t('codex.modelProviders.unnamedKey', '未命名 Key')}
                          </button>
                        ))}
                      </div>
                    </div>
                  )}

                  {selectedQuickSwitchProvider && selectedQuickSwitchProvider.apiKeys.length === 0 && (
                    <div className="add-status error">
                      <CircleAlert size={16} />
                      <span>{t('codex.quickSwitch.providerHasNoKeys', '该供应商没有可用 API Key，请先在模型供应商中添加。')}</span>
                    </div>
                  )}

                  {quickSwitchError && (
                    <div className="add-status error">
                      <CircleAlert size={16} />
                      <span>{quickSwitchError}</span>
                    </div>
                  )}

                  <div className="api-key-edit-actions">
                    <button
                      className="btn btn-secondary"
                      onClick={() => {
                        setActiveTab('providers');
                        closeQuickSwitchModal();
                      }}
                      disabled={quickSwitchSubmitting}
                    >
                      {t('codex.quickSwitch.gotoProviders', '管理供应商')}
                    </button>
                    <button
                      className="btn btn-primary"
                      onClick={() => void handleSubmitQuickSwitch()}
                      disabled={
                        quickSwitchSubmitting
                        || managedProvidersLoading
                        || !selectedQuickSwitchProvider
                        || !selectedQuickSwitchApiKey
                      }
                    >
                      {quickSwitchSubmitting
                        ? t('common.saving', '保存中...')
                        : t('codex.quickSwitch.apply', '立即切换')}
                    </button>
                  </div>
                </div>
              </div>
            </div>
          </div>
        )}

        {editingApiKeyCredentialsId && (
          <div className="modal-overlay" onClick={closeApiKeyCredentialsModal}>
            <div className="modal-content codex-add-modal" onClick={(e) => e.stopPropagation()}>
              <div className="modal-header">
                <h2>{`${t('instances.actions.edit', '编辑')} ${t('codex.addModal.token', 'API Key')}`}</h2>
                <button
                  className="modal-close"
                  onClick={closeApiKeyCredentialsModal}
                  aria-label={t('common.close', '关闭')}
                  disabled={savingApiKeyCredentials}
                >
                  <X />
                </button>
              </div>
              <div className="modal-body">
                <div className="add-section">
                  <div className="oauth-link">
                    <label>{t('codex.modelProviders.selectSavedProvider', '已保存供应商')}</label>
                    {managedProvidersLoading ? (
                      <div className="section-desc">{t('common.loading', '加载中...')}</div>
                    ) : managedProviders.length === 0 ? (
                      <div className="section-desc">
                        {t('codex.modelProviders.noSavedProviders', '暂无已保存供应商，可直接填写后自动保存。')}
                      </div>
                    ) : (
                      <div className="api-provider-chip-list">
                        {managedProviders.map((provider) => (
                          <button
                            key={provider.id}
                            className={`api-provider-chip ${editingManagedProviderId === provider.id ? 'active' : ''}`}
                            onClick={() => handleSelectEditingManagedProvider(provider.id)}
                            type="button"
                            disabled={savingApiKeyCredentials}
                          >
                            <span>{provider.name}</span>
                          </button>
                        ))}
                      </div>
                    )}
                  </div>
                  {selectedEditingManagedProvider && selectedEditingManagedProvider.apiKeys.length > 0 && (
                    <div className="oauth-link">
                      <label>{t('codex.modelProviders.selectSavedApiKey', '已保存 API Key')}</label>
                      <div className="api-provider-endpoint-list">
                        {selectedEditingManagedProvider.apiKeys.map((item) => (
                          <button
                            key={item.id}
                            className={`api-provider-endpoint-chip ${editingManagedProviderApiKeyId === item.id ? 'active' : ''}`}
                            onClick={() => handleSelectEditingManagedProviderApiKey(item.id)}
                            type="button"
                            disabled={savingApiKeyCredentials}
                          >
                            {item.name || t('codex.modelProviders.unnamedKey', '未命名 Key')}
                          </button>
                        ))}
                      </div>
                    </div>
                  )}
                  <div className="oauth-link">
                    <label>{t('codex.api.provider.label', '供应商')}</label>
                    <div className="api-provider-chip-list">
                      <button
                        className={`api-provider-chip ${editingApiProviderPresetId === CODEX_API_PROVIDER_CUSTOM_ID ? 'active' : ''}`}
                        onClick={() => handleSelectEditingApiProviderPreset(CODEX_API_PROVIDER_CUSTOM_ID)}
                        type="button"
                        disabled={savingApiKeyCredentials}
                      >
                        <span>{t('codex.api.provider.custom', '自定义')}</span>
                      </button>
                      {CODEX_API_PROVIDER_PRESETS.map((preset) => (
                        <button
                          key={preset.id}
                          className={`api-provider-chip ${editingApiProviderPresetId === preset.id ? 'active' : ''}`}
                          onClick={() => handleSelectEditingApiProviderPreset(preset.id)}
                          type="button"
                          disabled={savingApiKeyCredentials}
                        >
                          <span>{t(`codex.api.providers.${preset.id}.name`, preset.name)}</span>
                          {preset.isPartner && <Star size={12} className="api-provider-chip-badge" />}
                        </button>
                      ))}
                    </div>
                  </div>
                  {selectedEditingApiProviderPreset && selectedEditingApiProviderPreset.baseUrls.length > 1 && (
                    <div className="oauth-link">
                      <label>{t('codex.api.provider.endpoint', '供应商端点')}</label>
                      <div className="api-provider-endpoint-list">
                        {selectedEditingApiProviderPreset.baseUrls.map((baseUrl) => (
                          <button
                            key={baseUrl}
                            className={`api-provider-endpoint-chip ${editingApiBaseUrlCredentialsValue === baseUrl ? 'active' : ''}`}
                            onClick={() => setEditingApiBaseUrlCredentialsValue(baseUrl)}
                            type="button"
                            disabled={savingApiKeyCredentials}
                          >
                            {baseUrl}
                          </button>
                        ))}
                      </div>
                    </div>
                  )}
                  {selectedEditingApiProviderPreset && (
                    <div className="api-provider-hint-block">
                      <p className="api-provider-hint">
                        {t('codex.api.provider.hint', '已自动填写兼容 Base URL，可继续手动调整。')}
                      </p>
                      <div className="api-provider-links">
                        {selectedEditingApiProviderPreset.website && (
                          <button
                            className="btn btn-secondary"
                            onClick={() => void handleOpenProviderLink(selectedEditingApiProviderPreset.website || '')}
                            disabled={savingApiKeyCredentials}
                          >
                            <ExternalLink size={14} />
                            {t('codex.api.provider.website', '官网')}
                          </button>
                        )}
                        {selectedEditingApiProviderPreset.apiKeyUrl && (
                          <button
                            className="btn btn-secondary"
                            onClick={() => void handleOpenProviderLink(selectedEditingApiProviderPreset.apiKeyUrl || '')}
                            disabled={savingApiKeyCredentials}
                          >
                            <KeyRound size={14} />
                            {t('codex.api.provider.apiKeyPage', 'API Key 页面')}
                          </button>
                        )}
                      </div>
                    </div>
                  )}
                  <div className="oauth-link">
                    <label>{t('codex.addModal.token', 'API Key')}</label>
                    <div className="oauth-url-box oauth-manual-input">
                      <input
                        type="text"
                        value={editingApiKeyCredentialsValue}
                        onChange={(e) => setEditingApiKeyCredentialsValue(e.target.value)}
                        disabled={savingApiKeyCredentials}
                      />
                    </div>
                  </div>
                  <div className="oauth-link">
                    <label>{t('codex.api.baseUrl', 'Base URL')}</label>
                    <div className="oauth-url-box oauth-manual-input">
                      <input
                        type="text"
                        value={editingApiBaseUrlCredentialsValue}
                        onChange={(e) => setEditingApiBaseUrlCredentialsValue(e.target.value)}
                        placeholder={t('codex.api.baseUrlPlaceholder', '不填写则是官方默认')}
                        disabled={savingApiKeyCredentials}
                      />
                    </div>
                  </div>
                  <div className="oauth-link">
                    <label>{t('codex.modelProviders.newProviderName', '供应商名称（自动保存时使用，可选）')}</label>
                    <div className="oauth-url-box oauth-manual-input">
                      <input
                        type="text"
                        value={editingNewManagedProviderNameInput}
                        onChange={(e) => setEditingNewManagedProviderNameInput(e.target.value)}
                        placeholder={t('codex.modelProviders.newProviderNamePlaceholder', '不填则按域名自动生成')}
                        disabled={savingApiKeyCredentials}
                      />
                    </div>
                  </div>
                  <div className="api-key-edit-actions">
                    <button
                      className="btn btn-secondary"
                      onClick={closeApiKeyCredentialsModal}
                      disabled={savingApiKeyCredentials}
                    >
                      {t('common.cancel')}
                    </button>
                    <button
                      className="btn btn-primary"
                      onClick={() => void handleSubmitApiKeyCredentials()}
                      disabled={savingApiKeyCredentials || !editingApiKeyCredentialsValue.trim()}
                    >
                      {savingApiKeyCredentials ? t('common.saving', '保存中...') : t('common.save')}
                    </button>
                  </div>
                </div>
              </div>
            </div>
          </div>
        )}

        <ExportJsonModal
          isOpen={showExportModal}
          title={`${t('common.shared.export.title', '导出')} JSON`}
          jsonContent={exportJsonContent}
          hidden={exportJsonHidden}
          copied={exportJsonCopied}
          saving={savingExportJson}
          savedPath={exportSavedPath}
          canOpenSavedDirectory={canOpenExportSavedDirectory}
          pathCopied={exportPathCopied}
          onClose={closeExportModal}
          onToggleHidden={toggleExportJsonHidden}
          onCopyJson={copyExportJson}
          onSaveJson={saveExportJson}
          onOpenSavedDirectory={openExportSavedDirectory}
          onCopySavedPath={copyExportSavedPath}
        />

        {deleteConfirm && (<div className="modal-overlay" onClick={() => !deleting && setDeleteConfirm(null)}><div className="modal" onClick={(e) => e.stopPropagation()}>
          <div className="modal-header"><h2>{t('common.confirm')}</h2><button className="modal-close" onClick={() => !deleting && setDeleteConfirm(null)} aria-label={t('common.close', '关闭')}><X /></button></div>
          <div className="modal-body"><ModalErrorMessage message={deleteConfirmError} scrollKey={deleteConfirmErrorScrollKey} /><p>{deleteConfirm.message}</p></div>
          <div className="modal-footer"><button className="btn btn-secondary" onClick={() => setDeleteConfirm(null)} disabled={deleting}>{t('common.cancel')}</button>
            <button className="btn btn-danger" onClick={confirmDelete} disabled={deleting}>{t('common.confirm')}</button></div></div></div>)}

        {tagDeleteConfirm && (<div className="modal-overlay" onClick={() => !deletingTag && setTagDeleteConfirm(null)}><div className="modal" onClick={(e) => e.stopPropagation()}>
          <div className="modal-header"><h2>{t('common.confirm')}</h2><button className="modal-close" onClick={() => !deletingTag && setTagDeleteConfirm(null)} aria-label={t('common.close', '关闭')}><X /></button></div>
          <div className="modal-body"><ModalErrorMessage message={tagDeleteConfirmError} scrollKey={tagDeleteConfirmErrorScrollKey} /><p>{t('accounts.confirmDeleteTag', 'Delete tag "{{tag}}"? This tag will be removed from {{count}} accounts.', { tag: tagDeleteConfirm.tag, count: tagDeleteConfirm.count })}</p></div>
          <div className="modal-footer"><button className="btn btn-secondary" onClick={() => setTagDeleteConfirm(null)} disabled={deletingTag}>{t('common.cancel')}</button>
            <button className="btn btn-danger" onClick={confirmDeleteTag} disabled={deletingTag}>{deletingTag ? t('common.processing', '处理中...') : t('common.confirm')}</button></div></div></div>)}

        <TagEditModal isOpen={!!showTagModal} initialTags={accounts.find((a) => a.id === showTagModal)?.tags || []} availableTags={availableTags}
          onClose={() => setShowTagModal(null)} onSave={handleSaveTags} />

        {/* Codex 分组管理弹窗 */}
        <CodexAccountGroupModal
          isOpen={showCodexGroupModal}
          onClose={() => setShowCodexGroupModal(false)}
          onGroupsChanged={reloadCodexGroups}
          groupFilter={groupFilter}
          onToggleGroupFilter={toggleGroupFilterValue}
          onClearGroupFilter={clearGroupFilter}
        />

        {/* Codex 添加到分组弹窗 */}
        <CodexAddToGroupModal
          isOpen={showAddToCodexGroupModal}
          onClose={() => setShowAddToCodexGroupModal(false)}
          accountIds={Array.from(selected)}
          onAdded={reloadCodexGroups}
        />
      </>)}

      {activeTab === 'instances' && (
        <CodexInstancesContent accountsForSelect={sortedAccountsForInstances} />
      )}

      {activeTab === 'sessions' && (
        <CodexSessionManager />
      )}

      {activeTab === 'providers' && (
        <CodexModelProviderManager
          accounts={accounts}
          onProvidersChanged={setManagedProviders}
        />
      )}

      {activeTab === 'wakeup' && (
        <CodexWakeupContent
          accounts={accounts}
          onRefreshAccounts={async () => {
            await fetchAccounts();
            await fetchCurrentAccount();
          }}
        />
      )}
    </div>
  );
}
