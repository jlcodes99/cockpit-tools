/**
 * useProviderAccountsPage
 *
 * 通用 hook：封装所有 Provider AccountsPage（Kiro / Windsurf / GitHubCopilot / Codex）
 * 共享的 state、effects 和 handlers。
 *
 * 各平台页面只需提供一个 ProviderPageConfig 即可复用全部通用逻辑。
 */

import {
  useState,
  useEffect,
  useRef,
  useMemo,
  useCallback,
  type CSSProperties,
  type RefObject,
  type Dispatch,
  type SetStateAction,
} from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import { openUrl } from '@tauri-apps/plugin-opener';
import {
  isPrivacyModeEnabledByDefault,
  maskSensitiveValue,
  persistPrivacyModeEnabled,
} from '../utils/privacy';
import { useModalErrorState } from '../components/ModalErrorMessage';
import { useExportJsonModal } from './useExportJsonModal';
import { parseFileCorruptedError } from '../components/FileCorruptedModal';
import {
  emitAccountsChanged,
  emitCurrentAccountChanged,
  normalizeProviderPagePlatformId,
} from '../utils/accountSyncEvents';
import {
  consumeQueuedExternalProviderImportForPlatform,
  EXTERNAL_PROVIDER_IMPORT_EVENT,
  type ExternalProviderImportPayload,
} from '../utils/externalProviderImport';
import { useDropdownPanelPlacement } from './useDropdownPanelPlacement';
import { useEscClose } from './useEscClose';
import {
  ACCOUNTS_OVERVIEW_FILTER_PERSISTENCE_CHANGED_EVENT,
  type AccountsOverviewFilterPersistenceChangedDetail,
  normalizeAccountsOverviewScope,
  readAccountsOverviewFilterField,
  readAccountsOverviewFilterPersistenceEnabled,
  readAccountsOverviewFilterStringArray,
  removeAccountsOverviewFilterField,
  setAccountsOverviewFilterPersistenceEnabled,
  writeAccountsOverviewFilterField,
} from '../utils/accountsOverviewFilterPersistence';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type AddModalStatus = 'idle' | 'loading' | 'success' | 'error';
export type ExternalImportProgressStatus =
  | 'idle'
  | 'receiving'
  | 'fetching'
  | 'parsing'
  | 'importing'
  | 'refreshing'
  | 'success'
  | 'partial'
  | 'error';

export type ExternalImportProgressFailure = {
  index: number;
  label: string;
  error: string;
};

export type ExternalImportProgressState = {
  visible: boolean;
  status: ExternalImportProgressStatus;
  progress: number;
  total: number;
  success: number;
  failed: number;
  current: number;
  message: string;
  failures: ExternalImportProgressFailure[];
};
export type ViewMode = 'grid' | 'list';
export type SortDirection = 'asc' | 'desc';

/** 各平台需要提供的 OAuth 服务函数 */
export interface OAuthService {
  startLogin: () => Promise<OAuthStartResponse>;
  completeLogin: (loginId: string) => Promise<unknown>;
  cancelLogin: (loginId?: string) => Promise<void>;
  submitCallbackUrl?: (loginId: string, callbackUrl: string) => Promise<void>;
  openAuthUrl?: (url: string) => Promise<void>;
}

export interface OAuthStartResponse {
  loginId: string;
  userCode?: string;
  verificationUri?: string;
  verificationUriComplete?: string | null;
  expiresIn: number;
  intervalSeconds: number;
  callbackUrl?: string | null;
  /** Codex 模式使用 authUrl 而非 verificationUri */
  authUrl?: string;
}

/** 各平台需要提供的数据服务函数 */
export interface ProviderDataService {
  importFromJson: (content: string) => Promise<unknown[]>;
  importFromLocal?: () => Promise<unknown[]>;
  addWithToken?: (token: string) => Promise<unknown>;
  exportAccounts: (ids: string[]) => Promise<string>;
  injectToVSCode?: (accountId: string) => Promise<unknown>;
}

/** 各平台 store 需要提供的操作 */
export interface ProviderStoreActions<TAccount> {
  accounts: TAccount[];
  currentAccountId?: string | null;
  loading: boolean;
  error?: string | null;
  fetchCurrentAccountId?: () => Promise<string | null>;
  setCurrentAccountId?: (accountId: string | null) => void;
  fetchAccounts: () => Promise<void>;
  switchAccount?: (accountId: string) => Promise<unknown>;
  deleteAccounts: (ids: string[]) => Promise<void>;
  refreshToken: (id: string) => Promise<void>;
  refreshAllTokens: () => Promise<void>;
  updateAccountTags: (id: string, tags: string[]) => Promise<unknown>;
}

/** 配置对象：各平台页面的差异化配置 */
export interface ProviderPageConfig<TAccount extends ProviderAccountBase> {
  /** 平台标识，用于日志和 localStorage key */
  platformKey: string;
  /** OAuth 日志前缀 */
  oauthLogPrefix: string;
  /** localStorage key：flow notice 折叠状态 */
  flowNoticeCollapsedKey?: string;
  /** localStorage key：当前选中账号 */
  currentAccountIdKey?: string;
  /** 导出文件名前缀 */
  exportFilePrefix: string;
  /** Store 操作 */
  store: ProviderStoreActions<TAccount>;
  /** OAuth 服务（可选，Codex 等使用自定义 OAuth 流程的平台可不传） */
  oauthService?: OAuthService;
  /** 触发 OAuth 流程的 addTab key，默认 ['oauth'] */
  oauthTabKeys?: string[];
  /** 数据服务 */
  dataService: ProviderDataService;
  /** 获取展示用 email/displayName */
  getDisplayEmail: (account: TAccount) => string;
  /** 切号注入成功后的扩展回调（可选） */
  onInjectSuccess?: (params: {
    accountId: string;
    account: TAccount | undefined;
    displayEmail: string;
  }) => void | Promise<void>;
  /** OAuth 成功后的提示文案（可选） */
  resolveOauthSuccessMessage?: () => string;
  defaultSortBy?: string;
}

export interface ProviderAccountBase {
  id: string;
  created_at: number;
  tags?: string[] | null;
}

const DEFAULT_SORT_BY = 'created_at';
const DEFAULT_SORT_DIRECTION: SortDirection = 'desc';
const DEFAULT_VIEW_MODE: ViewMode = 'grid';

const normalizeSortDirection = (value: string | null): SortDirection =>
  value === 'asc' ? 'asc' : DEFAULT_SORT_DIRECTION;

const normalizeViewMode = (value: string | null): ViewMode =>
  value === 'list' ? 'list' : DEFAULT_VIEW_MODE;

const FILTER_FIELD_VIEW_MODE = 'view_mode';
const FILTER_FIELD_FILTER_TYPE = 'filter_type';
const FILTER_FIELD_SORT_BY = 'sort_by';
const FILTER_FIELD_SORT_DIRECTION = 'sort_direction';
const FILTER_FIELD_TAGS = 'tags';
const FILTER_FIELD_GROUP_BY_TAG = 'group_by_tag';

const normalizeStringArray = (value: unknown): string[] =>
  Array.isArray(value)
    ? value
        .filter((item): item is string => typeof item === 'string')
        .map((item) => item.trim())
        .filter(Boolean)
    : [];

type ExternalImportBundleParseMessages = {
  invalidJson: string;
  empty: string;
  providerMismatch: string;
  noItems: string;
  rawLineNoRefreshToken: (line: number) => string;
  rawLineMultipleRefreshTokens: (line: number) => string;
};

const CODEX_REFRESH_TOKEN_PATTERN = /rt_[A-Za-z0-9._-]+/g;
const COCKPIT_API_PROVIDER_ID = 'cockpit_api';
const COCKPIT_API_PROVIDER_NAME = 'Cockpit Api';
const COCKPIT_TOOLS_IMPORT_PATH_MARKERS = [
  '/api/cockpit-tools/import/',
  '/user/api/toolsimport/',
];

const readBundleMessage = (value: unknown): string | null => {
  if (typeof value !== 'string') return null;
  const trimmed = value.trim();
  return trimmed ? trimmed : null;
};

const readRecordString = (
  payload: Record<string, unknown>,
  keys: string[],
): string | null => {
  for (const key of keys) {
    const value = payload[key];
    if (typeof value === 'string' && value.trim()) {
      return value.trim();
    }
  }
  return null;
};

const parseLineDelimitedJsonObjects = (
  rawContent: string,
  invalidJsonMessage: string,
): unknown[] | null => {
  const lines = rawContent
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0);

  if (lines.length <= 1) return null;

  return lines.map((line) => {
    let parsed: unknown;
    try {
      parsed = JSON.parse(line) as unknown;
    } catch {
      throw new Error(invalidJsonMessage);
    }
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) {
      throw new Error(invalidJsonMessage);
    }
    return parsed;
  });
};

const isCodexDirectImportItem = (value: unknown): boolean => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const payload = value as Record<string, unknown>;
  const tokens = payload.tokens;
  if (
    typeof payload.id_token === 'string' &&
    payload.id_token.trim() &&
    typeof payload.access_token === 'string' &&
    payload.access_token.trim()
  ) {
    return true;
  }
  if (typeof payload.refresh_token === 'string' && payload.refresh_token.trim()) {
    return true;
  }
  if (
    typeof payload.auth_mode === 'string' &&
    payload.auth_mode.trim().toLowerCase() === 'apikey' &&
    typeof payload.OPENAI_API_KEY === 'string' &&
    payload.OPENAI_API_KEY.trim()
  ) {
    return true;
  }
  if (!tokens || typeof tokens !== 'object' || Array.isArray(tokens)) return false;
  const tokenPayload = tokens as Record<string, unknown>;
  const hasFullTokens =
    typeof tokenPayload.id_token === 'string' &&
    tokenPayload.id_token.trim() &&
    typeof tokenPayload.access_token === 'string' &&
    tokenPayload.access_token.trim();
  const hasRefreshTokenOnly =
    typeof tokenPayload.refresh_token === 'string' && tokenPayload.refresh_token.trim();
  return Boolean(hasFullTokens || hasRefreshTokenOnly);
};

const parseCodexRawRefreshTokenItems = (
  rawContent: string,
  messages: ExternalImportBundleParseMessages,
): unknown[] | null => {
  const lines = rawContent
    .split(/\r?\n/)
    .map((line, index) => ({ line: line.trim(), lineNumber: index + 1 }))
    .filter((item) => item.line.length > 0);

  if (lines.length === 0) return null;

  const items: unknown[] = [];
  for (const item of lines) {
    const matches = [...item.line.matchAll(CODEX_REFRESH_TOKEN_PATTERN)];
    if (matches.length === 0) {
      throw new Error(messages.rawLineNoRefreshToken(item.lineNumber));
    }
    if (matches.length > 1) {
      throw new Error(messages.rawLineMultipleRefreshTokens(item.lineNumber));
    }

    const match = matches[0];
    const refreshToken = match[0].trim();
    const accountNote = item.line.slice(0, match.index ?? 0).trim();
    items.push({
      refresh_token: refreshToken,
      ...(accountNote ? { account_note: accountNote } : {}),
    });
  }

  return items.length > 0 ? items : null;
};

const resolveExternalImportBundleItems = (
  rawContent: string,
  platformId: string,
  messages: ExternalImportBundleParseMessages,
): unknown[] => {
  let parsed: unknown;
  try {
    parsed = JSON.parse(rawContent) as unknown;
  } catch {
    let lineDelimitedError: unknown = null;
    try {
      const lineDelimitedItems = parseLineDelimitedJsonObjects(rawContent, messages.invalidJson);
      if (lineDelimitedItems && lineDelimitedItems.length > 0) {
        return lineDelimitedItems;
      }
    } catch (error) {
      lineDelimitedError = error;
    }

    if (platformId === 'codex') {
      try {
        const rawRefreshTokenItems = parseCodexRawRefreshTokenItems(rawContent, messages);
        if (rawRefreshTokenItems && rawRefreshTokenItems.length > 0) {
          return rawRefreshTokenItems;
        }
      } catch (error) {
        throw error;
      }
    }

    if (lineDelimitedError) throw lineDelimitedError;
    throw new Error(messages.invalidJson);
  }

  if (parsed && typeof parsed === 'object' && 'code' in parsed) {
    const code = (parsed as { code?: unknown }).code;
    if (code !== 200 && code !== '200') {
      throw new Error(
        readBundleMessage((parsed as { msg?: unknown }).msg) ??
          readBundleMessage((parsed as { message?: unknown }).message) ??
          readBundleMessage((parsed as { error?: unknown }).error) ??
          messages.empty,
      );
    }
  }

  const root =
    parsed && typeof parsed === 'object' && 'data' in parsed
      ? (parsed as { data?: unknown }).data
      : parsed;

  if (typeof root === 'string') {
    return resolveExternalImportBundleItems(root, platformId, messages);
  }

  if (Array.isArray(root)) {
    if (root.length === 0) {
      throw new Error(messages.noItems);
    }
    return root;
  }

  if (!root || typeof root !== 'object') {
    throw new Error(messages.empty);
  }

  const provider = (root as { provider?: unknown }).provider;
  if (typeof provider === 'string' && provider.trim() && provider.trim() !== platformId) {
    throw new Error(messages.providerMismatch);
  }

  const items = (root as { items?: unknown }).items;
  if (Array.isArray(items) && items.length > 0) {
    return items;
  }

  if (platformId === 'codex' && isCodexDirectImportItem(root)) {
    return [root];
  }

  if (!Array.isArray(items) || items.length === 0) {
    throw new Error(messages.noItems);
  }

  throw new Error(messages.noItems);
};

const normalizeExternalImportApiBaseUrl = (rawValue?: string | null): string | null => {
  const trimmed = (rawValue || '').trim();
  if (!trimmed) return null;
  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
      return null;
    }
    return `${parsed.origin}${parsed.pathname}`.replace(/\/+$/, '');
  } catch {
    return null;
  }
};

const deriveApiBaseUrlFromImportUrl = (importUrl?: string | null): string | null => {
  const trimmed = (importUrl || '').trim();
  if (!trimmed) return null;
  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
      return null;
    }
    return `${parsed.origin}/v1`;
  } catch {
    return null;
  }
};

const isCockpitToolsImportUrl = (importUrl?: string | null): boolean => {
  const trimmed = (importUrl || '').trim();
  if (!trimmed) return false;
  try {
    const parsed = new URL(trimmed);
    const pathname = parsed.pathname.toLowerCase();
    return COCKPIT_TOOLS_IMPORT_PATH_MARKERS.some((marker) => pathname.includes(marker));
  } catch {
    return false;
  }
};

const isCockpitApiImportItem = (item: Record<string, unknown>): boolean => {
  const providerId = readRecordString(item, ['api_provider_id', 'apiProviderId']);
  if (providerId?.toLowerCase() === COCKPIT_API_PROVIDER_ID) return true;

  const candidates = [
    readRecordString(item, ['api_provider_name', 'apiProviderName']),
    readRecordString(item, ['plan_type', 'planType']),
    readRecordString(item, ['account_note', 'accountNote']),
  ];
  const expected = COCKPIT_API_PROVIDER_NAME.toLowerCase();
  return candidates.some(
    (value) => value?.trim().toLowerCase().includes(expected),
  );
};

const withCockpitApiBaseUrl = (
  item: unknown,
  apiBaseUrl: string | null,
  isCockpitToolsImport: boolean,
): unknown => {
  if (!apiBaseUrl || !item || typeof item !== 'object' || Array.isArray(item)) {
    return item;
  }
  const payload = item as Record<string, unknown>;
  const authMode = readRecordString(payload, ['auth_mode', 'authMode']);
  const apiKey = readRecordString(payload, ['OPENAI_API_KEY', 'openai_api_key', 'openaiApiKey']);
  if (authMode?.toLowerCase() !== 'apikey' || !apiKey) {
    return item;
  }
  if (!isCockpitToolsImport && !isCockpitApiImportItem(payload)) {
    return item;
  }

  return {
    ...payload,
    base_url: apiBaseUrl,
    api_base_url: apiBaseUrl,
    api_provider_mode:
      readRecordString(payload, ['api_provider_mode', 'apiProviderMode']) ?? 'custom',
    api_provider_id:
      readRecordString(payload, ['api_provider_id', 'apiProviderId']) ?? COCKPIT_API_PROVIDER_ID,
    api_provider_name:
      readRecordString(payload, ['api_provider_name', 'apiProviderName']) ??
      COCKPIT_API_PROVIDER_NAME,
    plan_type:
      readRecordString(payload, ['plan_type', 'planType']) ?? COCKPIT_API_PROVIDER_NAME,
  };
};

const applyCockpitApiBaseUrlToExternalImportItems = (
  items: unknown[],
  request: ExternalProviderImportPayload,
): unknown[] => {
  const apiBaseUrl =
    normalizeExternalImportApiBaseUrl(request.apiBaseUrl) ??
    deriveApiBaseUrlFromImportUrl(request.importUrl);
  if (!apiBaseUrl) return items;

  const isCockpitToolsImport =
    Boolean(request.apiBaseUrl?.trim()) || isCockpitToolsImportUrl(request.importUrl);
  return items.map((item) => withCockpitApiBaseUrl(item, apiBaseUrl, isCockpitToolsImport));
};

const buildInitialExternalImportProgress = (): ExternalImportProgressState => ({
  visible: false,
  status: 'idle',
  progress: 0,
  total: 0,
  success: 0,
  failed: 0,
  current: 0,
  message: '',
  failures: [],
});

const isExternalImportRunning = (status: ExternalImportProgressStatus): boolean =>
  ['receiving', 'fetching', 'parsing', 'importing', 'refreshing'].includes(status);

const resolveExternalImportItemLabel = (
  item: unknown,
  fallback: string,
): string => {
  if (!item || typeof item !== 'object') return fallback;
  const payload = item as Record<string, unknown>;
  const profile = payload['https://api.openai.com/profile'];
  const auth = payload['https://api.openai.com/auth'];
  const candidates = [
    payload.email,
    payload.account_email,
    payload.id,
    profile && typeof profile === 'object'
      ? (profile as Record<string, unknown>).email
      : null,
    auth && typeof auth === 'object'
      ? (auth as Record<string, unknown>).chatgpt_user_id
      : null,
  ];
  for (const candidate of candidates) {
    if (typeof candidate === 'string' && candidate.trim()) {
      return candidate.trim();
    }
  }
  return fallback;
};

const collectImportedAccountIds = (imported: unknown): string[] => {
  const items = Array.isArray(imported) ? imported : [imported];
  return items
    .map((item) => {
      if (!item || typeof item !== 'object') return '';
      const id = (item as { id?: unknown }).id;
      return typeof id === 'string' ? id.trim() : '';
    })
    .filter(Boolean);
};

// ---------------------------------------------------------------------------
// Hook return type
// ---------------------------------------------------------------------------

export interface UseProviderAccountsPageReturn {
  // i18n
  t: ReturnType<typeof useTranslation>['t'];
  locale: string;

  // Privacy
  privacyModeEnabled: boolean;
  togglePrivacyMode: () => void;
  maskAccountText: (value?: string | null) => string;

  // View mode
  viewMode: ViewMode;
  setViewMode: (mode: ViewMode) => void;

  // Search & Filter
  searchQuery: string;
  setSearchQuery: (q: string) => void;
  filterPersistenceEnabled: boolean;
  setFilterPersistenceEnabled: (enabled: boolean) => void;
  filterPersistenceScope: string;
  filterType: string;
  setFilterType: (type: string) => void;

  // Sort
  sortBy: string;
  setSortBy: (sort: string) => void;
  sortDirection: SortDirection;
  setSortDirection: Dispatch<SetStateAction<SortDirection>>;

  // Selection
  selected: Set<string>;
  setSelected: Dispatch<SetStateAction<Set<string>>>;
  toggleSelect: (id: string) => void;
  toggleSelectAll: (filteredIds: string[]) => void;

  // Tags
  tagFilter: string[];
  setTagFilter: (tags: string[]) => void;
  groupByTag: boolean;
  setGroupByTag: (v: boolean) => void;
  showTagFilter: boolean;
  setShowTagFilter: Dispatch<SetStateAction<boolean>>;
  showTagModal: string | null;
  setShowTagModal: (id: string | null) => void;
  tagFilterRef: RefObject<HTMLDivElement | null>;
  tagFilterPanelRef: RefObject<HTMLDivElement | null>;
  tagFilterPanelPlacement: 'top' | 'bottom';
  tagFilterScrollContainerStyle: CSSProperties | undefined;
  availableTags: string[];
  toggleTagFilterValue: (tag: string) => void;
  clearTagFilter: () => void;
  tagDeleteConfirm: { tag: string; count: number } | null;
  tagDeleteConfirmError: string | null;
  tagDeleteConfirmErrorScrollKey: number;
  setTagDeleteConfirm: (v: { tag: string; count: number } | null) => void;
  deletingTag: boolean;
  requestDeleteTag: (tag: string) => void;
  confirmDeleteTag: () => Promise<void>;
  openTagModal: (accountId: string) => void;
  handleSaveTags: (tags: string[]) => Promise<void>;

  // CRUD
  refreshing: string | null;
  refreshingAll: boolean;
  injecting: string | null;
  handleRefresh: (accountId: string) => Promise<void>;
  handleRefreshAll: () => Promise<void>;
  handleDelete: (accountId: string) => void;
  handleBatchDelete: () => void;
  deleteConfirm: { ids: string[]; message: string } | null;
  deleteConfirmError: string | null;
  deleteConfirmErrorScrollKey: number;
  setDeleteConfirm: (v: { ids: string[]; message: string } | null) => void;
  deleting: boolean;
  confirmDelete: () => Promise<void>;

  // Messages
  message: { text: string; tone?: 'error' | 'success' } | null;
  setMessage: (
    msg:
      | {
          text: string;
          tone?: 'error' | 'success';
        }
      | null
  ) => void;

  // Export
  exporting: boolean;
  handleExport: (scopeIds?: string[]) => Promise<void>;
  handleExportByIds: (ids: string[], fileNameBase?: string) => Promise<void>;
  getScopedSelectedCount: (scopeIds?: string[]) => number;
  showExportModal: boolean;
  closeExportModal: () => void;
  exportJsonContent: string;
  exportJsonHidden: boolean;
  toggleExportJsonHidden: () => void;
  exportJsonCopied: boolean;
  copyExportJson: () => Promise<void>;
  savingExportJson: boolean;
  saveExportJson: () => Promise<void>;
  exportSavedPath: string | null;
  canOpenExportSavedDirectory: boolean;
  openExportSavedDirectory: () => Promise<void>;
  copyExportSavedPath: () => Promise<void>;
  exportPathCopied: boolean;

  // Add modal
  showAddModal: boolean;
  setShowAddModal: (v: boolean) => void;
  addTab: string;
  setAddTab: (tab: string) => void;
  addStatus: AddModalStatus;
  setAddStatus: (s: AddModalStatus) => void;
  addMessage: string | null;
  setAddMessage: (msg: string | null) => void;
  tokenInput: string;
  setTokenInput: (v: string) => void;
  importing: boolean;
  openAddModal: (tab: string) => void;
  closeAddModal: () => void;
  resetAddModalState: () => void;
  handleTokenImport: () => Promise<void>;
  handleImportJsonFile: (file: File) => Promise<void>;
  handleImportFromLocal: (() => Promise<void>) | null;
  handlePickImportFile: () => void;
  importFileInputRef: RefObject<HTMLInputElement | null>;
  externalImportProgress: ExternalImportProgressState;
  closeExternalImportProgressModal: () => void;

  // OAuth (device flow style: Kiro / Windsurf / GHCP)
  oauthUrl: string | null;
  oauthCallbackUrl: string | null;
  oauthUrlCopied: boolean;
  oauthUserCode: string | null;
  oauthUserCodeCopied: boolean;
  oauthMeta: { expiresIn: number; intervalSeconds: number } | null;
  oauthPrepareError: string | null;
  oauthCompleteError: string | null;
  oauthPolling: boolean;
  oauthTimedOut: boolean;
  oauthManualCallbackInput: string;
  setOauthManualCallbackInput: (value: string) => void;
  oauthManualCallbackSubmitting: boolean;
  oauthManualCallbackError: string | null;
  oauthSupportsManualCallback: boolean;
  handleCopyOauthUrl: () => Promise<void>;
  handleCopyOauthUserCode: () => Promise<void>;
  handleRetryOauth: () => void;
  handleRetryOauthComplete: () => void;
  handleOpenOauthUrl: () => Promise<void>;
  handleSubmitOauthCallbackUrl: () => Promise<void>;

  // Inject / Switch
  handleInjectToVSCode: ((accountId: string) => Promise<void>) | null;

  // Flow notice
  isFlowNoticeCollapsed: boolean;
  setIsFlowNoticeCollapsed: Dispatch<SetStateAction<boolean>>;

  // Current account
  currentAccountId: string | null;
  setCurrentAccountId: (id: string | null) => void;

  // Utilities
  formatDate: (timestamp: number) => string;
  normalizeTag: (tag: string) => string;
  resolveDefaultExportPath: (fileName: string) => Promise<string>;
  saveJsonFile: (json: string, defaultFileName: string) => Promise<string | null>;
}

// ---------------------------------------------------------------------------
// Hook implementation
// ---------------------------------------------------------------------------

export function useProviderAccountsPage<TAccount extends ProviderAccountBase>(
  config: ProviderPageConfig<TAccount>,
): UseProviderAccountsPageReturn {
  const { t, i18n } = useTranslation();
  const locale = i18n.language || 'zh-CN';


  const {
    platformKey,
    oauthLogPrefix,
    flowNoticeCollapsedKey,
    currentAccountIdKey,
    exportFilePrefix,
    store,
    oauthService,
    oauthTabKeys: oauthTabKeysConfig,
    dataService,
    defaultSortBy: defaultSortByConfig,
  } = config;
  const defaultSortBy = defaultSortByConfig?.trim() || DEFAULT_SORT_BY;

  const oauthTabKeys = useMemo(() => {
    const normalized = (oauthTabKeysConfig || [])
      .map((item) => item.trim())
      .filter(Boolean);
    return normalized.length > 0 ? normalized : ['oauth'];
  }, [oauthTabKeysConfig]);
  const platformId = useMemo(
    () => normalizeProviderPagePlatformId(platformKey),
    [platformKey],
  );

  const {
    accounts,
    currentAccountId: storeCurrentAccountId,
    error: storeError,
    fetchAccounts,
    deleteAccounts,
    refreshToken,
    refreshAllTokens,
    switchAccount,
    setCurrentAccountId: setStoreCurrentAccountId,
    updateAccountTags,
  } = store;
  const filterPersistenceScope = useMemo(
    () => normalizeAccountsOverviewScope(platformKey),
    [platformKey],
  );
  const [filterPersistenceEnabled, setFilterPersistenceEnabledState] = useState<boolean>(() =>
    readAccountsOverviewFilterPersistenceEnabled(filterPersistenceScope),
  );
  const managesCurrentAccountId = typeof setStoreCurrentAccountId === 'function';

  const setFilterPersistenceEnabled = useCallback(
    (enabled: boolean) => {
      setFilterPersistenceEnabledState(enabled);
      setAccountsOverviewFilterPersistenceEnabled(filterPersistenceScope, enabled);
    },
    [filterPersistenceScope],
  );

  useEffect(() => {
    const handleFilterPersistenceChanged = (event: Event) => {
      const detail = (event as CustomEvent<AccountsOverviewFilterPersistenceChangedDetail>).detail;
      if (!detail || detail.scope !== filterPersistenceScope) {
        return;
      }
      setFilterPersistenceEnabledState(Boolean(detail.enabled));
    };
    window.addEventListener(
      ACCOUNTS_OVERVIEW_FILTER_PERSISTENCE_CHANGED_EVENT,
      handleFilterPersistenceChanged as EventListener,
    );
    return () => {
      window.removeEventListener(
        ACCOUNTS_OVERVIEW_FILTER_PERSISTENCE_CHANGED_EVENT,
        handleFilterPersistenceChanged as EventListener,
      );
    };
  }, [filterPersistenceScope]);

  // ─── Privacy ──────────────────────────────────────────────────────────
  const [privacyModeEnabled, setPrivacyModeEnabled] = useState<boolean>(() =>
    isPrivacyModeEnabledByDefault(),
  );

  const togglePrivacyMode = useCallback(() => {
    setPrivacyModeEnabled((prev) => {
      const next = !prev;
      persistPrivacyModeEnabled(next);
      return next;
    });
  }, []);

  const maskAccountText = useCallback(
    (value?: string | null) => maskSensitiveValue(value, privacyModeEnabled),
    [privacyModeEnabled],
  );

  // ─── View Mode ────────────────────────────────────────────────────────
  const [viewMode, setViewMode] = useState<ViewMode>(() => {
    if (!readAccountsOverviewFilterPersistenceEnabled(filterPersistenceScope)) {
      return DEFAULT_VIEW_MODE;
    }
    const saved = readAccountsOverviewFilterField<string | null>(
      filterPersistenceScope,
      FILTER_FIELD_VIEW_MODE,
      null,
    );
    return normalizeViewMode(saved);
  });

  // ─── Search & Filter ──────────────────────────────────────────────────
  const [searchQuery, setSearchQuery] = useState('');
  const [filterType, setFilterType] = useState<string>(() => {
    if (!readAccountsOverviewFilterPersistenceEnabled(filterPersistenceScope)) {
      return 'all';
    }
    const saved = readAccountsOverviewFilterField<string | null>(
      filterPersistenceScope,
      FILTER_FIELD_FILTER_TYPE,
      null,
    );
    return saved?.trim() ? saved : 'all';
  });

  // ─── Sort ─────────────────────────────────────────────────────────────
  const [sortBy, setSortBy] = useState<string>(() => {
    if (!readAccountsOverviewFilterPersistenceEnabled(filterPersistenceScope)) {
      return defaultSortBy;
    }
    const saved = readAccountsOverviewFilterField<string | null>(
      filterPersistenceScope,
      FILTER_FIELD_SORT_BY,
      null,
    );
    return saved?.trim() ? saved : defaultSortBy;
  });
  const [sortDirection, setSortDirection] = useState<SortDirection>(() => {
    if (!readAccountsOverviewFilterPersistenceEnabled(filterPersistenceScope)) {
      return DEFAULT_SORT_DIRECTION;
    }
    const saved = readAccountsOverviewFilterField<string | null>(
      filterPersistenceScope,
      FILTER_FIELD_SORT_DIRECTION,
      null,
    );
    return normalizeSortDirection(saved);
  });

  useEffect(() => {
    if (!filterPersistenceEnabled) {
      removeAccountsOverviewFilterField(filterPersistenceScope, FILTER_FIELD_VIEW_MODE);
      return;
    }
    writeAccountsOverviewFilterField(filterPersistenceScope, FILTER_FIELD_VIEW_MODE, viewMode);
  }, [filterPersistenceEnabled, filterPersistenceScope, viewMode]);

  useEffect(() => {
    if (!filterPersistenceEnabled) {
      removeAccountsOverviewFilterField(filterPersistenceScope, FILTER_FIELD_SORT_BY);
      return;
    }
    writeAccountsOverviewFilterField(filterPersistenceScope, FILTER_FIELD_SORT_BY, sortBy);
  }, [filterPersistenceEnabled, filterPersistenceScope, sortBy]);

  useEffect(() => {
    if (!filterPersistenceEnabled) {
      removeAccountsOverviewFilterField(filterPersistenceScope, FILTER_FIELD_SORT_DIRECTION);
      return;
    }
    writeAccountsOverviewFilterField(
      filterPersistenceScope,
      FILTER_FIELD_SORT_DIRECTION,
      sortDirection,
    );
  }, [filterPersistenceEnabled, filterPersistenceScope, sortDirection]);

  useEffect(() => {
    if (!filterPersistenceEnabled) {
      removeAccountsOverviewFilterField(filterPersistenceScope, FILTER_FIELD_FILTER_TYPE);
      return;
    }
    writeAccountsOverviewFilterField(filterPersistenceScope, FILTER_FIELD_FILTER_TYPE, filterType);
  }, [filterPersistenceEnabled, filterPersistenceScope, filterType]);

  // ─── Selection ────────────────────────────────────────────────────────
  const [selected, setSelected] = useState<Set<string>>(new Set());

  const toggleSelect = useCallback((id: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const toggleSelectAll = useCallback(
    (filteredIds: string[]) => {
      const scopedIds = Array.from(new Set(filteredIds.filter(Boolean)));
      if (scopedIds.length === 0) {
        return;
      }
      const allSelected = scopedIds.every((id) => selected.has(id));
      setSelected((prev) => {
        const next = new Set(prev);
        if (allSelected) {
          scopedIds.forEach((id) => next.delete(id));
        } else {
          scopedIds.forEach((id) => next.add(id));
        }
        return next;
      });
    },
    [selected],
  );

  // ─── Tags ─────────────────────────────────────────────────────────────
  const [tagFilter, setTagFilter] = useState<string[]>(() => {
    if (!readAccountsOverviewFilterPersistenceEnabled(filterPersistenceScope)) {
      return [];
    }
    return normalizeStringArray(
      readAccountsOverviewFilterStringArray(filterPersistenceScope, FILTER_FIELD_TAGS),
    );
  });
  const [groupByTag, setGroupByTag] = useState<boolean>(() => {
    if (!readAccountsOverviewFilterPersistenceEnabled(filterPersistenceScope)) {
      return false;
    }
    return Boolean(
      readAccountsOverviewFilterField<unknown>(
        filterPersistenceScope,
        FILTER_FIELD_GROUP_BY_TAG,
        false,
      ),
    );
  });
  const [showTagFilter, setShowTagFilter] = useState(false);
  const [showTagModal, setShowTagModal] = useState<string | null>(null);
  const [tagDeleteConfirm, rawSetTagDeleteConfirm] = useState<{
    tag: string;
    count: number;
  } | null>(null);
  const {
    message: tagDeleteConfirmError,
    scrollKey: tagDeleteConfirmErrorScrollKey,
    set: setTagDeleteConfirmError,
  } = useModalErrorState();
  const [deletingTag, setDeletingTag] = useState(false);
  const setTagDeleteConfirm = useCallback((value: { tag: string; count: number } | null) => {
    setTagDeleteConfirmError(null);
    rawSetTagDeleteConfirm(value);
  }, []);
  const tagFilterRef = useRef<HTMLDivElement | null>(null);

  const normalizeTag = useCallback((tag: string) => tag.trim().toLowerCase(), []);

  const availableTags = useMemo(() => {
    const set = new Set<string>();
    accounts.forEach((account) => {
      (account.tags || []).forEach((tag) => {
        const normalized = normalizeTag(tag);
        if (normalized) set.add(normalized);
      });
    });
    return Array.from(set).sort((a, b) => a.localeCompare(b));
  }, [accounts, normalizeTag]);
  const {
    panelRef: tagFilterPanelRef,
    panelPlacement: tagFilterPanelPlacement,
    scrollContainerStyle: tagFilterScrollContainerStyle,
  } = useDropdownPanelPlacement(tagFilterRef, showTagFilter, availableTags.length);

  const toggleTagFilterValue = useCallback((tag: string) => {
    setTagFilter((prev) => {
      if (prev.includes(tag)) return prev.filter((item) => item !== tag);
      return [...prev, tag];
    });
  }, []);

  const clearTagFilter = useCallback(() => {
    setTagFilter([]);
  }, []);

  useEffect(() => {
    if (!filterPersistenceEnabled) {
      removeAccountsOverviewFilterField(filterPersistenceScope, FILTER_FIELD_TAGS);
      return;
    }
    writeAccountsOverviewFilterField(filterPersistenceScope, FILTER_FIELD_TAGS, tagFilter);
  }, [filterPersistenceEnabled, filterPersistenceScope, tagFilter]);

  useEffect(() => {
    if (!filterPersistenceEnabled) {
      removeAccountsOverviewFilterField(filterPersistenceScope, FILTER_FIELD_GROUP_BY_TAG);
      return;
    }
    writeAccountsOverviewFilterField(
      filterPersistenceScope,
      FILTER_FIELD_GROUP_BY_TAG,
      groupByTag,
    );
  }, [filterPersistenceEnabled, filterPersistenceScope, groupByTag]);

  const requestDeleteTag = useCallback(
    (tag: string) => {
      const normalized = normalizeTag(tag);
      if (!normalized) return;
      const count = accounts.filter((acc) =>
        (acc.tags || []).some((t) => normalizeTag(t) === normalized),
      ).length;
      setTagDeleteConfirm({ tag: normalized, count });
    },
    [accounts, normalizeTag],
  );

  const confirmDeleteTag = useCallback(async () => {
    if (!tagDeleteConfirm || deletingTag) return;
    setDeletingTag(true);
    setTagDeleteConfirmError(null);
    const target = tagDeleteConfirm.tag;
    try {
      const affectedAccounts = accounts.filter((acc) =>
        (acc.tags || []).some((t) => normalizeTag(t) === target),
      );
      for (const acc of affectedAccounts) {
        const newTags = (acc.tags || []).filter((t) => normalizeTag(t) !== target);
        await updateAccountTags(acc.id, newTags);
      }
      setTagFilter((prev) => prev.filter((t) => normalizeTag(t) !== target));
      setTagDeleteConfirm(null);
    } catch (error) {
      setTagDeleteConfirmError(
        t('messages.actionFailed', {
          action: t('common.delete'),
          error: String(error),
        }),
      );
    } finally {
      setDeletingTag(false);
    }
  }, [tagDeleteConfirm, deletingTag, accounts, normalizeTag, updateAccountTags, setTagDeleteConfirm, t]);

  const openTagModal = useCallback((accountId: string) => {
    setShowTagModal(accountId);
  }, []);

  const handleSaveTags = useCallback(
    async (tags: string[]) => {
      if (!showTagModal) return;
      const scrollY = window.scrollY;
      await updateAccountTags(showTagModal, tags);
      setShowTagModal(null);
      window.requestAnimationFrame(() => {
        window.requestAnimationFrame(() => {
          window.scrollTo({ top: scrollY, behavior: 'auto' });
        });
      });
    },
    [showTagModal, updateAccountTags],
  );

  // ─── Tag filter click-outside ─────────────────────────────────────────
  useEffect(() => {
    if (!showTagFilter) return;
    const handleClick = (event: MouseEvent) => {
      if (!tagFilterRef.current) return;
      if (!tagFilterRef.current.contains(event.target as Node)) {
        setShowTagFilter(false);
      }
    };
    document.addEventListener('mousedown', handleClick);
    return () => document.removeEventListener('mousedown', handleClick);
  }, [showTagFilter]);

  // ─── Fetch on mount ───────────────────────────────────────────────────
  useEffect(() => {
    fetchAccounts();
  }, [fetchAccounts]);

  // ─── CRUD ─────────────────────────────────────────────────────────────
  const [refreshing, setRefreshing] = useState<string | null>(null);
  const [refreshingAll, setRefreshingAll] = useState(false);
  const [injecting, setInjecting] = useState<string | null>(null);
  const [deleteConfirm, rawSetDeleteConfirm] = useState<{
    ids: string[];
    message: string;
  } | null>(null);
  const {
    message: deleteConfirmError,
    scrollKey: deleteConfirmErrorScrollKey,
    set: setDeleteConfirmError,
  } = useModalErrorState();
  const [deleting, setDeleting] = useState(false);
  const [message, setMessage] = useState<{ text: string; tone?: 'error' | 'success' } | null>(null);
  const setDeleteConfirm = useCallback((value: { ids: string[]; message: string } | null) => {
    setDeleteConfirmError(null);
    rawSetDeleteConfirm(value);
  }, []);

  useEffect(() => {
    if (!storeError) return;

    const corrupted = parseFileCorruptedError(storeError);
    if (corrupted) {
      setMessage({
        text: t('error.fileCorrupted.description', '文件 {{fileName}} 已损坏，无法解析。', {
          fileName: corrupted.file_name,
        }),
        tone: 'error',
      });
      return;
    }

    setMessage({
      text: String(storeError).replace(/^Error:\s*/, ''),
      tone: 'error',
    });
  }, [storeError, t]);

  const handleRefresh = useCallback(
    async (accountId: string) => {
      setRefreshing(accountId);
      try {
        await refreshToken(accountId);
      } catch (e) {
        console.error(e);
      }
      setRefreshing(null);
    },
    [refreshToken],
  );

  const handleRefreshAll = useCallback(async () => {
    setRefreshingAll(true);
    try {
      await refreshAllTokens();
    } catch (e) {
      console.error(e);
    }
    setRefreshingAll(false);
  }, [refreshAllTokens]);

  const handleDelete = useCallback(
    (accountId: string) => {
      setDeleteConfirm({
        ids: [accountId],
        message: t('messages.deleteConfirm', '确定要删除此账号吗？'),
      });
    },
    [t],
  );

  const handleBatchDelete = useCallback(() => {
    if (selected.size === 0) return;
    setDeleteConfirm({
      ids: Array.from(selected),
      message: t('messages.batchDeleteConfirm', { count: selected.size }),
    });
  }, [selected, t]);

  const confirmDelete = useCallback(async () => {
    if (!deleteConfirm || deleting) return;
    setDeleting(true);
    setDeleteConfirmError(null);
    try {
      await deleteAccounts(deleteConfirm.ids);
      setSelected((prev) => {
        const next = new Set(prev);
        deleteConfirm.ids.forEach((id) => next.delete(id));
        return next;
      });
      setDeleteConfirm(null);
    } catch (error) {
      setDeleteConfirmError(
        t('messages.actionFailed', {
          action: t('common.delete'),
          error: String(error),
        }),
      );
    } finally {
      setDeleting(false);
    }
  }, [deleteConfirm, deleting, deleteAccounts, setDeleteConfirm, t]);

  // ─── Inject ───────────────────────────────────────────────────────────
  const handleInjectToVSCode = useMemo(() => {
    if (!dataService.injectToVSCode) return null;
    const injectFn = dataService.injectToVSCode;
    return async (accountId: string) => {
      setMessage(null);
      setInjecting(accountId);
      const account = accounts.find((item) => item.id === accountId);
      const displayEmail = account ? config.getDisplayEmail(account) : accountId;
      try {
        await injectFn(accountId);
        setCurrentAccountId(accountId);
        if (platformId) {
          await emitCurrentAccountChanged({
            platformId,
            accountId,
            reason: 'switch',
          });
        }
        setMessage({ text: t('messages.switched', { email: maskAccountText(displayEmail) }) });
        if (config.onInjectSuccess) {
          try {
            await config.onInjectSuccess({
              accountId,
              account,
              displayEmail,
            });
          } catch (callbackError) {
            console.error(`[${platformKey}] onInjectSuccess callback failed:`, callbackError);
          }
        }
      } catch (e: unknown) {
        setMessage({
          text: t('messages.switchFailed', {
            error: String(e) || t('common.failed', 'Failed'),
          }),
          tone: 'error',
        });
      }
      setInjecting(null);
    };
  }, [accounts, config, dataService.injectToVSCode, maskAccountText, platformId, platformKey, t]);

  // ─── Export ───────────────────────────────────────────────────────────
  const handleExportError = useCallback(
    (error: unknown) => {
      setMessage({
        text: t('messages.exportFailed', { error: String(error) }),
        tone: 'error',
      });
    },
    [t],
  );

  const exportModal = useExportJsonModal({
    exportFilePrefix,
    exportJsonByIds: dataService.exportAccounts,
    onError: handleExportError,
  });

  const handleExportByIds = useCallback(
    async (ids: string[], fileNameBase?: string) => {
      if (!ids.length) return;
      await exportModal.startExport(ids, fileNameBase);
    },
    [exportModal.startExport],
  );

  const resolveScopedSelection = useCallback(
    (scopeIds?: string[]) => {
      const visibleIds = Array.isArray(scopeIds)
        ? scopeIds.filter(Boolean)
        : accounts.map((account) => account.id);
      const visibleIdSet = new Set(visibleIds);
      const selectedVisibleIds = Array.from(selected).filter((id) => visibleIdSet.has(id));

      return { visibleIds, selectedVisibleIds };
    },
    [accounts, selected],
  );

  const getScopedSelectedCount = useCallback(
    (scopeIds?: string[]) => resolveScopedSelection(scopeIds).selectedVisibleIds.length,
    [resolveScopedSelection],
  );

  const handleExport = useCallback(async (scopeIds?: string[]) => {
    try {
      const { visibleIds, selectedVisibleIds } = resolveScopedSelection(scopeIds);
      const ids = selectedVisibleIds.length > 0 ? selectedVisibleIds : visibleIds;
      await handleExportByIds(ids);
    } catch (error) {
      handleExportError(error);
    }
  }, [resolveScopedSelection, handleExportByIds, handleExportError]);

  const exporting = exportModal.preparing;

  // ─── Add Modal ────────────────────────────────────────────────────────
  const [showAddModal, setShowAddModal] = useState(false);
  const [addTab, setAddTab] = useState<string>('oauth');
  const [addStatus, setAddStatus] = useState<AddModalStatus>('idle');
  const [addMessage, setAddMessage] = useState<string | null>(null);
  const [tokenInput, setTokenInput] = useState('');
  const [importing, setImporting] = useState(false);
  const [externalAutoImportNonce, setExternalAutoImportNonce] = useState(0);
  const [externalImportProgress, setExternalImportProgress] =
    useState<ExternalImportProgressState>(() => buildInitialExternalImportProgress());
  const externalImportRunIdRef = useRef(0);

  const showAddModalRef = useRef(showAddModal);
  const addTabRef = useRef(addTab);
  const addStatusRef = useRef(addStatus);
  const importFileInputRef = useRef<HTMLInputElement | null>(null);
  const oauthServiceRef = useRef(oauthService);

  useEffect(() => {
    showAddModalRef.current = showAddModal;
    addTabRef.current = addTab;
    addStatusRef.current = addStatus;
    oauthServiceRef.current = oauthService;
  }, [showAddModal, addTab, addStatus, oauthService]);

  const resetAddModalState = useCallback(() => {
    oauthAttemptSeqRef.current += 1;
    setAddStatus('idle');
    setAddMessage('');
    setTokenInput('');
    setOauthUrl(null);
    setOauthCallbackUrl(null);
    setOauthUrlCopied(false);
    setOauthUserCode(null);
    setOauthUserCodeCopied(false);
    setOauthMeta(null);
    setOauthPrepareError(null);
    setOauthCompleteError(null);
    setOauthTimedOut(false);
    setOauthPolling(false);
    setOauthManualCallbackInput('');
    setOauthManualCallbackSubmitting(false);
    setOauthManualCallbackError(null);
    oauthActiveRef.current = false;
    oauthCompletingRef.current = false;
    oauthLoginIdRef.current = null;
  }, []);

  const openAddModal = useCallback(
    (tab: string) => {
      setAddTab(tab);
      setShowAddModal(true);
      resetAddModalState();
    },
    [resetAddModalState],
  );

  const closeAddModal = useCallback(() => {
    setShowAddModal(false);
    resetAddModalState();
  }, [resetAddModalState]);

  useEscClose(showAddModal, closeAddModal);

  const closeExternalImportProgressModal = useCallback(() => {
    setExternalImportProgress((current) => {
      if (isExternalImportRunning(current.status)) {
        return current;
      }
      return buildInitialExternalImportProgress();
    });
  }, []);

  const runExternalProviderImport = useCallback(
    (request: ExternalProviderImportPayload) => {
      if (!platformId) return;
      const importUrl = request.importUrl?.trim();
      const runId = externalImportRunIdRef.current + 1;
      externalImportRunIdRef.current = runId;

      const updateProgress = (patch: Partial<ExternalImportProgressState>) => {
        setExternalImportProgress((current) => {
          if (externalImportRunIdRef.current !== runId) {
            return current;
          }
          return {
            ...current,
            ...patch,
            visible: true,
          };
        });
      };

      updateProgress({
        status: 'receiving',
        progress: 5,
        total: 0,
        success: 0,
        failed: 0,
        current: 0,
        message: t(
          'common.shared.externalImport.statusReceiving',
          '正在接收导入请求...',
        ),
        failures: [],
      });

      void (async () => {
        try {
          let content = request.token.trim();
          if (importUrl) {
            updateProgress({
              status: 'fetching',
              progress: 20,
              message: t(
                'common.shared.externalImport.statusFetching',
                '正在获取导入包...',
              ),
            });
            content = await invoke<string>('external_import_fetch_import_url', {
              importUrl,
            });
          }
          if (!content.trim()) {
            throw new Error(t('common.shared.externalImport.bundleEmpty', '导入包内容为空'));
          }

          updateProgress({
            status: 'parsing',
            progress: 35,
            message: t(
              'common.shared.externalImport.statusParsing',
              '正在解析 Codex JSON...',
            ),
          });
          const resolvedItems = resolveExternalImportBundleItems(content, platformId, {
            invalidJson: t(
              'common.shared.externalImport.bundleInvalidJson',
              '导入包不是有效 JSON',
            ),
            empty: t('common.shared.externalImport.bundleEmpty', '导入包内容为空'),
            providerMismatch: t(
              'common.shared.externalImport.bundleProviderMismatch',
              '导入包平台不匹配',
            ),
            noItems: t(
              'common.shared.externalImport.bundleNoItems',
              '导入包没有可导入内容',
            ),
            rawLineNoRefreshToken: (line) =>
              t('common.shared.externalImport.rawLineNoRefreshToken', {
                line,
                defaultValue: '第 {{line}} 行没有匹配到 refresh_token',
              }),
            rawLineMultipleRefreshTokens: (line) =>
              t('common.shared.externalImport.rawLineMultipleRefreshTokens', {
                line,
                defaultValue: '第 {{line}} 行匹配到多个 refresh_token，请只保留一个',
              }),
          });
          const items =
            platformId === 'codex'
              ? applyCockpitApiBaseUrlToExternalImportItems(resolvedItems, request)
              : resolvedItems;

          let success = 0;
          const failures: ExternalImportProgressFailure[] = [];
          const importedAccountIds: string[] = [];
          const total = items.length;
          updateProgress({
            status: 'importing',
            progress: 40,
            total,
            success,
            failed: 0,
            current: 0,
            failures,
            message: t(
              'common.shared.externalImport.statusImporting',
              {
                current: 1,
                total,
                defaultValue: '正在导入第 {{current}} / {{total}} 个账号',
              },
            ),
          });

          for (let index = 0; index < items.length; index += 1) {
            const current = index + 1;
            const label = resolveExternalImportItemLabel(
              items[index],
              t('common.shared.externalImport.unknownItem', {
                index: current,
                defaultValue: '第 {{index}} 个 JSON',
              }),
            );
            updateProgress({
              status: 'importing',
              current,
              progress: Math.min(90, 40 + Math.round((index / total) * 50)),
              success,
              failed: failures.length,
              message: t(
                'common.shared.externalImport.statusImporting',
                {
                  current,
                  total,
                  defaultValue: '正在导入第 {{current}} / {{total}} 个账号',
                },
              ),
            });

            try {
              const imported = await dataService.importFromJson(JSON.stringify(items[index]));
              importedAccountIds.push(...collectImportedAccountIds(imported));
              success += Array.isArray(imported) ? imported.length : 1;
            } catch (error) {
              failures.push({
                index: current,
                label,
                error: String(error).replace(/^Error:\s*/, ''),
              });
            }
          }

          const activateImportedAccount = request.activate ? switchAccount : undefined;
          const activatedAccountId =
            activateImportedAccount
              ? importedAccountIds[importedAccountIds.length - 1]
              : '';
          if (activatedAccountId && activateImportedAccount) {
            updateProgress({
              status: 'refreshing',
              progress: 92,
              current: total,
              success,
              failed: failures.length,
              failures: [...failures],
              message: t(
                'common.shared.externalImport.statusActivating',
                '正在切换到导入账号...',
              ),
            });
            await activateImportedAccount(activatedAccountId);
            await refreshToken(activatedAccountId).catch(() => undefined);
            if (platformId) {
              await emitCurrentAccountChanged({
                platformId,
                accountId: activatedAccountId,
                reason: 'external-import',
              });
            }
          }
          updateProgress({
            status: 'refreshing',
            progress: 95,
            current: total,
            success,
            failed: failures.length,
            failures: [...failures],
            message: t(
              'common.shared.externalImport.statusRefreshing',
              '正在刷新账号列表...',
            ),
          });
          await fetchAccounts();
          if (success > 0 && platformId) {
            await emitAccountsChanged({
              platformId,
              reason: 'import',
            });
          }

          const status: ExternalImportProgressStatus =
            failures.length === 0 ? 'success' : success > 0 ? 'partial' : 'error';
          updateProgress({
            status,
            progress: 100,
            current: total,
            success,
            failed: failures.length,
            failures: [...failures],
            message:
              status === 'success'
                ? t('common.shared.externalImport.statusSuccess', '导入完成')
                : status === 'partial'
                  ? t('common.shared.externalImport.statusPartial', '部分导入完成')
                  : t('common.shared.externalImport.statusFailed', '导入失败'),
          });
        } catch (error) {
          updateProgress({
            status: 'error',
            progress: 100,
            message: String(error).replace(/^Error:\s*/, ''),
          });
        }
      })();
    },
    [dataService, fetchAccounts, platformId, refreshToken, switchAccount, t],
  );

  const consumeExternalProviderImport = useCallback(() => {
    if (!platformId) return;
    const request = consumeQueuedExternalProviderImportForPlatform(platformId);
    if (!request) return;
    if (request.importUrl || (platformId === 'codex' && request.token.trim())) {
      runExternalProviderImport(request);
      return;
    }

    openAddModal('token');
    setTokenInput(request.token);
    setAddStatus('idle');
    setAddMessage(null);
    if (request.autoImport) {
      setExternalAutoImportNonce((value) => value + 1);
    }
  }, [openAddModal, platformId, runExternalProviderImport]);

  useEffect(() => {
    if (!platformId) return;
    const handleExternalImportEvent = () => {
      consumeExternalProviderImport();
    };
    consumeExternalProviderImport();
    window.addEventListener(EXTERNAL_PROVIDER_IMPORT_EVENT, handleExternalImportEvent);
    return () => {
      window.removeEventListener(EXTERNAL_PROVIDER_IMPORT_EVENT, handleExternalImportEvent);
    };
  }, [consumeExternalProviderImport, platformId]);

  const handlePickImportFile = useCallback(() => {
    importFileInputRef.current?.click();
  }, []);

  // ─── Import ───────────────────────────────────────────────────────────
  const handleImportJsonFile = useCallback(
    async (file: File) => {
      setImporting(true);
      setAddStatus('loading');
      setAddMessage(t('common.shared.import.importing', '正在导入...'));

      try {
        const content = await file.text();
        const imported = await dataService.importFromJson(content);
        await fetchAccounts();
        if (platformId) {
          await emitAccountsChanged({
            platformId,
            reason: 'import',
          });
        }

        setAddStatus('success');
        setAddMessage(
          t('common.shared.token.importSuccessMsg', {
            count: imported.length,
            defaultValue: '成功导入 {{count}} 个账号',
          }),
        );
        setTimeout(() => {
          setShowAddModal(false);
          resetAddModalState();
        }, 1200);
      } catch (e) {
        setAddStatus('error');
        const errorMsg = String(e).replace(/^Error:\s*/, '');
        setAddMessage(
          t('common.shared.import.failedMsg', {
            error: errorMsg,
            defaultValue: '导入失败: {{error}}',
          }),
        );
      }

      setImporting(false);
    },
    [dataService, fetchAccounts, platformId, resetAddModalState, t],
  );

  const handleImportFromLocal = useMemo(() => {
    if (!dataService.importFromLocal) return null;
    const importFn = dataService.importFromLocal;
    return async () => {
      setImporting(true);
      setAddStatus('loading');
      setAddMessage(t('common.shared.import.importing', '正在导入...'));
      try {
        const imported = await importFn();
        await fetchAccounts();
        // 部分平台本机导入后本地索引存在极短暂写入延迟，补一次短延时刷新保障列表及时更新。
        await new Promise((resolve) => setTimeout(resolve, 180));
        await fetchAccounts();
        if (platformId) {
          await emitAccountsChanged({
            platformId,
            reason: 'import',
          });
        }
        setAddStatus('success');
        setAddMessage(
          t('common.shared.token.importSuccessMsg', {
            count: imported.length,
            defaultValue: '成功导入 {{count}} 个账号',
          }),
        );
        setTimeout(() => {
          setShowAddModal(false);
          resetAddModalState();
        }, 1200);
      } catch (e) {
        setAddStatus('error');
        const errorMsg = String(e).replace(/^Error:\s*/, '');
        setAddMessage(
          t('common.shared.import.failedMsg', {
            error: errorMsg,
            defaultValue: '导入失败: {{error}}',
          }),
        );
      }
      setImporting(false);
    };
  }, [dataService.importFromLocal, fetchAccounts, platformId, resetAddModalState, t]);

  const handleTokenImport = useCallback(async () => {
    const trimmed = tokenInput.trim();
    if (!trimmed) {
      setAddStatus('error');
      setAddMessage(t('common.shared.token.empty', '请输入 Token 或 JSON'));
      return;
    }

    setImporting(true);
    setAddStatus('loading');
    setAddMessage(t('common.shared.token.importing', '正在导入...'));

    try {
      let importedCount = 0;
      if (trimmed.startsWith('{') || trimmed.startsWith('[')) {
        const imported = await dataService.importFromJson(trimmed);
        importedCount = imported.length;
      } else if (dataService.addWithToken) {
        await dataService.addWithToken(trimmed);
        importedCount = 1;
      } else {
        const imported = await dataService.importFromJson(trimmed);
        importedCount = imported.length;
      }
      await fetchAccounts();
      if (platformId) {
        await emitAccountsChanged({
          platformId,
          reason: 'import',
        });
      }
      setAddStatus('success');
      setAddMessage(
        t('common.shared.token.importSuccessMsg', {
          count: importedCount,
          defaultValue: '成功导入 {{count}} 个账号',
        }),
      );
      setTimeout(() => {
        setShowAddModal(false);
        resetAddModalState();
      }, 1200);
    } catch (e) {
      setAddStatus('error');
      const errorMsg = String(e).replace(/^Error:\s*/, '');
      setAddMessage(
        t('common.shared.token.importFailedMsg', {
          error: errorMsg,
          defaultValue: '导入失败: {{error}}',
        }),
      );
    }
    setImporting(false);
  }, [dataService, fetchAccounts, platformId, resetAddModalState, t, tokenInput]);

  useEffect(() => {
    if (
      externalAutoImportNonce <= 0 ||
      !showAddModal ||
      addTab !== 'token' ||
      importing ||
      !tokenInput.trim()
    ) {
      return;
    }

    const timer = window.setTimeout(() => {
      setExternalAutoImportNonce(0);
      void handleTokenImport();
    }, 0);

    return () => {
      window.clearTimeout(timer);
    };
  }, [
    externalAutoImportNonce,
    showAddModal,
    addTab,
    importing,
    tokenInput,
    handleTokenImport,
  ]);

  // ─── OAuth (Device Flow) ──────────────────────────────────────────────
  const [oauthUrl, setOauthUrl] = useState<string | null>(null);
  const [oauthCallbackUrl, setOauthCallbackUrl] = useState<string | null>(null);
  const [oauthUrlCopied, setOauthUrlCopied] = useState(false);
  const [oauthUserCode, setOauthUserCode] = useState<string | null>(null);
  const [oauthUserCodeCopied, setOauthUserCodeCopied] = useState(false);
  const [oauthMeta, setOauthMeta] = useState<{
    expiresIn: number;
    intervalSeconds: number;
  } | null>(null);
  const [oauthPrepareError, setOauthPrepareError] = useState<string | null>(null);
  const [oauthCompleteError, setOauthCompleteError] = useState<string | null>(null);
  const [oauthPolling, setOauthPolling] = useState(false);
  const [oauthTimedOut, setOauthTimedOut] = useState(false);
  const [oauthManualCallbackInput, setOauthManualCallbackInput] = useState('');
  const [oauthManualCallbackSubmitting, setOauthManualCallbackSubmitting] = useState(false);
  const [oauthManualCallbackError, setOauthManualCallbackError] = useState<string | null>(null);

  const oauthActiveRef = useRef(false);
  const oauthLoginIdRef = useRef<string | null>(null);
  const oauthCompletingRef = useRef(false);
  const oauthAttemptSeqRef = useRef(0);

  const oauthLog = useCallback(
    (...args: unknown[]) => {
      console.info(`[${oauthLogPrefix}]`, ...args);
    },
    [oauthLogPrefix],
  );

  const handleOauthPrepareError = useCallback(
    (e: unknown) => {
      const msg = String(e).replace(/^Error:\s*/, '');
      console.error(`[${oauthLogPrefix}] 准备授权信息失败`, { error: msg });
      oauthActiveRef.current = false;
      oauthCompletingRef.current = false;
      setOauthPolling(false);
      setOauthCallbackUrl(null);
      setOauthManualCallbackSubmitting(false);
      setOauthManualCallbackError(null);
      setOauthPrepareError(t('common.shared.oauth.failed', '授权失败') + ': ' + msg);
    },
    [oauthLogPrefix, t],
  );

  const completeOauthSuccess = useCallback(async () => {
    oauthLog('授权完成并保存成功', {
      loginId: oauthLoginIdRef.current,
    });
    await fetchAccounts();
    if (platformId) {
      await emitAccountsChanged({
        platformId,
        reason: 'oauth',
      });
    }
    setAddStatus('success');
    setAddMessage(
      config.resolveOauthSuccessMessage?.() ?? t('common.shared.oauth.success', '授权成功'),
    );
    // 授权完成后不再触发 cancelLogin，避免误关仍需用户手动确认的授权页
    oauthLoginIdRef.current = null;
    oauthActiveRef.current = false;
    oauthCompletingRef.current = false;
    setOauthUrl(null);
    setOauthCallbackUrl(null);
    setOauthUrlCopied(false);
    setOauthUserCode(null);
    setOauthUserCodeCopied(false);
    setOauthMeta(null);
    setOauthPrepareError(null);
    setOauthCompleteError(null);
    setOauthTimedOut(false);
    setOauthPolling(false);
    setOauthManualCallbackSubmitting(false);
    setOauthManualCallbackError(null);
    setTimeout(() => {
      setShowAddModal(false);
      resetAddModalState();
    }, 1200);
  }, [fetchAccounts, config, oauthLog, platformId, resetAddModalState, t]);

  const handleOauthCompleteError = useCallback(
    (e: unknown) => {
      const msg = String(e).replace(/^Error:\s*/, '');
      setOauthCompleteError(msg);
      setOauthTimedOut(/超时|过期|expired|timeout/i.test(msg));
      setOauthPolling(false);
      setOauthManualCallbackSubmitting(false);
      oauthCompletingRef.current = false;
      oauthActiveRef.current = false;
      oauthLog(`${platformKey} OAuth 授权失败`, {
        loginId: oauthLoginIdRef.current,
        error: msg,
      });
    },
    [oauthLog, platformKey],
  );

  const prepareOauthUrl = useCallback(() => {
    if (!oauthService) return;
    if (!showAddModalRef.current || !oauthTabKeys.includes(addTabRef.current)) return;
    if (oauthActiveRef.current) return;
    if (oauthCompletingRef.current) return;
    const attemptSeq = ++oauthAttemptSeqRef.current;
    oauthActiveRef.current = true;
    setOauthPrepareError(null);
    setOauthCompleteError(null);
    setOauthTimedOut(false);
    setOauthPolling(false);
    setOauthUrlCopied(false);
    setOauthUserCodeCopied(false);
    setOauthMeta(null);
    setOauthUserCode(null);
    setOauthCallbackUrl(null);
    setOauthManualCallbackSubmitting(false);
    setOauthManualCallbackError(null);
    setOauthManualCallbackInput('');
    oauthLog(`开始准备 ${platformKey} OAuth 授权信息`);

    let started = false;

    void (async () => {
      try {
        const resp = await oauthService.startLogin();
        started = true;

        if (attemptSeq !== oauthAttemptSeqRef.current) {
          oauthService.cancelLogin(resp.loginId).catch(() => {});
          oauthLog('忽略过期 OAuth start 响应', { attemptSeq, loginId: resp.loginId });
          return;
        }

        oauthLoginIdRef.current = resp.loginId ?? null;

        const url =
          resp.verificationUriComplete || resp.verificationUri || resp.authUrl || '';
        setOauthUrl(url);
        setOauthCallbackUrl(resp.callbackUrl ?? null);
        setOauthUserCode(resp.userCode ?? null);
        if (resp.expiresIn || resp.intervalSeconds) {
          setOauthMeta({
            expiresIn: resp.expiresIn,
            intervalSeconds: resp.intervalSeconds,
          });
        }

        oauthLog('授权信息已就绪并展示在弹框', {
          loginId: resp.loginId,
          url,
          expiresIn: resp.expiresIn,
          intervalSeconds: resp.intervalSeconds,
          attemptSeq,
        });

        setOauthPolling(true);
        oauthCompletingRef.current = true;
        oauthActiveRef.current = false;
        await oauthService.completeLogin(resp.loginId);

        if (attemptSeq !== oauthAttemptSeqRef.current) {
          oauthLog('忽略过期 OAuth complete 成功回调', {
            attemptSeq,
            loginId: resp.loginId,
          });
          return;
        }

        setOauthPolling(false);
        oauthCompletingRef.current = false;
        await completeOauthSuccess();
      } catch (e) {
        if (attemptSeq !== oauthAttemptSeqRef.current) {
          oauthLog('忽略过期 OAuth 异常回调', {
            attemptSeq,
            error: String(e),
          });
          return;
        }
        if (!started) {
          handleOauthPrepareError(e);
          return;
        }
        handleOauthCompleteError(e);
      } finally {
        if (attemptSeq === oauthAttemptSeqRef.current) {
          oauthActiveRef.current = false;
        }
      }
    })();
  }, [
    oauthService,
    completeOauthSuccess,
    handleOauthCompleteError,
    handleOauthPrepareError,
    oauthLog,
    oauthTabKeys,
    platformKey,
  ]);

  // Auto-prepare OAuth when modal opens on oauth tab
  useEffect(() => {
    if (!showAddModal || !oauthTabKeys.includes(addTab) || oauthUrl) return;
    prepareOauthUrl();
  }, [showAddModal, addTab, oauthUrl, prepareOauthUrl, oauthTabKeys]);

  // Cancel OAuth when modal closes or tab changes
  useEffect(() => {
    if (showAddModal && oauthTabKeys.includes(addTab)) return;
    const loginId = oauthLoginIdRef.current ?? undefined;
    const hasOauthUiResidue = Boolean(oauthUrl)
      || Boolean(oauthCallbackUrl)
      || oauthUrlCopied
      || Boolean(oauthUserCode)
      || oauthUserCodeCopied
      || oauthMeta !== null
      || Boolean(oauthPrepareError)
      || Boolean(oauthCompleteError)
      || oauthTimedOut
      || oauthPolling
      || oauthManualCallbackInput.length > 0
      || oauthManualCallbackSubmitting
      || Boolean(oauthManualCallbackError);
    if (!loginId && !oauthActiveRef.current && !oauthCompletingRef.current && !hasOauthUiResidue) return;
    oauthAttemptSeqRef.current += 1;
    if (loginId) {
      oauthLog('弹框关闭或切换标签，准备取消授权流程', { loginId });
      oauthService?.cancelLogin(loginId).catch(() => {});
    }
    oauthActiveRef.current = false;
    oauthLoginIdRef.current = null;
    oauthCompletingRef.current = false;
    setOauthUrl(null);
    setOauthCallbackUrl(null);
    setOauthUrlCopied(false);
    setOauthUserCode(null);
    setOauthUserCodeCopied(false);
    setOauthMeta(null);
    setOauthPrepareError(null);
    setOauthCompleteError(null);
    setOauthTimedOut(false);
    setOauthPolling(false);
    setOauthManualCallbackInput('');
    setOauthManualCallbackSubmitting(false);
    setOauthManualCallbackError(null);
  }, [
    showAddModal,
    addTab,
    oauthLog,
    oauthService,
    oauthTabKeys,
    oauthUrl,
    oauthCallbackUrl,
    oauthUrlCopied,
    oauthUserCode,
    oauthUserCodeCopied,
    oauthMeta,
    oauthPrepareError,
    oauthCompleteError,
    oauthTimedOut,
    oauthPolling,
    oauthManualCallbackInput,
    oauthManualCallbackSubmitting,
    oauthManualCallbackError,
  ]);

  useEffect(
    () => () => {
      oauthAttemptSeqRef.current += 1;
      const loginId = oauthLoginIdRef.current ?? undefined;
      if (loginId) {
        oauthLog('页面卸载，准备取消授权流程', { loginId });
        oauthServiceRef.current?.cancelLogin(loginId).catch(() => {});
      }
      oauthActiveRef.current = false;
      oauthCompletingRef.current = false;
      oauthLoginIdRef.current = null;
    },
    [oauthLog],
  );

  const handleCopyOauthUrl = useCallback(async () => {
    if (!oauthUrl) return;
    try {
      await navigator.clipboard.writeText(oauthUrl);
      oauthLog('已复制授权链接', {
        loginId: oauthLoginIdRef.current,
        authUrl: oauthUrl,
      });
      setOauthUrlCopied(true);
      window.setTimeout(() => setOauthUrlCopied(false), 1200);
    } catch (e) {
      console.error('复制失败:', e);
    }
  }, [oauthUrl, oauthLog]);

  const handleCopyOauthUserCode = useCallback(async () => {
    if (!oauthUserCode) return;
    try {
      await navigator.clipboard.writeText(oauthUserCode);
      oauthLog('已复制 user_code', { loginId: oauthLoginIdRef.current });
      setOauthUserCodeCopied(true);
      window.setTimeout(() => setOauthUserCodeCopied(false), 1200);
    } catch (e) {
      console.error('复制失败:', e);
    }
  }, [oauthUserCode, oauthLog]);

  const handleRetryOauth = useCallback(() => {
    const previousLoginId = oauthLoginIdRef.current ?? undefined;
    oauthLog('用户点击刷新授权信息', {
      loginId: previousLoginId,
      error: oauthCompleteError,
      timedOut: oauthTimedOut,
    });
    oauthAttemptSeqRef.current += 1;
    if (previousLoginId) {
      oauthService?.cancelLogin(previousLoginId).catch(() => {});
    }
    oauthActiveRef.current = false;
    oauthLoginIdRef.current = null;
    oauthCompletingRef.current = false;
    setOauthPrepareError(null);
    setOauthCompleteError(null);
    setOauthTimedOut(false);
    setOauthPolling(false);
    setOauthMeta(null);
    setOauthUrl(null);
    setOauthCallbackUrl(null);
    setOauthUrlCopied(false);
    setOauthUserCode(null);
    setOauthUserCodeCopied(false);
    setOauthManualCallbackInput('');
    setOauthManualCallbackSubmitting(false);
    setOauthManualCallbackError(null);
    prepareOauthUrl();
  }, [oauthCompleteError, oauthTimedOut, oauthLog, oauthService, prepareOauthUrl]);

  const handleRetryOauthComplete = useCallback(() => {
    if (!oauthService) return;
    const loginId = oauthLoginIdRef.current;
    if (!loginId) return;
    if (oauthCompletingRef.current) return;
    const attemptSeq = ++oauthAttemptSeqRef.current;

    oauthLog('用户点击重新轮询授权结果', {
      loginId,
      error: oauthCompleteError,
      timedOut: oauthTimedOut,
      attemptSeq,
    });

    setOauthPrepareError(null);
    setOauthCompleteError(null);
    setOauthTimedOut(false);
    setOauthPolling(true);
    setOauthManualCallbackError(null);
    oauthCompletingRef.current = true;
    oauthActiveRef.current = false;

    oauthService
      .completeLogin(loginId)
      .then(async () => {
        if (attemptSeq !== oauthAttemptSeqRef.current) {
          oauthLog('忽略过期 OAuth 重试成功回调', { loginId, attemptSeq });
          return;
        }
        setOauthPolling(false);
        oauthCompletingRef.current = false;
        await completeOauthSuccess();
      })
      .catch((e) => {
        if (attemptSeq !== oauthAttemptSeqRef.current) {
          oauthLog('忽略过期 OAuth 重试异常回调', {
            loginId,
            attemptSeq,
            error: String(e),
          });
          return;
        }
        handleOauthCompleteError(e);
      });
  }, [
    oauthService,
    oauthLog,
    oauthCompleteError,
    oauthTimedOut,
    completeOauthSuccess,
    handleOauthCompleteError,
  ]);

  const handleOpenOauthUrl = useCallback(async () => {
    if (!oauthUrl) return;
    oauthLog('用户点击打开授权链接', {
      loginId: oauthLoginIdRef.current,
      authUrl: oauthUrl,
    });
    try {
      if (oauthService?.openAuthUrl) {
        await oauthService.openAuthUrl(oauthUrl);
      } else {
        await openUrl(oauthUrl);
      }
    } catch (e) {
      console.error('打开授权链接失败:', e);
      await navigator.clipboard.writeText(oauthUrl).catch(() => {});
      setOauthUrlCopied(true);
      setTimeout(() => setOauthUrlCopied(false), 1200);
    }
  }, [oauthUrl, oauthLog, oauthService]);

  const oauthSupportsManualCallback = useMemo(
    () => Boolean(oauthService?.submitCallbackUrl && oauthCallbackUrl),
    [oauthService, oauthCallbackUrl],
  );

  const handleSubmitOauthCallbackUrl = useCallback(async () => {
    if (!oauthService?.submitCallbackUrl) return;
    const loginId = oauthLoginIdRef.current;
    const callbackUrl = oauthManualCallbackInput.trim();
    if (!callbackUrl) return;
    if (!loginId) {
      setOauthManualCallbackError(t('common.shared.oauth.failed', '授权失败'));
      return;
    }

    setOauthManualCallbackSubmitting(true);
    setOauthManualCallbackError(null);
    try {
      await oauthService.submitCallbackUrl(loginId, callbackUrl);
      if (!oauthCompletingRef.current) {
        handleRetryOauthComplete();
      }
    } catch (e) {
      const msg = String(e).replace(/^Error:\s*/, '');
      setOauthManualCallbackError(msg);
    } finally {
      setOauthManualCallbackSubmitting(false);
    }
  }, [
    oauthService,
    oauthManualCallbackInput,
    t,
    handleRetryOauthComplete,
  ]);

  // ─── Flow Notice ──────────────────────────────────────────────────────
  const [isFlowNoticeCollapsed, setIsFlowNoticeCollapsed] = useState<boolean>(() => {
    if (!flowNoticeCollapsedKey) return false;
    try {
      return localStorage.getItem(flowNoticeCollapsedKey) === '1';
    } catch {
      return false;
    }
  });

  useEffect(() => {
    if (!flowNoticeCollapsedKey) return;
    try {
      localStorage.setItem(flowNoticeCollapsedKey, isFlowNoticeCollapsed ? '1' : '0');
    } catch {
      // ignore persistence failures
    }
  }, [isFlowNoticeCollapsed, flowNoticeCollapsedKey]);

  // ─── Current Account ──────────────────────────────────────────────────
  const [localCurrentAccountId, setLocalCurrentAccountId] = useState<string | null>(() => {
    if (!currentAccountIdKey) return null;
    try {
      const value = localStorage.getItem(currentAccountIdKey);
      return value && value.trim() ? value : null;
    } catch {
      return null;
    }
  });

  const currentAccountId = managesCurrentAccountId
    ? storeCurrentAccountId ?? null
    : localCurrentAccountId;

  const setCurrentAccountId = useCallback(
    (accountId: string | null) => {
      if (managesCurrentAccountId) {
        setStoreCurrentAccountId?.(accountId);
        return;
      }
      setLocalCurrentAccountId(accountId);
    },
    [managesCurrentAccountId, setStoreCurrentAccountId],
  );

  useEffect(() => {
    if (!managesCurrentAccountId && !currentAccountId) return;
    const exists = currentAccountId
      ? accounts.some((account) => account.id === currentAccountId)
      : true;
    if (!exists) {
      setCurrentAccountId(null);
    }
  }, [accounts, currentAccountId, managesCurrentAccountId, setCurrentAccountId]);

  useEffect(() => {
    if (managesCurrentAccountId || !currentAccountIdKey) return;
    try {
      if (currentAccountId) {
        localStorage.setItem(currentAccountIdKey, currentAccountId);
      } else {
        localStorage.removeItem(currentAccountIdKey);
      }
    } catch {
      // ignore persistence failures
    }
  }, [currentAccountId, currentAccountIdKey, managesCurrentAccountId]);

  // ─── Utilities ────────────────────────────────────────────────────────
  const formatDate = useCallback(
    (timestamp: number) => {
      const d = new Date(timestamp * 1000);
      return (
        d.toLocaleDateString(locale, {
          year: 'numeric',
          month: '2-digit',
          day: '2-digit',
        }) +
        ' ' +
        d.toLocaleTimeString(locale, { hour: '2-digit', minute: '2-digit' })
      );
    },
    [locale],
  );

  // ─── Return ───────────────────────────────────────────────────────────
  return {
    t,
    locale,
    privacyModeEnabled,
    togglePrivacyMode,
    maskAccountText,
    viewMode,
    setViewMode,
    searchQuery,
    setSearchQuery,
    filterPersistenceEnabled,
    setFilterPersistenceEnabled,
    filterPersistenceScope,
    filterType,
    setFilterType,
    sortBy,
    setSortBy,
    sortDirection,
    setSortDirection,
    selected,
    setSelected,
    toggleSelect,
    toggleSelectAll,
    tagFilter,
    setTagFilter,
    groupByTag,
    setGroupByTag,
    showTagFilter,
    setShowTagFilter,
    showTagModal,
    setShowTagModal,
    tagFilterRef,
    tagFilterPanelRef,
    tagFilterPanelPlacement,
    tagFilterScrollContainerStyle,
    availableTags,
    toggleTagFilterValue,
    clearTagFilter,
    tagDeleteConfirm,
    tagDeleteConfirmError,
    tagDeleteConfirmErrorScrollKey,
    setTagDeleteConfirm,
    deletingTag,
    requestDeleteTag,
    confirmDeleteTag,
    openTagModal,
    handleSaveTags,
    refreshing,
    refreshingAll,
    injecting,
    handleRefresh,
    handleRefreshAll,
    handleDelete,
    handleBatchDelete,
    deleteConfirm,
    deleteConfirmError,
    deleteConfirmErrorScrollKey,
    setDeleteConfirm,
    deleting,
    confirmDelete,
    message,
    setMessage,
    exporting,
    handleExport,
    handleExportByIds,
    getScopedSelectedCount,
    showExportModal: exportModal.showModal,
    closeExportModal: exportModal.closeModal,
    exportJsonContent: exportModal.jsonContent,
    exportJsonHidden: exportModal.hidden,
    toggleExportJsonHidden: exportModal.toggleHidden,
    exportJsonCopied: exportModal.copied,
    copyExportJson: exportModal.copyJson,
    savingExportJson: exportModal.saving,
    saveExportJson: exportModal.saveJson,
    exportSavedPath: exportModal.savedPath,
    canOpenExportSavedDirectory: exportModal.canOpenSavedDirectory,
    openExportSavedDirectory: exportModal.openSavedDirectory,
    copyExportSavedPath: exportModal.copySavedPath,
    exportPathCopied: exportModal.pathCopied,
    showAddModal,
    setShowAddModal,
    addTab,
    setAddTab,
    addStatus,
    setAddStatus,
    addMessage,
    setAddMessage,
    tokenInput,
    setTokenInput,
    importing,
    openAddModal,
    closeAddModal,
    resetAddModalState,
    handleTokenImport,
    handleImportJsonFile,
    handleImportFromLocal,
    handlePickImportFile,
    importFileInputRef,
    externalImportProgress,
    closeExternalImportProgressModal,
    oauthUrl,
    oauthCallbackUrl,
    oauthUrlCopied,
    oauthUserCode,
    oauthUserCodeCopied,
    oauthMeta,
    oauthPrepareError,
    oauthCompleteError,
    oauthPolling,
    oauthTimedOut,
    oauthManualCallbackInput,
    setOauthManualCallbackInput,
    oauthManualCallbackSubmitting,
    oauthManualCallbackError,
    oauthSupportsManualCallback,
    handleCopyOauthUrl,
    handleCopyOauthUserCode,
    handleRetryOauth,
    handleRetryOauthComplete,
    handleOpenOauthUrl,
    handleSubmitOauthCallbackUrl,
    handleInjectToVSCode,
    isFlowNoticeCollapsed,
    setIsFlowNoticeCollapsed,
    currentAccountId,
    setCurrentAccountId,
    formatDate,
    normalizeTag,
    resolveDefaultExportPath: exportModal.resolveDefaultExportPath,
    saveJsonFile: exportModal.saveJsonFile,
  };
}
