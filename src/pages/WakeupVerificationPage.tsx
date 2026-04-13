import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { confirm as confirmDialog } from '@tauri-apps/plugin-dialog';
import { openUrl } from '@tauri-apps/plugin-opener';
import { ShieldCheck, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { ModalErrorMessage, useModalErrorState } from '../components/ModalErrorMessage';
import { OverviewTabsHeader } from '../components/OverviewTabsHeader';
import { MultiSelectFilterDropdown, type MultiSelectFilterOption } from '../components/MultiSelectFilterDropdown';
import { useAccountStore } from '../stores/useAccountStore';
import { Page } from '../types/navigation';
import {
  collectAntigravityQuotaModelKeys,
  filterAntigravityModelOptions,
  getAntigravityModelDisplayName,
  type AntigravityModelOption,
} from '../utils/antigravityModels';
import {
  accountMatchesTagFilters,
  accountMatchesTypeFilters,
  buildAccountTierCounts,
  buildAccountTierFilterOptions,
  collectAvailableAccountTags,
  normalizeAccountTag,
  type AccountFilterType,
} from '../utils/accountFilters';
import { getAntigravityTierBadge } from '../utils/account';
import { getAccountGroups, type AccountGroup } from '../services/accountGroupService';
import {
  isPrivacyModeEnabledByDefault,
  maskSensitiveValue,
  PRIVACY_MODE_CHANGED_EVENT,
} from '../utils/privacy';
import {
  loadWakeupOfficialLsVersionMode,
  saveWakeupOfficialLsVersionMode,
  WAKEUP_OFFICIAL_LS_VERSION_CHANGED_EVENT,
  type WakeupOfficialLsVersionMode,
} from '../utils/wakeupOfficialLsVersion';

interface WakeupVerificationPageProps {
  onNavigate?: (page: Page) => void;
}

type AvailableModel = AntigravityModelOption;
type DetailFilter = 'all' | 'success' | 'verification_required' | 'tos_violation' | 'failed';
type DetailMode = 'running' | 'history';

interface WakeupVerificationStateItem {
  accountId: string;
  accountEmail: string;
  status: string;
  lastVerifyAt?: number | null;
  lastModel?: string | null;
  lastErrorCode?: number | null;
  lastMessage?: string | null;
  validationUrl?: string | null;
  appealUrl?: string | null;
  trajectoryId?: string | null;
  durationMs?: number | null;
}

interface WakeupVerificationProgressPayload {
  batchId: string;
  total: number;
  completed: number;
  successCount: number;
  verificationRequiredCount: number;
  tosViolationCount: number;
  failedCount: number;
  running: boolean;
  item?: WakeupVerificationStateItem | null;
}

interface WakeupVerificationBatchHistoryItem {
  batchId: string;
  verifiedAt: number;
  model: string;
  prompt: string;
  total: number;
  completed: number;
  successCount: number;
  verificationRequiredCount: number;
  tosViolationCount: number;
  failedCount: number;
  records: WakeupVerificationStateItem[];
}

interface WakeupVerificationBatchResult {
  batchId: string;
  verifiedAt: number;
  model: string;
  prompt: string;
  total: number;
  completed: number;
  successCount: number;
  verificationRequiredCount: number;
  tosViolationCount: number;
  failedCount: number;
  records: WakeupVerificationStateItem[];
}

const DEFAULT_PROMPT = 'hi';
const APP_PATH_NOT_FOUND_PREFIX = 'APP_PATH_NOT_FOUND:';
const STATUS_IDLE = 'idle';
const STATUS_RUNNING = 'running';
const STATUS_SUCCESS = 'success';
const STATUS_VERIFICATION_REQUIRED = 'verification_required';
const STATUS_TOS_VIOLATION = 'tos_violation';
const STATUS_AUTH_EXPIRED = 'auth_expired';
const STATUS_FAILED = 'failed';
const UNGROUPED_ACCOUNT_GROUP_FILTER_KEY = '__ungrouped__';

const formatErrorMessage = (error: unknown) => {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
};

const isAntigravityPathMissingError = (message: string) =>
  message.startsWith(`${APP_PATH_NOT_FOUND_PREFIX}antigravity`);

function normalizeStatus(value?: string | null): string {
  const status = (value || '').trim().toLowerCase();
  if (!status) return STATUS_IDLE;
  switch (status) {
    case STATUS_RUNNING:
    case STATUS_SUCCESS:
    case STATUS_VERIFICATION_REQUIRED:
    case STATUS_TOS_VIOLATION:
    case STATUS_AUTH_EXPIRED:
    case STATUS_FAILED:
    case STATUS_IDLE:
      return status;
    default:
      return STATUS_FAILED;
  }
}

function buildIdleState(accountId: string, accountEmail: string): WakeupVerificationStateItem {
  return {
    accountId,
    accountEmail,
    status: STATUS_IDLE,
    lastVerifyAt: null,
    lastModel: null,
    lastErrorCode: null,
    lastMessage: null,
    validationUrl: null,
    trajectoryId: null,
    durationMs: null,
  };
}

function mergeHistoryBatches(
  current: WakeupVerificationBatchHistoryItem[],
  updates: WakeupVerificationBatchHistoryItem[],
): WakeupVerificationBatchHistoryItem[] {
  if (updates.length === 0) return current;
  const map = new Map<string, WakeupVerificationBatchHistoryItem>();
  current.forEach((item) => map.set(item.batchId, item));
  updates.forEach((item) => map.set(item.batchId, item));
  return Array.from(map.values()).sort((a, b) => b.verifiedAt - a.verifiedAt);
}

function isFailedStatus(status: string): boolean {
  const normalized = normalizeStatus(status);
  return normalized === STATUS_FAILED || normalized === STATUS_AUTH_EXPIRED;
}


function matchesDetailFilter(item: WakeupVerificationStateItem, filter: DetailFilter): boolean {
  const normalized = normalizeStatus(item.status);
  if (filter === 'all') return true;
  if (filter === 'success') return normalized === STATUS_SUCCESS;
  if (filter === 'verification_required') return normalized === STATUS_VERIFICATION_REQUIRED;
  if (filter === 'tos_violation') return normalized === STATUS_TOS_VIOLATION;
  return isFailedStatus(normalized);
}

export function WakeupVerificationPage({ onNavigate }: WakeupVerificationPageProps) {
  const { t, i18n } = useTranslation();
  const { accounts, fetchAccounts } = useAccountStore();
  const locale = i18n.language || 'zh-CN';

  const [running, setRunning] = useState(false);
  const [notice, setNotice] = useState<{ text: string; tone: 'success' | 'warning' | 'error' } | null>(null);
  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([]);
  const [historyBatches, setHistoryBatches] = useState<WakeupVerificationBatchHistoryItem[]>([]);
  const [liveStates, setLiveStates] = useState<Record<string, WakeupVerificationStateItem>>({});

  const [showConfigModal, setShowConfigModal] = useState(false);
  const [showDetailModal, setShowDetailModal] = useState(false);
  const {
    message: configModalError,
    scrollKey: configModalErrorScrollKey,
    report: reportConfigModalError,
    clear: clearConfigModalError,
  } = useModalErrorState('');
  const {
    message: detailModalError,
    scrollKey: detailModalErrorScrollKey,
    report: reportDetailModalError,
    clear: clearDetailModalError,
  } = useModalErrorState('');
  const [selectedAccounts, setSelectedAccounts] = useState<string[]>([]);
  const [selectedModel, setSelectedModel] = useState('');
  const [batchAccountIds, setBatchAccountIds] = useState<string[]>([]);
  const [progress, setProgress] = useState<WakeupVerificationProgressPayload | null>(null);
  const [runningModel, setRunningModel] = useState('');
  const [runningPrompt, setRunningPrompt] = useState(DEFAULT_PROMPT);
  const [runningStartedAt, setRunningStartedAt] = useState<number | null>(null);
  const [detailMode, setDetailMode] = useState<DetailMode>('history');
  const [detailBatchId, setDetailBatchId] = useState<string | null>(null);
  const [detailFilter, setDetailFilter] = useState<DetailFilter>('all');
  const [selectedBatchIds, setSelectedBatchIds] = useState<string[]>([]);
  const [privacyModeEnabled, setPrivacyModeEnabled] = useState<boolean>(() =>
    isPrivacyModeEnabledByDefault(),
  );

  const [accountSearchQuery, setAccountSearchQuery] = useState('');
  const [configTypeFilter, setConfigTypeFilter] = useState<AccountFilterType[]>([]);
  const [configTagFilter, setConfigTagFilter] = useState<string[]>([]);
  const [configGroupFilter, setConfigGroupFilter] = useState<string[]>([]);
  const [accountGroups, setAccountGroups] = useState<AccountGroup[]>([]);
  const [officialLsVersionMode, setOfficialLsVersionMode] = useState<WakeupOfficialLsVersionMode>(
    () => loadWakeupOfficialLsVersionMode(),
  );

  const activeBatchIdRef = useRef<string | null>(null);
  const accountSelectAllRef = useRef<HTMLInputElement | null>(null);

  const accountById = useMemo(() => {
    const map = new Map<string, (typeof accounts)[number]>();
    accounts.forEach((account) => map.set(account.id, account));
    return map;
  }, [accounts]);
  const accountIds = useMemo(() => accounts.map((account) => account.id), [accounts]);
  const availableFilterTags = useMemo(() => collectAvailableAccountTags(accounts), [accounts]);
  const availableFilterTagOptions = useMemo<MultiSelectFilterOption[]>(
    () =>
      availableFilterTags.map((tag) => ({
        value: tag,
        label: tag,
      })),
    [availableFilterTags],
  );
  const tierCounts = useMemo(
    () => buildAccountTierCounts(accounts, {}),
    [accounts],
  );
  const typeFilterOptions = useMemo<MultiSelectFilterOption[]>(
    () => buildAccountTierFilterOptions(t, tierCounts),
    [t, tierCounts],
  );
  const groupIdsByAccountId = useMemo(() => {
    const map = new Map<string, string[]>();
    accountGroups.forEach((group) => {
      group.accountIds.forEach((accountId) => {
        const current = map.get(accountId);
        if (!current) {
          map.set(accountId, [group.id]);
          return;
        }
        current.push(group.id);
      });
    });
    return map;
  }, [accountGroups]);
  const accountIdsInAnyGroup = useMemo(() => {
    const ids = new Set<string>();
    accountGroups.forEach((group) => {
      group.accountIds.forEach((accountId) => ids.add(accountId));
    });
    return ids;
  }, [accountGroups]);
  const groupFilterOptions = useMemo<MultiSelectFilterOption[]>(() => {
    const groupOptions = accountGroups
      .map((group) => {
        const count = group.accountIds.filter((id) => accountById.has(id)).length;
        return {
          value: group.id,
          label: `${group.name} (${count})`,
        };
      })
      .sort((left, right) => left.label.localeCompare(right.label));
    const ungroupedCount = accounts.reduce(
      (count, account) => (accountIdsInAnyGroup.has(account.id) ? count : count + 1),
      0,
    );
    return [
      ...groupOptions,
      {
        value: UNGROUPED_ACCOUNT_GROUP_FILTER_KEY,
        label: `${t('accounts.groups.ungrouped', '未分组')} (${ungroupedCount})`,
      },
    ];
  }, [accountById, accountGroups, accountIdsInAnyGroup, accounts, t]);
  const filteredConfigAccounts = useMemo(
    () => {
      const query = accountSearchQuery.trim().toLowerCase();
      const selectedTypes = new Set<AccountFilterType>(configTypeFilter);
      const selectedTags = new Set(configTagFilter.map(normalizeAccountTag));
      const selectedGroups = new Set(configGroupFilter);

      return accounts
        .filter((account) => {
          const email = (account.email || '').toLowerCase();
          if (query && !email.includes(query)) {
            return false;
          }
          if (!accountMatchesTypeFilters(account, selectedTypes, {})) {
            return false;
          }
          if (!accountMatchesTagFilters(account, selectedTags)) {
            return false;
          }
          if (selectedGroups.size > 0) {
            const groupIds = groupIdsByAccountId.get(account.id) || [];
            const matchesGrouped = groupIds.some((groupId) => selectedGroups.has(groupId));
            const matchesUngrouped =
              groupIds.length === 0 && selectedGroups.has(UNGROUPED_ACCOUNT_GROUP_FILTER_KEY);
            if (!matchesGrouped && !matchesUngrouped) {
              return false;
            }
          }
          return true;
        })
        .sort((left, right) => left.email.localeCompare(right.email));
    },
    [
      accountSearchQuery,
      accounts,
      configGroupFilter,
      configTagFilter,
      configTypeFilter,
      groupIdsByAccountId,
    ],
  );
  const filteredConfigAccountIds = useMemo(
    () => filteredConfigAccounts.map((account) => account.id),
    [filteredConfigAccounts],
  );
  const selectedAccountSet = useMemo(() => new Set(selectedAccounts), [selectedAccounts]);
  const selectedVisibleAccountsCount = useMemo(
    () =>
      filteredConfigAccountIds.reduce(
        (count, id) => (selectedAccountSet.has(id) ? count + 1 : count),
        0,
      ),
    [filteredConfigAccountIds, selectedAccountSet],
  );
  const allVisibleAccountsSelected = useMemo(
    () =>
      filteredConfigAccountIds.length > 0
      && selectedVisibleAccountsCount === filteredConfigAccountIds.length,
    [filteredConfigAccountIds.length, selectedVisibleAccountsCount],
  );
  const partiallyVisibleAccountsSelected = useMemo(
    () =>
      selectedVisibleAccountsCount > 0
      && selectedVisibleAccountsCount < filteredConfigAccountIds.length,
    [filteredConfigAccountIds.length, selectedVisibleAccountsCount],
  );

  const quotaModelKeys = useMemo(() => collectAntigravityQuotaModelKeys(accounts), [accounts]);
  const filteredModels = useMemo(
    () =>
      filterAntigravityModelOptions(availableModels, {
        allowedModelKeys: quotaModelKeys,
        includeNonRecommended: false,
      }),
    [availableModels, quotaModelKeys],
  );

  const preferredDefaultModelId = useMemo(() => {
    if (filteredModels.length === 0) {
      return '';
    }
    const flashModel = filteredModels.find((model) => {
      const displayName = (
        model.displayName ||
        getAntigravityModelDisplayName(model.id) ||
        model.id
      )
        .trim()
        .toLowerCase();
      return displayName.includes('flash');
    });
    return flashModel?.id || filteredModels[0].id;
  }, [filteredModels]);

  const modelNameById = useMemo(() => {
    const map = new Map<string, string>();
    filteredModels.forEach((model) => {
      const displayName = (model.displayName || '').trim();
      map.set(model.id, displayName || getAntigravityModelDisplayName(model.id) || model.id);
    });
    return map;
  }, [filteredModels]);

  const historyById = useMemo(() => {
    const map = new Map<string, WakeupVerificationBatchHistoryItem>();
    historyBatches.forEach((item) => map.set(item.batchId, item));
    return map;
  }, [historyBatches]);

  const allHistorySelected = useMemo(
    () => historyBatches.length > 0 && selectedBatchIds.length === historyBatches.length,
    [historyBatches.length, selectedBatchIds.length],
  );

  const progressRows = useMemo(() => {
    if (batchAccountIds.length === 0) return [];
    return batchAccountIds
      .map((accountId) => {
        const account = accountById.get(accountId);
        return liveStates[accountId] || buildIdleState(accountId, account?.email || accountId);
      })
      .sort((a, b) => a.accountEmail.localeCompare(b.accountEmail));
  }, [batchAccountIds, accountById, liveStates]);

  const activeDetail = useMemo<WakeupVerificationBatchHistoryItem | null>(() => {
    if (!detailBatchId) {
      return null;
    }
    if (detailMode === 'history') {
      return historyById.get(detailBatchId) || null;
    }

    const batchId = activeBatchIdRef.current || progress?.batchId || detailBatchId;
    return {
      batchId,
      verifiedAt: runningStartedAt || Date.now(),
      model: runningModel || selectedModel,
      prompt: runningPrompt,
      total: progress?.total ?? batchAccountIds.length,
      completed: progress?.completed ?? 0,
      successCount: progress?.successCount ?? 0,
      verificationRequiredCount: progress?.verificationRequiredCount ?? 0,
      tosViolationCount: progress?.tosViolationCount ?? 0,
      failedCount: progress?.failedCount ?? 0,
      records: progressRows,
    };
  }, [
    detailBatchId,
    detailMode,
    historyById,
    progress,
    runningStartedAt,
    runningModel,
    selectedModel,
    runningPrompt,
    batchAccountIds.length,
    progressRows,
  ]);

  const detailRows = useMemo(() => {
    if (!activeDetail) return [];
    return activeDetail.records || [];
  }, [activeDetail]);

  const detailCounts = useMemo(() => {
    const all = detailRows.length;
    let success = 0;
    let verificationRequired = 0;
    let tosViolation = 0;
    let failed = 0;

    detailRows.forEach((item) => {
      const status = normalizeStatus(item.status);
      if (status === STATUS_SUCCESS) {
        success += 1;
      } else if (status === STATUS_VERIFICATION_REQUIRED) {
        verificationRequired += 1;
      } else if (status === STATUS_TOS_VIOLATION) {
        tosViolation += 1;
      } else if (isFailedStatus(status)) {
        failed += 1;
      }
    });

    return {
      all,
      success,
      verificationRequired,
      tosViolation,
      failed,
    };
  }, [detailRows]);

  const filteredDetailRows = useMemo(
    () => detailRows.filter((item) => matchesDetailFilter(item, detailFilter)),
    [detailRows, detailFilter],
  );

  const maskAccountText = useCallback(
    (value?: string | null) => maskSensitiveValue(value, privacyModeEnabled),
    [privacyModeEnabled],
  );
  const handleOfficialLsVersionModeChange = useCallback((value: string) => {
    const nextMode =
      value === 'lt_1_21_6'
        ? 'lt_1_21_6'
        : 'gte_1_21_6';
    setOfficialLsVersionMode(nextMode);
    saveWakeupOfficialLsVersionMode(nextMode);
  }, []);
  const resolveAccountPlanBadge = useCallback(
    (accountId?: string | null) => {
      if (!accountId) return null;
      const account = accountById.get(accountId);
      if (!account) return null;
      return getAntigravityTierBadge(account.quota);
    },
    [accountById],
  );

  const fetchHistory = async () => {
    const result = await invoke<WakeupVerificationBatchHistoryItem[]>('wakeup_verification_load_history');
    setHistoryBatches((result || []).sort((a, b) => b.verifiedAt - a.verifiedAt));
  };

  const fetchModels = async (accountList: typeof accounts = accounts) => {
    const allowedModelKeys = collectAntigravityQuotaModelKeys(accountList);
    try {
      const models = await invoke<AvailableModel[]>('fetch_available_models');
      const filtered = filterAntigravityModelOptions(models || [], {
        allowedModelKeys,
        includeNonRecommended: false,
      });
      if (filtered.length > 0) {
        setAvailableModels(filtered);
      } else {
        setNotice({ text: t('wakeup.notice.modelsFetchFailed'), tone: 'warning' });
        setAvailableModels([]);
      }
    } catch (error) {
      console.error('获取模型列表失败:', error);
      setNotice({ text: t('wakeup.notice.modelsFetchFailed'), tone: 'warning' });
      setAvailableModels([]);
    }
  };

  const fetchAccountGroups = useCallback(async () => {
    try {
      const groups = await getAccountGroups();
      setAccountGroups(groups || []);
    } catch (error) {
      console.error('加载账号分组失败:', error);
      setAccountGroups([]);
    }
  }, []);

  useEffect(() => {
    let active = true;
    const initPage = async () => {
      try {
        await fetchAccounts();
        if (!active) return;
        const latestAccounts = useAccountStore.getState().accounts;
        await Promise.all([fetchHistory(), fetchModels(latestAccounts)]);
      } catch (error) {
        if (!active) return;
        setNotice({ text: String(error), tone: 'error' });
      }
    };
    initPage();
    return () => {
      active = false;
    };
  }, [fetchAccounts]);

  useEffect(() => {
    const syncMode = () => {
      setOfficialLsVersionMode(loadWakeupOfficialLsVersionMode());
    };
    const handleModeChanged = (event: Event) => {
      const detail = (event as CustomEvent<WakeupOfficialLsVersionMode>).detail;
      if (detail === 'lt_1_21_6' || detail === 'gte_1_21_6') {
        setOfficialLsVersionMode(detail);
        return;
      }
      syncMode();
    };

    window.addEventListener(
      WAKEUP_OFFICIAL_LS_VERSION_CHANGED_EVENT,
      handleModeChanged as EventListener,
    );
    window.addEventListener('focus', syncMode);
    return () => {
      window.removeEventListener(
        WAKEUP_OFFICIAL_LS_VERSION_CHANGED_EVENT,
        handleModeChanged as EventListener,
      );
      window.removeEventListener('focus', syncMode);
    };
  }, []);

  useEffect(() => {
    invoke('wakeup_set_official_ls_version_mode', { mode: officialLsVersionMode }).catch((error) => {
      console.error('同步官方 LS 版本模式失败:', error);
    });
  }, [officialLsVersionMode]);

  useEffect(() => {
    if (!selectedModel && preferredDefaultModelId) {
      setSelectedModel(preferredDefaultModelId);
    }
  }, [preferredDefaultModelId, selectedModel]);

  useEffect(() => {
    if (!showConfigModal) return;
    void fetchAccountGroups();
  }, [fetchAccountGroups, showConfigModal]);

  useEffect(() => {
    const validIds = new Set(historyBatches.map((item) => item.batchId));
    setSelectedBatchIds((prev) => prev.filter((id) => validIds.has(id)));
    if (detailMode === 'history' && detailBatchId && !validIds.has(detailBatchId)) {
      setShowDetailModal(false);
      setDetailBatchId(null);
    }
  }, [historyBatches, detailBatchId, detailMode]);

  useEffect(() => {
    const validAccountIds = new Set(accountIds);
    setSelectedAccounts((prev) => prev.filter((id) => validAccountIds.has(id)));
  }, [accountIds]);

  useEffect(() => {
    const validGroupIds = new Set(accountGroups.map((group) => group.id));
    setConfigGroupFilter((prev) =>
      prev.filter(
        (value) =>
          value === UNGROUPED_ACCOUNT_GROUP_FILTER_KEY || validGroupIds.has(value),
      ),
    );
  }, [accountGroups]);

  useEffect(() => {
    const syncPrivacyMode = () => {
      setPrivacyModeEnabled(isPrivacyModeEnabledByDefault());
    };

    const handlePrivacyModeChanged = (event: Event) => {
      const detail = (event as CustomEvent<boolean>).detail;
      if (typeof detail === 'boolean') {
        setPrivacyModeEnabled(detail);
      } else {
        syncPrivacyMode();
      }
    };

    window.addEventListener(PRIVACY_MODE_CHANGED_EVENT, handlePrivacyModeChanged as EventListener);
    window.addEventListener('focus', syncPrivacyMode);
    return () => {
      window.removeEventListener(PRIVACY_MODE_CHANGED_EVENT, handlePrivacyModeChanged as EventListener);
      window.removeEventListener('focus', syncPrivacyMode);
    };
  }, []);

  useEffect(() => {
    if (!accountSelectAllRef.current) return;
    accountSelectAllRef.current.indeterminate = partiallyVisibleAccountsSelected;
  }, [partiallyVisibleAccountsSelected]);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    listen<WakeupVerificationProgressPayload>('wakeup://verification-progress', (event) => {
      const payload = event.payload;
      if (!payload?.batchId) return;

      if (activeBatchIdRef.current && payload.batchId !== activeBatchIdRef.current) {
        return;
      }
      if (!activeBatchIdRef.current) {
        activeBatchIdRef.current = payload.batchId;
      }

      setProgress(payload);
      setRunning(Boolean(payload.running));
      if (payload.item) {
        setLiveStates((prev) => ({
          ...prev,
          [payload.item!.accountId]: payload.item!,
        }));
      }
      setDetailBatchId((prev) => {
        if (detailMode !== 'running') return prev;
        return payload.batchId;
      });
    }).then((fn) => {
      unlisten = fn;
    });

    return () => {
      if (unlisten) {
        unlisten();
      }
    };
  }, [detailMode]);

  const closeConfigModal = () => {
    if (running) return;
    clearConfigModalError();
    setShowConfigModal(false);
  };

  const openConfigModal = () => {
    setSelectedAccounts(accounts.map((account) => account.id));
    setAccountSearchQuery('');
    setConfigTypeFilter([]);
    setConfigTagFilter([]);
    setConfigGroupFilter([]);
    if (!selectedModel && preferredDefaultModelId) {
      setSelectedModel(preferredDefaultModelId);
    }
    clearConfigModalError();
    setShowConfigModal(true);
  };

  const closeDetailModal = () => {
    clearDetailModalError();
    setShowDetailModal(false);
  };

  const handleDetailFilterChange = (filter: DetailFilter) => {
    clearDetailModalError();
    setDetailFilter(filter);
  };

  const openDetailModal = (batch: WakeupVerificationBatchHistoryItem) => {
    clearDetailModalError();
    setDetailFilter('all');
    setDetailMode('history');
    setDetailBatchId(batch.batchId);
    setShowDetailModal(true);
  };

  const toggleAccountSelection = (accountId: string) => {
    setSelectedAccounts((prev) => {
      const next = new Set(prev);
      if (next.has(accountId)) {
        next.delete(accountId);
      } else {
        next.add(accountId);
      }
      return Array.from(next);
    });
  };

  const toggleConfigFilterTypeValue = useCallback((value: string) => {
    setConfigTypeFilter((prev) => {
      if (prev.includes(value as AccountFilterType)) {
        return prev.filter((item) => item !== value);
      }
      return [...prev, value as AccountFilterType];
    });
  }, []);

  const clearConfigTypeFilter = useCallback(() => {
    setConfigTypeFilter([]);
  }, []);

  const toggleConfigTagFilterValue = useCallback((value: string) => {
    setConfigTagFilter((prev) => {
      if (prev.includes(value)) {
        return prev.filter((item) => item !== value);
      }
      return [...prev, value];
    });
  }, []);

  const clearConfigTagFilter = useCallback(() => {
    setConfigTagFilter([]);
  }, []);

  const toggleConfigGroupFilterValue = useCallback((value: string) => {
    setConfigGroupFilter((prev) => {
      if (prev.includes(value)) {
        return prev.filter((item) => item !== value);
      }
      return [...prev, value];
    });
  }, []);

  const clearConfigGroupFilter = useCallback(() => {
    setConfigGroupFilter([]);
  }, []);

  const selectVisibleAccounts = useCallback(() => {
    setSelectedAccounts((prev) => {
      const next = new Set(prev);
      filteredConfigAccountIds.forEach((accountId) => next.add(accountId));
      return Array.from(next);
    });
  }, [filteredConfigAccountIds]);

  const clearVisibleAccounts = useCallback(() => {
    setSelectedAccounts((prev) => {
      if (prev.length === 0) return prev;
      const next = new Set(prev);
      filteredConfigAccountIds.forEach((accountId) => next.delete(accountId));
      return Array.from(next);
    });
  }, [filteredConfigAccountIds]);

  const toggleAllAccountsSelection = useCallback(() => {
    if (filteredConfigAccountIds.length === 0) return;
    if (allVisibleAccountsSelected) {
      clearVisibleAccounts();
      return;
    }
    selectVisibleAccounts();
  }, [
    allVisibleAccountsSelected,
    clearVisibleAccounts,
    filteredConfigAccountIds.length,
    selectVisibleAccounts,
  ]);

  const toggleBatchSelection = (batchId: string) => {
    setSelectedBatchIds((prev) => {
      if (prev.includes(batchId)) {
        return prev.filter((id) => id !== batchId);
      }
      return [...prev, batchId];
    });
  };

  const toggleSelectAllBatches = () => {
    if (allHistorySelected) {
      setSelectedBatchIds([]);
      return;
    }
    setSelectedBatchIds(historyBatches.map((item) => item.batchId));
  };

  const deleteHistoryBatches = async (batchIds: string[]) => {
    if (batchIds.length === 0) return;
    const confirmText = t('wakeup.verification.deleteConfirm');
    const confirmed = await confirmDialog(confirmText, {
      kind: 'warning',
      okLabel: t('common.delete'),
      cancelLabel: t('common.cancel'),
    });
    if (!confirmed) return;

    try {
      const deleted = await invoke<number>('wakeup_verification_delete_history', { batchIds });
      if (deleted > 0) {
        const deletedSet = new Set(batchIds);
        setHistoryBatches((prev) => prev.filter((item) => !deletedSet.has(item.batchId)));
        setSelectedBatchIds((prev) => prev.filter((id) => !deletedSet.has(id)));
        setNotice({ text: t('messages.actionSuccess', { action: t('common.delete') }), tone: 'success' });
      }
    } catch (error) {
      console.error('删除验证记录失败:', error);
      setNotice({
        text: t('messages.actionFailed', { action: t('common.delete'), error: String(error) }),
        tone: 'error',
      });
    }
  };

  const getStatusText = (status?: string | null) => {
    const normalized = normalizeStatus(status);
    switch (normalized) {
      case STATUS_RUNNING:
        return t('wakeup.statusRunning');
      case STATUS_SUCCESS:
        return t('common.success');
      case STATUS_VERIFICATION_REQUIRED:
        return t('wakeup.errorUi.verificationRequiredTitle');
      case STATUS_TOS_VIOLATION:
        return t('wakeup.errorUi.tosViolationTitle');
      case STATUS_AUTH_EXPIRED:
        return t('accounts.status.authInvalid');
      case STATUS_FAILED:
        return t('common.failed');
      default:
        return t('common.none');
    }
  };

  const getStatusClass = (status?: string | null) => {
    const normalized = normalizeStatus(status);
    switch (normalized) {
      case STATUS_RUNNING:
        return 'is-running';
      case STATUS_SUCCESS:
        return 'is-success';
      case STATUS_VERIFICATION_REQUIRED:
        return 'is-warning';
      case STATUS_TOS_VIOLATION:
        return 'is-tos-violation';
      case STATUS_AUTH_EXPIRED:
      case STATUS_FAILED:
        return 'is-failed';
      default:
        return 'is-idle';
    }
  };

  const formatTime = (timestamp?: number | null) => {
    if (!timestamp) return t('wakeup.format.none');
    const date = new Date(timestamp);
    if (Number.isNaN(date.getTime())) return t('wakeup.format.none');
    return date.toLocaleString(locale);
  };

  const copyText = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setNotice({ text: t('wakeup.errorUi.copySuccess'), tone: 'success' });
    } catch (error) {
      console.error('复制失败:', error);
      setNotice({ text: t('wakeup.errorUi.copyFailed'), tone: 'error' });
    }
  };

  const openValidationUrl = async (url: string) => {
    try {
      await openUrl(url);
    } catch (error) {
      console.error('打开验证地址失败:', error);
      setNotice({ text: t('wakeup.errorUi.openFailed'), tone: 'error' });
      window.open(url, '_blank', 'noopener,noreferrer');
    }
  };

  const ensureWakeupRuntimeReady = useCallback(async (options?: {
    reportError?: (message: string) => void;
  }): Promise<boolean> => {
    const reportError = options?.reportError ?? ((message: string) => {
      setNotice({ text: message, tone: 'error' });
    });
    try {
      await invoke('wakeup_ensure_runtime_ready', { officialLsVersionMode });
      return true;
    } catch (error) {
      const message = formatErrorMessage(error);
      if (isAntigravityPathMissingError(message)) {
        window.dispatchEvent(
          new CustomEvent('app-path-missing', {
            detail: { app: 'antigravity', retry: { kind: 'default' } },
          }),
        );
        const pathErrorText = t('appPath.modal.desc', { app: 'Antigravity' });
        reportError(pathErrorText);
        return false;
      }
      reportError(message);
      return false;
    }
  }, [officialLsVersionMode, t]);

  const runBatchVerification = async (options: {
    accountIds: string[];
    model: string;
    prompt: string;
    source: 'config' | 'detail';
    reportError: (message: string) => void;
    fallbackDetailBatchId?: string | null;
  }) => {
    const { accountIds: targetAccountIds, model, prompt, source, reportError, fallbackDetailBatchId } = options;
    const now = Date.now();
    const pendingStates: Record<string, WakeupVerificationStateItem> = {};
    targetAccountIds.forEach((accountId) => {
      const account = accountById.get(accountId);
      if (!account) return;
      pendingStates[accountId] = {
        accountId,
        accountEmail: account.email,
        status: STATUS_RUNNING,
        lastVerifyAt: now,
        lastModel: model,
        lastErrorCode: null,
        lastMessage: null,
        validationUrl: null,
        trajectoryId: null,
        durationMs: null,
      };
    });

    const localBatchId = `pending_${now}`;
    setLiveStates(pendingStates);
    setBatchAccountIds([...targetAccountIds]);
    setProgress({
      batchId: localBatchId,
      total: targetAccountIds.length,
      completed: 0,
      successCount: 0,
      verificationRequiredCount: 0,
      tosViolationCount: 0,
      failedCount: 0,
      running: true,
      item: null,
    });
    setRunningStartedAt(now);
    setRunningModel(model);
    setRunningPrompt(prompt);
    if (source === 'config') {
      setShowConfigModal(false);
    }
    setShowDetailModal(true);
    setDetailMode('running');
    setDetailBatchId(localBatchId);
    setDetailFilter('all');
    setRunning(true);
    activeBatchIdRef.current = null;

    try {
      const result = await invoke<WakeupVerificationBatchResult>('wakeup_verification_run_batch', {
        accountIds: targetAccountIds,
        model,
        prompt,
        maxOutputTokens: 0,
        officialLsVersionMode,
      });

      const finalBatch: WakeupVerificationBatchHistoryItem = {
        batchId: result.batchId,
        verifiedAt: result.verifiedAt,
        model: result.model,
        prompt: result.prompt,
        total: result.total,
        completed: result.completed,
        successCount: result.successCount,
        verificationRequiredCount: result.verificationRequiredCount,
        tosViolationCount: result.tosViolationCount,
        failedCount: result.failedCount,
        records: result.records || [],
      };

      activeBatchIdRef.current = result.batchId;
      setProgress({
        batchId: result.batchId,
        total: result.total,
        completed: result.completed,
        successCount: result.successCount,
        verificationRequiredCount: result.verificationRequiredCount,
        tosViolationCount: result.tosViolationCount,
        failedCount: result.failedCount,
        running: false,
        item: null,
      });
      setRunning(false);
      setHistoryBatches((prev) => mergeHistoryBatches(prev, [finalBatch]));
      setDetailMode('history');
      setDetailBatchId(result.batchId);
      setShowDetailModal(true);

      const tone = result.failedCount > 0 ? 'warning' : 'success';
      setNotice({
        text:
          result.failedCount > 0
            ? t('wakeup.notice.testFailed', { count: result.failedCount })
            : t('wakeup.notice.testCompleted'),
        tone,
      });

      await fetchHistory();
    } catch (error) {
      console.error('批量验证失败:', error);
      activeBatchIdRef.current = null;
      setRunning(false);
      setProgress(null);
      setLiveStates({});
      setBatchAccountIds([]);
      setRunningStartedAt(null);
      if (source === 'config') {
        setShowDetailModal(false);
        setDetailBatchId(null);
        setShowConfigModal(true);
      } else {
        setShowDetailModal(true);
        setDetailMode('history');
        setDetailBatchId(fallbackDetailBatchId || null);
      }
      reportError(formatErrorMessage(error));
    }
  };

  const startBatchVerification = async () => {
    if (running) return;
    clearConfigModalError();
    if (!selectedModel) {
      reportConfigModalError(t('wakeup.notice.testMissingModel'));
      return;
    }
    if (selectedAccounts.length === 0) {
      reportConfigModalError(t('wakeup.notice.testMissingAccount'));
      return;
    }
    const runtimeReady = await ensureWakeupRuntimeReady({ reportError: reportConfigModalError });
    if (!runtimeReady) {
      return;
    }
    await runBatchVerification({
      accountIds: selectedAccounts,
      model: selectedModel,
      prompt: DEFAULT_PROMPT,
      source: 'config',
      reportError: reportConfigModalError,
    });
  };

  const retryFilteredVerification = async () => {
    if (running || detailMode !== 'history' || !activeDetail) return;
    clearDetailModalError();

    if (!activeDetail.model) {
      reportDetailModalError(t('wakeup.verification.actions.retryMissingModel'));
      return;
    }

    const retryAccountIds = Array.from(
      new Set(
        filteredDetailRows
          .map((item) => item.accountId)
          .filter((accountId) => accountById.has(accountId)),
      ),
    );
    if (retryAccountIds.length === 0) {
      reportDetailModalError(t('wakeup.verification.actions.retryMissingAccount'));
      return;
    }

    const runtimeReady = await ensureWakeupRuntimeReady({ reportError: reportDetailModalError });
    if (!runtimeReady) {
      return;
    }

    await runBatchVerification({
      accountIds: retryAccountIds,
      model: activeDetail.model,
      prompt: activeDetail.prompt || DEFAULT_PROMPT,
      source: 'detail',
      reportError: reportDetailModalError,
      fallbackDetailBatchId: activeDetail.batchId,
    });
  };

  const renderValidationActions = (item: WakeupVerificationStateItem) => {
    if (!item.validationUrl) return null;
    return (
      <div className="verification-inline-actions">
        <button
          type="button"
          className="btn btn-secondary btn-sm"
          onClick={() => openValidationUrl(item.validationUrl!)}
        >
          {t('wakeup.errorUi.completeVerification')}
        </button>
        <button
          type="button"
          className="btn btn-secondary btn-sm"
          onClick={() => copyText(item.validationUrl!)}
        >
          {t('wakeup.errorUi.copyValidationUrl')}
        </button>
      </div>
    );
  };

  const renderDetailMessage = (item: WakeupVerificationStateItem) => {
    const normalizedStatus = normalizeStatus(item.status);
    if (normalizedStatus === STATUS_SUCCESS || normalizedStatus === STATUS_RUNNING) {
      return null;
    }
    const message = item.lastMessage?.replace(/\s+/g, ' ').trim();
    if (!message) return null;
    const maxLength = 260;
    const display = message.length > maxLength ? `${message.slice(0, maxLength)}...` : message;
    return (
      <div
        className={`verification-progress-message ${isFailedStatus(normalizedStatus) ? 'is-failed' : ''}`}
        title={message}
      >
        {display}
      </div>
    );
  };

  const resolveModelLabel = (modelId?: string | null) => {
    if (!modelId) return t('wakeup.format.none');
    return modelNameById.get(modelId) || getAntigravityModelDisplayName(modelId) || modelId;
  };

  const activeDetailBatchLabel = useMemo(() => {
    if (!activeDetail?.batchId) return '--';
    return activeDetail.batchId;
  }, [activeDetail]);

  return (
    <main className="main-content wakeup-page wakeup-verification-page">
      <OverviewTabsHeader
        active="verification"
        onNavigate={onNavigate}
        subtitle={t('wakeup.subtitle')}
      />

      <div className="toolbar">
        <div className="toolbar-left">
          <span className="wakeup-hint">{t('wakeup.historyCount', { count: historyBatches.length })}</span>
        </div>
        <div className="toolbar-right verification-toolbar-actions">
          <button
            className="btn btn-secondary"
            onClick={() => deleteHistoryBatches(selectedBatchIds)}
            disabled={running || selectedBatchIds.length === 0}
          >
            {t('common.delete')} ({selectedBatchIds.length})
          </button>
          <button
            className="btn btn-primary"
            onClick={openConfigModal}
            disabled={running || accounts.length === 0}
          >
            <ShieldCheck size={16} />
            {t('wakeup.verification.actions.runCheckNow', '立即检测')}
          </button>
        </div>
      </div>

      {notice ? (
        <div className={`verification-notice is-${notice.tone}`}>
          <span className="verification-notice-text">{notice.text}</span>
          <button
            type="button"
            className="verification-notice-close"
            onClick={() => setNotice(null)}
            aria-label={t('common.close')}
          >
            <X size={14} />
          </button>
        </div>
      ) : null}

      <div className="account-table-container">
        <table className="account-table verification-table">
          <thead>
            <tr>
              <th className="verification-check-col">
                <input
                  type="checkbox"
                  checked={allHistorySelected}
                  onChange={toggleSelectAllBatches}
                  aria-label={t('common.shared.filter.all', { count: historyBatches.length })}
                />
              </th>
              <th>{t('wakeup.verification.columns.verifiedAt')}</th>
              <th>{t('wakeup.form.modelSelect')}</th>
              <th>{t('wakeup.verification.filters.all')}</th>
              <th>{t('common.success')}</th>
              <th>{t('wakeup.errorUi.verificationRequiredTitle')}</th>
              <th>{t('wakeup.errorUi.tosViolationTitle')}</th>
              <th>{t('common.failed')}</th>
              <th>{t('accounts.columns.actions')}</th>
            </tr>
          </thead>
          <tbody>
            {historyBatches.length === 0 ? (
              <tr>
                <td colSpan={9} className="verification-empty-cell">
                  {t('wakeup.historyEmpty')}
                </td>
              </tr>
            ) : (
              historyBatches.map((batch) => (
                <tr key={batch.batchId}>
                  <td className="verification-check-col">
                    <input
                      type="checkbox"
                      checked={selectedBatchIds.includes(batch.batchId)}
                      onChange={() => toggleBatchSelection(batch.batchId)}
                      aria-label={batch.batchId}
                    />
                  </td>
                  <td>{formatTime(batch.verifiedAt)}</td>
                  <td>{resolveModelLabel(batch.model)}</td>
                  <td>
                    <span className="verification-count-badge is-all">{batch.total}</span>
                  </td>
                  <td>
                    <span className="verification-count-badge is-success">{batch.successCount}</span>
                  </td>
                  <td>
                    <span className="verification-count-badge is-warning">{batch.verificationRequiredCount}</span>
                  </td>
                  <td>
                    <span className="verification-count-badge is-tos-violation">{batch.tosViolationCount ?? 0}</span>
                  </td>
                  <td>
                    <span className="verification-count-badge is-failed">{batch.failedCount}</span>
                  </td>
                  <td>
                    <div className="verification-inline-actions">
                      <button
                        type="button"
                        className="btn btn-secondary btn-sm"
                        onClick={() => openDetailModal(batch)}
                      >
                        {t('accounts.actions.viewDetails')}
                      </button>
                      <button
                        type="button"
                        className="btn btn-secondary btn-sm"
                        onClick={() => deleteHistoryBatches([batch.batchId])}
                      >
                        {t('common.delete')}
                      </button>
                    </div>
                  </td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>

      {showConfigModal && (
        <div className="modal-overlay" onClick={closeConfigModal}>
          <div className="modal wakeup-modal verification-config-modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('wakeup.verification.actions.runCheckNow', '立即检测')}</h2>
              <button className="modal-close" onClick={closeConfigModal} disabled={running}>
                <X />
              </button>
            </div>
            <div className="modal-body verification-modal-body">
              <ModalErrorMessage message={configModalError} scrollKey={configModalErrorScrollKey} />
              <div className="wakeup-form-group">
                <label>Antigravity Version</label>
                <div className="verification-select-wrap">
                  <select
                    className="wakeup-input verification-select"
                    value={officialLsVersionMode}
                    onChange={(event) => handleOfficialLsVersionModeChange(event.target.value)}
                  >
                    <option value="gte_1_21_6">&gt;=1.21.6</option>
                    <option value="lt_1_21_6">&lt;1.21.6</option>
                  </select>
                </div>
              </div>
              <div className="wakeup-form-group">
                <label>{t('wakeup.form.modelSelect')}</label>
                <div className="verification-select-wrap">
                  <select
                    className="wakeup-input verification-select"
                    value={selectedModel}
                    onChange={(event) => setSelectedModel(event.target.value)}
                  >
                    {filteredModels.length === 0 ? (
                      <option value="">{t('wakeup.form.modelsEmpty')}</option>
                    ) : (
                      filteredModels.map((model) => (
                        <option key={model.id} value={model.id}>
                          {model.displayName || getAntigravityModelDisplayName(model.id) || model.id}
                        </option>
                      ))
                    )}
                  </select>
                </div>
              </div>
              <div className="verification-confirm-hint verification-run-hint">
                <h3>{t('wakeup.verification.runHint.title')}</h3>
                <ul>
                  <li>{t('wakeup.verification.runHint.line1')}</li>
                  <li>{t('wakeup.verification.runHint.line2')}</li>
                  <li>{t('wakeup.verification.runHint.line3')}</li>
                  <li>{t('wakeup.verification.runHint.line4')}</li>
                </ul>
              </div>
              <div className="wakeup-form-group">
                <label>{t('wakeup.test.accountsLabel')}</label>
                <div className="verification-account-select-all">
                  <label className="verification-checkbox-row verification-checkbox-row-head">
                    <input
                      ref={accountSelectAllRef}
                      type="checkbox"
                      className="verification-checkbox-input"
                      checked={allVisibleAccountsSelected}
                      disabled={running || filteredConfigAccountIds.length === 0}
                      onChange={toggleAllAccountsSelection}
                    />
                    <span className="verification-checkbox-ui" aria-hidden="true" />
                    <span className="verification-checkbox-label">{t('wakeup.verification.actions.selectAllAccounts')}</span>
                  </label>
                  <div className="verification-account-search-wrap">
                    <input
                      type="text"
                      className="verification-account-search"
                      placeholder={t('accounts.search.placeholder', '搜索账号...')}
                      value={accountSearchQuery}
                      onChange={(e) => setAccountSearchQuery(e.target.value)}
                    />
                  </div>
                  <div className="verification-account-filters">
                    <MultiSelectFilterDropdown
                      options={typeFilterOptions}
                      selectedValues={configTypeFilter}
                      allLabel={t('wakeup.verification.filters.typeShort', '类型')}
                      filterLabel={t('wakeup.verification.filters.typeShort', '类型')}
                      clearLabel={t('accounts.clearFilter', '清空筛选')}
                      emptyLabel={t('common.none', '暂无')}
                      ariaLabel={t('wakeup.verification.filters.typeShort', '类型')}
                      onToggleValue={toggleConfigFilterTypeValue}
                      onClear={clearConfigTypeFilter}
                    />
                    <MultiSelectFilterDropdown
                      options={availableFilterTagOptions}
                      selectedValues={configTagFilter}
                      allLabel={t('wakeup.verification.filters.tagsShort', '标签')}
                      filterLabel={t('wakeup.verification.filters.tagsShort', '标签')}
                      clearLabel={t('accounts.clearFilter', '清空筛选')}
                      emptyLabel={t('accounts.noAvailableTags', '暂无可用标签')}
                      ariaLabel={t('wakeup.verification.filters.tagsShort', '标签')}
                      onToggleValue={toggleConfigTagFilterValue}
                      onClear={clearConfigTagFilter}
                    />
                    <MultiSelectFilterDropdown
                      options={groupFilterOptions}
                      selectedValues={configGroupFilter}
                      allLabel={t('wakeup.verification.filters.groupsShort', '分组')}
                      filterLabel={t('wakeup.verification.filters.groupsShort', '分组')}
                      clearLabel={t('accounts.clearFilter', '清空筛选')}
                      emptyLabel={t('accounts.groups.noGroups', '暂无分组')}
                      ariaLabel={t('wakeup.verification.filters.groupsShort', '分组')}
                      onToggleValue={toggleConfigGroupFilterValue}
                      onClear={clearConfigGroupFilter}
                    />
                  </div>
                  <span className="verification-account-select-count">
                    {selectedVisibleAccountsCount}/{filteredConfigAccountIds.length}
                    {filteredConfigAccountIds.length !== accountIds.length
                      ? ` · ${selectedAccounts.length}/${accountIds.length}`
                      : ''}
                  </span>
                </div>
                <div className="verification-account-list">
                  {filteredConfigAccounts.map((account) => {
                    const maskedEmail = maskAccountText(account.email);
                    const isSelected = selectedAccountSet.has(account.id);
                    return (
                      <label
                        key={account.id}
                        className={`verification-account-item ${isSelected ? 'selected' : ''}`}
                      >
                        <input
                          type="checkbox"
                          className="verification-checkbox-input"
                          checked={isSelected}
                          onChange={() => toggleAccountSelection(account.id)}
                        />
                        <span className="verification-checkbox-ui" aria-hidden="true" />
                        <span className="verification-account-item-email" title={maskedEmail}>
                          {maskedEmail}
                        </span>
                      </label>
                    );
                  })}
                </div>
              </div>
            </div>
            <div className="modal-footer">
              <button className="btn btn-secondary" onClick={closeConfigModal} disabled={running}>
                {t('common.cancel')}
              </button>
              <button
                className="btn btn-primary"
                onClick={startBatchVerification}
                disabled={running || filteredModels.length === 0}
              >
                {running ? t('wakeup.statusRunning') : t('wakeup.errorUi.completeVerification')}
              </button>
            </div>
          </div>
        </div>
      )}

      {showDetailModal && (
        <div className="modal-overlay" onClick={closeDetailModal}>
          <div className="modal wakeup-modal verification-progress-modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('accounts.actions.viewDetails')}</h2>
              <button className="modal-close" onClick={closeDetailModal}>
                <X />
              </button>
            </div>
            <div className="modal-body verification-modal-body">
              <ModalErrorMessage message={detailModalError} scrollKey={detailModalErrorScrollKey} />
              <div className="verification-detail-meta">
                <span>
                  {t('accounts.columns.lastUsed')}: {formatTime(activeDetail?.verifiedAt)}
                </span>
                <span>{t('wakeup.form.modelSelect')}: {resolveModelLabel(activeDetail?.model)}</span>
                <span>Batch: {activeDetailBatchLabel}</span>
              </div>

              <div className="verification-progress-metrics">
                <button
                  type="button"
                  className={`pill pill-secondary verification-filter-pill ${detailFilter === 'all' ? 'active' : ''}`}
                  onClick={() => handleDetailFilterChange('all')}
                >
                  {t('common.shared.filter.all', { count: detailCounts.all })}
                </button>
                <button
                  type="button"
                  className={`pill pill-success verification-filter-pill ${detailFilter === 'success' ? 'active' : ''}`}
                  onClick={() => handleDetailFilterChange('success')}
                >
                  {t('common.success')} {detailCounts.success}
                </button>
                <button
                  type="button"
                  className={`pill pill-emphasis verification-filter-pill ${detailFilter === 'verification_required' ? 'active' : ''}`}
                  onClick={() => handleDetailFilterChange('verification_required')}
                >
                  {t('wakeup.errorUi.verificationRequiredTitle')} {detailCounts.verificationRequired}
                </button>
                <button
                  type="button"
                  className={`pill pill-warning verification-filter-pill ${detailFilter === 'tos_violation' ? 'active' : ''}`}
                  onClick={() => handleDetailFilterChange('tos_violation')}
                >
                  {t('wakeup.errorUi.tosViolationTitle')} {detailCounts.tosViolation}
                </button>
                <button
                  type="button"
                  className={`pill pill-danger verification-filter-pill ${detailFilter === 'failed' ? 'active' : ''}`}
                  onClick={() => handleDetailFilterChange('failed')}
                >
                  {t('common.failed')} {detailCounts.failed}
                </button>
              </div>

              <ul className="verification-progress-list">
                {filteredDetailRows.length === 0 ? (
                  <li className="verification-progress-empty">{t('wakeup.historyEmpty')}</li>
                ) : (
                  filteredDetailRows.map((item) => {
                    const planBadge = resolveAccountPlanBadge(item.accountId);
                    return (
                      <li key={`${activeDetailBatchLabel}-${item.accountId}`} className="verification-progress-item">
                        <div className="verification-progress-main">
                          <span className="verification-account-email">{maskAccountText(item.accountEmail)}</span>
                          {planBadge ? (
                            <span className={`tier-badge ${planBadge.className}`}>
                              {planBadge.label}
                            </span>
                          ) : null}
                          <span className={`verification-status-pill ${getStatusClass(item.status)}`}>
                            {getStatusText(item.status)}
                          </span>
                          {item.durationMs ? <span className="verification-duration">{item.durationMs}ms</span> : null}
                        </div>
                        <div className="verification-progress-sub">
                          {resolveModelLabel(item.lastModel)}
                          {typeof item.lastErrorCode === 'number'
                            ? ` · ${t('wakeup.errorUi.errorCode', { code: item.lastErrorCode })}`
                            : ''}
                          {item.trajectoryId
                            ? ` · ${t('wakeup.errorUi.trajectoryId', { id: item.trajectoryId })}`
                            : ''}
                        </div>
                        {renderDetailMessage(item)}
                        {item.validationUrl ? renderValidationActions(item) : null}
                        {item.appealUrl ? (
                          <div className="verification-inline-actions">
                            <button
                              type="button"
                              className="btn btn-secondary btn-sm"
                              onClick={() => openValidationUrl(item.appealUrl!)}
                            >
                              {t('wakeup.errorUi.submitAppeal')}
                            </button>
                            <button
                              type="button"
                              className="btn btn-secondary btn-sm"
                              onClick={() => copyText(item.appealUrl!)}
                            >
                              {t('wakeup.errorUi.copyAppealUrl')}
                            </button>
                          </div>
                        ) : null}
                      </li>
                    );
                  })
                )}
              </ul>
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-primary"
                onClick={retryFilteredVerification}
                disabled={running || detailMode !== 'history' || !activeDetail || filteredDetailRows.length === 0}
              >
                {t('wakeup.verification.actions.retryFiltered', { count: filteredDetailRows.length })}
              </button>
              <button className="btn btn-secondary" onClick={closeDetailModal}>
                {t('common.close')}
              </button>
            </div>
          </div>
        </div>
      )}
    </main>
  );
}
