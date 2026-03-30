import { useState, useMemo, useCallback, Fragment } from 'react';
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
import {
  getWindsurfCreditsSummary,
  getWindsurfOfficialUsageMode,
  getWindsurfPlanDisplayName,
  getWindsurfPlanLabel,
  getWindsurfQuotaClass,
  getWindsurfQuotaUsageSummary,
  formatWindsurfResetTime,
} from '../types/windsurf';
import { buildWindsurfAccountPresentation } from '../presentation/platformAccountPresentation';

import { WindsurfOverviewTabsHeader, WindsurfTab } from '../components/WindsurfOverviewTabsHeader';
import { WindsurfInstancesContent } from './WindsurfInstancesPage';
import { QuickSettingsPopover } from '../components/QuickSettingsPopover';
import { useProviderAccountsPage } from '../hooks/useProviderAccountsPage';
import { MultiSelectFilterDropdown, type MultiSelectFilterOption } from '../components/MultiSelectFilterDropdown';
import type { WindsurfAccount, WindsurfPlanBadge } from '../types/windsurf';

const WINDSURF_FLOW_NOTICE_COLLAPSED_KEY = 'agtools.windsurf.flow_notice_collapsed';
const WINDSURF_CURRENT_ACCOUNT_ID_KEY = 'agtools.windsurf.current_account_id';
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

export function WindsurfAccountsPage() {
  const [activeTab, setActiveTab] = useState<WindsurfTab>('overview');
  const [filterTypes, setFilterTypes] = useState<string[]>([]);
  const untaggedKey = '__untagged__';

  const store = useWindsurfAccountStore();

  const page = useProviderAccountsPage<WindsurfAccount>({
    platformKey: 'Windsurf',
    oauthLogPrefix: 'WindsurfOAuth',
    flowNoticeCollapsedKey: WINDSURF_FLOW_NOTICE_COLLAPSED_KEY,
    currentAccountIdKey: WINDSURF_CURRENT_ACCOUNT_ID_KEY,
    exportFilePrefix: 'windsurf_accounts',
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

  // ─── Windsurf-specific: Password login ──────────────────────────────
  const [passwordEmail, setPasswordEmail] = useState('');
  const [passwordPassword, setPasswordPassword] = useState('');
  const [passwordLoading, setPasswordLoading] = useState(false);

  const handlePasswordLogin = useCallback(async () => {
    const email = passwordEmail.trim();
    const pwd = passwordPassword;
    if (!email || !pwd) {
      setAddStatus('error');
      setAddMessage(t('windsurf.password.empty', '请输入邮箱和密码'));
      return;
    }
    setPasswordLoading(true);
    setAddStatus('loading');
    setAddMessage(t('windsurf.password.logging', '正在登录...'));
    try {
      const account = await windsurfService.addWindsurfAccountWithPassword(email, pwd);
      await store.fetchAccounts();
      setAddStatus('success');
      setAddMessage(
        t('windsurf.password.success', {
          login: account.github_login || account.github_email || email,
          defaultValue: '登录成功: {{login}}',
        })
      );
      setTimeout(() => {
        closeAddModal();
        setPasswordEmail('');
        setPasswordPassword('');
      }, 1200);
    } catch (e) {
      setAddStatus('error');
      const errorMsg = String(e).replace(/^Error:\s*/, '');
      setAddMessage(
        t('windsurf.password.failed', { error: errorMsg, defaultValue: '登录失败: {{error}}' })
      );
    }
    setPasswordLoading(false);
  }, [passwordEmail, passwordPassword, store, t, setAddStatus, setAddMessage, closeAddModal]);

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
    const counts: Record<string, number> = { all: accounts.length };
    WINDSURF_PLAN_FILTERS.forEach((planKey) => {
      counts[planKey] = 0;
    });
    accounts.forEach((account) => {
      const tier = resolvePlanKey(account);
      if (tier in counts) counts[tier as keyof typeof counts] += 1;
    });
    return counts;
  }, [accounts, resolvePlanKey]);

  const tierFilterOptions = useMemo<MultiSelectFilterOption[]>(
    () =>
      WINDSURF_PLAN_FILTERS.map((planKey) => ({
        value: planKey,
        label: `${getWindsurfPlanLabel(planKey)} (${tierCounts[planKey] ?? 0})`,
      })),
    [tierCounts],
  );

  // ─── Filtering & Sorting ────────────────────────────────────────────
  const compareAccountsBySort = useCallback((a: WindsurfAccount, b: WindsurfAccount) => {
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
    const aValue = resolveCreditsSummary(a).creditsLeft ?? -1;
    const bValue = resolveCreditsSummary(b).creditsLeft ?? -1;
    const diff = bValue - aValue;
    return sortDirection === 'desc' ? diff : -diff;
  }, [resolveCreditsSummary, sortBy, sortDirection]);

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
      const selectedTypes = new Set(filterTypes);
      result = result.filter((account) => selectedTypes.has(resolvePlanKey(account)));
    }
    if (tagFilter.length > 0) {
      const selectedTags = new Set(tagFilter.map(normalizeTag));
      result = result.filter((acc) => (acc.tags || []).map(normalizeTag).some((tag) => selectedTags.has(tag)));
    }
    result.sort(compareAccountsBySort);
    return result;
  }, [accounts, compareAccountsBySort, filterTypes, normalizeTag, resolvePlanKey, resolvePresentation, searchQuery, tagFilter]);

  const filteredIds = useMemo(() => filteredAccounts.map((account) => account.id), [filteredAccounts]);
  const exportSelectionCount = getScopedSelectedCount(filteredIds);

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
    return Array.from(groups.entries()).sort(([aKey], [bKey]) => { if (aKey === untaggedKey) return 1; if (bKey === untaggedKey) return -1; return aKey.localeCompare(bKey); });
  }, [filteredAccounts, groupByTag, normalizeTag, tagFilter, untaggedKey]);

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

      return (
        <div key={groupKey ? `${groupKey}-${account.id}` : account.id} className={`ghcp-account-card ${isCurrent ? 'current' : ''} ${isSelected ? 'selected' : ''}`}>
          <div className="card-top">
            <div className="card-select"><input type="checkbox" checked={isSelected} onChange={() => toggleSelect(account.id)} /></div>
            <span className="account-email" title={maskAccountText(emailText)}>{maskAccountText(emailText)}</span>
            {isCurrent && <span className="current-tag">{t('accounts.status.current')}</span>}
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
                title={t('common.shared.export', '导出')}
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
      return (
        <tr key={groupKey ? `${groupKey}-${account.id}` : account.id} className={isCurrent ? 'current' : ''}>
          <td><input type="checkbox" checked={selected.has(account.id)} onChange={() => toggleSelect(account.id)} /></td>
          <td>
            <div className="account-cell">
              <div className="account-main-line">
                <span className="account-email-text" title={maskAccountText(emailText)}>{maskAccountText(emailText)}</span>
                {isCurrent && <span className="mini-tag current">{t('accounts.status.current')}</span>}
              </div>
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
                title={t('common.shared.export', '导出')}
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
              <div className="tag-filter-panel">
                {availableTags.length === 0 ? (
                  <div className="tag-filter-empty">{t('accounts.noAvailableTags', '暂无可用标签')}</div>
                ) : (
                  <div className="tag-filter-options">
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
          <div className="sort-select">
            <ArrowDownWideNarrow size={14} className="sort-icon" />
            <select value={sortBy} onChange={(e) => setSortBy(e.target.value)} aria-label={t('common.shared.sortLabel', '排序')}>
              <option value="created_at">{t('common.shared.sort.createdAt', '按创建时间')}</option>
              <option value="credits">{t('common.shared.sort.credits', '按剩余 Credits')}</option>
              <option value="plan_end">{t('common.shared.sort.planEnd', '按配额周期结束时间')}</option>
            </select>
          </div>
          <button className="sort-direction-btn" onClick={() => setSortDirection((prev) => (prev === 'desc' ? 'asc' : 'desc'))}
            title={sortDirection === 'desc' ? t('common.shared.sort.descTooltip', '当前：降序，点击切换为升序') : t('common.shared.sort.ascTooltip', '当前：升序，点击切换为降序')}
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
            title={exportSelectionCount > 0 ? `${t('common.shared.export', '导出')} (${exportSelectionCount})` : t('common.shared.export', '导出')}><Upload size={14} /></button>
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
          {filteredAccounts.length > 0 && (
            <div className="grid-view-header" style={{ marginBottom: '12px', paddingLeft: '4px' }}>
              <label style={{ display: 'inline-flex', alignItems: 'center', gap: '8px', cursor: 'pointer', fontSize: '13px', color: 'var(--text-color)' }}>
                <input type="checkbox" checked={selected.size === filteredAccounts.length && filteredAccounts.length > 0} onChange={() => toggleSelectAll(filteredAccounts.map((a) => a.id))} />
                {t('common.selectAll', '全选')}
              </label>
            </div>
          )}
          {groupByTag ? (
          <div className="tag-group-list">{groupedAccounts.map(([groupKey, groupAccounts]) => (
            <div key={groupKey} className="tag-group-section"><div className="tag-group-header"><span className="tag-group-title">{resolveGroupLabel(groupKey)}</span><span className="tag-group-count">{groupAccounts.length}</span></div>
              <div className="tag-group-grid ghcp-accounts-grid">{renderGridCards(groupAccounts, groupKey)}</div></div>
          ))}</div>
        ) : (<div className="ghcp-accounts-grid">{renderGridCards(filteredAccounts)}</div>)}
        </div>
      ) : groupByTag ? (
        <div className="account-table-container grouped"><table className="account-table"><thead><tr>
          <th style={{ width: 40 }}><input type="checkbox" checked={selected.size === filteredAccounts.length && filteredAccounts.length > 0} onChange={() => toggleSelectAll(filteredAccounts.map((a) => a.id))} /></th>
          <th style={{ width: 240 }}>{t('common.shared.columns.email', '邮箱')}</th><th style={{ width: 120 }}>{t('common.shared.columns.plan', '计划')}</th>
          <th>{t('common.shared.columns.credits', 'Credits')}</th><th>{t('common.detail', '详情')}</th>
          <th className="sticky-action-header table-action-header">{t('common.shared.columns.actions', '操作')}</th></tr></thead>
          <tbody>{groupedAccounts.map(([groupKey, groupAccounts]) => (
            <Fragment key={groupKey}><tr className="tag-group-row"><td colSpan={6}><div className="tag-group-header"><span className="tag-group-title">{resolveGroupLabel(groupKey)}</span><span className="tag-group-count">{groupAccounts.length}</span></div></td></tr>
              {renderTableRows(groupAccounts, groupKey)}</Fragment>
          ))}</tbody></table></div>
      ) : (
        <div className="account-table-container"><table className="account-table"><thead><tr>
          <th style={{ width: 40 }}><input type="checkbox" checked={selected.size === filteredAccounts.length && filteredAccounts.length > 0} onChange={() => toggleSelectAll(filteredAccounts.map((a) => a.id))} /></th>
          <th style={{ width: 240 }}>{t('common.shared.columns.email', '邮箱')}</th><th style={{ width: 120 }}>{t('common.shared.columns.plan', '计划')}</th>
          <th>{t('common.shared.columns.credits', 'Credits')}</th><th>{t('common.detail', '详情')}</th>
          <th className="sticky-action-header table-action-header">{t('common.shared.columns.actions', '操作')}</th></tr></thead>
          <tbody>{renderTableRows(filteredAccounts)}</tbody></table></div>
      )}

      {showAddModal && (
        <div className="modal-overlay" onClick={closeAddModal}><div className="modal-content ghcp-add-modal windsurf-add-modal" onClick={(e) => e.stopPropagation()}>
          <div className="modal-header"><h2>{t('windsurf.addModal.title', '添加 Windsurf 账号')}</h2><button className="modal-close" onClick={closeAddModal} aria-label={t('common.close', '关闭')}><X /></button></div>
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
                <p className="section-desc">{t('windsurf.password.desc', '使用 Windsurf 账号的邮箱和密码登录，自动获取 API Key 和账号信息。')}</p>
                <input type="email" className="token-input" style={{ minHeight: 'auto', height: 40, resize: 'none', fontFamily: 'inherit' }}
                  value={passwordEmail} onChange={(e) => setPasswordEmail(e.target.value)} placeholder={t('windsurf.password.emailPlaceholder', '邮箱地址')} disabled={passwordLoading} />
                <input type="password" className="token-input" style={{ minHeight: 'auto', height: 40, resize: 'none', fontFamily: 'inherit', marginTop: 8 }}
                  value={passwordPassword} onChange={(e) => setPasswordPassword(e.target.value)} placeholder={t('windsurf.password.passwordPlaceholder', '密码')} disabled={passwordLoading}
                  onKeyDown={(e) => { if (e.key === 'Enter') handlePasswordLogin(); }} />
                <button className="btn btn-primary btn-full" style={{ marginTop: 12 }} onClick={handlePasswordLogin}
                  disabled={passwordLoading || !passwordEmail.trim() || passwordPassword.length === 0}>
                  {passwordLoading ? <RefreshCw size={16} className="loading-spinner" /> : <Mail size={16} />}
                  {passwordLoading ? t('windsurf.password.logging', '正在登录...') : t('windsurf.password.login', '登录')}
                </button>
              </div>
            )}
            {addStatus !== 'idle' && addStatus !== 'loading' && (
              <div className={`add-status ${addStatus}`}>{addStatus === 'success' ? <Check size={16} /> : <CircleAlert size={16} />}<span>{addMessage}</span></div>
            )}
          </div>
        </div></div>
      )}

      <ExportJsonModal
        isOpen={showExportModal}
        title={`${t('common.shared.export', '导出')} JSON`}
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
