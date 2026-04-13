import { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { createPortal } from 'react-dom';
import { open } from '@tauri-apps/plugin-dialog';
import { openPath } from '@tauri-apps/plugin-opener';
import { invoke } from '@tauri-apps/api/core';
import { Settings, RefreshCw, FolderOpen, Zap, X } from 'lucide-react';
import * as accountService from '../services/accountService';
import * as codexService from '../services/codexService';
import { getAccountGroups, type AccountGroup } from '../services/accountGroupService';
import {
  getCodexAccountGroups,
  type CodexAccountGroup,
} from '../services/codexAccountGroupService';
import {
  AutoSwitchAccountScopeSelector,
  type AutoSwitchAccountScopeMode,
  type AutoSwitchScopeAccount,
} from './AutoSwitchAccountScopeSelector';
import {
  buildAccountTierCounts,
  buildAccountTierFilterOptions,
} from '../utils/accountFilters';
import { getSubscriptionTier } from '../utils/account';
import {
  isCodexCodeReviewQuotaVisibleByDefault,
  persistCodexCodeReviewQuotaVisible,
} from '../utils/codexPreferences';
import {
  FEATURE_UNLOCK_CHANGED_EVENT,
  type FeatureUnlockChangedDetail,
  isAntigravitySeamlessSwitchFeatureUnlocked,
} from '../utils/featureUnlocks';
import {
  buildDefaultCurrentAccountRefreshMinutesMap,
  type CurrentAccountRefreshMinutesMap,
  type CurrentAccountRefreshPlatform,
  loadCurrentAccountRefreshMinutesMap,
  saveCurrentAccountRefreshMinutesMap,
} from '../utils/currentAccountRefresh';
import type { Account } from '../types/account';
import type { CodexAccount } from '../types/codex';
import { getDisplayGroups, type DisplayGroup } from '../services/groupService';
import './QuickSettingsPopover.css';

/** GeneralConfig from backend */
interface GeneralConfig {
  language: string;
  theme: string;
  ui_scale: number;
  auto_refresh_minutes: number;
  codex_auto_refresh_minutes: number;
  ghcp_auto_refresh_minutes: number;
  windsurf_auto_refresh_minutes: number;
  kiro_auto_refresh_minutes: number;
  cursor_auto_refresh_minutes: number;
  gemini_auto_refresh_minutes: number;
  codebuddy_auto_refresh_minutes: number;
  codebuddy_cn_auto_refresh_minutes: number;
  qoder_auto_refresh_minutes: number;
  trae_auto_refresh_minutes: number;
  workbuddy_auto_refresh_minutes: number;
  zed_auto_refresh_minutes: number;
  close_behavior: string;
  minimize_behavior?: 'dock_and_tray' | 'tray_only';
  hide_dock_icon?: boolean;
  opencode_app_path: string;
  antigravity_app_path: string;
  codex_app_path: string;
  vscode_app_path: string;
  windsurf_app_path: string;
  kiro_app_path: string;
  cursor_app_path: string;
  codebuddy_app_path: string;
  codebuddy_cn_app_path: string;
  qoder_app_path: string;
  trae_app_path: string;
  workbuddy_app_path: string;
  zed_app_path: string;
  opencode_sync_on_switch: boolean;
  opencode_auth_overwrite_on_switch: boolean;
  ghcp_opencode_sync_on_switch: boolean;
  ghcp_opencode_auth_overwrite_on_switch: boolean;
  ghcp_launch_on_switch: boolean;
  openclaw_auth_overwrite_on_switch: boolean;
  codex_launch_on_switch: boolean;
  antigravity_dual_switch_no_restart_enabled: boolean;
  auto_switch_enabled: boolean;
  auto_switch_threshold: number;
  auto_switch_scope_mode: string;
  auto_switch_selected_group_ids: string[];
  auto_switch_account_scope_mode?: string;
  auto_switch_selected_account_ids?: string[];
  codex_auto_switch_enabled: boolean;
  codex_auto_switch_primary_threshold: number;
  codex_auto_switch_secondary_threshold: number;
  codex_auto_switch_account_scope_mode?: string;
  codex_auto_switch_selected_account_ids?: string[];
  quota_alert_enabled: boolean;
  quota_alert_threshold: number;
  codex_quota_alert_enabled: boolean;
  codex_quota_alert_threshold: number;
  codex_quota_alert_primary_threshold: number;
  codex_quota_alert_secondary_threshold: number;
  ghcp_quota_alert_enabled: boolean;
  ghcp_quota_alert_threshold: number;
  windsurf_quota_alert_enabled: boolean;
  windsurf_quota_alert_threshold: number;
  kiro_quota_alert_enabled: boolean;
  kiro_quota_alert_threshold: number;
  cursor_quota_alert_enabled: boolean;
  cursor_quota_alert_threshold: number;
  gemini_quota_alert_enabled: boolean;
  gemini_quota_alert_threshold: number;
  codebuddy_quota_alert_enabled: boolean;
  codebuddy_quota_alert_threshold: number;
  codebuddy_cn_quota_alert_enabled: boolean;
  codebuddy_cn_quota_alert_threshold: number;
  qoder_quota_alert_enabled: boolean;
  qoder_quota_alert_threshold: number;
  trae_quota_alert_enabled: boolean;
  trae_quota_alert_threshold: number;
  workbuddy_quota_alert_enabled: boolean;
  workbuddy_quota_alert_threshold: number;
  zed_quota_alert_enabled: boolean;
  zed_quota_alert_threshold: number;
}

export type QuickSettingsType =
  | 'antigravity'
  | 'codex'
  | 'github_copilot'
  | 'windsurf'
  | 'kiro'
  | 'cursor'
  | 'gemini'
  | 'codebuddy'
  | 'codebuddy_cn'
  | 'qoder'
  | 'trae'
  | 'workbuddy'
  | 'zed';

type QuotaAlertEnabledKey =
  | 'quota_alert_enabled'
  | 'codex_quota_alert_enabled'
  | 'ghcp_quota_alert_enabled'
  | 'windsurf_quota_alert_enabled'
  | 'kiro_quota_alert_enabled'
  | 'cursor_quota_alert_enabled'
  | 'gemini_quota_alert_enabled'
  | 'codebuddy_quota_alert_enabled'
  | 'codebuddy_cn_quota_alert_enabled'
  | 'qoder_quota_alert_enabled'
  | 'trae_quota_alert_enabled'
  | 'workbuddy_quota_alert_enabled'
  | 'zed_quota_alert_enabled';
type QuotaAlertThresholdKey =
  | 'quota_alert_threshold'
  | 'codex_quota_alert_threshold'
  | 'ghcp_quota_alert_threshold'
  | 'windsurf_quota_alert_threshold'
  | 'kiro_quota_alert_threshold'
  | 'cursor_quota_alert_threshold'
  | 'gemini_quota_alert_threshold'
  | 'codebuddy_quota_alert_threshold'
  | 'codebuddy_cn_quota_alert_threshold'
  | 'qoder_quota_alert_threshold'
  | 'trae_quota_alert_threshold'
  | 'workbuddy_quota_alert_threshold'
  | 'zed_quota_alert_threshold';
type CodexWindowThresholdKey =
  | 'codex_auto_switch_primary_threshold'
  | 'codex_auto_switch_secondary_threshold'
  | 'codex_quota_alert_primary_threshold'
  | 'codex_quota_alert_secondary_threshold';

interface QuickSettingsPopoverProps {
  type: QuickSettingsType;
}

const AUTO_SWITCH_SCOPE_ALL_ACCOUNTS: AutoSwitchAccountScopeMode = 'all_accounts';
const AUTO_SWITCH_SCOPE_SELECTED_ACCOUNTS: AutoSwitchAccountScopeMode = 'selected_accounts';
const CURRENT_ACCOUNT_REFRESH_PRESETS = ['1', '2', '5', '10', '15'];

const getCurrentAccountRefreshPlatformForType = (
  platformType: QuickSettingsType,
): CurrentAccountRefreshPlatform => {
  switch (platformType) {
    case 'antigravity':
      return 'antigravity';
    case 'codex':
      return 'codex';
    case 'github_copilot':
      return 'ghcp';
    case 'windsurf':
      return 'windsurf';
    case 'kiro':
      return 'kiro';
    case 'cursor':
      return 'cursor';
    case 'gemini':
      return 'gemini';
    case 'codebuddy':
      return 'codebuddy';
    case 'codebuddy_cn':
      return 'codebuddy_cn';
    case 'qoder':
      return 'qoder';
    case 'trae':
      return 'trae';
    case 'workbuddy':
      return 'workbuddy';
    case 'zed':
      return 'zed';
  }
};

const normalizeAutoSwitchAccountScopeMode = (
  value?: string | null,
): AutoSwitchAccountScopeMode =>
  value === AUTO_SWITCH_SCOPE_SELECTED_ACCOUNTS
    ? AUTO_SWITCH_SCOPE_SELECTED_ACCOUNTS
    : AUTO_SWITCH_SCOPE_ALL_ACCOUNTS;

export function QuickSettingsPopover({ type }: QuickSettingsPopoverProps) {
  const { t } = useTranslation();
  const [isOpen, setIsOpen] = useState(false);
  const [config, setConfig] = useState<GeneralConfig | null>(null);
  const [saving, setSaving] = useState(false);
  const [pathDetecting, setPathDetecting] = useState(false);
  const [openingCodexConfig, setOpeningCodexConfig] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [refreshEditing, setRefreshEditing] = useState(false);
  const [currentAccountRefreshEditing, setCurrentAccountRefreshEditing] = useState(false);
  const [thresholdEditing, setThresholdEditing] = useState(false);
  const [quotaAlertThresholdEditing, setQuotaAlertThresholdEditing] = useState(false);
  const [customRefresh, setCustomRefresh] = useState('');
  const [currentAccountCustomRefresh, setCurrentAccountCustomRefresh] = useState('');
  const [customThreshold, setCustomThreshold] = useState('');
  const [quotaAlertCustomThreshold, setQuotaAlertCustomThreshold] = useState('');
  const [codexAutoSwitchPrimaryCustomThreshold, setCodexAutoSwitchPrimaryCustomThreshold] = useState('');
  const [codexAutoSwitchSecondaryCustomThreshold, setCodexAutoSwitchSecondaryCustomThreshold] = useState('');
  const [codexQuotaAlertPrimaryCustomThreshold, setCodexQuotaAlertPrimaryCustomThreshold] = useState('');
  const [codexQuotaAlertSecondaryCustomThreshold, setCodexQuotaAlertSecondaryCustomThreshold] = useState('');
  const [autoSwitchDisplayGroups, setAutoSwitchDisplayGroups] = useState<DisplayGroup[]>([]);
  const [antigravityAccounts, setAntigravityAccounts] = useState<Account[]>([]);
  const [antigravityAccountGroups, setAntigravityAccountGroups] = useState<AccountGroup[]>([]);
  const [codexAccounts, setCodexAccounts] = useState<CodexAccount[]>([]);
  const [codexAccountGroups, setCodexAccountGroups] = useState<CodexAccountGroup[]>([]);
  const [codexShowCodeReviewQuota, setCodexShowCodeReviewQuota] = useState(
    isCodexCodeReviewQuotaVisibleByDefault,
  );
  const [currentAccountRefreshMap, setCurrentAccountRefreshMap] =
    useState<CurrentAccountRefreshMinutesMap>(() => buildDefaultCurrentAccountRefreshMinutesMap());
  const [antigravitySeamlessSwitchUnlocked, setAntigravitySeamlessSwitchUnlocked] = useState(
    isAntigravitySeamlessSwitchFeatureUnlocked,
  );
  const modalRef = useRef<HTMLDivElement>(null);
  const refreshPresets = ['-1', '2', '5', '10', '15'];
  const thresholdPresets = ['0', '20', '40', '60'];
  const antigravityScopeTypeOptions = useMemo(
    () => buildAccountTierFilterOptions(t, buildAccountTierCounts(antigravityAccounts, {})),
    [antigravityAccounts, t],
  );
  const antigravityScopeAccounts = useMemo<AutoSwitchScopeAccount[]>(
    () =>
      antigravityAccounts.map((account) => {
        const disabledReason = account.disabled_reason || '';
        const typeValue =
          disabledReason === 'verification_required'
            ? 'VERIFICATION_REQUIRED'
            : disabledReason === 'tos_violation'
              ? 'TOS_VIOLATION'
              : getSubscriptionTier(account.quota);
        return {
          id: account.id,
          label: account.email,
          searchableText: account.email,
          tags: account.tags || [],
          type: typeValue,
        };
      }),
    [antigravityAccounts],
  );
  const antigravityScopeGroups = useMemo(
    () =>
      antigravityAccountGroups.map((group) => ({
        id: group.id,
        name: group.name,
        accountIds: group.accountIds || [],
      })),
    [antigravityAccountGroups],
  );
  const codexScopeAccounts = useMemo<AutoSwitchScopeAccount[]>(
    () =>
      codexAccounts.map((account) => ({
        id: account.id,
        label: account.email,
        searchableText: account.email,
        tags: account.tags || [],
      })),
    [codexAccounts],
  );
  const codexScopeGroups = useMemo(
    () =>
      codexAccountGroups.map((group) => ({
        id: group.id,
        name: group.name,
        accountIds: group.accountIds || [],
      })),
    [codexAccountGroups],
  );

  // Load config when modal opens
  useEffect(() => {
    if (isOpen) {
      loadConfig();
      setCodexShowCodeReviewQuota(isCodexCodeReviewQuotaVisibleByDefault());
      setAntigravitySeamlessSwitchUnlocked(isAntigravitySeamlessSwitchFeatureUnlocked());
    }
  }, [isOpen]);

  useEffect(() => {
    const handleFeatureUnlockChanged = (event: Event) => {
      const detail = (event as CustomEvent<FeatureUnlockChangedDetail>).detail;
      if (!detail || detail.feature !== 'antigravity.seamless_switch') {
        return;
      }
      setAntigravitySeamlessSwitchUnlocked(Boolean(detail.unlocked));
    };

    window.addEventListener(FEATURE_UNLOCK_CHANGED_EVENT, handleFeatureUnlockChanged as EventListener);
    return () => {
      window.removeEventListener(
        FEATURE_UNLOCK_CHANGED_EVENT,
        handleFeatureUnlockChanged as EventListener,
      );
    };
  }, []);

  // Close on Escape
  useEffect(() => {
    if (!isOpen) return;

    const handleEsc = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setIsOpen(false);
    };

    document.addEventListener('keydown', handleEsc);
    return () => {
      document.removeEventListener('keydown', handleEsc);
    };
  }, [isOpen]);

  // 外部触发：按平台类型打开设置弹框
  useEffect(() => {
    const handleExternalOpen = (event: Event) => {
      const customEvent = event as CustomEvent<{ type?: QuickSettingsType }>;
      if (customEvent.detail?.type !== type) {
        return;
      }
      setIsOpen(true);
    };

    window.addEventListener('quick-settings:open', handleExternalOpen as EventListener);
    return () => {
      window.removeEventListener('quick-settings:open', handleExternalOpen as EventListener);
    };
  }, [type]);

  const loadConfig = async () => {
    try {
      setError(null);
      const antigravityScopeDataPromise =
        type === 'antigravity'
          ? Promise.all([
              accountService.listAccounts(),
              getAccountGroups(),
            ]).catch(() => [[] as Account[], [] as AccountGroup[]] as const)
          : Promise.resolve([[] as Account[], [] as AccountGroup[]] as const);
      const codexScopeDataPromise =
        type === 'codex'
          ? Promise.all([
              codexService.listCodexAccounts(),
              getCodexAccountGroups(),
            ]).catch(() => [[] as CodexAccount[], [] as CodexAccountGroup[]] as const)
          : Promise.resolve([[] as CodexAccount[], [] as CodexAccountGroup[]] as const);

      const [cfg, groups, antigravityScopeData, codexScopeData] = await Promise.all([
        invoke<GeneralConfig>('get_general_config'),
        getDisplayGroups().catch(() => [] as DisplayGroup[]),
        antigravityScopeDataPromise,
        codexScopeDataPromise,
      ]);
      const [nextAntigravityAccounts, nextAntigravityGroups] = antigravityScopeData;
      const [nextCodexAccounts, nextCodexGroups] = codexScopeData;
      setConfig(cfg);
      setAutoSwitchDisplayGroups(groups);
      setAntigravityAccounts(nextAntigravityAccounts || []);
      setAntigravityAccountGroups(nextAntigravityGroups || []);
      setCodexAccounts(nextCodexAccounts || []);
      setCodexAccountGroups(nextCodexGroups || []);
      // 非预设值通过下拉中的动态选项展示，不默认进入输入态
      setRefreshEditing(false);
      setCurrentAccountRefreshEditing(false);
      setThresholdEditing(false);
      setQuotaAlertThresholdEditing(false);
      setCustomRefresh('');
      setCurrentAccountCustomRefresh('');
      setCustomThreshold('');
      setQuotaAlertCustomThreshold('');
      setCurrentAccountRefreshMap(loadCurrentAccountRefreshMinutesMap());
      setCodexAutoSwitchPrimaryCustomThreshold(String(cfg.codex_auto_switch_primary_threshold));
      setCodexAutoSwitchSecondaryCustomThreshold(String(cfg.codex_auto_switch_secondary_threshold));
      setCodexQuotaAlertPrimaryCustomThreshold(String(cfg.codex_quota_alert_primary_threshold));
      setCodexQuotaAlertSecondaryCustomThreshold(String(cfg.codex_quota_alert_secondary_threshold));
    } catch (err) {
      console.error('Failed to load config:', err);
      setError(t('quickSettings.error.loadFailed', {
        error: String(err),
        defaultValue: '加载配置失败：{{error}}',
      }));
    }
  };

  const getRefreshKeyForType = (t: QuickSettingsType): keyof GeneralConfig => {
    switch (t) {
      case 'antigravity': return 'auto_refresh_minutes';
      case 'codex': return 'codex_auto_refresh_minutes';
      case 'github_copilot': return 'ghcp_auto_refresh_minutes';
      case 'windsurf': return 'windsurf_auto_refresh_minutes';
      case 'kiro': return 'kiro_auto_refresh_minutes';
      case 'cursor': return 'cursor_auto_refresh_minutes';
      case 'gemini': return 'gemini_auto_refresh_minutes';
      case 'codebuddy': return 'codebuddy_auto_refresh_minutes';
      case 'codebuddy_cn': return 'codebuddy_cn_auto_refresh_minutes';
      case 'qoder': return 'qoder_auto_refresh_minutes';
      case 'trae': return 'trae_auto_refresh_minutes';
      case 'workbuddy': return 'workbuddy_auto_refresh_minutes';
      case 'zed': return 'zed_auto_refresh_minutes';
      default: return 'auto_refresh_minutes';
    }
  };

  const saveConfig = useCallback(
    async (updates: Partial<GeneralConfig>) => {
      if (!config || saving) return;
      const merged = { ...config, ...updates };
      setConfig(merged);
      setSaving(true);
      try {
        await invoke('save_general_config', {
          language: merged.language,
          theme: merged.theme,
          uiScale: merged.ui_scale,
          autoRefreshMinutes: merged.auto_refresh_minutes,
          codexAutoRefreshMinutes: merged.codex_auto_refresh_minutes,
          ghcpAutoRefreshMinutes: merged.ghcp_auto_refresh_minutes,
          windsurfAutoRefreshMinutes: merged.windsurf_auto_refresh_minutes,
          kiroAutoRefreshMinutes: merged.kiro_auto_refresh_minutes,
          cursorAutoRefreshMinutes: merged.cursor_auto_refresh_minutes,
          geminiAutoRefreshMinutes: merged.gemini_auto_refresh_minutes,
          codebuddyAutoRefreshMinutes: merged.codebuddy_auto_refresh_minutes,
          codebuddyCnAutoRefreshMinutes: merged.codebuddy_cn_auto_refresh_minutes,
          workbuddyAutoRefreshMinutes: merged.workbuddy_auto_refresh_minutes,
          qoderAutoRefreshMinutes: merged.qoder_auto_refresh_minutes,
          traeAutoRefreshMinutes: merged.trae_auto_refresh_minutes,
          zedAutoRefreshMinutes: merged.zed_auto_refresh_minutes,
          closeBehavior: merged.close_behavior,
          minimizeBehavior: merged.minimize_behavior,
          hideDockIcon: merged.hide_dock_icon,
          opencodeAppPath: merged.opencode_app_path,
          antigravityAppPath: merged.antigravity_app_path,
          codexAppPath: merged.codex_app_path,
          vscodeAppPath: merged.vscode_app_path,
          windsurfAppPath: merged.windsurf_app_path,
          kiroAppPath: merged.kiro_app_path,
          cursorAppPath: merged.cursor_app_path,
          codebuddyAppPath: merged.codebuddy_app_path,
          codebuddyCnAppPath: merged.codebuddy_cn_app_path,
          qoderAppPath: merged.qoder_app_path,
          traeAppPath: merged.trae_app_path,
          workbuddyAppPath: merged.workbuddy_app_path,
          zedAppPath: merged.zed_app_path,
          opencodeSyncOnSwitch: merged.opencode_sync_on_switch,
          opencodeAuthOverwriteOnSwitch: merged.opencode_auth_overwrite_on_switch,
          ghcpOpencodeSyncOnSwitch: merged.ghcp_opencode_sync_on_switch,
          ghcpOpencodeAuthOverwriteOnSwitch: merged.ghcp_opencode_auth_overwrite_on_switch,
          ghcpLaunchOnSwitch: merged.ghcp_launch_on_switch,
          openclawAuthOverwriteOnSwitch: merged.openclaw_auth_overwrite_on_switch,
          codexLaunchOnSwitch: merged.codex_launch_on_switch,
          antigravityDualSwitchNoRestartEnabled: merged.antigravity_dual_switch_no_restart_enabled,
          autoSwitchEnabled: merged.auto_switch_enabled,
          autoSwitchThreshold: merged.auto_switch_threshold,
          autoSwitchScopeMode: merged.auto_switch_scope_mode,
          autoSwitchSelectedGroupIds: merged.auto_switch_selected_group_ids,
          autoSwitchAccountScopeMode: merged.auto_switch_account_scope_mode,
          autoSwitchSelectedAccountIds: merged.auto_switch_selected_account_ids,
          codexAutoSwitchEnabled: merged.codex_auto_switch_enabled,
          codexAutoSwitchPrimaryThreshold: merged.codex_auto_switch_primary_threshold,
          codexAutoSwitchSecondaryThreshold: merged.codex_auto_switch_secondary_threshold,
          codexAutoSwitchAccountScopeMode: merged.codex_auto_switch_account_scope_mode,
          codexAutoSwitchSelectedAccountIds: merged.codex_auto_switch_selected_account_ids,
          quotaAlertEnabled: merged.quota_alert_enabled,
          quotaAlertThreshold: merged.quota_alert_threshold,
          codexQuotaAlertEnabled: merged.codex_quota_alert_enabled,
          codexQuotaAlertThreshold: merged.codex_quota_alert_threshold,
          codexQuotaAlertPrimaryThreshold: merged.codex_quota_alert_primary_threshold,
          codexQuotaAlertSecondaryThreshold: merged.codex_quota_alert_secondary_threshold,
          ghcpQuotaAlertEnabled: merged.ghcp_quota_alert_enabled,
          ghcpQuotaAlertThreshold: merged.ghcp_quota_alert_threshold,
          windsurfQuotaAlertEnabled: merged.windsurf_quota_alert_enabled,
          windsurfQuotaAlertThreshold: merged.windsurf_quota_alert_threshold,
          kiroQuotaAlertEnabled: merged.kiro_quota_alert_enabled,
          kiroQuotaAlertThreshold: merged.kiro_quota_alert_threshold,
          cursorQuotaAlertEnabled: merged.cursor_quota_alert_enabled,
          cursorQuotaAlertThreshold: merged.cursor_quota_alert_threshold,
          geminiQuotaAlertEnabled: merged.gemini_quota_alert_enabled,
          geminiQuotaAlertThreshold: merged.gemini_quota_alert_threshold,
          codebuddyQuotaAlertEnabled: merged.codebuddy_quota_alert_enabled,
          codebuddyQuotaAlertThreshold: merged.codebuddy_quota_alert_threshold,
          codebuddyCnQuotaAlertEnabled: merged.codebuddy_cn_quota_alert_enabled,
          codebuddyCnQuotaAlertThreshold: merged.codebuddy_cn_quota_alert_threshold,
          qoderQuotaAlertEnabled: merged.qoder_quota_alert_enabled,
          qoderQuotaAlertThreshold: merged.qoder_quota_alert_threshold,
          traeQuotaAlertEnabled: merged.trae_quota_alert_enabled,
          traeQuotaAlertThreshold: merged.trae_quota_alert_threshold,
          workbuddyQuotaAlertEnabled: merged.workbuddy_quota_alert_enabled,
          workbuddyQuotaAlertThreshold: merged.workbuddy_quota_alert_threshold,
          zedQuotaAlertEnabled: merged.zed_quota_alert_enabled,
          zedQuotaAlertThreshold: merged.zed_quota_alert_threshold,
        });
        window.dispatchEvent(new Event('config-updated'));
      } catch (err) {
        console.error('Failed to save config:', err);
        setError(t('quickSettings.error.saveFailed', {
          error: String(err),
          defaultValue: '保存配置失败：{{error}}',
        }));
      } finally {
        setSaving(false);
      }
    },
    [config, saving]
  );

  const handlePickAppPath = async (
    target:
      | 'antigravity'
      | 'codex'
      | 'vscode'
      | 'windsurf'
      | 'kiro'
      | 'cursor'
      | 'codebuddy'
      | 'codebuddy_cn'
      | 'qoder'
      | 'trae'
      | 'workbuddy'
      | 'zed',
  ) => {
    try {
      const selected = await open({ multiple: false, directory: false });
      const path = Array.isArray(selected) ? selected[0] : selected;
      if (!path || !config) return;

      const key =
        target === 'antigravity'
          ? 'antigravity_app_path'
          : target === 'codex'
            ? 'codex_app_path'
            : target === 'vscode'
              ? 'vscode_app_path'
              : target === 'windsurf'
                ? 'windsurf_app_path'
                : target === 'cursor'
                  ? 'cursor_app_path'
                  : target === 'codebuddy'
                    ? 'codebuddy_app_path'
                    : target === 'codebuddy_cn'
                      ? 'codebuddy_cn_app_path'
                    : target === 'qoder'
                      ? 'qoder_app_path'
                    : target === 'trae'
                      ? 'trae_app_path'
                    : target === 'workbuddy'
                      ? 'workbuddy_app_path'
                    : target === 'zed'
                      ? 'zed_app_path'
                      : 'kiro_app_path';

      saveConfig({ [key]: path });
    } catch (err) {
      console.error('Failed to pick path:', err);
      setError(t('quickSettings.error.pickPathFailed', {
        error: String(err),
        defaultValue: '选择路径失败：{{error}}',
      }));
    }
  };

  const handleResetAppPath = async (
    target:
      | 'antigravity'
      | 'codex'
      | 'vscode'
      | 'windsurf'
      | 'kiro'
      | 'cursor'
      | 'codebuddy'
      | 'codebuddy_cn'
      | 'qoder'
      | 'trae'
      | 'workbuddy'
      | 'zed',
  ) => {
    if (pathDetecting) return;
    setPathDetecting(true);
    try {
      const detected = await invoke<string | null>('detect_app_path', { app: target, force: true });
      const path = detected || '';
      const key =
        target === 'antigravity'
          ? 'antigravity_app_path'
          : target === 'codex'
            ? 'codex_app_path'
            : target === 'vscode'
              ? 'vscode_app_path'
              : target === 'windsurf'
                ? 'windsurf_app_path'
                : target === 'cursor'
                  ? 'cursor_app_path'
                  : target === 'codebuddy'
                    ? 'codebuddy_app_path'
                    : target === 'codebuddy_cn'
                      ? 'codebuddy_cn_app_path'
                    : target === 'qoder'
                      ? 'qoder_app_path'
                    : target === 'trae'
                      ? 'trae_app_path'
                    : target === 'workbuddy'
                      ? 'workbuddy_app_path'
                    : target === 'zed'
                      ? 'zed_app_path'
                      : 'kiro_app_path';
      saveConfig({ [key]: path });
    } catch (err) {
      console.error('Failed to reset path:', err);
      setError(t('quickSettings.error.resetPathFailed', {
        error: String(err),
        defaultValue: '重置路径失败：{{error}}',
      }));
    } finally {
      setPathDetecting(false);
    }
  };

  const handleOpenCodexConfigToml = useCallback(async () => {
    if (openingCodexConfig) return;
    setOpeningCodexConfig(true);
    try {
      const configTomlPath = await codexService.getCodexConfigTomlPath();
      await openPath(configTomlPath);
    } catch (err) {
      setError(t('quickSettings.error.openCodexConfigFailed', {
        error: String(err),
        defaultValue: '打开 Codex config.toml 失败：{{error}}',
      }));
    } finally {
      setOpeningCodexConfig(false);
    }
  }, [openingCodexConfig, t]);

  const getTitle = () => {
    const platformLabel = (() => {
      switch (type) {
        case 'antigravity':
          return 'Antigravity';
        case 'codex':
          return 'Codex';
        case 'github_copilot':
          return 'GitHub Copilot';
        case 'windsurf':
          return 'Windsurf';
        case 'kiro':
          return 'Kiro';
        case 'cursor':
          return 'Cursor';
        case 'gemini':
          return 'Gemini Cli';
        case 'codebuddy':
          return 'CodeBuddy';
        case 'codebuddy_cn':
          return 'CodeBuddy CN';
        case 'qoder':
          return 'Qoder';
        case 'trae':
          return 'Trae';
        case 'workbuddy':
          return 'WorkBuddy';
        case 'zed':
          return 'Zed';
      }
    })();
    return `${platformLabel} ${t('nav.settings', '设置')}`;
  };

  const getRefreshKey = (): keyof GeneralConfig => {
    return getRefreshKeyForType(type);
  };

  const getQuotaAlertEnabledKeyForType = (t: QuickSettingsType): QuotaAlertEnabledKey => {
    switch (t) {
      case 'codex':
        return 'codex_quota_alert_enabled';
      case 'github_copilot':
        return 'ghcp_quota_alert_enabled';
      case 'windsurf':
        return 'windsurf_quota_alert_enabled';
      case 'kiro':
        return 'kiro_quota_alert_enabled';
      case 'cursor':
        return 'cursor_quota_alert_enabled';
      case 'gemini':
        return 'gemini_quota_alert_enabled';
      case 'codebuddy':
        return 'codebuddy_quota_alert_enabled';
      case 'codebuddy_cn':
        return 'codebuddy_cn_quota_alert_enabled';
      case 'qoder':
        return 'qoder_quota_alert_enabled';
      case 'trae':
        return 'trae_quota_alert_enabled';
      case 'workbuddy':
        return 'workbuddy_quota_alert_enabled';
      case 'zed':
        return 'zed_quota_alert_enabled';
      default:
        return 'quota_alert_enabled';
    }
  };

  const getQuotaAlertThresholdKeyForType = (t: QuickSettingsType): QuotaAlertThresholdKey => {
    switch (t) {
      case 'codex':
        return 'codex_quota_alert_threshold';
      case 'github_copilot':
        return 'ghcp_quota_alert_threshold';
      case 'windsurf':
        return 'windsurf_quota_alert_threshold';
      case 'kiro':
        return 'kiro_quota_alert_threshold';
      case 'cursor':
        return 'cursor_quota_alert_threshold';
      case 'gemini':
        return 'gemini_quota_alert_threshold';
      case 'codebuddy':
        return 'codebuddy_quota_alert_threshold';
      case 'codebuddy_cn':
        return 'codebuddy_cn_quota_alert_threshold';
      case 'qoder':
        return 'qoder_quota_alert_threshold';
      case 'trae':
        return 'trae_quota_alert_threshold';
      case 'workbuddy':
        return 'workbuddy_quota_alert_threshold';
      case 'zed':
        return 'zed_quota_alert_threshold';
      default:
        return 'quota_alert_threshold';
    }
  };

  const getRefreshLabel = () => {
    switch (type) {
      case 'antigravity':
        return t('quickSettings.refreshInterval', '配额自动刷新');
      case 'codex':
        return t('quickSettings.codexRefreshInterval', '配额自动刷新');
      case 'github_copilot':
        return t('quickSettings.ghcpRefreshInterval', '配额自动刷新');
      case 'windsurf':
        return t('quickSettings.windsurfRefreshInterval', '配额自动刷新');
      case 'kiro':
        return t('quickSettings.kiroRefreshInterval', '配额自动刷新');
      case 'cursor':
        return t('quickSettings.cursorRefreshInterval', '配额自动刷新');
      case 'gemini':
        return t('quickSettings.geminiRefreshInterval', '配额自动刷新');
      case 'codebuddy':
        return t('quickSettings.refreshInterval', '配额自动刷新');
      case 'codebuddy_cn':
        return t('quickSettings.refreshInterval', '配额自动刷新');
      case 'qoder':
        return t('quickSettings.refreshInterval', '配额自动刷新');
      case 'trae':
        return t('quickSettings.refreshInterval', '配额自动刷新');
      case 'workbuddy':
        return t('quickSettings.refreshInterval', '配额自动刷新');
      case 'zed':
        return t('quickSettings.refreshInterval', '配额自动刷新');
    }
  };

  const showAppPathSection = type !== 'gemini';

  const getAppPath = (): string => {
    if (!config) return '';
    switch (type) {
      case 'antigravity':
        return config.antigravity_app_path;
      case 'codex':
        return config.codex_app_path;
      case 'github_copilot':
        return config.vscode_app_path;
      case 'windsurf':
        return config.windsurf_app_path;
      case 'kiro':
        return config.kiro_app_path;
      case 'cursor':
        return config.cursor_app_path;
      case 'gemini':
        return '';
      case 'codebuddy':
        return config.codebuddy_app_path;
      case 'codebuddy_cn':
        return config.codebuddy_cn_app_path;
      case 'qoder':
        return config.qoder_app_path;
      case 'trae':
        return config.trae_app_path;
      case 'workbuddy':
        return config.workbuddy_app_path;
      case 'zed':
        return config.zed_app_path;
      default:
        return '';
    }
  };

  const getAppPathLabel = () => {
    switch (type) {
      case 'antigravity':
        return t('quickSettings.antigravity.appPath', '启动路径');
      case 'codex':
        return t('quickSettings.codex.appPath', '启动路径');
      case 'github_copilot':
        return t('quickSettings.githubCopilot.appPath', 'VS Code 路径');
      case 'windsurf':
        return t('quickSettings.windsurf.appPath', 'Windsurf 路径');
      case 'kiro':
        return t('quickSettings.kiro.appPath', 'Kiro 路径');
      case 'cursor':
        return t('quickSettings.cursor.appPath', 'Cursor 路径');
      case 'gemini':
        return t('quickSettings.gemini.appPath', 'Gemini Cli 路径');
      case 'codebuddy':
        return t('quickSettings.codebuddy.appPath', 'CodeBuddy 路径');
      case 'codebuddy_cn':
        return t('quickSettings.codebuddyCn.appPath', 'CodeBuddy CN 路径');
      case 'qoder':
        return t('quickSettings.qoder.appPath', 'Qoder 路径');
      case 'trae':
        return t('quickSettings.trae.appPath', 'Trae 路径');
      case 'workbuddy':
        return t('quickSettings.workbuddy.appPath', 'WorkBuddy 路径');
      case 'zed':
        return t('quickSettings.zed.appPath', 'Zed 路径');
    }
  };

  const getAppTarget = ():
    | 'antigravity'
    | 'codex'
    | 'vscode'
    | 'windsurf'
    | 'kiro'
    | 'cursor'
    | 'codebuddy'
    | 'codebuddy_cn'
    | 'qoder'
    | 'trae'
    | 'workbuddy'
    | 'zed' => {
    switch (type) {
      case 'antigravity':
        return 'antigravity';
      case 'codex':
        return 'codex';
      case 'github_copilot':
        return 'vscode';
      case 'windsurf':
        return 'windsurf';
      case 'kiro':
        return 'kiro';
      case 'cursor':
        return 'cursor';
      case 'gemini':
        return 'antigravity';
      case 'codebuddy':
        return 'codebuddy';
      case 'codebuddy_cn':
        return 'codebuddy_cn';
      case 'qoder':
        return 'qoder';
      case 'trae':
        return 'trae';
      case 'workbuddy':
        return 'workbuddy';
      case 'zed':
        return 'zed';
    }
  };

  const refreshValue = config ? (config[getRefreshKey()] as number) : 10;
  const isPreset = refreshPresets.includes(String(refreshValue));
  const showRefreshInput = refreshEditing;
  const currentAccountRefreshPlatform = getCurrentAccountRefreshPlatformForType(type);
  const currentAccountRefreshValue = currentAccountRefreshMap[currentAccountRefreshPlatform] ?? 1;
  const isCurrentAccountRefreshAllowed = refreshValue > 0;
  const currentAccountRefreshDisplayValue = isCurrentAccountRefreshAllowed
    ? String(currentAccountRefreshValue)
    : '-1';
  const isCurrentAccountRefreshPreset = CURRENT_ACCOUNT_REFRESH_PRESETS.includes(
    String(currentAccountRefreshValue),
  );
  const showCurrentAccountRefreshInput = currentAccountRefreshEditing && isCurrentAccountRefreshAllowed;

  const isThresholdPreset = config ? thresholdPresets.includes(String(config.auto_switch_threshold)) : true;
  const showThresholdInput = thresholdEditing;
  const autoSwitchScopeMode = config?.auto_switch_scope_mode === 'selected_groups'
    ? 'selected_groups'
    : 'any_group';
  const autoSwitchSelectedGroupIds = config?.auto_switch_selected_group_ids ?? [];
  const validAutoSwitchGroupIdSet = new Set(autoSwitchDisplayGroups.map((group) => group.id));
  const normalizedAutoSwitchSelectedGroupIds = autoSwitchSelectedGroupIds.filter((groupId) =>
    validAutoSwitchGroupIdSet.has(groupId)
  );
  const quotaAlertEnabledKey = getQuotaAlertEnabledKeyForType(type);
  const quotaAlertThresholdKey = getQuotaAlertThresholdKeyForType(type);
  const quotaAlertEnabledValue = config ? Boolean(config[quotaAlertEnabledKey]) : false;
  const quotaAlertThresholdValue = config ? Number(config[quotaAlertThresholdKey]) : 20;
  const isQuotaAlertThresholdPreset = thresholdPresets.includes(String(quotaAlertThresholdValue));
  const showQuotaAlertThresholdInput = quotaAlertThresholdEditing;
  const codexAutoSwitchPrimaryThresholdValue = config
    ? Number(config.codex_auto_switch_primary_threshold)
    : 20;
  const codexAutoSwitchSecondaryThresholdValue = config
    ? Number(config.codex_auto_switch_secondary_threshold)
    : 20;
  const codexQuotaAlertPrimaryThresholdValue = config
    ? Number(config.codex_quota_alert_primary_threshold)
    : 20;
  const codexQuotaAlertSecondaryThresholdValue = config
    ? Number(config.codex_quota_alert_secondary_threshold)
    : 20;
  const autoSwitchAccountScopeMode = normalizeAutoSwitchAccountScopeMode(
    config?.auto_switch_account_scope_mode,
  );
  const autoSwitchSelectedAccountIds = config?.auto_switch_selected_account_ids ?? [];
  const codexAutoSwitchAccountScopeMode = normalizeAutoSwitchAccountScopeMode(
    config?.codex_auto_switch_account_scope_mode,
  );
  const codexAutoSwitchSelectedAccountIds = config?.codex_auto_switch_selected_account_ids ?? [];

  const handleRefreshSelectChange = (val: string) => {
    if (val === 'custom') {
      setCustomRefresh(String(refreshValue > 0 ? refreshValue : 1));
      setRefreshEditing(true);
    } else {
      setCustomRefresh('');
      setRefreshEditing(false);
      saveConfig({ [getRefreshKey()]: parseInt(val, 10) });
    }
  };

  const handleCustomRefreshApply = () => {
    const parsed = parseInt(customRefresh, 10);
    if (!isNaN(parsed) && parsed >= 1) {
      saveConfig({ [getRefreshKey()]: parsed });
      setCustomRefresh('');
      setRefreshEditing(false);
      return;
    }
    setCustomRefresh('');
    setRefreshEditing(false);
  };

  const saveCurrentAccountRefresh = (minutes: number) => {
    setCurrentAccountRefreshMap((prev) => {
      const next = saveCurrentAccountRefreshMinutesMap({
        ...prev,
        [currentAccountRefreshPlatform]: minutes,
      });
      window.dispatchEvent(new Event('config-updated'));
      return next;
    });
  };

  const handleCurrentAccountRefreshSelectChange = (value: string) => {
    if (!isCurrentAccountRefreshAllowed) {
      setCurrentAccountCustomRefresh('');
      setCurrentAccountRefreshEditing(false);
      return;
    }
    if (value === 'custom') {
      setCurrentAccountCustomRefresh(String(currentAccountRefreshValue || 1));
      setCurrentAccountRefreshEditing(true);
      return;
    }
    const parsed = parseInt(value, 10);
    if (!isNaN(parsed) && parsed >= 1) {
      saveCurrentAccountRefresh(parsed);
    }
    setCurrentAccountCustomRefresh('');
    setCurrentAccountRefreshEditing(false);
  };

  const handleCurrentAccountCustomRefreshApply = () => {
    if (!isCurrentAccountRefreshAllowed) {
      setCurrentAccountCustomRefresh('');
      setCurrentAccountRefreshEditing(false);
      return;
    }
    const parsed = parseInt(currentAccountCustomRefresh, 10);
    if (!isNaN(parsed) && parsed >= 1) {
      saveCurrentAccountRefresh(parsed);
      setCurrentAccountCustomRefresh('');
      setCurrentAccountRefreshEditing(false);
      return;
    }
    setCurrentAccountCustomRefresh('');
    setCurrentAccountRefreshEditing(false);
  };

  const handleThresholdSelectChange = (val: string) => {
    if (val === 'custom') {
      setCustomThreshold(String(config?.auto_switch_threshold ?? 20));
      setThresholdEditing(true);
    } else {
      setCustomThreshold('');
      setThresholdEditing(false);
      saveConfig({ auto_switch_threshold: parseInt(val, 10) });
    }
  };

  const handleCustomThresholdApply = () => {
    const parsed = parseInt(customThreshold, 10);
    if (!isNaN(parsed) && parsed >= 0 && parsed <= 100) {
      saveConfig({ auto_switch_threshold: parsed });
      setCustomThreshold('');
      setThresholdEditing(false);
      return;
    }
    setCustomThreshold('');
    setThresholdEditing(false);
  };

  const handleAutoSwitchScopeModeChange = (value: string) => {
    if (value !== 'selected_groups') {
      saveConfig({ auto_switch_scope_mode: 'any_group' });
      return;
    }
    const nextSelected = normalizedAutoSwitchSelectedGroupIds.length > 0
      ? normalizedAutoSwitchSelectedGroupIds
      : autoSwitchDisplayGroups.map((group) => group.id);
    saveConfig({
      auto_switch_scope_mode: 'selected_groups',
      auto_switch_selected_group_ids: nextSelected,
    });
  };

  const handleAutoSwitchGroupToggle = (groupId: string) => {
    const selected = new Set(normalizedAutoSwitchSelectedGroupIds);
    if (selected.has(groupId)) {
      if (selected.size === 1) {
        return;
      }
      selected.delete(groupId);
    } else {
      selected.add(groupId);
    }
    saveConfig({ auto_switch_selected_group_ids: [...selected] });
  };

  const handleQuotaAlertThresholdSelectChange = (val: string) => {
    if (val === 'custom') {
      setQuotaAlertCustomThreshold(String(quotaAlertThresholdValue));
      setQuotaAlertThresholdEditing(true);
    } else {
      setQuotaAlertCustomThreshold('');
      setQuotaAlertThresholdEditing(false);
      saveConfig({ [quotaAlertThresholdKey]: parseInt(val, 10) } as Partial<GeneralConfig>);
    }
  };

  const handleQuotaAlertCustomThresholdApply = () => {
    const parsed = parseInt(quotaAlertCustomThreshold, 10);
    if (!isNaN(parsed) && parsed >= 0 && parsed <= 100) {
      saveConfig({ [quotaAlertThresholdKey]: parsed } as Partial<GeneralConfig>);
      setQuotaAlertCustomThreshold('');
      setQuotaAlertThresholdEditing(false);
      return;
    }
    setQuotaAlertCustomThreshold('');
    setQuotaAlertThresholdEditing(false);
  };

  const handleCodexWindowThresholdInputChange = (
    rawValue: string,
    setCustomValue: (value: string) => void,
  ) => {
    setCustomValue(rawValue.replace(/[^\d]/g, '').slice(0, 3));
  };

  const handleCodexWindowCustomThresholdApply = (
    customValue: string,
    setCustomValue: (value: string) => void,
    key: CodexWindowThresholdKey,
    fallbackValue: number,
  ) => {
    const parsed = parseInt(customValue, 10);
    if (!isNaN(parsed) && parsed >= 0 && parsed <= 100) {
      saveConfig({ [key]: parsed } as Partial<GeneralConfig>);
      setCustomValue(String(parsed));
      return;
    }
    setCustomValue(String(fallbackValue));
  };

  /** 共用的配额预警 enable + threshold 控件 */
  const renderQuotaAlertControls = () => {
    const isCodexAlert = type === 'codex';
    return (
      <>
        <div className="qs-row" style={{ marginTop: type === 'antigravity' ? 10 : 0 }}>
          <div className="qs-row-label">
            <span>{t('quickSettings.quotaAlert.enable', '超额预警')}</span>
          </div>
          <div className="qs-row-control">
            <label className="qs-switch">
              <input
                type="checkbox"
                checked={quotaAlertEnabledValue}
                onChange={(e) =>
                  saveConfig({ [quotaAlertEnabledKey]: e.target.checked } as Partial<GeneralConfig>)
                }
              />
              <span className="qs-switch-slider"></span>
            </label>
          </div>
        </div>

        {quotaAlertEnabledValue && (
          <div className="qs-field-group" style={{ animation: 'qsFadeUp 0.2s ease both' }}>
            {isCodexAlert ? (
              <>
                <div className="qs-row">
                  <div className="qs-row-label">
                    <span>
                      primary_window ({t('codex.quota.hourly', '5小时配额')}) {t('quickSettings.quotaAlert.threshold', '预警阈值')}
                    </span>
                  </div>
                  <div className="qs-row-control">
                    <div className="qs-inline-input">
                      <input
                        type="number"
                        min={0}
                        max={100}
                        className="qs-select qs-select--input-mode qs-select--with-unit"
                        value={codexQuotaAlertPrimaryCustomThreshold}
                        placeholder={t('quickSettings.inputPercent', '输入百分比')}
                        onChange={(e) =>
                          handleCodexWindowThresholdInputChange(
                            e.target.value,
                            setCodexQuotaAlertPrimaryCustomThreshold,
                          )
                        }
                        onBlur={() =>
                          handleCodexWindowCustomThresholdApply(
                            codexQuotaAlertPrimaryCustomThreshold,
                            setCodexQuotaAlertPrimaryCustomThreshold,
                            'codex_quota_alert_primary_threshold',
                            codexQuotaAlertPrimaryThresholdValue,
                          )
                        }
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') {
                            e.preventDefault();
                            handleCodexWindowCustomThresholdApply(
                              codexQuotaAlertPrimaryCustomThreshold,
                              setCodexQuotaAlertPrimaryCustomThreshold,
                              'codex_quota_alert_primary_threshold',
                              codexQuotaAlertPrimaryThresholdValue,
                            );
                          }
                        }}
                      />
                      <span className="qs-input-unit">%</span>
                    </div>
                  </div>
                </div>

                <div className="qs-hint" style={{ marginTop: 0, marginBottom: 4 }}>
                  {t('quickSettings.codexWindow.orDivider', 'OR（命中任一即触发）')}
                </div>

                <div className="qs-row">
                  <div className="qs-row-label">
                    <span>
                      secondary_window ({t('codex.quota.weekly', '周配额')}) {t('quickSettings.quotaAlert.threshold', '预警阈值')}
                    </span>
                  </div>
                  <div className="qs-row-control">
                    <div className="qs-inline-input">
                      <input
                        type="number"
                        min={0}
                        max={100}
                        className="qs-select qs-select--input-mode qs-select--with-unit"
                        value={codexQuotaAlertSecondaryCustomThreshold}
                        placeholder={t('quickSettings.inputPercent', '输入百分比')}
                        onChange={(e) =>
                          handleCodexWindowThresholdInputChange(
                            e.target.value,
                            setCodexQuotaAlertSecondaryCustomThreshold,
                          )
                        }
                        onBlur={() =>
                          handleCodexWindowCustomThresholdApply(
                            codexQuotaAlertSecondaryCustomThreshold,
                            setCodexQuotaAlertSecondaryCustomThreshold,
                            'codex_quota_alert_secondary_threshold',
                            codexQuotaAlertSecondaryThresholdValue,
                          )
                        }
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') {
                            e.preventDefault();
                            handleCodexWindowCustomThresholdApply(
                              codexQuotaAlertSecondaryCustomThreshold,
                              setCodexQuotaAlertSecondaryCustomThreshold,
                              'codex_quota_alert_secondary_threshold',
                              codexQuotaAlertSecondaryThresholdValue,
                            );
                          }
                        }}
                      />
                      <span className="qs-input-unit">%</span>
                    </div>
                  </div>
                </div>
              </>
            ) : (
              <div className="qs-row">
                <div className="qs-row-label">
                  <span>{t('quickSettings.quotaAlert.threshold', '预警阈值')}</span>
                </div>
                <div className="qs-row-control">
                  {showQuotaAlertThresholdInput ? (
                    <div className="qs-inline-input">
                      <input
                        type="number"
                        min={0}
                        max={100}
                        className="qs-select qs-select--input-mode qs-select--with-unit"
                        value={quotaAlertCustomThreshold}
                        placeholder={t('quickSettings.inputPercent', '输入百分比')}
                        onChange={(e) => setQuotaAlertCustomThreshold(e.target.value.replace(/[^\d]/g, ''))}
                        onBlur={handleQuotaAlertCustomThresholdApply}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') {
                            e.preventDefault();
                            handleQuotaAlertCustomThresholdApply();
                          }
                        }}
                      />
                      <span className="qs-input-unit">%</span>
                    </div>
                  ) : (
                    <select
                      className="qs-select"
                      value={String(quotaAlertThresholdValue)}
                      onChange={(e) => handleQuotaAlertThresholdSelectChange(e.target.value)}
                    >
                      {!isQuotaAlertThresholdPreset && (
                        <option value={String(quotaAlertThresholdValue)}>
                          {quotaAlertThresholdValue}%
                        </option>
                      )}
                      <option value="0">0%</option>
                      <option value="20">20%</option>
                      <option value="40">40%</option>
                      <option value="60">60%</option>
                      <option value="custom">{t('quickSettings.customInput', '自定义')}</option>
                    </select>
                  )}
                </div>
              </div>
            )}
            <div className="qs-hint" style={{ marginTop: 6 }}>
              {t(
                'quickSettings.quotaAlert.hint',
                '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。'
              )}
              {isCodexAlert && (
                <>
                  <div>
                    {t(
                      'quickSettings.codexWindow.primaryWindowMeaning',
                      'primary_window 一般指 5 小时配额；免费用户下 primary_window 可能对应周配额，不同订阅可能不同。'
                    )}
                  </div>
                  <div>
                    {`primary_window <= ${codexQuotaAlertPrimaryThresholdValue}% OR secondary_window <= ${codexQuotaAlertSecondaryThresholdValue}%`}
                  </div>
                </>
              )}
            </div>
          </div>
        )}
      </>
    );
  };

  const handleCodexCodeReviewQuotaToggle = (checked: boolean) => {
    setCodexShowCodeReviewQuota(checked);
    persistCodexCodeReviewQuotaVisible(checked);
  };

  const overlayContent = isOpen ? (
    <div className="qs-overlay" onClick={(e) => { if (e.target === e.currentTarget) setIsOpen(false); }}>
      <div className="qs-modal" ref={modalRef}>
        <div className="qs-header">
          <span className="qs-title">{getTitle()}</span>
          <button className="qs-close" onClick={() => setIsOpen(false)} aria-label={t('common.close')}>
            <X size={16} />
          </button>
        </div>

        {/* 错误提示 */}
        {error && (
          <div className="qs-error">
            {error}
            <button className="qs-error-close" onClick={() => setError(null)} aria-label={t('common.close')}>
              <X size={12} />
            </button>
          </div>
        )}

        {config && (
          <div className="qs-body">
            {/* ─── Refresh Interval ─── */}
            <div className="qs-section">
              <div className="qs-section-header">
                <RefreshCw size={15} />
                <span>{getRefreshLabel()}</span>
              </div>
              <div className="qs-field-group">
                {showRefreshInput ? (
                  <div className="qs-inline-input">
                    <input
                      type="number"
                      min={1}
                      max={999}
                      className="qs-select qs-select--input-mode qs-select--with-unit"
                      value={customRefresh}
                      placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                      onChange={(e) => setCustomRefresh(e.target.value.replace(/[^\d]/g, ''))}
                      onBlur={handleCustomRefreshApply}
                      onKeyDown={(e) => {
                        if (e.key === 'Enter') {
                          e.preventDefault();
                          handleCustomRefreshApply();
                        }
                      }}
                    />
                    <span className="qs-input-unit">{t('settings.general.minutes')}</span>
                  </div>
                ) : (
                  <select
                    className="qs-select"
                    value={String(refreshValue)}
                    onChange={(e) => handleRefreshSelectChange(e.target.value)}
                  >
                    {!isPreset && (
                      <option value={String(refreshValue)}>
                        {refreshValue} {t('settings.general.minutes')}
                      </option>
                    )}
                    <option value="-1">{t('settings.general.autoRefreshDisabled')}</option>
                    <option value="2">2 {t('settings.general.minutes')}</option>
                    <option value="5">5 {t('settings.general.minutes')}</option>
                    <option value="10">10 {t('settings.general.minutes')}</option>
                    <option value="15">15 {t('settings.general.minutes')}</option>
                    <option value="custom">{t('quickSettings.customInput', '自定义')}</option>
                  </select>
                )}
              </div>
            </div>

            <div className="qs-section">
              <div className="qs-section-header">
                <RefreshCw size={15} />
                <span>{t('settings.general.currentAccountRefreshTitle')}</span>
              </div>
              <div className="qs-field-group">
                {showCurrentAccountRefreshInput ? (
                  <div className="qs-inline-input">
                    <input
                      type="number"
                      min={1}
                      max={999}
                      className="qs-select qs-select--input-mode qs-select--with-unit"
                      value={currentAccountCustomRefresh}
                      placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                      onChange={(e) =>
                        setCurrentAccountCustomRefresh(e.target.value.replace(/[^\d]/g, ''))
                      }
                      onBlur={handleCurrentAccountCustomRefreshApply}
                      onKeyDown={(e) => {
                        if (e.key === 'Enter') {
                          e.preventDefault();
                          handleCurrentAccountCustomRefreshApply();
                        }
                      }}
                    />
                    <span className="qs-input-unit">{t('settings.general.minutes')}</span>
                  </div>
                ) : (
                  <select
                    className="qs-select"
                    value={currentAccountRefreshDisplayValue}
                    onChange={(e) => handleCurrentAccountRefreshSelectChange(e.target.value)}
                    disabled={!isCurrentAccountRefreshAllowed}
                  >
                    {!isCurrentAccountRefreshAllowed && (
                      <option value="-1">{t('settings.general.autoRefreshDisabled')}</option>
                    )}
                    {!isCurrentAccountRefreshPreset && (
                      <option value={String(currentAccountRefreshValue)}>
                        {currentAccountRefreshValue} {t('settings.general.minutes')}
                      </option>
                    )}
                    <option value="1">1 {t('settings.general.minutes')}</option>
                    <option value="2">2 {t('settings.general.minutes')}</option>
                    <option value="5">5 {t('settings.general.minutes')}</option>
                    <option value="10">10 {t('settings.general.minutes')}</option>
                    <option value="15">15 {t('settings.general.minutes')}</option>
                    <option value="custom">{t('quickSettings.customInput', '自定义')}</option>
                  </select>
                )}
                <div className="qs-hint" style={{ marginTop: 6 }}>
                  {isCurrentAccountRefreshAllowed
                    ? t('settings.general.currentAccountRefreshItemDesc')
                    : t(
                      'settings.general.currentAccountRefreshRequiresAutoRefresh',
                      '需先开启“配额自动刷新”后，才能设置当前账号刷新。',
                    )}
                </div>
              </div>
            </div>

            {/* ─── App Path ─── */}
            {showAppPathSection && (
              <div className="qs-section">
                <div className="qs-section-header">
                  <FolderOpen size={15} />
                  <span>{getAppPathLabel()}</span>
                </div>
                <div className="qs-path-control">
                  <input
                    type="text"
                    className="qs-path-input"
                    value={getAppPath()}
                    placeholder={t('settings.general.codexAppPathPlaceholder', '默认路径')}
                    onChange={(e) => {
                      const key =
                        type === 'antigravity'
                          ? 'antigravity_app_path'
                          : type === 'codex'
                            ? 'codex_app_path'
                            : type === 'github_copilot'
                              ? 'vscode_app_path'
                              : type === 'windsurf'
                              ? 'windsurf_app_path'
                                : type === 'cursor'
                                  ? 'cursor_app_path'
                                  : type === 'codebuddy'
                                    ? 'codebuddy_app_path'
                                    : type === 'codebuddy_cn'
                                      ? 'codebuddy_cn_app_path'
                                    : type === 'qoder'
                                      ? 'qoder_app_path'
                                    : type === 'trae'
                                      ? 'trae_app_path'
                                    : type === 'workbuddy'
                                      ? 'workbuddy_app_path'
                                    : type === 'zed'
                                      ? 'zed_app_path'
                                  : 'kiro_app_path';
                      saveConfig({ [key]: e.target.value });
                    }}
                  />
                  <div className="qs-path-actions">
                    <button
                      className="qs-btn"
                      onClick={() => handlePickAppPath(getAppTarget())}
                      disabled={pathDetecting}
                      title={t('settings.general.codexPathSelect', '选择')}
                    >
                      {t('settings.general.codexPathSelect', '选择')}
                    </button>
                    <button
                      className="qs-btn"
                      onClick={() => handleResetAppPath(getAppTarget())}
                      disabled={pathDetecting}
                      title={
                        pathDetecting
                          ? t('common.loading', '加载中...')
                          : t('settings.general.codexPathReset', '恢复默认')
                      }
                    >
                      <RefreshCw size={12} className={pathDetecting ? 'spin' : undefined} />
                    </button>
                  </div>
                </div>
              </div>
            )}

            {/* ─── Codex: opencode sync ─── */}
            {type === 'codex' && (
              <div className="qs-section">
                <div className="qs-row">
                  <div className="qs-row-label">
                    <FolderOpen size={15} />
                    <span>{t('quickSettings.codex.configToml', 'Codex config.toml')}</span>
                  </div>
                  <div className="qs-row-control">
                    <button
                      className="qs-btn"
                      onClick={() => void handleOpenCodexConfigToml()}
                      disabled={openingCodexConfig}
                    >
                      {openingCodexConfig
                        ? t('common.loading', '加载中...')
                        : t('quickSettings.codex.openConfigToml', '打开文件')}
                    </button>
                  </div>
                </div>
                <div className="qs-hint" style={{ marginTop: -2, marginBottom: 2 }}>
                  {t('quickSettings.codex.openConfigHint', '快速打开当前使用的 Codex config.toml 文件。')}
                </div>

                <div className="qs-row">
                  <div className="qs-row-label">
                    <Zap size={15} />
                    <span>
                      {t(
                        'settings.general.codexLaunchOnSwitch',
                        '切换 Codex 时自动启动 Codex App'
                      )}
                    </span>
                  </div>
                  <div className="qs-row-control">
                    <label className="qs-switch">
                      <input
                        type="checkbox"
                        checked={config.codex_launch_on_switch}
                        onChange={(e) => saveConfig({ codex_launch_on_switch: e.target.checked })}
                      />
                      <span className="qs-switch-slider"></span>
                    </label>
                  </div>
                </div>

                <div className="qs-row">
                  <div className="qs-row-label">
                    <Zap size={15} />
                    <span>
                      {t(
                        'settings.general.openclawAuthOverwrite',
                        '切换 Codex 时覆盖 OpenClaw 登录信息'
                      )}
                    </span>
                  </div>
                  <div className="qs-row-control">
                    <label className="qs-switch">
                      <input
                        type="checkbox"
                        checked={config.openclaw_auth_overwrite_on_switch}
                        onChange={(e) =>
                          saveConfig({ openclaw_auth_overwrite_on_switch: e.target.checked })
                        }
                      />
                      <span className="qs-switch-slider"></span>
                    </label>
                  </div>
                </div>

                <div className="qs-row">
                  <div className="qs-row-label">
                    <Zap size={15} />
                    <span>
                      {t(
                        'settings.general.opencodeAuthOverwrite',
                        '切换 Codex 时覆盖 OpenCode 登录信息'
                      )}
                    </span>
                  </div>
                  <div className="qs-row-control">
                    <label className="qs-switch">
                      <input
                        type="checkbox"
                        checked={config.opencode_auth_overwrite_on_switch}
                        onChange={(e) =>
                          saveConfig(
                            e.target.checked
                              ? { opencode_auth_overwrite_on_switch: true }
                              : {
                                  opencode_auth_overwrite_on_switch: false,
                                  opencode_sync_on_switch: false,
                                }
                          )
                        }
                      />
                      <span className="qs-switch-slider"></span>
                    </label>
                  </div>
                </div>

                <div className="qs-row">
                  <div className="qs-row-label">
                    <Zap size={15} />
                    <span>{t('settings.general.opencodeRestart', '切换时自动重启 OpenCode')}</span>
                  </div>
                  <div className="qs-row-control">
                    <label className="qs-switch">
                      <input
                        type="checkbox"
                        checked={config.opencode_sync_on_switch}
                        disabled={!config.opencode_auth_overwrite_on_switch}
                        onChange={(e) => saveConfig({ opencode_sync_on_switch: e.target.checked })}
                      />
                      <span className="qs-switch-slider"></span>
                    </label>
                  </div>
                </div>

                <div className="qs-row">
                  <div className="qs-row-label">
                    <Zap size={15} />
                    <span>{t('codex.list.showCodeReviewQuota', '显示 Code Review 配额')}</span>
                  </div>
                  <div className="qs-row-control">
                    <label className="qs-switch">
                      <input
                        type="checkbox"
                        checked={codexShowCodeReviewQuota}
                        onChange={(e) => handleCodexCodeReviewQuotaToggle(e.target.checked)}
                      />
                      <span className="qs-switch-slider"></span>
                    </label>
                  </div>
                </div>

                <div
                  className="qs-field-group"
                  style={{ marginTop: 6, paddingTop: 8, borderTop: '1px solid var(--border-light)' }}
                >
                  <div className="qs-row">
                    <div className="qs-row-label">
                      <Zap size={15} />
                      <span>{t('quickSettings.autoSwitch.enable', '启用自动切号')}</span>
                    </div>
                    <div className="qs-row-control">
                      <label className="qs-switch">
                        <input
                          type="checkbox"
                          checked={config.codex_auto_switch_enabled}
                          onChange={(e) => saveConfig({ codex_auto_switch_enabled: e.target.checked })}
                        />
                        <span className="qs-switch-slider"></span>
                      </label>
                    </div>
                  </div>

                  {config.codex_auto_switch_enabled && (
                    <div className="qs-field-group" style={{ animation: 'qsFadeUp 0.2s ease both' }}>
                      <div className="qs-row">
                        <div className="qs-row-label">
                          <span>
                            primary_window ({t('codex.quota.hourly', '5小时配额')}) {t('quickSettings.autoSwitch.threshold', '切号阈值')}
                          </span>
                        </div>
                        <div className="qs-row-control">
                          <div className="qs-inline-input">
                            <input
                              type="number"
                              min={0}
                              max={100}
                              className="qs-select qs-select--input-mode qs-select--with-unit"
                              value={codexAutoSwitchPrimaryCustomThreshold}
                              placeholder={t('quickSettings.inputPercent', '输入百分比')}
                              onChange={(e) =>
                                handleCodexWindowThresholdInputChange(
                                  e.target.value,
                                  setCodexAutoSwitchPrimaryCustomThreshold,
                                )
                              }
                              onBlur={() =>
                                handleCodexWindowCustomThresholdApply(
                                  codexAutoSwitchPrimaryCustomThreshold,
                                  setCodexAutoSwitchPrimaryCustomThreshold,
                                  'codex_auto_switch_primary_threshold',
                                  codexAutoSwitchPrimaryThresholdValue,
                                )
                              }
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  handleCodexWindowCustomThresholdApply(
                                    codexAutoSwitchPrimaryCustomThreshold,
                                    setCodexAutoSwitchPrimaryCustomThreshold,
                                    'codex_auto_switch_primary_threshold',
                                    codexAutoSwitchPrimaryThresholdValue,
                                  );
                                }
                              }}
                            />
                            <span className="qs-input-unit">%</span>
                          </div>
                        </div>
                      </div>

                      <div className="qs-hint" style={{ marginTop: 0, marginBottom: 4 }}>
                        {t('quickSettings.codexWindow.orDivider', 'OR（命中任一即触发）')}
                      </div>

                      <div className="qs-row">
                        <div className="qs-row-label">
                          <span>
                            secondary_window ({t('codex.quota.weekly', '周配额')}) {t('quickSettings.autoSwitch.threshold', '切号阈值')}
                          </span>
                        </div>
                        <div className="qs-row-control">
                          <div className="qs-inline-input">
                            <input
                              type="number"
                              min={0}
                              max={100}
                              className="qs-select qs-select--input-mode qs-select--with-unit"
                              value={codexAutoSwitchSecondaryCustomThreshold}
                              placeholder={t('quickSettings.inputPercent', '输入百分比')}
                              onChange={(e) =>
                                handleCodexWindowThresholdInputChange(
                                  e.target.value,
                                  setCodexAutoSwitchSecondaryCustomThreshold,
                                )
                              }
                              onBlur={() =>
                                handleCodexWindowCustomThresholdApply(
                                  codexAutoSwitchSecondaryCustomThreshold,
                                  setCodexAutoSwitchSecondaryCustomThreshold,
                                  'codex_auto_switch_secondary_threshold',
                                  codexAutoSwitchSecondaryThresholdValue,
                                )
                              }
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  handleCodexWindowCustomThresholdApply(
                                    codexAutoSwitchSecondaryCustomThreshold,
                                    setCodexAutoSwitchSecondaryCustomThreshold,
                                    'codex_auto_switch_secondary_threshold',
                                    codexAutoSwitchSecondaryThresholdValue,
                                  );
                                }
                              }}
                            />
                            <span className="qs-input-unit">%</span>
                          </div>
                        </div>
                      </div>

                      <div className="qs-row qs-row--top">
                        <div className="qs-row-label">
                          <span>{t('settings.general.codexAutoSwitchAccountScope', 'Codex 自动切号账号范围')}</span>
                        </div>
                        <div className="qs-row-control qs-row-control--fill">
                          <AutoSwitchAccountScopeSelector
                            mode={codexAutoSwitchAccountScopeMode}
                            onModeChange={(mode) =>
                              saveConfig({ codex_auto_switch_account_scope_mode: mode })
                            }
                            selectedAccountIds={codexAutoSwitchSelectedAccountIds}
                            onSelectedAccountIdsChange={(ids) =>
                              saveConfig({ codex_auto_switch_selected_account_ids: ids })
                            }
                            accounts={codexScopeAccounts}
                            groups={codexScopeGroups}
                            useDialog
                          />
                        </div>
                      </div>

                      <div className="qs-hint">
                        {t(
                          'quickSettings.autoSwitch.hint',
                          '当任意模型配额低于阈值时，自动切换到配额最高的账号。'
                        )}
                        <div>
                          {t(
                            'quickSettings.codexWindow.primaryWindowMeaning',
                            'primary_window 一般指 5 小时配额；免费用户下 primary_window 可能对应周配额，不同订阅可能不同。'
                          )}
                        </div>
                        <div>
                          {`primary_window <= ${codexAutoSwitchPrimaryThresholdValue}% OR secondary_window <= ${codexAutoSwitchSecondaryThresholdValue}%`}
                        </div>
                      </div>
                    </div>
                  )}
                </div>
              </div>
            )}

            {/* ─── GitHub Copilot: opencode sync ─── */}
            {type === 'github_copilot' && (
              <div className="qs-section">
                <div className="qs-row">
                  <div className="qs-row-label">
                    <Zap size={15} />
                    <span>
                      {t(
                        'settings.general.ghcpLaunchOnSwitch',
                        '切换 GitHub Copilot 时自动启动 GitHub Copilot'
                      )}
                    </span>
                  </div>
                  <div className="qs-row-control">
                    <label className="qs-switch">
                      <input
                        type="checkbox"
                        checked={config.ghcp_launch_on_switch}
                        onChange={(e) => saveConfig({ ghcp_launch_on_switch: e.target.checked })}
                      />
                      <span className="qs-switch-slider"></span>
                    </label>
                  </div>
                </div>

                <div className="qs-row">
                  <div className="qs-row-label">
                    <Zap size={15} />
                    <span>
                      {t(
                        'settings.general.ghcpOpencodeAuthOverwrite',
                        '切换 GitHub Copilot 时覆盖 OpenCode 登录信息'
                      )}
                    </span>
                  </div>
                  <div className="qs-row-control">
                    <label className="qs-switch">
                      <input
                        type="checkbox"
                        checked={config.ghcp_opencode_auth_overwrite_on_switch}
                        onChange={(e) =>
                          saveConfig(
                            e.target.checked
                              ? { ghcp_opencode_auth_overwrite_on_switch: true }
                              : {
                                  ghcp_opencode_auth_overwrite_on_switch: false,
                                  ghcp_opencode_sync_on_switch: false,
                                }
                          )
                        }
                      />
                      <span className="qs-switch-slider"></span>
                    </label>
                  </div>
                </div>

                <div className="qs-row">
                  <div className="qs-row-label">
                    <Zap size={15} />
                    <span>
                      {t(
                        'settings.general.ghcpOpencodeRestart',
                        '切换 GitHub Copilot 时自动重启 OpenCode'
                      )}
                    </span>
                  </div>
                  <div className="qs-row-control">
                    <label className="qs-switch">
                      <input
                        type="checkbox"
                        checked={config.ghcp_opencode_sync_on_switch}
                        disabled={!config.ghcp_opencode_auth_overwrite_on_switch}
                        onChange={(e) =>
                          saveConfig({ ghcp_opencode_sync_on_switch: e.target.checked })
                        }
                      />
                      <span className="qs-switch-slider"></span>
                    </label>
                  </div>
                </div>
              </div>
            )}

            {/* ─── Antigravity: Auto-switch ─── */}
            {type === 'antigravity' && (
              <div className="qs-section qs-section--highlight">
                <div className="qs-section-header">
                  <Zap size={15} />
                  <span>{t('quickSettings.autoSwitch.title', '自动切号')}</span>
                </div>

                {antigravitySeamlessSwitchUnlocked && (
                  <>
                    <div className="qs-row">
                      <div className="qs-row-label">
                        <span>
                          {t(
                            'settings.general.antigravityDualSwitchNoRestart',
                            '无感双通道切号（不重启）'
                          )}
                        </span>
                      </div>
                      <div className="qs-row-control">
                        <label className="qs-switch">
                          <input
                            type="checkbox"
                            checked={config.antigravity_dual_switch_no_restart_enabled}
                            onChange={(e) =>
                              saveConfig({
                                antigravity_dual_switch_no_restart_enabled: e.target.checked,
                              })
                            }
                          />
                          <span className="qs-switch-slider"></span>
                        </label>
                      </div>
                    </div>

                    <div className="qs-hint">
                      {t(
                        'settings.general.antigravityDualSwitchNoRestartDesc',
                        '切号时同时执行本地落盘与扩展无感切号，不再自动重启 Antigravity。'
                      )}
                    </div>
                  </>
                )}

                <div className="qs-row">
                  <div className="qs-row-label">
                    <span>{t('quickSettings.autoSwitch.enable', '启用自动切号')}</span>
                  </div>
                  <div className="qs-row-control">
                    <label className="qs-switch">
                      <input
                        type="checkbox"
                        checked={config.auto_switch_enabled}
                        onChange={(e) => saveConfig({ auto_switch_enabled: e.target.checked })}
                      />
                      <span className="qs-switch-slider"></span>
                    </label>
                  </div>
                </div>

                {config.auto_switch_enabled && (
                  <div className="qs-field-group" style={{ animation: 'qsFadeUp 0.2s ease both' }}>
                    <div className="qs-row">
                      <div className="qs-row-label">
                        <span>{t('quickSettings.autoSwitch.threshold', '切号阈值')}</span>
                      </div>
                      <div className="qs-row-control">
                        {showThresholdInput ? (
                          <div className="qs-inline-input">
                            <input
                              type="number"
                              min={0}
                              max={100}
                              className="qs-select qs-select--input-mode qs-select--with-unit"
                              value={customThreshold}
                              placeholder={t('quickSettings.inputPercent', '输入百分比')}
                              onChange={(e) => setCustomThreshold(e.target.value.replace(/[^\d]/g, ''))}
                              onBlur={handleCustomThresholdApply}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  handleCustomThresholdApply();
                                }
                              }}
                            />
                            <span className="qs-input-unit">%</span>
                          </div>
                        ) : (
                          <select
                            className="qs-select"
                            value={String(config.auto_switch_threshold)}
                            onChange={(e) => handleThresholdSelectChange(e.target.value)}
                          >
                            {!isThresholdPreset && (
                              <option value={String(config.auto_switch_threshold)}>
                                {config.auto_switch_threshold}%
                              </option>
                            )}
                            <option value="0">0%</option>
                            <option value="20">20%</option>
                            <option value="40">40%</option>
                            <option value="60">60%</option>
                            <option value="custom">{t('quickSettings.customInput', '自定义')}</option>
                          </select>
                        )}
                      </div>
                    </div>

                    <div className="qs-row">
                      <div className="qs-row-label">
                        <span>{t('quickSettings.autoSwitch.triggerModel', '触发模型')}</span>
                      </div>
                      <div className="qs-row-control">
                        <select
                          className="qs-select"
                          value={autoSwitchScopeMode}
                          onChange={(e) => handleAutoSwitchScopeModeChange(e.target.value)}
                        >
                          <option value="any_group">
                            {t('quickSettings.autoSwitch.scopeAnyGroup', '任一模型分组')}
                          </option>
                          <option value="selected_groups">
                            {t('quickSettings.autoSwitch.scopeSelectedGroups', '指定模型分组')}
                          </option>
                        </select>
                      </div>
                    </div>

                    {autoSwitchScopeMode === 'selected_groups' && (
                      <div className="qs-row qs-row--top">
                        <div className="qs-row-label">
                          <span>{t('quickSettings.autoSwitch.selectedGroups', '指定分组')}</span>
                        </div>
                        <div className="qs-row-control qs-row-control--fill">
                          {autoSwitchDisplayGroups.length === 0 ? (
                            <div className="qs-hint qs-hint--compact">
                              {t('quickSettings.autoSwitch.selectedGroupsEmpty', '暂无可选分组')}
                            </div>
                          ) : (
                            <div className="qs-check-group-inline">
                              {autoSwitchDisplayGroups.map((group) => {
                                const checked = normalizedAutoSwitchSelectedGroupIds.includes(group.id);
                                return (
                                  <label
                                    key={group.id}
                                    className="qs-check-item"
                                  >
                                    <input
                                      type="checkbox"
                                      checked={checked}
                                      onChange={() => handleAutoSwitchGroupToggle(group.id)}
                                    />
                                    <span>{group.name}</span>
                                  </label>
                                );
                              })}
                            </div>
                          )}
                        </div>
                      </div>
                    )}

                    <div className="qs-row qs-row--top">
                      <div className="qs-row-label">
                        <span>{t('settings.general.autoSwitchAccountScope', '自动切号账号范围')}</span>
                      </div>
                      <div className="qs-row-control qs-row-control--fill">
                        <AutoSwitchAccountScopeSelector
                          mode={autoSwitchAccountScopeMode}
                          onModeChange={(mode) =>
                            saveConfig({ auto_switch_account_scope_mode: mode })
                          }
                          selectedAccountIds={autoSwitchSelectedAccountIds}
                          onSelectedAccountIdsChange={(ids) =>
                            saveConfig({ auto_switch_selected_account_ids: ids })
                          }
                          accounts={antigravityScopeAccounts}
                          groups={antigravityScopeGroups}
                          typeOptions={antigravityScopeTypeOptions}
                          useDialog
                        />
                      </div>
                    </div>
                  </div>
                )}

                <div className="qs-hint">
                  {t(
                    'quickSettings.autoSwitch.hint',
                    '当命中监控的模型分组阈值时，自动切换到配额最高的账号。'
                  )}
                </div>

                {renderQuotaAlertControls()}
              </div>
            )}

            {type !== 'antigravity' && (
              <div className="qs-section qs-section--highlight">
                <div className="qs-section-header">
                  <Zap size={15} />
                  <span>{t('quickSettings.quotaAlert.enable', '超额预警')}</span>
                </div>
                {renderQuotaAlertControls()}
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  ) : null;

  return (
    <div className="quick-settings-wrapper">
      <button
        className={`btn btn-secondary icon-only ${isOpen ? 'active' : ''}`}
        onClick={() => setIsOpen(!isOpen)}
        title={getTitle()}
        aria-label={getTitle()}
      >
        <Settings size={14} />
      </button>
      {overlayContent && createPortal(overlayContent, document.body)}
    </div>
  );
}
