import { useState, useMemo, useCallback, useEffect, useRef, Fragment } from 'react';
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
  RotateCw,
  CircleAlert,
  LayoutGrid,
  List,
  Search,
  ArrowDownWideNarrow,
  Tag,
  ChevronDown,
  Play,
  Eye,
  EyeOff,
  Mail,
  BookOpen,
} from 'lucide-react';
import { useWindsurfAccountStore } from '../stores/useWindsurfAccountStore';
import * as windsurfService from '../services/windsurfService';
import { TagEditModal } from '../components/TagEditModal';
import { ExportJsonModal } from '../components/ExportJsonModal';
import { ModalErrorMessage } from '../components/ModalErrorMessage';
import { PaginationControls } from '../components/PaginationControls';
import {
  getWindsurfCreditsSummary,
  getWindsurfOfficialUsageMode,
  getWindsurfPlanDisplayName,
  getWindsurfPlanLabel,
  getWindsurfQuotaClass,
  getWindsurfQuotaUsageSummary,
  formatWindsurfResetTime,
  hasWindsurfQuotaData,
  getWindsurfUsage,
} from '../types/windsurf';
import { buildWindsurfAccountPresentation } from '../presentation/platformAccountPresentation';

import { WindsurfOverviewTabsHeader, WindsurfTab } from '../components/WindsurfOverviewTabsHeader';
import { WindsurfInstancesContent } from './WindsurfInstancesPage';
import { QuickSettingsPopover } from '../components/QuickSettingsPopover';
import { useProviderAccountsPage } from '../hooks/useProviderAccountsPage';
import { MultiSelectFilterDropdown, type MultiSelectFilterOption } from '../components/MultiSelectFilterDropdown';
import { SingleSelectFilterDropdown } from '../components/SingleSelectFilterDropdown';
import type { WindsurfAccount, WindsurfPlanBadge } from '../types/windsurf';
import { compareCurrentAccountFirst } from '../utils/currentAccountSort';
import { computeWindsurfRecommendScoreStatic } from '../utils/windsurfRecommend';
import { emitAccountsChanged } from '../utils/accountSyncEvents';
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
import {
  normalizeAccountsOverviewScope,
  readAccountsOverviewFilterPersistenceEnabled,
  readAccountsOverviewFilterStringArray,
  removeAccountsOverviewFilterField,
  writeAccountsOverviewFilterField,
} from '../utils/accountsOverviewFilterPersistence';

const WINDSURF_FLOW_NOTICE_COLLAPSED_KEY = 'agtools.windsurf.flow_notice_collapsed';
const WINDSURF_CURRENT_ACCOUNT_ID_KEY = 'agtools.windsurf.current_account_id';
const WINDSURF_FILTER_PERSISTENCE_SCOPE = normalizeAccountsOverviewScope('Windsurf');
const FILTER_TYPES_FIELD = 'filter_types';
const WINDSURF_TOKEN_SINGLE_EXAMPLE = `sk-ws-xxxxx 或 eyJxxxxx`;
const WINDSURF_TOKEN_BATCH_EXAMPLE = `[
  {
    "id": "ws_demo_1",
    "github_login": "octocat",
    "github_id": 12345,
    "github_access_token": "sk-ws-xxxxx",
    "copilot_token": "copilot_token_xxx",
    "created_at": 1730000000,
    "last_used": 1730000000
  }
]`;
const WINDSURF_PASSWORD_BATCH_JSON_EXAMPLE = `[
  {
    "email": "user1@example.com",
    "password": "password123"
  },
  {
    "email": "user2@example.com",
    "password": "abc456789"
  }
]`;
const WINDSURF_PASSWORD_BATCH_TEXT_EXAMPLES = {
  tab: 'user1@example.com\tpassword123\nuser2@example.com\tabc456789',
  space: 'user1@example.com password123\nuser2@example.com abc456789',
  comma: 'user1@example.com,password123\nuser2@example.com,abc456789',
  pipe: 'user1@example.com|password123\nuser2@example.com|abc456789',
  dash: 'user1@example.com----password123\nuser2@example.com----abc456789',
} as const;

const WINDSURF_PLAN_FILTERS: WindsurfPlanBadge[] = [
  'FREE',
  'TRIAL',
  'INDIVIDUAL',
  'PRO',
  'PRO_ULTIMATE',
  'TEAMS',
  'TEAMS_ULTIMATE',
  'BUSINESS',
  'ENTERPRISE',
];

type WindsurfOfficialUsagePanelItem = {
  key: string;
  label: string;
  value: string;
  detail: string;
  title: string;
  progressPercent?: number | null;
  quotaClass?: string;
  showProgress?: boolean;
};

type WindsurfOfficialUsagePanel = {
  mode: ReturnType<typeof getWindsurfOfficialUsageMode>;
  headline: string;
  note: string;
  items: WindsurfOfficialUsagePanelItem[];
  title: string;
};

type WindsurfPasswordMode = 'single' | 'batch';
type WindsurfPasswordBatchInputMode = 'json' | 'text';
type WindsurfPasswordTextDelimiter = keyof typeof WINDSURF_PASSWORD_BATCH_TEXT_EXAMPLES | 'custom';
type WindsurfParsedPasswordCredential = {
  email: string;
  password: string;
  sourceLine: number;
};
type WindsurfPasswordBatchParseError =
  | { code: 'empty' }
  | { code: 'json_invalid'; detail: string }
  | { code: 'json_array_required' }
  | { code: 'json_item_invalid'; line: number }
  | { code: 'text_line_invalid'; line: number };

function buildWindsurfPasswordBatchTextExample(delimiter: string): string {
  return `user1@example.com${delimiter}password123\nuser2@example.com${delimiter}abc456789`;
}

function resolveWindsurfPasswordDelimiterValue(
  delimiter: WindsurfPasswordTextDelimiter,
  customDelimiter: string,
): { value: string | null; whitespace: boolean } {
  switch (delimiter) {
    case 'tab':
      return { value: '\t', whitespace: false };
    case 'space':
      return { value: ' ', whitespace: true };
    case 'comma':
      return { value: ',', whitespace: false };
    case 'pipe':
      return { value: '|', whitespace: false };
    case 'dash':
      return { value: '----', whitespace: false };
    case 'custom': {
      const value = customDelimiter;
      return { value: value.length > 0 ? value : null, whitespace: false };
    }
    default:
      return { value: null, whitespace: false };
  }
}

function parseWindsurfPasswordBatchJsonInput(
  input: string,
): { credentials?: WindsurfParsedPasswordCredential[]; error?: WindsurfPasswordBatchParseError } {
  const trimmedInput = input.trim();
  if (!trimmedInput) {
    return { error: { code: 'empty' } };
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(trimmedInput);
  } catch (error) {
    return {
      error: {
        code: 'json_invalid',
        detail: String(error).replace(/^SyntaxError:\s*/, ''),
      },
    };
  }

  if (!Array.isArray(parsed)) {
    return { error: { code: 'json_array_required' } };
  }

  const credentials: WindsurfParsedPasswordCredential[] = [];
  for (let index = 0; index < parsed.length; index += 1) {
    const item = parsed[index];
    if (!item || typeof item !== 'object' || Array.isArray(item)) {
      return { error: { code: 'json_item_invalid', line: index + 1 } };
    }

    const record = item as Record<string, unknown>;
    const email = typeof record.email === 'string' ? record.email.trim() : '';
    const password = typeof record.password === 'string' ? record.password : '';
    if (!email || password.length === 0) {
      return { error: { code: 'json_item_invalid', line: index + 1 } };
    }

    credentials.push({
      email,
      password,
      sourceLine: index + 1,
    });
  }

  if (credentials.length === 0) {
    return { error: { code: 'empty' } };
  }

  return { credentials };
}

function parseWindsurfPasswordTextLine(
  rawLine: string,
  delimiterValue: string,
  whitespaceDelimiter: boolean,
): { email: string; password: string } | null {
  const trimmedLine = rawLine.trim();
  if (!trimmedLine) {
    return null;
  }

  if (whitespaceDelimiter) {
    const segments = trimmedLine.split(/\s+/);
    if (segments.length !== 2) {
      return null;
    }
    const [email, password] = segments;
    if (!email || !password) {
      return null;
    }
    return { email, password };
  }

  const segments = trimmedLine.split(delimiterValue);
  if (segments.length !== 2) {
    return null;
  }

  const email = segments[0]?.trim() ?? '';
  const password = segments[1]?.trim() ?? '';
  if (!email || !password) {
    return null;
  }

  return { email, password };
}

function parseWindsurfPasswordBatchTextInput(
  input: string,
  delimiterValue: string,
  whitespaceDelimiter: boolean,
): { credentials?: WindsurfParsedPasswordCredential[]; error?: WindsurfPasswordBatchParseError } {
  if (!input.trim()) {
    return { error: { code: 'empty' } };
  }

  const credentials: WindsurfParsedPasswordCredential[] = [];
  const lines = input.split(/\r?\n/);
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index];
    if (!line.trim()) {
      continue;
    }
    const parsedLine = parseWindsurfPasswordTextLine(line, delimiterValue, whitespaceDelimiter);
    if (!parsedLine) {
      return { error: { code: 'text_line_invalid', line: index + 1 } };
    }
    credentials.push({
      ...parsedLine,
      sourceLine: index + 1,
    });
  }

  if (credentials.length === 0) {
    return { error: { code: 'empty' } };
  }

  return { credentials };
}

export function WindsurfAccountsPage() {
  const [activeTab, setActiveTab] = useState<WindsurfTab>('overview');
  const [filterTypes, setFilterTypes] = useState<string[]>(() =>
    readAccountsOverviewFilterPersistenceEnabled(WINDSURF_FILTER_PERSISTENCE_SCOPE)
      ? readAccountsOverviewFilterStringArray(
          WINDSURF_FILTER_PERSISTENCE_SCOPE,
          FILTER_TYPES_FIELD,
        )
      : [],
  );
  const untaggedKey = '__untagged__';

  const store = useWindsurfAccountStore();

  const page = useProviderAccountsPage<WindsurfAccount>({
    platformKey: 'Windsurf',
    oauthLogPrefix: 'WindsurfOAuth',
    flowNoticeCollapsedKey: WINDSURF_FLOW_NOTICE_COLLAPSED_KEY,
    currentAccountIdKey: WINDSURF_CURRENT_ACCOUNT_ID_KEY,
    exportFilePrefix: 'windsurf_accounts',
    defaultSortBy: 'recommended',
    store: {
      accounts: store.accounts,
      currentAccountId: store.currentAccountId,
      loading: store.loading,
      error: store.error,
      fetchAccounts: store.fetchAccounts,
      fetchCurrentAccountId: store.fetchCurrentAccountId,
      deleteAccounts: store.deleteAccounts,
      refreshToken: store.refreshToken,
      refreshAllTokens: store.refreshAllTokens,
      setCurrentAccountId: store.setCurrentAccountId,
      updateAccountTags: store.updateAccountTags,
    },
    oauthService: {
      startLogin: windsurfService.startWindsurfOAuthLogin,
      completeLogin: windsurfService.completeWindsurfOAuthLogin,
      cancelLogin: windsurfService.cancelWindsurfOAuthLogin,
      submitCallbackUrl: windsurfService.submitWindsurfOAuthCallbackUrl,
    },
    dataService: {
      importFromJson: windsurfService.importWindsurfFromJson,
      importFromLocal: windsurfService.importWindsurfFromLocal,
      addWithToken: windsurfService.addWindsurfAccountWithToken,
      exportAccounts: windsurfService.exportWindsurfAccounts,
      injectToVSCode: windsurfService.injectWindsurfToVSCode,
    },
    getDisplayEmail: (account) =>
      account.github_email ?? account.github_login ?? account.id,
  });

  const {
    t, locale, privacyModeEnabled, togglePrivacyMode, maskAccountText,
    viewMode, setViewMode, searchQuery, setSearchQuery,
    filterPersistenceEnabled, filterPersistenceScope,
    sortBy, setSortBy, sortDirection, setSortDirection,
    selected, toggleSelect, toggleSelectAll,
    tagFilter, groupByTag, setGroupByTag, showTagFilter, setShowTagFilter,
    showTagModal, setShowTagModal, tagFilterRef, availableTags,
    toggleTagFilterValue, clearTagFilter, tagDeleteConfirm, tagDeleteConfirmError, tagDeleteConfirmErrorScrollKey, setTagDeleteConfirm,
    deletingTag, requestDeleteTag, confirmDeleteTag, openTagModal, handleSaveTags,
    refreshing, refreshingAll, injecting,
    handleRefresh, handleRefreshAll, handleDelete, handleBatchDelete,
    deleteConfirm, deleteConfirmError, deleteConfirmErrorScrollKey, setDeleteConfirm, deleting, confirmDelete,
    message, setMessage,
    exporting, handleExport, handleExportByIds, getScopedSelectedCount,
    showExportModal, closeExportModal, exportJsonContent, exportJsonHidden,
    toggleExportJsonHidden, exportJsonCopied, copyExportJson,
    savingExportJson, saveExportJson, exportSavedPath,
    canOpenExportSavedDirectory, openExportSavedDirectory, copyExportSavedPath, exportPathCopied,
    showAddModal, addTab, addStatus, addMessage, tokenInput, setTokenInput,
    importing, openAddModal, closeAddModal, setAddStatus, setAddMessage,
    handleTokenImport, handleImportJsonFile, handleImportFromLocal, handlePickImportFile, importFileInputRef,
    oauthUrl, oauthUrlCopied, oauthUserCode, oauthUserCodeCopied, oauthMeta,
    oauthPrepareError, oauthCompleteError, oauthPolling, oauthTimedOut,
    oauthManualCallbackInput, setOauthManualCallbackInput,
    oauthManualCallbackSubmitting, oauthManualCallbackError, oauthSupportsManualCallback,
    handleCopyOauthUrl, handleCopyOauthUserCode, handleRetryOauth, handleOpenOauthUrl,
    handleSubmitOauthCallbackUrl,
    handleInjectToVSCode,
    isFlowNoticeCollapsed, setIsFlowNoticeCollapsed,
    currentAccountId,
    formatDate, normalizeTag,
  } = page;

  useEffect(() => {
    if (!filterPersistenceEnabled) {
      removeAccountsOverviewFilterField(filterPersistenceScope, FILTER_TYPES_FIELD);
      return;
    }
    writeAccountsOverviewFilterField(filterPersistenceScope, FILTER_TYPES_FIELD, filterTypes);
  }, [filterPersistenceEnabled, filterPersistenceScope, filterTypes]);

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

  const accounts = store.accounts;
  const loading = store.loading;
  const recommendNowSec = useMemo(() => Math.floor(Date.now() / 1000), [accounts]);
  const [passwordEmail, setPasswordEmail] = useState('');
  const [passwordPassword, setPasswordPassword] = useState('');
  const [passwordMode, setPasswordMode] = useState<WindsurfPasswordMode>('single');
  const [passwordBatchInputMode, setPasswordBatchInputMode] = useState<WindsurfPasswordBatchInputMode>('json');
  const [passwordBatchDelimiter, setPasswordBatchDelimiter] = useState<WindsurfPasswordTextDelimiter>('tab');
  const [passwordBatchCustomDelimiter, setPasswordBatchCustomDelimiter] = useState('');
  const [passwordBatchInput, setPasswordBatchInput] = useState('');
  const [passwordLoading, setPasswordLoading] = useState(false);
  const [passwordBatchLoading, setPasswordBatchLoading] = useState(false);
  const [passwordFieldError, setPasswordFieldError] = useState<string | null>(null);
  const [passwordBatchDelimiterFieldError, setPasswordBatchDelimiterFieldError] = useState<string | null>(null);
  const [passwordBatchFieldError, setPasswordBatchFieldError] = useState<string | null>(null);
  const passwordFieldErrorRef = useRef<HTMLDivElement | null>(null);
  const passwordBatchDelimiterFieldErrorRef = useRef<HTMLDivElement | null>(null);
  const passwordBatchFieldErrorRef = useRef<HTMLDivElement | null>(null);
  const passwordStatusRef = useRef<HTMLDivElement | null>(null);

  const scrollModalFeedbackIntoView = useCallback((ref: { current: HTMLDivElement | null }) => {
    requestAnimationFrame(() => {
      ref.current?.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
    });
  }, []);

  const clearPasswordFeedback = useCallback(() => {
    setPasswordFieldError(null);
    setPasswordBatchDelimiterFieldError(null);
    setPasswordBatchFieldError(null);
    setAddStatus('idle');
    setAddMessage(null);
  }, [setAddMessage, setAddStatus]);

  const resetPasswordLoginForm = useCallback(() => {
    setPasswordEmail('');
    setPasswordPassword('');
    setPasswordMode('single');
    setPasswordBatchInputMode('json');
    setPasswordBatchDelimiter('tab');
    setPasswordBatchCustomDelimiter('');
    setPasswordBatchInput('');
    setPasswordLoading(false);
    setPasswordBatchLoading(false);
    setPasswordFieldError(null);
    setPasswordBatchDelimiterFieldError(null);
    setPasswordBatchFieldError(null);
  }, []);

  const handleCloseAddModal = useCallback(() => {
    resetPasswordLoginForm();
    closeAddModal();
  }, [closeAddModal, resetPasswordLoginForm]);

  const resolvePasswordBatchParseError = useCallback((error: WindsurfPasswordBatchParseError) => {
    switch (error.code) {
      case 'empty':
        return t('windsurf.password.batchEmpty', '请先输入批量导入内容');
      case 'json_invalid':
        return t('windsurf.password.batchJsonInvalid', {
          error: error.detail,
          defaultValue: 'JSON 格式无效：{{error}}',
        });
      case 'json_array_required':
        return t('windsurf.password.batchJsonArrayRequired', 'JSON 顶层必须是数组');
      case 'json_item_invalid':
        return t('windsurf.password.batchJsonItemInvalid', {
          line: error.line,
          defaultValue: '第 {{line}} 项必须包含 email 和 password 字段',
        });
      case 'text_line_invalid':
        return t('windsurf.password.batchTextLineInvalid', {
          line: error.line,
          defaultValue: '第 {{line}} 行格式无效，请检查当前分隔符设置',
        });
      default:
        return t('common.shared.import.failedMsg', {
          error: '未知错误',
          defaultValue: '导入失败: {{error}}',
        });
    }
  }, [t]);

  const formatPasswordBatchFailures = useCallback(
    (failures: windsurfService.WindsurfPasswordCredentialFailure[]) => {
      const preview = failures.slice(0, 3).map((failure) => {
        const error = String(failure.error).replace(/^Error:\s*/, '');
        if (failure.source_line) {
          return t('windsurf.password.batchFailureEntryWithLine', {
            line: failure.source_line,
            email: failure.email || '-',
            error,
            defaultValue: '第 {{line}} 项（{{email}}）：{{error}}',
          });
        }
        return t('windsurf.password.batchFailureEntry', {
          email: failure.email || '-',
          error,
          defaultValue: '{{email}}：{{error}}',
        });
      });

      if (failures.length > preview.length) {
        preview.push(
          t('windsurf.password.batchFailureMore', {
            count: failures.length - preview.length,
            defaultValue: '还有 {{count}} 项失败',
          }),
        );
      }

      return preview.join('；');
    },
    [t],
  );

  const passwordBatchExample = useMemo(
    () => {
      if (passwordBatchInputMode === 'json') {
        return WINDSURF_PASSWORD_BATCH_JSON_EXAMPLE;
      }
      if (passwordBatchDelimiter === 'custom') {
        return buildWindsurfPasswordBatchTextExample(passwordBatchCustomDelimiter || '::');
      }
      return WINDSURF_PASSWORD_BATCH_TEXT_EXAMPLES[passwordBatchDelimiter];
    },
    [passwordBatchCustomDelimiter, passwordBatchDelimiter, passwordBatchInputMode],
  );

  const passwordBatchPlaceholder = useMemo(
    () => (
      passwordBatchInputMode === 'json'
        ? t('windsurf.password.batchPlaceholderJson', '粘贴 JSON 数组，每项包含 email 和 password')
        : t('windsurf.password.batchPlaceholderText', '每行一组账号，使用当前分隔符填写 email 和 password')
    ),
    [passwordBatchInputMode, t],
  );

  const handlePasswordLogin = useCallback(async () => {
    clearPasswordFeedback();

    const email = passwordEmail.trim();
    const password = passwordPassword;
    if (!email || !password) {
      setPasswordFieldError(t('windsurf.password.empty', '请输入邮箱和密码'));
      scrollModalFeedbackIntoView(passwordFieldErrorRef);
      return;
    }

    setPasswordLoading(true);
    setAddStatus('loading');
    setAddMessage(t('windsurf.password.logging', '正在登录...'));

    try {
      const account = await windsurfService.addWindsurfAccountWithPassword(email, password);
      await store.fetchAccounts();
      await emitAccountsChanged({
        platformId: 'windsurf',
        reason: 'import',
      });
      setAddStatus('success');
      setAddMessage(
        t('windsurf.password.success', {
          login: account.github_login || account.github_email || email,
          defaultValue: '登录成功：{{login}}',
        }),
      );
      setTimeout(() => {
        handleCloseAddModal();
      }, 1200);
    } catch (error) {
      const errorMsg = String(error).replace(/^Error:\s*/, '');
      setAddStatus('error');
      setAddMessage(
        t('windsurf.password.failed', {
          error: errorMsg,
          defaultValue: '登录失败：{{error}}',
        }),
      );
    } finally {
      setPasswordLoading(false);
    }
  }, [
    clearPasswordFeedback,
    handleCloseAddModal,
    passwordEmail,
    passwordPassword,
    scrollModalFeedbackIntoView,
    setAddMessage,
    setAddStatus,
    store,
    t,
  ]);

  const handlePasswordBatchImport = useCallback(async () => {
    clearPasswordFeedback();

    if (passwordBatchInputMode === 'text') {
      const delimiter = resolveWindsurfPasswordDelimiterValue(
        passwordBatchDelimiter,
        passwordBatchCustomDelimiter,
      );
      if (!delimiter.value) {
        setPasswordBatchDelimiterFieldError(
          t('windsurf.password.batchCustomDelimiterEmpty', '请输入自定义分隔符'),
        );
        scrollModalFeedbackIntoView(passwordBatchDelimiterFieldErrorRef);
        return;
      }
    }

    const selectedDelimiter = resolveWindsurfPasswordDelimiterValue(
      passwordBatchDelimiter,
      passwordBatchCustomDelimiter,
    );
    const parseResult =
      passwordBatchInputMode === 'json'
        ? parseWindsurfPasswordBatchJsonInput(passwordBatchInput)
        : parseWindsurfPasswordBatchTextInput(
            passwordBatchInput,
            selectedDelimiter.value ?? '',
            selectedDelimiter.whitespace,
          );

    if (parseResult.error || !parseResult.credentials) {
      setPasswordBatchFieldError(
        resolvePasswordBatchParseError(parseResult.error ?? { code: 'empty' }),
      );
      scrollModalFeedbackIntoView(passwordBatchFieldErrorRef);
      return;
    }

    setPasswordBatchLoading(true);
    setAddStatus('loading');
    setAddMessage(
      t('windsurf.password.batchImporting', {
        count: parseResult.credentials.length,
        defaultValue: '正在导入 {{count}} 个账号...',
      }),
    );

    try {
      const result = await windsurfService.addWindsurfAccountsWithPassword(
        parseResult.credentials.map((item) => ({
          email: item.email,
          password: item.password,
          sourceLine: item.sourceLine,
        })),
      );

      if (result.success_count > 0) {
        await store.fetchAccounts();
        await emitAccountsChanged({
          platformId: 'windsurf',
          reason: 'import',
        });
      }

      const failureSummary = formatPasswordBatchFailures(result.failures);
      if (result.failed_count === 0) {
        setAddStatus('success');
        setAddMessage(
          t('windsurf.password.batchSuccess', {
            count: result.success_count,
            defaultValue: '成功导入 {{count}} 个账号',
          }),
        );
        setTimeout(() => {
          handleCloseAddModal();
        }, 1200);
        return;
      }

      if (result.success_count > 0) {
        setAddStatus('error');
        setAddMessage(
          `${t('windsurf.password.batchPartial', {
            success: result.success_count,
            failed: result.failed_count,
            defaultValue: '成功 {{success}} 个，失败 {{failed}} 个',
          })}${failureSummary ? `：${failureSummary}` : ''}`,
        );
        return;
      }

      setAddStatus('error');
      setAddMessage(
        `${t('windsurf.password.batchFailed', {
          failed: result.failed_count,
          defaultValue: '批量导入失败，共 {{failed}} 个账号失败',
        })}${failureSummary ? `：${failureSummary}` : ''}`,
      );
    } catch (error) {
      const errorMsg = String(error).replace(/^Error:\s*/, '');
      setAddStatus('error');
      setAddMessage(
        t('common.shared.import.failedMsg', {
          error: errorMsg,
          defaultValue: '导入失败: {{error}}',
        }),
      );
    } finally {
      setPasswordBatchLoading(false);
    }
  }, [
    clearPasswordFeedback,
    formatPasswordBatchFailures,
    handleCloseAddModal,
    passwordBatchCustomDelimiter,
    passwordBatchDelimiter,
    passwordBatchInput,
    passwordBatchInputMode,
    resolvePasswordBatchParseError,
    scrollModalFeedbackIntoView,
    setPasswordBatchDelimiterFieldError,
    setAddMessage,
    setAddStatus,
    store,
    t,
  ]);

  useEffect(() => {
    if (addStatus === 'error') {
      scrollModalFeedbackIntoView(passwordStatusRef);
    }
  }, [addMessage, addStatus, scrollModalFeedbackIntoView]);

  const passwordSubmitting = passwordLoading || passwordBatchLoading;

  // ─── Platform-specific: Presentation & Credits ──────────────────────

  const creditsSummaryById = useMemo(() => {
    const map = new Map<string, ReturnType<typeof getWindsurfCreditsSummary>>();
    accounts.forEach((account) => { map.set(account.id, getWindsurfCreditsSummary(account)); });
    return map;
  }, [accounts]);

  const resolveCreditsSummary = useCallback(
    (account: WindsurfAccount) => creditsSummaryById.get(account.id) ?? getWindsurfCreditsSummary(account),
    [creditsSummaryById],
  );

  const quotaSummaryById = useMemo(() => {
    const map = new Map<string, ReturnType<typeof getWindsurfQuotaUsageSummary>>();
    accounts.forEach((account) => { map.set(account.id, getWindsurfQuotaUsageSummary(account)); });
    return map;
  }, [accounts]);

  const resolveQuotaSummary = useCallback(
    (account: WindsurfAccount) => quotaSummaryById.get(account.id) ?? getWindsurfQuotaUsageSummary(account),
    [quotaSummaryById],
  );

  const usageModeById = useMemo(() => {
    const map = new Map<string, ReturnType<typeof getWindsurfOfficialUsageMode>>();
    accounts.forEach((account) => { map.set(account.id, getWindsurfOfficialUsageMode(account)); });
    return map;
  }, [accounts]);

  const resolveUsageMode = useCallback(
    (account: WindsurfAccount) => usageModeById.get(account.id) ?? getWindsurfOfficialUsageMode(account),
    [usageModeById],
  );

  const accountPresentations = useMemo(() => {
    const map = new Map<string, ReturnType<typeof buildWindsurfAccountPresentation>>();
    accounts.forEach((account) => { map.set(account.id, buildWindsurfAccountPresentation(account, t)); });
    return map;
  }, [accounts, t]);

  const resolvePresentation = useCallback(
    (account: WindsurfAccount) => accountPresentations.get(account.id) ?? buildWindsurfAccountPresentation(account, t),
    [accountPresentations, t],
  );

  const resolveSingleExportBaseName = useCallback(
    (account: WindsurfAccount) => {
      const display = (resolvePresentation(account).displayName || account.id).trim();
      const atIndex = display.indexOf('@');
      return atIndex > 0 ? display.slice(0, atIndex) : display;
    },
    [resolvePresentation],
  );

  const resolvePlanKey = useCallback(
    (account: WindsurfAccount) => getWindsurfPlanDisplayName(resolvePresentation(account).planLabel || account.plan_type || null),
    [resolvePresentation],
  );

  const isAbnormalAccount = useCallback((_account: WindsurfAccount) => false, []);

  const formatCreditValue = useCallback(
    (value: number | null | undefined) => {
      if (typeof value !== 'number' || !Number.isFinite(value)) {
        return t('common.none', '暂无');
      }
      return value.toLocaleString(locale, { maximumFractionDigits: 2 });
    },
    [locale, t],
  );

  const formatCycleDate = useCallback(
    (timestamp: number | null | undefined) => {
      if (!timestamp) return '';
      const d = new Date(timestamp * 1000);
      if (Number.isNaN(d.getTime())) return '';
      return d.toLocaleDateString(locale, { year: 'numeric', month: '2-digit', day: '2-digit' });
    },
    [locale],
  );

  const resolveCycleDisplay = useCallback(
    (credits: ReturnType<typeof getWindsurfCreditsSummary>) => {
      const end = credits.planEndsAt ?? null;
      const start = credits.planStartsAt ?? null;
      if (!end) {
        const summary = t('common.shared.credits.planEndsUnknown', '配额周期时间未知');
        return { summary, detail: '', title: summary };
      }
      const now = Math.floor(Date.now() / 1000);
      const secondsLeft = end - now;
      const summary =
        secondsLeft > 0 && secondsLeft < 86400
          ? t('common.shared.credits.planEndsInHours', { hours: Math.max(1, Math.floor(secondsLeft / 3600)), defaultValue: '配额周期剩余 {{hours}} 小时' })
          : t('common.shared.credits.planEndsIn', { days: secondsLeft <= 0 ? 0 : Math.floor(secondsLeft / 86400), defaultValue: '配额周期剩余 {{days}} 天' });
      const startText = formatCycleDate(start);
      const endText = formatCycleDate(end);
      let detail = '';
      if (startText && endText) {
        detail = t('common.shared.credits.periodRange', { start: startText, end: endText, defaultValue: '周期：{{start}} - {{end}}' });
      } else if (endText) {
        detail = t('common.shared.credits.periodEndOnly', { end: endText, defaultValue: '周期结束：{{end}}' });
      }
      const title = detail ? `${summary} · ${detail}` : summary;
      return { summary, detail, title };
    },
    [formatCycleDate, t],
  );

  const formatQuotaUsagePercent = useCallback(
    (value: number | null | undefined) => {
      if (typeof value !== 'number' || !Number.isFinite(value)) {
        return t('common.none', '暂无');
      }
      return `${Math.max(0, Math.min(100, value))}%`;
    },
    [t],
  );

  const formatMicrosAsUsd = useCallback(
    (value: number | null | undefined) => {
      if (typeof value !== 'number' || !Number.isFinite(value)) {
        return t('common.none', '暂无');
      }
      return `$${(value / 1_000_000).toFixed(2)}`;
    },
    [t],
  );

  const buildQuotaDisplayItems = useCallback(
    (account: WindsurfAccount) => {
      const summary = resolveQuotaSummary(account);
      const items: WindsurfOfficialUsagePanelItem[] = [];

      if (summary.dailyUsedPercent != null) {
        const label = t('windsurf.usageSummary.dailyQuota', 'Daily quota usage');
        const value = formatQuotaUsagePercent(summary.dailyUsedPercent);
        const detail = summary.dailyResetAt
          ? t('common.shared.quota.resetAt', {
              time: formatWindsurfResetTime(summary.dailyResetAt, t),
              defaultValue: 'Reset: {{time}}',
            })
          : '';
        items.push({
          key: 'daily',
          label,
          value,
          detail,
          title: detail ? `${label}: ${value} · ${detail}` : `${label}: ${value}`,
          progressPercent: summary.dailyUsedPercent,
          quotaClass: getWindsurfQuotaClass(summary.dailyUsedPercent),
          showProgress: true,
        });
      }

      if (summary.weeklyUsedPercent != null) {
        const label = t('windsurf.usageSummary.weeklyQuota', 'Weekly quota usage');
        const value = formatQuotaUsagePercent(summary.weeklyUsedPercent);
        const detail = summary.weeklyResetAt
          ? t('common.shared.quota.resetAt', {
              time: formatWindsurfResetTime(summary.weeklyResetAt, t),
              defaultValue: 'Reset: {{time}}',
            })
          : '';
        items.push({
          key: 'weekly',
          label,
          value,
          detail,
          title: detail ? `${label}: ${value} · ${detail}` : `${label}: ${value}`,
          progressPercent: summary.weeklyUsedPercent,
          quotaClass: getWindsurfQuotaClass(summary.weeklyUsedPercent),
          showProgress: true,
        });
      }

      const label = t('windsurf.usageSummary.extraUsageBalance', 'Extra usage balance');
      const value = formatMicrosAsUsd(summary.overageBalanceMicros ?? 0);
      items.push({
        key: 'extraUsage',
        label,
        value,
        detail: '',
        title: `${label}: ${value}`,
        showProgress: false,
      });

      return items;
    },
    [formatMicrosAsUsd, formatQuotaUsagePercent, resolveQuotaSummary, t],
  );

  const buildCreditsDisplayItems = useCallback(
    (account: WindsurfAccount) => {
      const credits = resolveCreditsSummary(account);
      const promptLabel = t('windsurf.credits.promptCreditsLeftLabel', 'prompt credits left');
      const addOnLabel = t('windsurf.credits.addOnCreditsAvailableLabel', 'add-on credits available');
      const promptValue =
        credits.promptCreditsLeft != null && credits.promptCreditsTotal != null
          ? `${formatCreditValue(credits.promptCreditsLeft)} / ${formatCreditValue(credits.promptCreditsTotal)}`
          : credits.promptCreditsLeft != null
          ? formatCreditValue(credits.promptCreditsLeft)
          : t('common.none', '暂无');
      const addOnValue =
        credits.addOnCredits != null
          ? formatCreditValue(credits.addOnCredits)
          : t('common.none', '暂无');

      return [
        {
          key: 'promptCredits',
          label: promptLabel,
          value: promptValue,
          detail: '',
          title: `${promptLabel}: ${promptValue}`,
          showProgress: false,
        },
        {
          key: 'addOnCredits',
          label: addOnLabel,
          value: addOnValue,
          detail: '',
          title: `${addOnLabel}: ${addOnValue}`,
          showProgress: false,
        },
      ];
    },
    [formatCreditValue, resolveCreditsSummary, t],
  );

  const buildOfficialUsagePanel = useCallback(
    (account: WindsurfAccount): WindsurfOfficialUsagePanel => {
      if (!hasWindsurfQuotaData(account)) {
        const note = t('common.shared.quota.noData', '暂无配额数据');
        return {
          mode: resolveUsageMode(account),
          headline: '',
          note,
          items: [],
          title: note,
        };
      }

      const mode = resolveUsageMode(account);
      if (mode === 'quota') {
        const items = buildQuotaDisplayItems(account);
        return {
          mode,
          headline: '',
          note: '',
          items,
          title: items.map((item) => item.title).join(' | '),
        };
      }

      const credits = resolveCreditsSummary(account);
      const items = buildCreditsDisplayItems(account);
      const headline =
        credits.creditsLeft != null
          ? t('windsurf.credits.left', { value: formatCreditValue(credits.creditsLeft) })
          : t('windsurf.credits.leftUnknown', 'Credits left -');

      return {
        mode,
        headline,
        note: t('windsurf.credits.renewMonthly', 'Credits renew every month'),
        items,
        title: [headline, ...items.map((item) => item.title)].join(' | '),
      };
    },
    [buildCreditsDisplayItems, buildQuotaDisplayItems, formatCreditValue, resolveCreditsSummary, resolveUsageMode, t],
  );

  // ─── Tier filter ────────────────────────────────────────────────────
  const tierCounts = useMemo(() => {
    const counts: Record<string, number> = { all: accounts.length, VALID: 0 };
    WINDSURF_PLAN_FILTERS.forEach((planKey) => {
      counts[planKey] = 0;
    });
    accounts.forEach((account) => {
      if (!isAbnormalAccount(account)) {
        counts.VALID += 1;
      }
      const tier = resolvePlanKey(account);
      if (tier in counts) counts[tier as keyof typeof counts] += 1;
    });
    return counts;
  }, [accounts, isAbnormalAccount, resolvePlanKey]);

  const tierFilterOptions = useMemo<MultiSelectFilterOption[]>(
    () => [
      ...WINDSURF_PLAN_FILTERS.map((planKey) => ({
        value: planKey,
        label: `${getWindsurfPlanLabel(planKey)} (${tierCounts[planKey] ?? 0})`,
      })),
      buildValidAccountsFilterOption(t, tierCounts.VALID ?? 0),
    ],
    [t, tierCounts],
  );

  // ─── Filtering & Sorting ────────────────────────────────────────────
  const compareAccountsBySort = useCallback((a: WindsurfAccount, b: WindsurfAccount) => {
    const currentFirstDiff = compareCurrentAccountFirst(a.id, b.id, currentAccountId);
    if (currentFirstDiff !== 0) {
      return currentFirstDiff;
    }

    if (sortBy === 'created_at') {
      const diff = b.created_at - a.created_at;
      return sortDirection === 'desc' ? diff : -diff;
    }
    if (sortBy === 'plan_end') {
      const aReset = resolveCreditsSummary(a).planEndsAt ?? null;
      const bReset = resolveCreditsSummary(b).planEndsAt ?? null;
      if (aReset == null && bReset == null) return 0;
      if (aReset == null) return 1;
      if (bReset == null) return -1;
      return sortDirection === 'desc' ? bReset - aReset : aReset - bReset;
    }
    if (sortBy === 'recommended') {
      const aScore = computeWindsurfRecommendScoreStatic(a, {
        nowSec: recommendNowSec,
        usage: getWindsurfUsage(a),
        quotaSummary: resolveQuotaSummary(a),
        summary: resolveCreditsSummary(a),
      });
      const bScore = computeWindsurfRecommendScoreStatic(b, {
        nowSec: recommendNowSec,
        usage: getWindsurfUsage(b),
        quotaSummary: resolveQuotaSummary(b),
        summary: resolveCreditsSummary(b),
      });
      const diff = bScore - aScore;
      return sortDirection === 'desc' ? diff : -diff;
    }
    const aValue = resolveCreditsSummary(a).creditsLeft ?? -1;
    const bValue = resolveCreditsSummary(b).creditsLeft ?? -1;
    const diff = bValue - aValue;
    return sortDirection === 'desc' ? diff : -diff;
  }, [currentAccountId, recommendNowSec, resolveCreditsSummary, resolveQuotaSummary, sortBy, sortDirection]);

  const sortedAccountsForInstances = useMemo(
    () => [...accounts].sort(compareAccountsBySort),
    [accounts, compareAccountsBySort],
  );

  const filteredAccounts = useMemo(() => {
    let result = [...accounts];
    if (searchQuery.trim()) {
      const query = searchQuery.toLowerCase();
      result = result.filter((account) => resolvePresentation(account).displayName.toLowerCase().includes(query));
    }
    if (filterTypes.length > 0) {
      const { requireValidAccounts, selectedTypes } = splitValidityFilterValues(filterTypes);
      if (requireValidAccounts) {
        result = result.filter((account) => !isAbnormalAccount(account));
      }
      if (selectedTypes.size > 0) {
        result = result.filter((account) => selectedTypes.has(resolvePlanKey(account)));
      }
    }
    if (tagFilter.length > 0) {
      const selectedTags = new Set(tagFilter.map(normalizeTag));
      result = result.filter((acc) => (acc.tags || []).map(normalizeTag).some((tag) => selectedTags.has(tag)));
    }
    result.sort(compareAccountsBySort);
    return result;
  }, [accounts, compareAccountsBySort, filterTypes, isAbnormalAccount, normalizeTag, resolvePlanKey, resolvePresentation, searchQuery, tagFilter]);

  const filteredIds = useMemo(() => filteredAccounts.map((account) => account.id), [filteredAccounts]);
  const exportSelectionCount = getScopedSelectedCount(filteredIds);
  const pagination = usePagination({
    items: filteredAccounts,
    storageKey: buildPaginationPageSizeStorageKey('Windsurf'),
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
    filteredAccounts.forEach((account) => {
      const tags = (account.tags || []).map(normalizeTag).filter(Boolean);
      const matchedTags = selectedTags.size > 0 ? tags.filter((tag) => selectedTags.has(tag)) : tags;
      if (matchedTags.length === 0) { if (!groups.has(untaggedKey)) groups.set(untaggedKey, []); groups.get(untaggedKey)?.push(account); return; }
      matchedTags.forEach((tag) => { if (!groups.has(tag)) groups.set(tag, []); groups.get(tag)?.push(account); });
    });
    return Array.from(groups.entries()).sort(([aKey], [bKey]) => { if (aKey === untaggedKey) return -1; if (bKey === untaggedKey) return 1; return aKey.localeCompare(bKey); });
  }, [filteredAccounts, groupByTag, normalizeTag, tagFilter, untaggedKey]);

  const paginatedGroupedAccounts = useMemo(
    () => buildPaginatedGroups(groupedAccounts, paginatedAccounts),
    [groupedAccounts, paginatedAccounts],
  );

  const resolveGroupLabel = (groupKey: string) => groupKey === untaggedKey ? t('accounts.defaultGroup', '默认分组') : groupKey;

  // ─── Render helpers ──────────────────────────────────────────────────

  const renderUsagePanel = (
    panel: WindsurfOfficialUsagePanel,
    options?: { compact?: boolean },
  ) => (
    <div className={`windsurf-official-usage ${options?.compact ? 'compact' : ''}`} title={panel.title}>
      {panel.headline ? <div className="windsurf-official-usage-headline">{panel.headline}</div> : null}
      {panel.note ? <div className="windsurf-official-usage-note">{panel.note}</div> : null}
      <div className="windsurf-official-usage-list">
        {panel.items.map((item) => (
          <div
            key={item.key}
            className={`windsurf-official-usage-item ${item.showProgress ? 'with-progress' : ''}`}
            title={item.title}
          >
            <div className="windsurf-official-usage-main">
              <span className="windsurf-official-usage-label">{item.label}</span>
              <span className={`windsurf-official-usage-value quota-value ${item.quotaClass ?? ''}`}>{item.value}</span>
            </div>
            {item.showProgress ? (
              <div className="windsurf-official-usage-progress quota-progress-track">
                <div
                  className={`quota-progress-bar ${item.quotaClass ?? 'high'}`}
                  style={{ width: `${Math.max(0, Math.min(100, item.progressPercent ?? 0))}%` }}
                />
              </div>
            ) : null}
            {item.detail ? <div className="windsurf-official-usage-detail">{item.detail}</div> : null}
          </div>
        ))}
      </div>
    </div>
  );

  const renderPlanDetails = (
    cycleDisplay: ReturnType<typeof resolveCycleDisplay>,
    options?: { compact?: boolean },
  ) => (
    <div className={`windsurf-plan-cycle ${options?.compact ? 'compact' : ''}`} title={cycleDisplay.title}>
      <span className="windsurf-plan-cycle-summary">{cycleDisplay.summary}</span>
      {cycleDisplay.detail ? <span className="windsurf-plan-cycle-detail">{cycleDisplay.detail}</span> : null}
    </div>
  );

  const renderGridCards = (items: typeof filteredAccounts, groupKey?: string) =>
    items.map((account) => {
      const presentation = resolvePresentation(account);
      const emailText = presentation.displayName || '-';
      const credits = resolveCreditsSummary(account);
      const cycleDisplay = resolveCycleDisplay(credits);
      const usagePanel = buildOfficialUsagePanel(account);
      const accountTags = (account.tags || []).map((tag) => tag.trim()).filter(Boolean);
      const visibleTags = accountTags.slice(0, 2);
      const moreTagCount = Math.max(0, accountTags.length - visibleTags.length);
      const isSelected = selected.has(account.id);
      const isCurrent = currentAccountId === account.id;
      const quotaError = account.quota_query_last_error?.trim();

      return (
        <div key={groupKey ? `${groupKey}-${account.id}` : account.id} className={`ghcp-account-card ${isCurrent ? 'current' : ''} ${isSelected ? 'selected' : ''}`}>
          <div className="card-top">
            <div className="card-select"><input type="checkbox" checked={isSelected} onChange={() => toggleSelect(account.id)} /></div>
            <span className="account-email" title={maskAccountText(emailText)}>{maskAccountText(emailText)}</span>
            {isCurrent && <span className="current-tag">{t('accounts.status.current')}</span>}
            {quotaError && (
              <span className="status-pill warning" title={quotaError}>
                <CircleAlert size={12} />
                {t('common.shared.quota.queryFailed', '配额查询失败')}
              </span>
            )}
            <span className={`tier-badge ${presentation.planClass}`}>{presentation.planLabel}</span>
          </div>
          {accountTags.length > 0 && (
            <div className="card-tags">
              {visibleTags.map((tag, idx) => (<span key={`${account.id}-${tag}-${idx}`} className="tag-pill">{tag}</span>))}
              {moreTagCount > 0 && <span className="tag-pill more">+{moreTagCount}</span>}
            </div>
          )}
          {renderUsagePanel(usagePanel)}
          {renderPlanDetails(cycleDisplay)}
          <div className="card-footer">
            <span className="card-date">{formatDate(account.created_at)}</span>
            <div className="card-actions">
              <button className="card-action-btn success" onClick={() => handleInjectToVSCode?.(account.id)} disabled={!!injecting} title={t('windsurf.injectToVSCode', '切换到 Windsurf')}>
                {injecting === account.id ? <RefreshCw size={14} className="loading-spinner" /> : <Play size={14} />}
              </button>
              <button className="card-action-btn" onClick={() => openTagModal(account.id)} title={t('accounts.editTags', '编辑标签')}><Tag size={14} /></button>
              <button className="card-action-btn" onClick={() => handleRefresh(account.id)} disabled={refreshing === account.id} title={t('common.shared.refreshQuota', '刷新配额')}>
                <RotateCw size={14} className={refreshing === account.id ? 'loading-spinner' : ''} />
              </button>
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
      const emailText = presentation.displayName || '-';
      const credits = resolveCreditsSummary(account);
      const cycleDisplay = resolveCycleDisplay(credits);
      const usagePanel = buildOfficialUsagePanel(account);
      const accountTags = (account.tags || []).map((tag) => tag.trim()).filter(Boolean);
      const visibleTags = accountTags.slice(0, 3);
      const moreTagCount = Math.max(0, accountTags.length - visibleTags.length);
      const isCurrent = currentAccountId === account.id;
      const quotaError = account.quota_query_last_error?.trim();
      return (
        <tr key={groupKey ? `${groupKey}-${account.id}` : account.id} className={isCurrent ? 'current' : ''}>
          <td><input type="checkbox" checked={selected.has(account.id)} onChange={() => toggleSelect(account.id)} /></td>
          <td>
            <div className="account-cell">
              <div className="account-main-line">
                <span className="account-email-text" title={maskAccountText(emailText)}>{maskAccountText(emailText)}</span>
                {isCurrent && <span className="mini-tag current">{t('accounts.status.current')}</span>}
              </div>
              {quotaError && (
                <div className="account-sub-line">
                  <span className="status-pill warning" title={quotaError}>
                    <CircleAlert size={12} />
                    {t('common.shared.quota.queryFailed', '配额查询失败')}
                  </span>
                </div>
              )}
              {accountTags.length > 0 && (
                <div className="account-tags-inline">
                  {visibleTags.map((tag, idx) => (<span key={`${account.id}-inline-${tag}-${idx}`} className="tag-pill">{tag}</span>))}
                  {moreTagCount > 0 && <span className="tag-pill more">+{moreTagCount}</span>}
                </div>
              )}
            </div>
          </td>
          <td><span className={`tier-badge ${presentation.planClass}`}>{presentation.planLabel}</span></td>
          <td>
            {renderUsagePanel(usagePanel, { compact: true })}
          </td>
          <td>
            {renderPlanDetails(cycleDisplay, { compact: true })}
          </td>
          <td className="sticky-action-cell table-action-cell">
            <div className="action-buttons">
              <button className="action-btn success" onClick={() => handleInjectToVSCode?.(account.id)} disabled={!!injecting} title={t('windsurf.injectToVSCode', '切换到 Windsurf')}>
                {injecting === account.id ? <RefreshCw size={14} className="loading-spinner" /> : <Play size={14} />}
              </button>
              <button className="action-btn" onClick={() => openTagModal(account.id)} title={t('accounts.editTags', '编辑标签')}><Tag size={14} /></button>
              <button className="action-btn" onClick={() => handleRefresh(account.id)} disabled={refreshing === account.id} title={t('common.shared.refreshQuota', '刷新配额')}>
                <RotateCw size={14} className={refreshing === account.id ? 'loading-spinner' : ''} />
              </button>
              <button
                className="action-btn"
                onClick={() => handleExportByIds([account.id], resolveSingleExportBaseName(account))}
                title={t('common.shared.export.title', '导出')}
              >
                <Upload size={14} />
              </button>
              <button className="action-btn danger" onClick={() => handleDelete(account.id)} title={t('common.delete', '删除')}><Trash2 size={14} /></button>
            </div>
          </td>
        </tr>
      );
    });

  return (
    <div className="ghcp-accounts-page windsurf-accounts-page">
      <WindsurfOverviewTabsHeader active={activeTab} onTabChange={setActiveTab} />
      <div className={`ghcp-flow-notice ${isFlowNoticeCollapsed ? 'collapsed' : ''}`} role="note" aria-live="polite">
        <button type="button" className="ghcp-flow-notice-toggle" onClick={() => setIsFlowNoticeCollapsed((prev) => !prev)} aria-expanded={!isFlowNoticeCollapsed}>
          <div className="ghcp-flow-notice-title"><CircleAlert size={16} /><span>{t('windsurf.flowNotice.title', 'Windsurf 账号管理说明（点击展开/收起）')}</span></div>
          <ChevronDown size={16} className={`ghcp-flow-notice-arrow ${isFlowNoticeCollapsed ? 'collapsed' : ''}`} />
        </button>
        {!isFlowNoticeCollapsed && (
          <div className="ghcp-flow-notice-body">
            <div className="ghcp-flow-notice-desc">{t('windsurf.flowNotice.desc', 'Switching accounts requires reading VS Code local auth storage and using the system credential service for decrypt/re-encrypt. Data is processed locally only.')}</div>
            <ul className="ghcp-flow-notice-list">
              <li>{t('windsurf.flowNotice.reason', 'Permission scope: read VS Code auth database (state.vscdb) and call system credential capability (Windows DPAPI / macOS Keychain / Linux Secret Service) for decrypt/write-back.')}</li>
              <li>{t('windsurf.flowNotice.storage', 'Data scope: only Windsurf auth-session related entries are read/updated; system secrets are not modified and no key/token is uploaded.')}</li>
            </ul>
          </div>
        )}
      </div>

      {activeTab === 'overview' && (
        <>
      {message && (
        <div className={`message-bar ${message.tone === 'error' ? 'error' : 'success'}`}>
          {message.text}
          <button onClick={() => setMessage(null)}><X size={14} /></button>
        </div>
      )}

      <div className="toolbar">
        <div className="toolbar-left">
          <div className="search-box"><Search size={16} className="search-icon" /><input type="text" placeholder={t('common.shared.search', '搜索账号...')} value={searchQuery} onChange={(e) => setSearchQuery(e.target.value)} /></div>
          <div className="view-switcher">
            <button className={`view-btn ${viewMode === 'list' ? 'active' : ''}`} onClick={() => setViewMode('list')} title={t('common.shared.view.list', '列表视图')}><List size={16} /></button>
            <button className={`view-btn ${viewMode === 'grid' ? 'active' : ''}`} onClick={() => setViewMode('grid')} title={t('common.shared.view.grid', '卡片视图')}><LayoutGrid size={16} /></button>
          </div>
          <MultiSelectFilterDropdown
            options={tierFilterOptions}
            selectedValues={filterTypes}
            allLabel={`ALL (${tierCounts.all})`}
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
            {showTagFilter && (
              <div
                ref={page.tagFilterPanelRef}
                className={`tag-filter-panel ${page.tagFilterPanelPlacement === 'top' ? 'open-top' : ''}`}
              >
                {availableTags.length === 0 ? (
                  <div className="tag-filter-empty">{t('accounts.noAvailableTags', '暂无可用标签')}</div>
                ) : (
                  <div className="tag-filter-options" style={page.tagFilterScrollContainerStyle}>
                    {availableTags.map((tag) => (
                      <label key={tag} className={`tag-filter-option ${tagFilter.includes(tag) ? 'selected' : ''}`}>
                        <input type="checkbox" checked={tagFilter.includes(tag)} onChange={() => toggleTagFilterValue(tag)} />
                        <span className="tag-filter-name">{tag}</span>
                        <button type="button" className="tag-filter-delete" onClick={(e) => { e.preventDefault(); e.stopPropagation(); requestDeleteTag(tag); }}
                          aria-label={t('accounts.deleteTagAria', { tag, defaultValue: '删除标签 {{tag}}' })}><X size={12} /></button>
                      </label>
                    ))}
                  </div>
                )}
                <div className="tag-filter-divider" />
                <label className="tag-filter-group-toggle"><input type="checkbox" checked={groupByTag} onChange={(e) => setGroupByTag(e.target.checked)} /><span>{t('accounts.groupByTag', '按标签分组展示')}</span></label>
                {tagFilter.length > 0 && (<button type="button" className="tag-filter-clear" onClick={clearTagFilter}>{t('accounts.clearFilter', '清空筛选')}</button>)}
              </div>
            )}
          </div>
          <SingleSelectFilterDropdown
            value={sortBy}
            options={[
              { value: 'recommended', label: t('common.shared.sort.recommended', '按推荐') },
              { value: 'created_at', label: t('common.shared.sort.createdAt', '按创建时间') },
              { value: 'credits', label: t('common.shared.sort.credits', '按剩余 Credits') },
              { value: 'plan_end', label: t('common.shared.sort.planEnd', '按配额周期结束时间') },
            ]}
            ariaLabel={t('common.shared.sortLabel', '排序')}
            icon={<ArrowDownWideNarrow size={14} />}
            onChange={setSortBy}
          />
          <button className="sort-direction-btn" onClick={() => setSortDirection((prev) => (prev === 'desc' ? 'asc' : 'desc'))}
            title={sortBy === 'recommended'
              ? (sortDirection === 'desc'
                ? t('common.shared.sort.recommendedDescTooltip', '按推荐度降序（最值得切去的在前）')
                : t('common.shared.sort.recommendedAscTooltip', '按推荐度升序（最不值得切去的在前）'))
              : (sortDirection === 'desc'
                ? t('common.shared.sort.descTooltip', '当前：降序，点击切换为升序')
                : t('common.shared.sort.ascTooltip', '当前：升序，点击切换为降序'))}
            aria-label={t('common.shared.sort.toggleDirection', '切换排序方向')}>{sortDirection === 'desc' ? '⬇' : '⬆'}</button>
        </div>
        <div className="toolbar-right">
          <button className="btn btn-primary icon-only" onClick={() => openAddModal('oauth')} title={t('common.shared.addAccount', '添加账号')}><Plus size={14} /></button>
          <button className="btn btn-secondary icon-only" onClick={handleRefreshAll} disabled={refreshingAll || accounts.length === 0} title={t('common.shared.refreshAll', '刷新全部')}>
            <RefreshCw size={14} className={refreshingAll ? 'loading-spinner' : ''} />
          </button>
          <button className="btn btn-secondary icon-only" onClick={togglePrivacyMode}
            title={privacyModeEnabled ? t('privacy.showSensitive', '显示邮箱') : t('privacy.hideSensitive', '隐藏邮箱')}>
            {privacyModeEnabled ? <EyeOff size={14} /> : <Eye size={14} />}
          </button>
          <button className="btn btn-secondary icon-only" onClick={() => openAddModal('token')} disabled={importing} title={t('common.shared.import.label', '导入')}><Download size={14} /></button>
          <button className="btn btn-secondary export-btn icon-only" onClick={() => void handleExport(filteredIds)} disabled={exporting || filteredIds.length === 0}
            title={exportSelectionCount > 0 ? `${t('common.shared.export.title', '导出')} (${exportSelectionCount})` : t('common.shared.export.title', '导出')}><Upload size={14} /></button>
          {selected.size > 0 && (
            <button className="btn btn-danger icon-only" onClick={handleBatchDelete} title={`${t('common.delete', '删除')} (${selected.size})`}><Trash2 size={14} /></button>
          )}
          <QuickSettingsPopover type="windsurf" />
        </div>
      </div>

      {loading && accounts.length === 0 ? (
        <div className="loading-container"><RefreshCw size={24} className="loading-spinner" /><p>{t('common.loading', '加载中...')}</p></div>
      ) : accounts.length === 0 ? (
        <div className="empty-state">
          <Globe size={48} />
          <h3>{t('common.shared.empty.title', '暂无账号')}</h3>
          <p>{t('windsurf.empty.description', '点击"添加账号"开始管理您的 Windsurf 账号')}</p>
          <div style={{ display: 'flex', gap: '12px', justifyContent: 'center', marginTop: '16px' }}>
            <button className="btn btn-primary" onClick={() => openAddModal('oauth')}>
              <Plus size={16} />
              {t('common.shared.addAccount', '添加账号')}
            </button>
            <button className="btn btn-secondary" onClick={() => window.dispatchEvent(new CustomEvent('app-request-navigate', { detail: 'manual' }))}>
              <BookOpen size={16} />
              {t('manual.navTitle', '功能使用手册')}
            </button>
          </div>
        </div>
      ) : filteredAccounts.length === 0 ? (
        <div className="empty-state"><h3>{t('common.shared.noMatch.title', '没有匹配的账号')}</h3><p>{t('common.shared.noMatch.desc', '请尝试调整搜索或筛选条件')}</p></div>
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
          {groupByTag ? (
          <div className="tag-group-list">{paginatedGroupedAccounts.map(({ groupKey, items, totalCount }) => (
            <div key={groupKey} className="tag-group-section"><div className="tag-group-header"><span className="tag-group-title">{resolveGroupLabel(groupKey)}</span><span className="tag-group-count">{totalCount}</span></div>
              <div className="tag-group-grid ghcp-accounts-grid">{renderGridCards(items, groupKey)}</div></div>
          ))}</div>
        ) : (<div className="ghcp-accounts-grid">{renderGridCards(paginatedAccounts)}</div>)}
        </div>
      ) : groupByTag ? (
        <div className="account-table-container grouped"><table className="account-table"><thead><tr>
          <th style={{ width: 40 }}><input type="checkbox" checked={isAllPaginatedSelected} onChange={() => toggleSelectAll(paginatedIds)} /></th>
          <th style={{ width: 240 }}>{t('common.shared.columns.email', '邮箱')}</th><th style={{ width: 120 }}>{t('common.shared.columns.plan', '计划')}</th>
          <th>{t('common.shared.columns.credits', 'Credits')}</th><th>{t('common.detail', '详情')}</th>
          <th className="sticky-action-header table-action-header">{t('common.shared.columns.actions', '操作')}</th></tr></thead>
          <tbody>{paginatedGroupedAccounts.map(({ groupKey, items, totalCount }) => (
            <Fragment key={groupKey}><tr className="tag-group-row"><td colSpan={6}><div className="tag-group-header"><span className="tag-group-title">{resolveGroupLabel(groupKey)}</span><span className="tag-group-count">{totalCount}</span></div></td></tr>
              {renderTableRows(items, groupKey)}</Fragment>
          ))}</tbody></table></div>
      ) : (
        <div className="account-table-container"><table className="account-table"><thead><tr>
          <th style={{ width: 40 }}><input type="checkbox" checked={isAllPaginatedSelected} onChange={() => toggleSelectAll(paginatedIds)} /></th>
          <th style={{ width: 240 }}>{t('common.shared.columns.email', '邮箱')}</th><th style={{ width: 120 }}>{t('common.shared.columns.plan', '计划')}</th>
          <th>{t('common.shared.columns.credits', 'Credits')}</th><th>{t('common.detail', '详情')}</th>
          <th className="sticky-action-header table-action-header">{t('common.shared.columns.actions', '操作')}</th></tr></thead>
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

      {showAddModal && (
        <div className="modal-overlay" onClick={handleCloseAddModal}><div className="modal-content ghcp-add-modal windsurf-add-modal" onClick={(e) => e.stopPropagation()}>
          <div className="modal-header"><h2>{t('windsurf.addModal.title', '添加 Windsurf 账号')}</h2><button className="modal-close" onClick={handleCloseAddModal} aria-label={t('common.close', '关闭')}><X /></button></div>
          <div className="modal-tabs">
            <button className={`modal-tab ${addTab === 'oauth' ? 'active' : ''}`} onClick={() => openAddModal('oauth')}><Globe size={14} />{t('common.shared.addModal.oauth', 'OAuth Authorization')}</button>
            <button className={`modal-tab ${addTab === 'token' ? 'active' : ''}`} onClick={() => openAddModal('token')}><KeyRound size={14} />Token / JSON</button>
            <button className={`modal-tab ${addTab === 'import' ? 'active' : ''}`} onClick={() => openAddModal('import')}><Database size={14} />{t('common.shared.addModal.import', '本地导入')}</button>
            <button className={`modal-tab ${addTab === 'password' ? 'active' : ''}`} onClick={() => openAddModal('password')}><Mail size={14} />{t('windsurf.addModal.password', '邮箱密码')}</button>
          </div>
          <div className="modal-body">
            {addTab === 'oauth' && (
              <div className="add-section">
                <p className="section-desc">{t('windsurf.oauth.desc', '点击下方按钮，在浏览器中完成 Windsurf OAuth 授权。')}</p>
                {oauthPrepareError ? (
                  <div className="add-status error"><CircleAlert size={16} /><span>{oauthPrepareError}</span>
                    <button className="btn btn-sm btn-outline" onClick={handleRetryOauth}>{t('common.shared.oauth.retry', '重新生成授权信息')}</button></div>
                ) : oauthUrl ? (
                  <div className="oauth-url-section">
                    <div className="oauth-link">
                      <label>{t('accounts.oauth.linkLabel', '授权链接')}</label>
                      <div className="oauth-url-box"><input type="text" value={oauthUrl} readOnly /><button onClick={handleCopyOauthUrl}>{oauthUrlCopied ? <Check size={16} /> : <Copy size={16} />}</button></div>
                    </div>
                    {!oauthUrl.includes('user_code=') && oauthUserCode && (
                      <div className="oauth-url-box"><input type="text" value={oauthUserCode} readOnly /><button onClick={handleCopyOauthUserCode}>{oauthUserCodeCopied ? <Check size={16} /> : <Copy size={16} />}</button></div>
                    )}
                    {oauthMeta && (<p className="oauth-hint">{t('common.shared.oauth.meta', '授权有效期：{{expires}}s；轮询间隔：{{interval}}s', { expires: oauthMeta.expiresIn, interval: oauthMeta.intervalSeconds })}</p>)}
                    <button className="btn btn-primary btn-full" onClick={handleOpenOauthUrl}><Globe size={16} />{t('common.shared.oauth.openBrowser', '在浏览器中打开')}</button>
                    {oauthSupportsManualCallback && (
                      <div className="oauth-link">
                        <label>{t('common.shared.oauth.manualCallbackLabel', '手动输入回调地址')}</label>
                        <div className="oauth-url-box oauth-manual-input">
                          <input
                            type="text"
                            value={oauthManualCallbackInput}
                            onChange={(e) => setOauthManualCallbackInput(e.target.value)}
                            placeholder={t('common.shared.oauth.manualCallbackPlaceholder', '粘贴完整回调地址，例如：http://localhost:1455/auth/callback?code=...&state=...')}
                          />
                          <button
                            className="oauth-copy-button"
                            onClick={() => void handleSubmitOauthCallbackUrl()}
                            disabled={oauthManualCallbackSubmitting || !oauthManualCallbackInput.trim()}
                          >
                            {oauthManualCallbackSubmitting ? <RefreshCw size={16} className="loading-spinner" /> : <Check size={16} />}
                            {t('accounts.oauth.continue', '我已授权，继续')}
                          </button>
                        </div>
                      </div>
                    )}
                    {oauthManualCallbackError && (
                      <div className="add-status error"><CircleAlert size={16} /><span>{oauthManualCallbackError}</span></div>
                    )}
                    {oauthPolling && (<div className="add-status loading"><RefreshCw size={16} className="loading-spinner" /><span>{t('common.shared.oauth.waiting', '等待授权完成...')}</span></div>)}
                    {oauthCompleteError && (<div className="add-status error"><CircleAlert size={16} /><span>{oauthCompleteError}</span>{oauthTimedOut && (<button className="btn btn-sm btn-outline" onClick={handleRetryOauth}>{t('common.shared.oauth.timeoutRetry', '刷新授权链接')}</button>)}</div>)}
                    <p className="oauth-hint">{t('common.shared.oauth.hint', 'Once authorized, this window will update automatically')}</p>
                  </div>
                ) : (<div className="oauth-loading"><RefreshCw size={24} className="loading-spinner" /><span>{t('common.shared.oauth.preparing', '正在准备授权信息...')}</span></div>)}
              </div>
            )}
            {addTab === 'token' && (
              <div className="add-section">
                <p className="section-desc">{t('windsurf.token.desc', '粘贴您的 Windsurf API Key 或导出的 JSON 数据。')}</p>
                <details className="token-format-collapse"><summary className="token-format-collapse-summary">必填字段与示例（点击展开）</summary>
                  <div className="token-format"><p className="token-format-required">必填字段：github_access_token (API Key)</p>
                    <div className="token-format-group"><div className="token-format-label">单条示例</div><pre className="token-format-code">{WINDSURF_TOKEN_SINGLE_EXAMPLE}</pre></div>
                    <div className="token-format-group"><div className="token-format-label">批量示例（JSON）</div><pre className="token-format-code">{WINDSURF_TOKEN_BATCH_EXAMPLE}</pre></div>
                  </div></details>
                <textarea className="token-input" value={tokenInput} onChange={(e) => setTokenInput(e.target.value)} placeholder={t('common.shared.token.placeholder', '粘贴 Token 或 JSON...')} />
                <button className="btn btn-primary btn-full" onClick={handleTokenImport} disabled={importing || !tokenInput.trim()}>
                  {importing ? <RefreshCw size={16} className="loading-spinner" /> : <Download size={16} />}{t('common.shared.token.import', 'Import')}</button>
              </div>
            )}
            {addTab === 'import' && (
              <div className="add-section">
                <p className="section-desc">{t('windsurf.import.localDesc', '支持从本机 Windsurf 客户端或 JSON 文件导入账号数据。')}</p>
                <button className="btn btn-secondary btn-full" onClick={() => handleImportFromLocal?.()} disabled={importing}>
                  {importing ? <RefreshCw size={16} className="loading-spinner" /> : <Database size={16} />}{t('windsurf.import.localClient', '从本机 Windsurf 导入')}</button>
                <div className="oauth-hint" style={{ margin: '8px 0 4px' }}>{t('common.shared.import.orJson', '或从 JSON 文件导入')}</div>
                <input ref={importFileInputRef} type="file" accept="application/json" style={{ display: 'none' }}
                  onChange={(e) => { const file = e.target.files?.[0]; e.target.value = ''; if (!file) return; void handleImportJsonFile(file); }} />
                <button className="btn btn-primary btn-full" onClick={handlePickImportFile} disabled={importing}>
                  {importing ? <RefreshCw size={16} className="loading-spinner" /> : <Database size={16} />}{t('common.shared.import.pickFile', '选择 JSON 文件导入')}</button>
              </div>
            )}
            {addTab === 'password' && (
              <div className="add-section">
                <p className="section-desc">{t('windsurf.password.desc', '使用 Windsurf 账号邮箱和密码登录，自动获取 API Key 与账号信息。')}</p>
                <div className="oauth-hint" style={{ marginBottom: 4 }}>{t('windsurf.password.modeLabel', '操作方式')}</div>
                <div className="inline-radio-group">
                  <label className="inline-radio-option">
                    <input
                      type="radio"
                      className="inline-radio-input"
                      name="windsurf-password-mode"
                      checked={passwordMode === 'single'}
                      onChange={() => {
                        clearPasswordFeedback();
                        setPasswordMode('single');
                      }}
                      disabled={passwordSubmitting}
                    />
                    <span className="inline-radio-text">{t('windsurf.password.modeSingle', '单个登录')}</span>
                  </label>
                  <label className="inline-radio-option">
                    <input
                      type="radio"
                      className="inline-radio-input"
                      name="windsurf-password-mode"
                      checked={passwordMode === 'batch'}
                      onChange={() => {
                        clearPasswordFeedback();
                        setPasswordMode('batch');
                      }}
                      disabled={passwordSubmitting}
                    />
                    <span className="inline-radio-text">{t('windsurf.password.modeBatch', '批量导入')}</span>
                  </label>
                </div>

                {passwordMode === 'single' ? (
                  <>
                    <input
                      type="email"
                      className="token-input"
                      style={{ minHeight: 'auto', height: 40, resize: 'none', fontFamily: 'inherit', marginTop: 12 }}
                      value={passwordEmail}
                      onChange={(e) => {
                        clearPasswordFeedback();
                        setPasswordEmail(e.target.value);
                      }}
                      placeholder={t('windsurf.password.emailPlaceholder', '邮箱地址')}
                      disabled={passwordSubmitting}
                    />
                    <input
                      type="password"
                      className="token-input"
                      style={{ minHeight: 'auto', height: 40, resize: 'none', fontFamily: 'inherit', marginTop: 8 }}
                      value={passwordPassword}
                      onChange={(e) => {
                        clearPasswordFeedback();
                        setPasswordPassword(e.target.value);
                      }}
                      placeholder={t('windsurf.password.passwordPlaceholder', '密码')}
                      disabled={passwordSubmitting}
                      onKeyDown={(e) => {
                        if (e.key === 'Enter') {
                          void handlePasswordLogin();
                        }
                      }}
                    />
                    {passwordFieldError && (
                      <div ref={passwordFieldErrorRef} className="add-status error" style={{ marginTop: 8 }}>
                        <CircleAlert size={16} />
                        <span>{passwordFieldError}</span>
                      </div>
                    )}
                    <button
                      className="btn btn-primary btn-full"
                      style={{ marginTop: 12 }}
                      onClick={() => void handlePasswordLogin()}
                      disabled={passwordSubmitting || !passwordEmail.trim() || passwordPassword.length === 0}
                    >
                      {passwordLoading ? <RefreshCw size={16} className="loading-spinner" /> : <Mail size={16} />}
                      {passwordLoading ? t('windsurf.password.logging', '正在登录...') : t('windsurf.password.login', '登录')}
                    </button>
                  </>
                ) : (
                  <>
                    <p className="section-desc" style={{ marginTop: 12 }}>
                      {t('windsurf.password.batchDesc', '批量导入支持 JSON 或逐行文本；文本模式必须显式选择分隔符。')}
                    </p>
                    <div className="oauth-hint" style={{ marginBottom: 4 }}>
                      {t('windsurf.password.batchInputModeLabel', '导入格式')}
                    </div>
                    <div className="inline-radio-group">
                      <label className="inline-radio-option">
                        <input
                          type="radio"
                          className="inline-radio-input"
                          name="windsurf-password-batch-input-mode"
                          checked={passwordBatchInputMode === 'json'}
                          onChange={() => {
                            clearPasswordFeedback();
                            setPasswordBatchInputMode('json');
                          }}
                          disabled={passwordSubmitting}
                        />
                        <span className="inline-radio-text">{t('windsurf.password.batchInputModeJson', 'JSON')}</span>
                      </label>
                      <label className="inline-radio-option">
                        <input
                          type="radio"
                          className="inline-radio-input"
                          name="windsurf-password-batch-input-mode"
                          checked={passwordBatchInputMode === 'text'}
                          onChange={() => {
                            clearPasswordFeedback();
                            setPasswordBatchInputMode('text');
                          }}
                          disabled={passwordSubmitting}
                        />
                        <span className="inline-radio-text">{t('windsurf.password.batchInputModeText', '文本')}</span>
                      </label>
                    </div>
                    {passwordBatchInputMode === 'text' && (
                      <>
                        <div className="oauth-hint" style={{ margin: '8px 0 4px' }}>
                          {t('windsurf.password.batchDelimiterLabel', '分隔符')}
                        </div>
                        <div className="inline-radio-group">
                          <label className="inline-radio-option">
                            <input
                              type="radio"
                              className="inline-radio-input"
                              name="windsurf-password-batch-delimiter"
                              checked={passwordBatchDelimiter === 'tab'}
                              onChange={() => {
                                clearPasswordFeedback();
                                setPasswordBatchDelimiter('tab');
                              }}
                              disabled={passwordSubmitting}
                            />
                            <span className="inline-radio-text">{t('windsurf.password.batchDelimiterTab', 'Tab')}</span>
                          </label>
                          <label className="inline-radio-option">
                            <input
                              type="radio"
                              className="inline-radio-input"
                              name="windsurf-password-batch-delimiter"
                              checked={passwordBatchDelimiter === 'space'}
                              onChange={() => {
                                clearPasswordFeedback();
                                setPasswordBatchDelimiter('space');
                              }}
                              disabled={passwordSubmitting}
                            />
                            <span className="inline-radio-text">{t('windsurf.password.batchDelimiterSpace', '空格')}</span>
                          </label>
                          <label className="inline-radio-option">
                            <input
                              type="radio"
                              className="inline-radio-input"
                              name="windsurf-password-batch-delimiter"
                              checked={passwordBatchDelimiter === 'comma'}
                              onChange={() => {
                                clearPasswordFeedback();
                                setPasswordBatchDelimiter('comma');
                              }}
                              disabled={passwordSubmitting}
                            />
                            <span className="inline-radio-text">{t('windsurf.password.batchDelimiterComma', '逗号')}</span>
                          </label>
                          <label className="inline-radio-option">
                            <input
                              type="radio"
                              className="inline-radio-input"
                              name="windsurf-password-batch-delimiter"
                              checked={passwordBatchDelimiter === 'pipe'}
                              onChange={() => {
                                clearPasswordFeedback();
                                setPasswordBatchDelimiter('pipe');
                              }}
                              disabled={passwordSubmitting}
                            />
                            <span className="inline-radio-text">{t('windsurf.password.batchDelimiterPipe', '竖线')}</span>
                          </label>
                          <label className="inline-radio-option">
                            <input
                              type="radio"
                              className="inline-radio-input"
                              name="windsurf-password-batch-delimiter"
                              checked={passwordBatchDelimiter === 'dash'}
                              onChange={() => {
                                clearPasswordFeedback();
                                setPasswordBatchDelimiter('dash');
                              }}
                              disabled={passwordSubmitting}
                            />
                            <span className="inline-radio-text">{t('windsurf.password.batchDelimiterDash', '----')}</span>
                          </label>
                          <label className="inline-radio-option">
                            <input
                              type="radio"
                              className="inline-radio-input"
                              name="windsurf-password-batch-delimiter"
                              checked={passwordBatchDelimiter === 'custom'}
                              onChange={() => {
                                clearPasswordFeedback();
                                setPasswordBatchDelimiter('custom');
                              }}
                              disabled={passwordSubmitting}
                            />
                            <span className="inline-radio-text">{t('windsurf.password.batchDelimiterCustom', '自定义')}</span>
                          </label>
                        </div>
                        {passwordBatchDelimiter === 'custom' && (
                          <>
                            <input
                              type="text"
                              className="token-input"
                              style={{ minHeight: 'auto', height: 40, resize: 'none', fontFamily: 'inherit', marginTop: 8 }}
                              value={passwordBatchCustomDelimiter}
                              onChange={(e) => {
                                clearPasswordFeedback();
                                setPasswordBatchCustomDelimiter(e.target.value);
                              }}
                              placeholder={t('windsurf.password.batchCustomDelimiterPlaceholder', '输入自定义分隔符')}
                              disabled={passwordSubmitting}
                            />
                            {passwordBatchDelimiterFieldError && (
                              <div ref={passwordBatchDelimiterFieldErrorRef} className="add-status error" style={{ marginTop: 8 }}>
                                <CircleAlert size={16} />
                                <span>{passwordBatchDelimiterFieldError}</span>
                              </div>
                            )}
                          </>
                        )}
                      </>
                    )}
                    <div className="oauth-hint" style={{ margin: '8px 0 4px' }}>
                      {passwordBatchInputMode === 'json'
                        ? t('windsurf.password.batchJsonDesc', '粘贴 JSON 数组，每项包含 email 和 password。')
                        : t('windsurf.password.batchTextDesc', '每行一组账号，严格按当前分隔符解析。')}
                    </div>
                    <details className="token-format-collapse" style={{ marginTop: 8 }}>
                      <summary className="token-format-collapse-summary">
                        {t('windsurf.password.batchExampleTitle', '格式示例（点击展开）')}
                      </summary>
                      <div className="token-format">
                        <div className="token-format-group">
                          <div className="token-format-label">
                            {passwordBatchInputMode === 'json'
                              ? t('windsurf.password.batchExampleJsonLabel', 'JSON 示例')
                              : t('windsurf.password.batchExampleTextLabel', '文本示例')}
                          </div>
                          <pre className="token-format-code">{passwordBatchExample}</pre>
                        </div>
                      </div>
                    </details>
                    <textarea
                      className="token-input"
                      style={{ marginTop: 12 }}
                      value={passwordBatchInput}
                      onChange={(e) => {
                        clearPasswordFeedback();
                        setPasswordBatchInput(e.target.value);
                      }}
                      placeholder={passwordBatchPlaceholder}
                      disabled={passwordSubmitting}
                    />
                    {passwordBatchFieldError && (
                      <div ref={passwordBatchFieldErrorRef} className="add-status error" style={{ marginTop: 8 }}>
                        <CircleAlert size={16} />
                        <span>{passwordBatchFieldError}</span>
                      </div>
                    )}
                    <button
                      className="btn btn-primary btn-full"
                      style={{ marginTop: 12 }}
                      onClick={() => void handlePasswordBatchImport()}
                      disabled={passwordSubmitting || !passwordBatchInput.trim()}
                    >
                      {passwordBatchLoading ? <RefreshCw size={16} className="loading-spinner" /> : <Download size={16} />}
                      {passwordBatchLoading
                        ? t('windsurf.password.batchImportingShort', '导入中...')
                        : t('windsurf.password.batchImport', '批量导入')}
                    </button>
                  </>
                )}
              </div>
            )}
            {addStatus !== 'idle' && addStatus !== 'loading' && (
              <div ref={passwordStatusRef} className={`add-status ${addStatus}`}>{addStatus === 'success' ? <Check size={16} /> : <CircleAlert size={16} />}<span>{addMessage}</span></div>
            )}
          </div>
        </div></div>
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

      {deleteConfirm && (
        <div className="modal-overlay" onClick={() => !deleting && setDeleteConfirm(null)}><div className="modal" onClick={(e) => e.stopPropagation()}>
          <div className="modal-header"><h2>{t('common.confirm')}</h2><button className="modal-close" onClick={() => !deleting && setDeleteConfirm(null)} aria-label={t('common.close', '关闭')}><X /></button></div>
          <div className="modal-body"><ModalErrorMessage message={deleteConfirmError} scrollKey={deleteConfirmErrorScrollKey} /><p>{deleteConfirm.message}</p></div>
          <div className="modal-footer"><button className="btn btn-secondary" onClick={() => setDeleteConfirm(null)} disabled={deleting}>{t('common.cancel')}</button>
            <button className="btn btn-danger" onClick={confirmDelete} disabled={deleting}>{t('common.confirm')}</button></div>
        </div></div>
      )}

      {tagDeleteConfirm && (
        <div className="modal-overlay" onClick={() => !deletingTag && setTagDeleteConfirm(null)}><div className="modal" onClick={(e) => e.stopPropagation()}>
          <div className="modal-header"><h2>{t('common.confirm')}</h2><button className="modal-close" onClick={() => !deletingTag && setTagDeleteConfirm(null)} aria-label={t('common.close', '关闭')}><X /></button></div>
          <div className="modal-body"><ModalErrorMessage message={tagDeleteConfirmError} scrollKey={tagDeleteConfirmErrorScrollKey} /><p>{t('accounts.confirmDeleteTag', 'Delete tag "{{tag}}"? This tag will be removed from {{count}} accounts.', { tag: tagDeleteConfirm.tag, count: tagDeleteConfirm.count })}</p></div>
          <div className="modal-footer"><button className="btn btn-secondary" onClick={() => setTagDeleteConfirm(null)} disabled={deletingTag}>{t('common.cancel')}</button>
            <button className="btn btn-danger" onClick={confirmDeleteTag} disabled={deletingTag}>{deletingTag ? t('common.processing', '处理中...') : t('common.confirm')}</button></div>
        </div></div>
      )}

      <TagEditModal isOpen={!!showTagModal} initialTags={accounts.find((a) => a.id === showTagModal)?.tags || []} availableTags={availableTags}
        onClose={() => setShowTagModal(null)} onSave={handleSaveTags} />
        </>
      )}

      {activeTab === 'instances' && (
        <WindsurfInstancesContent accountsForSelect={sortedAccountsForInstances} />
      )}
    </div>
  );
}
