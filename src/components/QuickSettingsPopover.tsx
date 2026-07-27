import { useState, useEffect, useRef, useCallback, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { createPortal } from 'react-dom';
import { open } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';
import { Settings, RefreshCw, FolderOpen, Gauge, Terminal, Zap, X } from 'lucide-react';
import { useEscClose } from '../hooks/useEscClose';
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
  getCodexPlanBadgeStyle,
  isCodexAdditionalQuotaVisibleByDefault,
  isCodexCodeReviewQuotaVisibleByDefault,
  persistCodexAdditionalQuotaVisible,
  persistCodexCodeReviewQuotaVisible,
  persistCodexPlanBadgeStyle,
  type CodexPlanBadgeStyle,
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
import type { CodexAccount, CodexQuickConfig } from '../types/codex';
import { getDisplayGroups, type DisplayGroup } from '../services/groupService';
import { usePlatformRuntimeSupport } from '../hooks/usePlatformRuntimeSupport';
import {
  readAccountsOverviewFilterPersistenceEnabled,
  resolveAccountsOverviewScopeFromQuickSettingsType,
  setAccountsOverviewFilterPersistenceEnabled,
} from '../utils/accountsOverviewFilterPersistence';
import { CodexSshSyncSettingsControl } from './codex/CodexSshSyncSettingsControl';
import './QuickSettingsPopover.css';

/** GeneralConfig from backend */
interface GeneralConfig {
  language: string;
  theme: string;
  ui_scale: number;
  auto_refresh_minutes: number;
  codex_auto_refresh_minutes: number;
  claude_auto_refresh_minutes: number;
  codex_sync_wsl: boolean;
  codex_app_ui_injection_enabled?: boolean;
  codex_wsl_config_dir: string;
  ghcp_auto_refresh_minutes: number;
  windsurf_auto_refresh_minutes: number;
  kiro_auto_refresh_minutes: number;
  cursor_auto_refresh_minutes: number;
  grok_auto_refresh_minutes: number;
  grok_sync_official_auth_on_switch: boolean;
  codebuddy_auto_refresh_minutes: number;
  codebuddy_cn_auto_refresh_minutes: number;
  qoder_auto_refresh_minutes: number;
  zcode_auto_refresh_minutes: number;
  trae_auto_refresh_minutes: number;
  trae_solo_auto_refresh_minutes: number;
  trae_cn_auto_refresh_minutes: number;
  trae_solo_cn_auto_refresh_minutes: number;
  workbuddy_auto_refresh_minutes: number;
  zed_auto_refresh_minutes: number;
  close_behavior: string;
  minimize_behavior?: 'dock_and_tray' | 'tray_only';
  hide_dock_icon?: boolean;
  tray_icon_style?: 'template' | 'color';
  opencode_app_path: string;
  antigravity_app_path: string;
  codex_app_path: string;
  claude_app_path: string;
  claude_app_scan_roots: string;
  codex_specified_app_path: string;
  vscode_app_path: string;
  windsurf_app_path: string;
  kiro_app_path: string;
  cursor_app_path: string;
  codebuddy_app_path: string;
  codebuddy_share_sessions_on_switch: boolean;
  codebuddy_cn_app_path: string;
  qoder_app_path: string;
  zcode_app_path: string;
  trae_app_path: string;
  trae_solo_app_path: string;
  trae_cn_app_path: string;
  trae_solo_cn_app_path: string;
  trae_app_scan_roots: string;
  trae_solo_app_scan_roots: string;
  trae_cn_app_scan_roots: string;
  trae_solo_cn_app_scan_roots: string;
  workbuddy_app_path: string;
  workbuddy_share_sessions_on_switch: boolean;
  zed_app_path: string;
  opencode_sync_on_switch: boolean;
  opencode_auth_overwrite_on_switch: boolean;
  ghcp_opencode_sync_on_switch: boolean;
  ghcp_opencode_auth_overwrite_on_switch: boolean;
  ghcp_launch_on_switch: boolean;
  openclaw_auth_overwrite_on_switch: boolean;
  hermes_auth_overwrite_on_switch?: boolean;
  codex_launch_on_switch: boolean;
  antigravity_launch_on_switch: boolean;
  codex_restart_specified_app_on_switch: boolean;
  codex_local_access_entry_visible: boolean;
  antigravity_dual_switch_no_restart_enabled: boolean;
  auto_switch_enabled: boolean;
  auto_switch_threshold: number;
  auto_switch_credits_enabled: boolean;
  auto_switch_credits_threshold: number;
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
  grok_quota_alert_enabled: boolean;
  grok_quota_alert_threshold: number;
  claude_quota_alert_enabled: boolean;
  claude_quota_alert_threshold: number;
  codebuddy_quota_alert_enabled: boolean;
  codebuddy_quota_alert_threshold: number;
  codebuddy_cn_quota_alert_enabled: boolean;
  codebuddy_cn_quota_alert_threshold: number;
  qoder_quota_alert_enabled: boolean;
  qoder_quota_alert_threshold: number;
  trae_quota_alert_enabled: boolean;
  trae_quota_alert_threshold: number;
  trae_solo_quota_alert_enabled: boolean;
  trae_solo_quota_alert_threshold: number;
  trae_cn_quota_alert_enabled: boolean;
  trae_cn_quota_alert_threshold: number;
  trae_solo_cn_quota_alert_enabled: boolean;
  trae_solo_cn_quota_alert_threshold: number;
  workbuddy_quota_alert_enabled: boolean;
  workbuddy_quota_alert_threshold: number;
  zed_quota_alert_enabled: boolean;
  zed_quota_alert_threshold: number;
}

export type QuickSettingsType =
  | 'antigravity'
  | 'codex'
  | 'claude'
  | 'github_copilot'
  | 'windsurf'
  | 'kiro'
  | 'cursor'
  | 'grok'
  | 'codebuddy'
  | 'codebuddy_cn'
  | 'qoder'
  | 'zcode'
  | 'trae'
  | 'trae_solo'
  | 'trae_cn'
  | 'trae_solo_cn'
  | 'workbuddy'
  | 'zed';

type AppPathTarget =
  | 'antigravity'
  | 'antigravity_legacy'
  | 'codex'
  | 'claude'
  | 'vscode'
  | 'windsurf'
  | 'kiro'
  | 'cursor'
  | 'codebuddy'
  | 'codebuddy_cn'
  | 'qoder'
  | 'zcode'
  | 'trae'
  | 'trae_solo'
  | 'trae_cn'
  | 'trae_solo_cn'
  | 'workbuddy'
  | 'zed';

type QuotaAlertEnabledKey =
  | 'quota_alert_enabled'
  | 'codex_quota_alert_enabled'
  | 'claude_quota_alert_enabled'
  | 'ghcp_quota_alert_enabled'
  | 'windsurf_quota_alert_enabled'
  | 'kiro_quota_alert_enabled'
  | 'cursor_quota_alert_enabled'
  | 'grok_quota_alert_enabled'
  | 'codebuddy_quota_alert_enabled'
  | 'codebuddy_cn_quota_alert_enabled'
  | 'qoder_quota_alert_enabled'
  | 'trae_quota_alert_enabled'
  | 'trae_solo_quota_alert_enabled'
  | 'trae_cn_quota_alert_enabled'
  | 'trae_solo_cn_quota_alert_enabled'
  | 'workbuddy_quota_alert_enabled'
  | 'zed_quota_alert_enabled';
type QuotaAlertThresholdKey =
  | 'quota_alert_threshold'
  | 'codex_quota_alert_threshold'
  | 'claude_quota_alert_threshold'
  | 'ghcp_quota_alert_threshold'
  | 'windsurf_quota_alert_threshold'
  | 'kiro_quota_alert_threshold'
  | 'cursor_quota_alert_threshold'
  | 'grok_quota_alert_threshold'
  | 'codebuddy_quota_alert_threshold'
  | 'codebuddy_cn_quota_alert_threshold'
  | 'qoder_quota_alert_threshold'
  | 'trae_quota_alert_threshold'
  | 'trae_solo_quota_alert_threshold'
  | 'trae_cn_quota_alert_threshold'
  | 'trae_solo_cn_quota_alert_threshold'
  | 'workbuddy_quota_alert_threshold'
  | 'zed_quota_alert_threshold';
type CodexWindowThresholdKey =
  | 'codex_auto_switch_primary_threshold'
  | 'codex_auto_switch_secondary_threshold'
  | 'codex_quota_alert_primary_threshold'
  | 'codex_quota_alert_secondary_threshold';

type AppLaunchCandidate = {
  target_type: string;
  label: string;
  target: string;
  source: string;
  supports_multi_instance: boolean;
};

const getAppPathKeyForTarget = (target: AppPathTarget): keyof GeneralConfig => {
  switch (target) {
    case 'antigravity':
    case 'antigravity_legacy':
      return 'antigravity_app_path';
    case 'codex':
      return 'codex_app_path';
    case 'claude':
      return 'claude_app_path';
    case 'vscode':
      return 'vscode_app_path';
    case 'windsurf':
      return 'windsurf_app_path';
    case 'kiro':
      return 'kiro_app_path';
    case 'cursor':
      return 'cursor_app_path';
    case 'codebuddy':
      return 'codebuddy_app_path';
    case 'codebuddy_cn':
      return 'codebuddy_cn_app_path';
    case 'qoder':
      return 'qoder_app_path';
    case 'zcode':
      return 'zcode_app_path';
    case 'trae':
      return 'trae_app_path';
    case 'trae_solo':
      return 'trae_solo_app_path';
    case 'trae_cn':
      return 'trae_cn_app_path';
    case 'trae_solo_cn':
      return 'trae_solo_cn_app_path';
    case 'workbuddy':
      return 'workbuddy_app_path';
    case 'zed':
      return 'zed_app_path';
  }
};

interface QuickSettingsPopoverProps {
  type: QuickSettingsType;
}

const AUTO_SWITCH_SCOPE_ALL_ACCOUNTS: AutoSwitchAccountScopeMode = 'all_accounts';
const AUTO_SWITCH_SCOPE_SELECTED_ACCOUNTS: AutoSwitchAccountScopeMode = 'selected_accounts';
const CURRENT_ACCOUNT_REFRESH_PRESETS = ['1', '2', '5', '10', '15'];
const DEFAULT_AUTO_COMPACT_TOKEN_LIMIT = 900000;
const CONTEXT_WINDOW_516K = 516000;
const AUTO_COMPACT_TOKEN_LIMIT_516K = 460000;
const CONTEXT_WINDOW_1M = 1000000;
const AUTO_COMPACT_TOKEN_LIMIT_1M = 900000;

type CodexQuickConfigBuiltInPresetId = 'default' | 'preset_516k' | 'preset_1m';
type CodexQuickConfigPresetId = CodexQuickConfigBuiltInPresetId | 'custom';

interface CodexQuickConfigTarget {
  modelContextWindow: number | null;
  autoCompactTokenLimit: number | null;
}

const CODEX_QUICK_CONFIG_PRESETS: Record<CodexQuickConfigBuiltInPresetId, CodexQuickConfigTarget> = {
  default: {
    modelContextWindow: null,
    autoCompactTokenLimit: null,
  },
  preset_516k: {
    modelContextWindow: CONTEXT_WINDOW_516K,
    autoCompactTokenLimit: AUTO_COMPACT_TOKEN_LIMIT_516K,
  },
  preset_1m: {
    modelContextWindow: CONTEXT_WINDOW_1M,
    autoCompactTokenLimit: AUTO_COMPACT_TOKEN_LIMIT_1M,
  },
};

function parsePositiveInteger(value: string): number | null {
  const parsed = Number.parseInt(value.trim(), 10);
  if (!Number.isFinite(parsed) || parsed <= 0) return null;
  return parsed;
}

function resolveCodexQuickConfigPresetId(
  modelContextWindow: number | null,
  autoCompactTokenLimit: number | null,
): CodexQuickConfigPresetId {
  if (modelContextWindow === null && autoCompactTokenLimit === null) {
    return 'default';
  }
  if (
    modelContextWindow === CODEX_QUICK_CONFIG_PRESETS.preset_516k.modelContextWindow &&
    autoCompactTokenLimit === CODEX_QUICK_CONFIG_PRESETS.preset_516k.autoCompactTokenLimit
  ) {
    return 'preset_516k';
  }
  if (
    modelContextWindow === CODEX_QUICK_CONFIG_PRESETS.preset_1m.modelContextWindow &&
    autoCompactTokenLimit === CODEX_QUICK_CONFIG_PRESETS.preset_1m.autoCompactTokenLimit
  ) {
    return 'preset_1m';
  }
  return 'custom';
}

const getCurrentAccountRefreshPlatformForType = (
  platformType: QuickSettingsType,
): CurrentAccountRefreshPlatform => {
  switch (platformType) {
    case 'antigravity':
      return 'antigravity';
    case 'codex':
      return 'codex';
    case 'claude':
      return 'claude';
    case 'github_copilot':
      return 'ghcp';
    case 'windsurf':
      return 'windsurf';
    case 'kiro':
      return 'kiro';
    case 'cursor':
      return 'cursor';
    case 'grok':
      return 'grok';
    case 'codebuddy':
      return 'codebuddy';
    case 'codebuddy_cn':
      return 'codebuddy_cn';
    case 'qoder':
      return 'qoder';
    case 'zcode':
      return 'zcode';
    case 'trae':
      return 'trae';
    case 'trae_solo':
      return 'trae_solo';
    case 'trae_cn':
      return 'trae_cn';
    case 'trae_solo_cn':
      return 'trae_solo_cn';
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
  const isWindows = usePlatformRuntimeSupport('windows-only');
  const overviewFilterScope = useMemo(
    () => resolveAccountsOverviewScopeFromQuickSettingsType(type),
    [type],
  );
  const [overviewFilterPersistenceEnabled, setOverviewFilterPersistenceEnabledState] =
    useState<boolean>(() =>
      readAccountsOverviewFilterPersistenceEnabled(overviewFilterScope),
    );
  const [isOpen, setIsOpen] = useState(false);
  const [config, setConfig] = useState<GeneralConfig | null>(null);
  const [pathDetecting, setPathDetecting] = useState(false);
  const [appLaunchCandidates, setAppLaunchCandidates] = useState<AppLaunchCandidate[]>([]);
  const [openingCodexConfig, setOpeningCodexConfig] = useState(false);
  const [codexQuickConfig, setCodexQuickConfig] = useState<CodexQuickConfig | null>(null);
  const [codexQuickConfigPresetId, setCodexQuickConfigPresetId] =
    useState<CodexQuickConfigPresetId>('default');
  const [codexQuickContextWindowInput, setCodexQuickContextWindowInput] = useState(
    String(CONTEXT_WINDOW_1M),
  );
  const [codexQuickCompactLimitInput, setCodexQuickCompactLimitInput] = useState(
    String(DEFAULT_AUTO_COMPACT_TOKEN_LIMIT),
  );
  const [codexQuickConfigLoading, setCodexQuickConfigLoading] = useState(false);
  const [codexQuickConfigSaving, setCodexQuickConfigSaving] = useState(false);
  const [codexQuickConfigError, setCodexQuickConfigError] = useState<string | null>(null);
  const [codexQuickConfigNotice, setCodexQuickConfigNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [refreshEditing, setRefreshEditing] = useState(false);
  const [currentAccountRefreshEditing, setCurrentAccountRefreshEditing] = useState(false);
  const [thresholdEditing, setThresholdEditing] = useState(false);
  const [creditsThresholdEditing, setCreditsThresholdEditing] = useState(false);
  const [quotaAlertThresholdEditing, setQuotaAlertThresholdEditing] = useState(false);
  const [customRefresh, setCustomRefresh] = useState('');
  const [currentAccountCustomRefresh, setCurrentAccountCustomRefresh] = useState('');
  const [customThreshold, setCustomThreshold] = useState('');
  const [customCreditsThreshold, setCustomCreditsThreshold] = useState('');
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
  const [codexShowAdditionalQuota, setCodexShowAdditionalQuota] = useState(
    isCodexAdditionalQuotaVisibleByDefault,
  );
  const [codexPlanBadgeStyle, setCodexPlanBadgeStyle] = useState<CodexPlanBadgeStyle>(
    getCodexPlanBadgeStyle,
  );
  const [currentAccountRefreshMap, setCurrentAccountRefreshMap] =
    useState<CurrentAccountRefreshMinutesMap>(() => buildDefaultCurrentAccountRefreshMinutesMap());
  const [antigravitySeamlessSwitchUnlocked, setAntigravitySeamlessSwitchUnlocked] = useState(
    isAntigravitySeamlessSwitchFeatureUnlocked,
  );
  const modalRef = useRef<HTMLDivElement>(null);
  const configRef = useRef<GeneralConfig | null>(null);
  const configSaveQueueRef = useRef<Promise<void>>(Promise.resolve());
  const configSaveVersionRef = useRef(0);
  const configLoadVersionRef = useRef(0);
  const refreshPresets = ['-1', '2', '5', '10', '15'];
  const thresholdPresets = ['0', '20', '40', '60'];
  const creditsThresholdPresets = ['0', '5', '10', '20'];
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
  const applyCodexQuickConfig = useCallback((nextConfig: CodexQuickConfig) => {
    const detectedModelContextWindow = nextConfig.detected_model_context_window ?? null;
    const detectedAutoCompactTokenLimit = nextConfig.detected_auto_compact_token_limit ?? null;
    const presetId = resolveCodexQuickConfigPresetId(
      detectedModelContextWindow,
      detectedAutoCompactTokenLimit,
    );
    setCodexQuickConfig(nextConfig);
    setCodexQuickConfigPresetId(presetId);
    setCodexQuickContextWindowInput(
      String(detectedModelContextWindow ?? CONTEXT_WINDOW_1M),
    );
    setCodexQuickCompactLimitInput(
      String(detectedAutoCompactTokenLimit ?? DEFAULT_AUTO_COMPACT_TOKEN_LIMIT),
    );
  }, []);

  const loadCodexQuickConfig = useCallback(async () => {
    if (type !== 'codex') {
      setCodexQuickConfig(null);
      setCodexQuickConfigPresetId('default');
      setCodexQuickContextWindowInput(String(CONTEXT_WINDOW_1M));
      setCodexQuickCompactLimitInput(String(DEFAULT_AUTO_COMPACT_TOKEN_LIMIT));
      setCodexQuickConfigError(null);
      setCodexQuickConfigNotice(null);
      setCodexQuickConfigLoading(false);
      setCodexQuickConfigSaving(false);
      return;
    }

    setCodexQuickConfigLoading(true);
    setCodexQuickConfigError(null);
    setCodexQuickConfigNotice(null);
    try {
      const quickConfig = await codexService.getCodexQuickConfig();
      applyCodexQuickConfig(quickConfig);
    } catch (err) {
      setCodexQuickConfigError(
        t('quickSettings.codex.quickConfig.loadFailed', {
          defaultValue: '加载当前 Codex 配置失败：{{error}}',
          error: String(err),
        }),
      );
    } finally {
      setCodexQuickConfigLoading(false);
    }
  }, [applyCodexQuickConfig, t, type]);

  const codexQuickPresetOptions = useMemo(
    () => [
      {
        id: 'default' as CodexQuickConfigPresetId,
        label: t('quickSettings.codex.quickConfig.presetDefaultShort', '默认'),
        desc: t(
          'quickSettings.codex.quickConfig.presetDefaultDesc',
          '移除两个字段，回到官方默认',
        ),
      },
      {
        id: 'preset_516k' as CodexQuickConfigPresetId,
        label: t('quickSettings.codex.quickConfig.preset516kShort', '516K'),
        desc: t(
          'quickSettings.codex.quickConfig.preset516kDesc',
          'context=516000 / compact=460000',
        ),
      },
      {
        id: 'preset_1m' as CodexQuickConfigPresetId,
        label: t('quickSettings.codex.quickConfig.preset1mShort', '1M'),
        desc: t(
          'quickSettings.codex.quickConfig.preset1mDesc',
          'context=1000000 / compact=900000',
        ),
      },
      {
        id: 'custom' as CodexQuickConfigPresetId,
        label: t('quickSettings.codex.quickConfig.presetCustomShort', '自定义'),
        desc: t(
          'quickSettings.codex.quickConfig.presetCustomDesc',
          '手动填写上下文与压缩阈值',
        ),
      },
    ],
    [t],
  );

  const codexQuickIsCustomPreset = codexQuickConfigPresetId === 'custom';
  const codexQuickDetectedModelContextWindow =
    codexQuickConfig?.detected_model_context_window ?? null;
  const codexQuickDetectedAutoCompactTokenLimit =
    codexQuickConfig?.detected_auto_compact_token_limit ?? null;
  const codexQuickParsedContextWindow = useMemo(
    () => parsePositiveInteger(codexQuickContextWindowInput),
    [codexQuickContextWindowInput],
  );
  const codexQuickParsedCompactLimit = useMemo(
    () => parsePositiveInteger(codexQuickCompactLimitInput),
    [codexQuickCompactLimitInput],
  );
  const codexQuickContextWindowError = useMemo(() => {
    if (!codexQuickIsCustomPreset) return null;
    if (codexQuickParsedContextWindow !== null) return null;
    return t(
      'quickSettings.codex.quickConfig.validation.contextWindowInvalid',
      '上下文窗口必须是大于 0 的整数',
    );
  }, [codexQuickIsCustomPreset, codexQuickParsedContextWindow, t]);
  const codexQuickCompactLimitError = useMemo(() => {
    if (!codexQuickIsCustomPreset) return null;
    if (codexQuickParsedCompactLimit !== null) return null;
    return t(
      'quickSettings.codex.quickConfig.validation.autoCompactInvalid',
      '自动压缩阈值必须是大于 0 的整数',
    );
  }, [codexQuickIsCustomPreset, codexQuickParsedCompactLimit, t]);
  const codexQuickValidationError =
    codexQuickContextWindowError ?? codexQuickCompactLimitError;
  const codexQuickTargetConfig = useMemo<CodexQuickConfigTarget>(() => {
    if (codexQuickConfigPresetId === 'custom') {
      return {
        modelContextWindow: codexQuickParsedContextWindow,
        autoCompactTokenLimit: codexQuickParsedCompactLimit,
      };
    }
    return CODEX_QUICK_CONFIG_PRESETS[codexQuickConfigPresetId];
  }, [
    codexQuickConfigPresetId,
    codexQuickParsedCompactLimit,
    codexQuickParsedContextWindow,
  ]);
  const codexQuickDetectedPresetId = useMemo(
    () =>
      resolveCodexQuickConfigPresetId(
        codexQuickDetectedModelContextWindow,
        codexQuickDetectedAutoCompactTokenLimit,
      ),
    [codexQuickDetectedAutoCompactTokenLimit, codexQuickDetectedModelContextWindow],
  );
  const codexQuickConfigDirty = useMemo(() => {
    if (!codexQuickConfig) return false;
    return (
      codexQuickDetectedModelContextWindow !==
        codexQuickTargetConfig.modelContextWindow ||
      codexQuickDetectedAutoCompactTokenLimit !==
        codexQuickTargetConfig.autoCompactTokenLimit
    );
  }, [
    codexQuickConfig,
    codexQuickDetectedAutoCompactTokenLimit,
    codexQuickDetectedModelContextWindow,
    codexQuickTargetConfig.autoCompactTokenLimit,
    codexQuickTargetConfig.modelContextWindow,
  ]);
  const codexQuickConfigWarning = useMemo(() => {
    if (!codexQuickConfig) return null;
    if (
      (codexQuickDetectedModelContextWindow == null) !==
      (codexQuickDetectedAutoCompactTokenLimit == null)
    ) {
      return t('quickSettings.codex.quickConfig.partialDetected', {
        defaultValue:
          '检测到当前两个字段并不完整：model_context_window={{context}}，model_auto_compact_token_limit={{compact}}。保存后会按当前方案改写。',
        context:
          codexQuickDetectedModelContextWindow ??
          t('quickSettings.codex.quickConfig.notSet', '未设置'),
        compact:
          codexQuickDetectedAutoCompactTokenLimit ??
          t('quickSettings.codex.quickConfig.notSet', '未设置'),
      });
    }
    if (codexQuickDetectedPresetId === 'custom' && codexQuickConfigPresetId !== 'custom') {
      return t('quickSettings.codex.quickConfig.customDetected', {
        defaultValue:
          '检测到当前 config.toml 为自定义值：model_context_window={{context}}，model_auto_compact_token_limit={{compact}}。保存后会按你选择的预设改写。',
        context:
          codexQuickDetectedModelContextWindow ??
          t('quickSettings.codex.quickConfig.notSet', '未设置'),
        compact:
          codexQuickDetectedAutoCompactTokenLimit ??
          t('quickSettings.codex.quickConfig.notSet', '未设置'),
      });
    }
    return null;
  }, [
    codexQuickConfig,
    codexQuickConfigPresetId,
    codexQuickDetectedAutoCompactTokenLimit,
    codexQuickDetectedModelContextWindow,
    codexQuickDetectedPresetId,
    t,
  ]);

  const handleCodexQuickPresetChange = useCallback(
    (nextPreset: CodexQuickConfigPresetId) => {
      setCodexQuickConfigNotice(null);
      setCodexQuickConfigError(null);
      setCodexQuickConfigPresetId(nextPreset);
      if (nextPreset !== 'custom') {
        const preset = CODEX_QUICK_CONFIG_PRESETS[nextPreset];
        setCodexQuickContextWindowInput(
          String(preset.modelContextWindow ?? CONTEXT_WINDOW_1M),
        );
        setCodexQuickCompactLimitInput(
          String(
            preset.autoCompactTokenLimit ?? DEFAULT_AUTO_COMPACT_TOKEN_LIMIT,
          ),
        );
      }
    },
    [],
  );

  const handleSaveCodexQuickConfig = useCallback(async () => {
    if (type !== 'codex' || codexQuickConfigLoading || codexQuickConfigSaving) {
      return;
    }
    setCodexQuickConfigError(null);
    setCodexQuickConfigNotice(null);
    if (codexQuickValidationError) {
      setCodexQuickConfigError(codexQuickValidationError);
      return;
    }
    setCodexQuickConfigSaving(true);
    try {
      const saved = await codexService.saveCodexQuickConfig(
        codexQuickTargetConfig.modelContextWindow ?? undefined,
        codexQuickTargetConfig.autoCompactTokenLimit ?? undefined,
      );
      applyCodexQuickConfig(saved);
      setCodexQuickConfigNotice(
        t(
          'quickSettings.codex.quickConfig.saveSuccess',
          '当前 Codex 配置已保存',
        ),
      );
      window.dispatchEvent(new Event('config-updated'));
    } catch (err) {
      setCodexQuickConfigError(
        t('quickSettings.codex.quickConfig.saveFailed', {
          defaultValue: '保存当前 Codex 配置失败：{{error}}',
          error: String(err),
        }),
      );
    } finally {
      setCodexQuickConfigSaving(false);
    }
  }, [
    applyCodexQuickConfig,
    codexQuickConfigLoading,
    codexQuickConfigSaving,
    codexQuickTargetConfig.autoCompactTokenLimit,
    codexQuickTargetConfig.modelContextWindow,
    codexQuickValidationError,
    t,
    type,
  ]);

  const handleOverviewFilterPersistenceToggle = useCallback(
    (checked: boolean) => {
      setOverviewFilterPersistenceEnabledState(checked);
      setAccountsOverviewFilterPersistenceEnabled(overviewFilterScope, checked);
    },
    [overviewFilterScope],
  );

  // Load config when modal opens
  useEffect(() => {
    if (isOpen) {
      loadConfig();
      if (type === 'codex') {
        void loadCodexQuickConfig();
      }
      setCodexShowCodeReviewQuota(isCodexCodeReviewQuotaVisibleByDefault());
      setAntigravitySeamlessSwitchUnlocked(isAntigravitySeamlessSwitchFeatureUnlocked());
      setOverviewFilterPersistenceEnabledState(
        readAccountsOverviewFilterPersistenceEnabled(overviewFilterScope),
      );
    } else {
      configLoadVersionRef.current += 1;
      configRef.current = null;
      setConfig(null);
    }
  }, [isOpen, loadCodexQuickConfig, overviewFilterScope, type]);

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

  useEscClose(isOpen, () => setIsOpen(false));

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
    const loadVersion = configLoadVersionRef.current + 1;
    configLoadVersionRef.current = loadVersion;
    try {
      while (true) {
        const pendingSaves = configSaveQueueRef.current;
        await pendingSaves;
        if (pendingSaves === configSaveQueueRef.current) {
          break;
        }
      }
      if (loadVersion !== configLoadVersionRef.current) {
        return;
      }
      const saveVersionAtStart = configSaveVersionRef.current;
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
      if (
        loadVersion !== configLoadVersionRef.current ||
        saveVersionAtStart !== configSaveVersionRef.current
      ) {
        return;
      }
      configRef.current = cfg;
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
      setAppLaunchCandidates([]);
    } catch (err) {
      if (loadVersion !== configLoadVersionRef.current) {
        return;
      }
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
      case 'claude': return 'claude_auto_refresh_minutes';
      case 'github_copilot': return 'ghcp_auto_refresh_minutes';
      case 'windsurf': return 'windsurf_auto_refresh_minutes';
      case 'kiro': return 'kiro_auto_refresh_minutes';
      case 'cursor': return 'cursor_auto_refresh_minutes';
      case 'grok': return 'grok_auto_refresh_minutes';
      case 'codebuddy': return 'codebuddy_auto_refresh_minutes';
      case 'codebuddy_cn': return 'codebuddy_cn_auto_refresh_minutes';
      case 'qoder': return 'qoder_auto_refresh_minutes';
      case 'zcode': return 'zcode_auto_refresh_minutes';
      case 'trae': return 'trae_auto_refresh_minutes';
      case 'trae_solo': return 'trae_solo_auto_refresh_minutes';
      case 'trae_cn': return 'trae_cn_auto_refresh_minutes';
      case 'trae_solo_cn': return 'trae_solo_cn_auto_refresh_minutes';
      case 'workbuddy': return 'workbuddy_auto_refresh_minutes';
      case 'zed': return 'zed_auto_refresh_minutes';
      default: return 'auto_refresh_minutes';
    }
  };

  const saveConfig = useCallback(
    async (updates: Partial<GeneralConfig>) => {
      const current = configRef.current;
      if (!current) return;
      const optimisticConfig = { ...current, ...updates };
      configRef.current = optimisticConfig;
      setConfig(optimisticConfig);
      setError(null);
      const saveVersion = configSaveVersionRef.current + 1;
      configSaveVersionRef.current = saveVersion;

      const operation = configSaveQueueRef.current.then(async () => {
        const latest = await invoke<GeneralConfig>('get_general_config');
        const merged = { ...latest, ...updates };
        await invoke('patch_general_config', { updates });
        if (saveVersion === configSaveVersionRef.current) {
          configRef.current = merged;
          setConfig(merged);
        }
        window.dispatchEvent(new Event('config-updated'));
      }).catch((err) => {
        console.error('Failed to save config:', err);
        setError(t('quickSettings.error.saveFailed', {
          error: String(err),
          defaultValue: '保存配置失败：{{error}}',
        }));
        if (saveVersion === configSaveVersionRef.current) {
          void loadConfig();
        }
      });

      configSaveQueueRef.current = operation;
      await operation;
    },
    [t]
  );

  const handlePickAppPath = async (target: AppPathTarget) => {
    try {
      const selected = await open({ multiple: false, directory: false });
      const path = Array.isArray(selected) ? selected[0] : selected;
      if (!path || !config) return;

      setAppLaunchCandidates([]);
      saveConfig({ [getAppPathKeyForTarget(target)]: path });
    } catch (err) {
      console.error('Failed to pick path:', err);
      setError(t('quickSettings.error.pickPathFailed', {
        error: String(err),
        defaultValue: '选择路径失败：{{error}}',
      }));
    }
  };

  const handleResetAppPath = async (target: AppPathTarget) => {
    if (pathDetecting) return;
    if (isWindows) {
      setPathDetecting(true);
      setError(null);
      try {
        const candidates = await invoke<AppLaunchCandidate[]>('scan_app_launch_targets', {
          app: target,
        });
        setAppLaunchCandidates(candidates);
        if (candidates.length === 0) {
          setError(
            t(
              'quickSettings.appPath.scanEmpty',
              '未检测到正在运行的应用，请先启动后重试，或手动选择路径。',
            ),
          );
        }
      } catch (err) {
        console.error('Failed to scan app launch targets:', err);
        setError(t('quickSettings.error.resetPathFailed', {
          error: String(err),
          defaultValue: '重置路径失败：{{error}}',
        }));
      } finally {
        setPathDetecting(false);
      }
      return;
    }
    setPathDetecting(true);
    setError(null);
    try {
      const detected = await invoke<string | null>('detect_app_path', { app: target, force: true });
      setAppLaunchCandidates([]);
      saveConfig({ [getAppPathKeyForTarget(target)]: detected || '' });
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

  const handleSelectAppLaunchCandidate = (candidate: AppLaunchCandidate) => {
    setError(null);
    saveConfig({ [getAppPathKeyForTarget(getAppTarget())]: candidate.target });
  };

  const handlePickCodexSpecifiedAppPath = async () => {
    try {
      const selected = await open({ multiple: false, directory: false });
      const path = Array.isArray(selected) ? selected[0] : selected;
      if (!path) return;
      saveConfig({ codex_specified_app_path: path });
    } catch (err) {
      console.error('Failed to pick codex specified app path:', err);
      setError(t('quickSettings.error.pickPathFailed', {
        error: String(err),
        defaultValue: '选择路径失败：{{error}}',
      }));
    }
  };

  const handleOpenCodexConfigToml = useCallback(async () => {
    if (openingCodexConfig) return;
    setOpeningCodexConfig(true);
    try {
      await codexService.openCodexConfigToml();
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
        case 'claude':
          return 'Claude';
        case 'github_copilot':
          return 'GitHub Copilot';
        case 'windsurf':
          return 'Devin';
        case 'kiro':
          return 'Kiro';
        case 'cursor':
          return 'Cursor';
        case 'grok':
          return 'Grok CLI';
        case 'codebuddy':
          return 'CodeBuddy';
        case 'codebuddy_cn':
          return 'CodeBuddy CN';
        case 'qoder':
          return 'Qoder';
        case 'zcode':
          return 'ZCode';
        case 'trae':
          return 'Trae';
        case 'trae_solo':
          return 'TRAE SOLO';
        case 'trae_cn':
          return 'Trae CN';
        case 'trae_solo_cn':
          return 'TRAE SOLO CN';
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
      case 'claude':
        return 'claude_quota_alert_enabled';
      case 'github_copilot':
        return 'ghcp_quota_alert_enabled';
      case 'windsurf':
        return 'windsurf_quota_alert_enabled';
      case 'kiro':
        return 'kiro_quota_alert_enabled';
      case 'cursor':
        return 'cursor_quota_alert_enabled';
      case 'grok':
        return 'grok_quota_alert_enabled';
      case 'codebuddy':
        return 'codebuddy_quota_alert_enabled';
      case 'codebuddy_cn':
        return 'codebuddy_cn_quota_alert_enabled';
      case 'qoder':
        return 'qoder_quota_alert_enabled';
      case 'trae':
        return 'trae_quota_alert_enabled';
      case 'trae_solo':
        return 'trae_solo_quota_alert_enabled';
      case 'trae_cn':
        return 'trae_cn_quota_alert_enabled';
      case 'trae_solo_cn':
        return 'trae_solo_cn_quota_alert_enabled';
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
      case 'claude':
        return 'claude_quota_alert_threshold';
      case 'github_copilot':
        return 'ghcp_quota_alert_threshold';
      case 'windsurf':
        return 'windsurf_quota_alert_threshold';
      case 'kiro':
        return 'kiro_quota_alert_threshold';
      case 'cursor':
        return 'cursor_quota_alert_threshold';
      case 'grok':
        return 'grok_quota_alert_threshold';
      case 'codebuddy':
        return 'codebuddy_quota_alert_threshold';
      case 'codebuddy_cn':
        return 'codebuddy_cn_quota_alert_threshold';
      case 'qoder':
        return 'qoder_quota_alert_threshold';
      case 'trae':
        return 'trae_quota_alert_threshold';
      case 'trae_solo':
        return 'trae_solo_quota_alert_threshold';
      case 'trae_cn':
        return 'trae_cn_quota_alert_threshold';
      case 'trae_solo_cn':
        return 'trae_solo_cn_quota_alert_threshold';
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
      case 'claude':
        return t('quickSettings.claudeRefreshInterval', '配额自动刷新');
      case 'github_copilot':
        return t('quickSettings.ghcpRefreshInterval', '配额自动刷新');
      case 'windsurf':
        return t('quickSettings.windsurfRefreshInterval', '配额自动刷新');
      case 'kiro':
        return t('quickSettings.kiroRefreshInterval', '配额自动刷新');
      case 'cursor':
        return t('quickSettings.cursorRefreshInterval', '配额自动刷新');
      case 'grok':
        return t('quickSettings.refreshInterval', '配额自动刷新');
      case 'codebuddy':
        return t('quickSettings.refreshInterval', '配额自动刷新');
      case 'codebuddy_cn':
        return t('quickSettings.refreshInterval', '配额自动刷新');
      case 'qoder':
        return t('quickSettings.refreshInterval', '配额自动刷新');
      case 'zcode':
        return t('quickSettings.refreshInterval', '配额自动刷新');
      case 'trae':
      case 'trae_solo':
      case 'trae_cn':
      case 'trae_solo_cn':
        return t('quickSettings.refreshInterval', '配额自动刷新');
      case 'workbuddy':
        return t('quickSettings.refreshInterval', '配额自动刷新');
      case 'zed':
        return t('quickSettings.refreshInterval', '配额自动刷新');
    }
  };

  const showAppPathSection = type !== 'grok';
  const antigravityLaunchOnSwitch = config?.antigravity_launch_on_switch ?? true;

  const getAppPath = (): string => {
    if (!config) return '';
    switch (type) {
      case 'antigravity':
        return config.antigravity_app_path;
      case 'codex':
        return config.codex_app_path;
      case 'claude':
        return config.claude_app_path;
      case 'github_copilot':
        return config.vscode_app_path;
      case 'windsurf':
        return config.windsurf_app_path;
      case 'kiro':
        return config.kiro_app_path;
      case 'cursor':
        return config.cursor_app_path;
      case 'grok':
        return '';
      case 'codebuddy':
        return config.codebuddy_app_path;
      case 'codebuddy_cn':
        return config.codebuddy_cn_app_path;
      case 'qoder':
        return config.qoder_app_path;
      case 'zcode':
        return config.zcode_app_path || '';
      case 'trae':
        return config.trae_app_path;
      case 'trae_solo':
        return config.trae_solo_app_path;
      case 'trae_cn':
        return config.trae_cn_app_path;
      case 'trae_solo_cn':
        return config.trae_solo_cn_app_path;
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
      case 'claude':
        return t('quickSettings.claude.appPath', 'Claude 启动路径');
      case 'github_copilot':
        return t('quickSettings.githubCopilot.appPath', 'VS Code 路径');
      case 'windsurf':
        return t('quickSettings.windsurf.appPath', 'Devin 路径');
      case 'kiro':
        return t('quickSettings.kiro.appPath', 'Kiro 路径');
      case 'cursor':
        return t('quickSettings.cursor.appPath', 'Cursor 路径');
      case 'grok':
        return t('quickSettings.grok.appPath', 'Grok CLI 路径');
      case 'codebuddy':
        return t('quickSettings.codebuddy.appPath', 'CodeBuddy 路径');
      case 'codebuddy_cn':
        return t('quickSettings.codebuddyCn.appPath', 'CodeBuddy CN 路径');
      case 'qoder':
        return t('quickSettings.qoder.appPath', 'Qoder 路径');
      case 'zcode':
        return t('quickSettings.zcode.appPath', 'ZCode 启动路径');
      case 'trae':
        return t('quickSettings.trae.appPath', 'Trae 路径');
      case 'trae_solo':
        return t('quickSettings.traeSolo.appPath', 'TRAE SOLO 路径');
      case 'trae_cn':
        return t('quickSettings.traeCn.appPath', 'Trae CN 路径');
      case 'trae_solo_cn':
        return t('quickSettings.traeSoloCn.appPath', 'TRAE SOLO CN 路径');
      case 'workbuddy':
        return t('quickSettings.workbuddy.appPath', 'WorkBuddy 路径');
      case 'zed':
        return t('quickSettings.zed.appPath', 'Zed 路径');
    }
  };

  const getAppTarget = (): AppPathTarget => {
    switch (type) {
      case 'antigravity':
        return 'antigravity_legacy';
      case 'codex':
        return 'codex';
      case 'claude':
        return 'claude';
      case 'github_copilot':
        return 'vscode';
      case 'windsurf':
        return 'windsurf';
      case 'kiro':
        return 'kiro';
      case 'cursor':
        return 'cursor';
      case 'grok':
        return 'antigravity';
      case 'codebuddy':
        return 'codebuddy';
      case 'codebuddy_cn':
        return 'codebuddy_cn';
      case 'qoder':
        return 'qoder';
      case 'zcode':
        return 'zcode';
      case 'trae':
        return 'trae';
      case 'trae_solo':
        return 'trae_solo';
      case 'trae_cn':
        return 'trae_cn';
      case 'trae_solo_cn':
        return 'trae_solo_cn';
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
  const creditsAutoSwitchEnabled = config?.auto_switch_credits_enabled ?? false;
  const creditsAutoSwitchThresholdValue = config ? Number(config.auto_switch_credits_threshold) : 5;
  const isCreditsThresholdPreset = creditsThresholdPresets.includes(
    String(creditsAutoSwitchThresholdValue),
  );
  const showCreditsThresholdInput = creditsThresholdEditing;
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

  // Drop stale selected-ID lists when scope is "all accounts" (runtime already ignores them).
  useEffect(() => {
    if (!config) return;
    if (
      type === 'codex' &&
      codexAutoSwitchAccountScopeMode === AUTO_SWITCH_SCOPE_ALL_ACCOUNTS &&
      codexAutoSwitchSelectedAccountIds.length > 0
    ) {
      void saveConfig({ codex_auto_switch_selected_account_ids: [] });
      return;
    }
    if (
      type === 'antigravity' &&
      autoSwitchAccountScopeMode === AUTO_SWITCH_SCOPE_ALL_ACCOUNTS &&
      autoSwitchSelectedAccountIds.length > 0
    ) {
      void saveConfig({ auto_switch_selected_account_ids: [] });
    }
  }, [
    autoSwitchAccountScopeMode,
    autoSwitchSelectedAccountIds.length,
    codexAutoSwitchAccountScopeMode,
    codexAutoSwitchSelectedAccountIds.length,
    config,
    saveConfig,
    type,
  ]);

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

  const handleCreditsThresholdSelectChange = (val: string) => {
    if (val === 'custom') {
      setCustomCreditsThreshold(String(creditsAutoSwitchThresholdValue));
      setCreditsThresholdEditing(true);
      return;
    }
    setCustomCreditsThreshold('');
    setCreditsThresholdEditing(false);
    saveConfig({ auto_switch_credits_threshold: parseInt(val, 10) });
  };

  const handleCustomCreditsThresholdApply = () => {
    const parsed = parseInt(customCreditsThreshold, 10);
    if (!isNaN(parsed) && parsed >= 0) {
      saveConfig({ auto_switch_credits_threshold: parsed });
      setCustomCreditsThreshold('');
      setCreditsThresholdEditing(false);
      return;
    }
    setCustomCreditsThreshold('');
    setCreditsThresholdEditing(false);
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
    const isGrokAlert = type === 'grok';
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
              {isGrokAlert
                ? t(
                    'grok.quotaAlert.hint',
                    '当当前账号任意配额项低于阈值时，发送原生通知并在页面提示快捷切号。',
                  )
                : t(
                    'quickSettings.quotaAlert.hint',
                    '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。',
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

  const handleCodexAdditionalQuotaToggle = (checked: boolean) => {
    setCodexShowAdditionalQuota(checked);
    persistCodexAdditionalQuotaVisible(checked);
  };

  const handleCodexPlanBadgeStyleChange = (style: CodexPlanBadgeStyle) => {
    setCodexPlanBadgeStyle(style);
    persistCodexPlanBadgeStyle(style);
  };

  const overlayContent = isOpen ? (
    <div className="qs-overlay">
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
            {type === 'grok' && (
              <div className="qs-section">
                <div className="qs-row">
                  <div className="qs-row-label">
                    <span>
                      {t(
                        'quickSettings.grok.syncOfficialAuthOnSwitch',
                        '切号同步官方登录',
                      )}
                    </span>
                  </div>
                  <div className="qs-row-control">
                    <label className="qs-switch">
                      <input
                        type="checkbox"
                        checked={config.grok_sync_official_auth_on_switch}
                        onChange={(event) =>
                          saveConfig({
                            grok_sync_official_auth_on_switch: event.target.checked,
                          })
                        }
                      />
                      <span className="qs-switch-slider"></span>
                    </label>
                  </div>
                </div>
                <div className="qs-hint">
                  {t(
                    'quickSettings.grok.syncOfficialAuthOnSwitchDesc',
                    '开启后，默认实例切换 OAuth 账号会写入官方 ~/.grok/auth.json；关闭时使用独立 GROK_HOME。API Key 和多开实例不改写官方登录。',
                  )}
                </div>
              </div>
            )}
            {type === 'codex' && (
              <div className="qs-section">
                <div className="qs-row">
                  <div className="qs-row-label">
                    <FolderOpen size={15} />
                    <span>
                      {t(
                        'settings.general.codexLocalAccessEntryVisible',
                        '显示 API 服务入口',
                      )}
                    </span>
                  </div>
                  <div className="qs-row-control">
                    <label className="qs-switch">
                      <input
                        type="checkbox"
                        checked={config.codex_local_access_entry_visible}
                        onChange={(e) =>
                          saveConfig({ codex_local_access_entry_visible: e.target.checked })
                        }
                      />
                      <span className="qs-switch-slider"></span>
                    </label>
                  </div>
                </div>
                <div className="qs-hint">
                  {t(
                    'settings.general.codexLocalAccessEntryVisibleDesc',
                    '仅控制 Codex 总览中的 API 服务入口显示，不会停止本地 API 服务；关闭后可在这里重新打开。',
                  )}
                </div>
                <div className="qs-row" style={{ marginTop: 8 }}>
                  <div className="qs-row-label">
                    <Gauge size={15} />
                    <span>
                      {t(
                        'settings.general.codexAppUiInjection',
                        '显示 API 服务额度',
                      )}
                    </span>
                  </div>
                  <div className="qs-row-control">
                    <label className="qs-switch">
                      <input
                        type="checkbox"
                        checked={Boolean(config.codex_app_ui_injection_enabled)}
                        onChange={(event) =>
                          saveConfig({
                            codex_app_ui_injection_enabled: event.target.checked,
                          })
                        }
                      />
                      <span className="qs-switch-slider"></span>
                    </label>
                  </div>
                </div>
                <div className="qs-hint">
                  {t(
                    'settings.general.codexAppUiInjectionDesc',
                    '重启 Codex 实例后，在输入框下方显示 Cockpit Tools API 服务的账号数、周额度和 5h 额度。需保持 Cockpit Tools 在后台运行；完全退出或网络不可用时，额度不会继续刷新。',
                  )}
                </div>
                {isWindows && (
                  <>
                    <div className="qs-row" style={{ marginTop: 8 }}>
                      <div className="qs-row-label">
                        <Terminal size={15} />
                        <span>{t('settings.general.codexSyncWsl', '同步 Codex 到 WSL')}</span>
                      </div>
                      <div className="qs-row-control">
                        <label className="qs-switch">
                          <input
                            type="checkbox"
                            checked={config.codex_sync_wsl}
                            onChange={(e) =>
                              saveConfig({ codex_sync_wsl: e.target.checked })
                            }
                          />
                          <span className="qs-switch-slider"></span>
                        </label>
                      </div>
                    </div>
                    <div className="qs-hint">
                      {t(
                        'settings.general.codexSyncWslDesc',
                        '切换默认 Codex 账号后，同时写入 WSL 的 Codex 配置目录。',
                      )}
                    </div>
                    {config.codex_sync_wsl && (
                      <div className="qs-path-control" style={{ marginTop: 8 }}>
                        <input
                          type="text"
                          className="qs-path-input"
                          value={config.codex_wsl_config_dir}
                          placeholder={t(
                            'settings.general.codexWslConfigDirPlaceholder',
                            '\\\\wsl.localhost\\Ubuntu-24.04\\home\\user\\.codex',
                          )}
                          onChange={(e) =>
                            saveConfig({ codex_wsl_config_dir: e.target.value })
                          }
                        />
                      </div>
                    )}
                  </>
                )}
                <CodexSshSyncSettingsControl variant="quick" />
              </div>
            )}

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

            <div className="qs-section">
              <div className="qs-section-header">
                <Settings size={15} />
                <span>{t('quickSettings.filterPersistence.title', '筛选记忆')}</span>
              </div>
              <div className="qs-row">
                <div className="qs-row-label">
                  <span>
                    {t(
                      'quickSettings.filterPersistence.enable',
                      '记住账号总览筛选（不含搜索）',
                    )}
                  </span>
                </div>
                <div className="qs-row-control">
                  <label className="qs-switch">
                    <input
                      type="checkbox"
                      checked={overviewFilterPersistenceEnabled}
                      onChange={(event) =>
                        handleOverviewFilterPersistenceToggle(event.target.checked)
                      }
                    />
                    <span className="qs-switch-slider"></span>
                  </label>
                </div>
              </div>
              <div className="qs-hint">
                {t(
                  'quickSettings.filterPersistence.hint',
                  '默认关闭。开启后会按平台记住筛选、标签和排序。',
                )}
              </div>
            </div>

            {/* ─── App Path ─── */}
            {showAppPathSection && (
              <div className="qs-section">
	                <div className="qs-section-header">
	                  <FolderOpen size={15} />
	                  <span>{getAppPathLabel()}</span>
	                </div>
                {type === 'antigravity' && config && (
                  <>
                    <div className="qs-row">
                      <div className="qs-row-label">
                        <span>
                          {t(
                            'settings.general.antigravityLaunchOnSwitch',
                            '切换时启动 Antigravity',
                          )}
                        </span>
                      </div>
                      <div className="qs-row-control">
                        <label className="qs-switch">
                          <input
                            type="checkbox"
                            checked={antigravityLaunchOnSwitch}
                            onChange={(event) =>
                              saveConfig({
                                antigravity_launch_on_switch: event.target.checked,
                              })
                            }
                          />
                          <span className="qs-switch-slider"></span>
                        </label>
                      </div>
                    </div>
                    <div className="qs-hint">
                      {t(
                        'settings.general.antigravityLaunchOnSwitchDesc',
                        '关闭后切号只写入 Antigravity 默认账号数据，不会关闭、启动或重启应用。',
                      )}
                    </div>
                  </>
                )}
                {config && (type !== 'antigravity' || antigravityLaunchOnSwitch) && (
	                <div className="qs-path-control">
                  <input
                    type="text"
                    className="qs-path-input"
                    value={getAppPath()}
                    placeholder={
                      type === 'claude'
                        ? t(
                            'quickSettings.claude.appTargetPlaceholder',
                            'Claude.exe 路径或 shell:AppsFolder\\...',
                          )
                        : t('settings.general.codexAppPathPlaceholder', '默认路径')
                    }
                    onChange={(e) => {
                      setAppLaunchCandidates([]);
                      saveConfig({ [getAppPathKeyForTarget(getAppTarget())]: e.target.value });
                    }}
                  />
                  <div className="qs-path-actions">
                    {type === 'zcode' && (
                      <button
                        className="qs-btn"
                        onClick={() => {
                          setAppLaunchCandidates([]);
                          saveConfig({ zcode_app_path: '' });
                        }}
                        disabled={pathDetecting || !getAppPath().trim()}
                        title={t('common.clear', '清除')}
                      >
                        {t('common.clear', '清除')}
                      </button>
                    )}
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
                          : isWindows
                            ? t('appPath.missing.scanApps', '检测运行中应用')
                            : t('settings.general.codexPathReset', '恢复默认')
                      }
                    >
                      {isWindows ? (
                        pathDetecting
                          ? t('common.loading', '加载中...')
                          : t('appPath.missing.scanApps', '检测运行中应用')
                      ) : (
                        <RefreshCw size={12} className={pathDetecting ? 'spin' : undefined} />
                      )}
                    </button>
	                  </div>
	                </div>
                )}

	                {isWindows && config && (
                  <>
                    {appLaunchCandidates.length > 0 && (
                      <div className="qs-claude-candidate-list">
                        {appLaunchCandidates.map((candidate) => (
                          <button
                            key={`${candidate.target_type}:${candidate.target}`}
                            type="button"
                            className={`qs-claude-candidate-item${
                              getAppPath().trim() === candidate.target ? ' selected' : ''
                            }`}
                            onClick={() => handleSelectAppLaunchCandidate(candidate)}
                          >
                            <div className="qs-claude-candidate-main">
                              <span>{candidate.label || getTitle()}</span>
                              <span className="qs-claude-candidate-badge">
                                {candidate.target_type === 'windows_app'
                                  ? t('appPath.missing.windowsApp', 'Microsoft Store')
                                  : 'EXE'}
                              </span>
                            </div>
                            <div className="qs-claude-candidate-target">{candidate.target}</div>
                            {!candidate.supports_multi_instance ? (
                              <div className="qs-claude-candidate-note">
                                {t(
                                  'appPath.missing.defaultOnly',
                                  '仅适用于默认桌面端；应用多开请选择真实 Claude.exe',
                                )}
                              </div>
                            ) : null}
                          </button>
                        ))}
                      </div>
                    )}
                  </>
                )}

                {type === 'codex' && (
                  <div className="qs-codex-quick-settings">
                    <div className="qs-row" style={{ marginTop: 8 }}>
                      <div className="qs-row-label">
                        <Zap size={15} />
                        <span>
                          {t(
                            'settings.general.codexRestartSpecifiedAppOnSwitch',
                            '切换 Codex 时重启指定应用',
                          )}
                        </span>
                      </div>
                      <div className="qs-row-control">
                        <label className="qs-switch">
                          <input
                            type="checkbox"
                            checked={config.codex_restart_specified_app_on_switch}
                            onChange={(e) =>
                              saveConfig({ codex_restart_specified_app_on_switch: e.target.checked })
                            }
                          />
                          <span className="qs-switch-slider"></span>
                        </label>
                      </div>
                    </div>

                    {config.codex_restart_specified_app_on_switch && (
                      <div className="qs-path-control">
                        <input
                          type="text"
                          className="qs-path-input"
                          value={config.codex_specified_app_path}
                          placeholder={t(
                            'settings.general.codexSpecifiedAppPathPlaceholder',
                            '例如 /Applications/Host.app',
                          )}
                          onChange={(e) =>
                            saveConfig({ codex_specified_app_path: e.target.value })
                          }
                        />
                        <div className="qs-path-actions">
                          <button
                            className="qs-btn"
                            onClick={() => void handlePickCodexSpecifiedAppPath()}
                            title={t('settings.general.codexPathSelect', '选择')}
                          >
                            {t('settings.general.codexPathSelect', '选择')}
                          </button>
                          <button
                            className="qs-btn"
                            onClick={() => saveConfig({ codex_specified_app_path: '' })}
                            title={t('settings.general.codexPathReset', '恢复默认')}
                          >
                            <RefreshCw size={12} />
                          </button>
                        </div>
                      </div>
                    )}
              </div>
            )}

            {type === 'codebuddy' && (
              <div className="qs-section">
                <div className="qs-row qs-row--top">
                  <div className="qs-row-label">
                    <Zap size={15} />
                    <span>{t('settings.general.codebuddyShareSessionsOnSwitch')}</span>
                  </div>
                  <div className="qs-row-control">
                    <label className="qs-switch">
                      <input
                        type="checkbox"
                        checked={config.codebuddy_share_sessions_on_switch ?? false}
                        onChange={(event) =>
                          saveConfig({
                            codebuddy_share_sessions_on_switch: event.target.checked,
                          })
                        }
                      />
                      <span className="qs-switch-slider"></span>
                    </label>
                  </div>
                </div>
                <div className="qs-hint">
                  {t('settings.general.codebuddyShareSessionsOnSwitchDesc')}
                </div>
              </div>
            )}

            {type === 'workbuddy' && (
              <div className="qs-section">
                <div className="qs-row qs-row--top">
                  <div className="qs-row-label">
                    <Zap size={15} />
                    <span>{t('settings.general.workbuddyShareSessionsOnSwitch')}</span>
                  </div>
                  <div className="qs-row-control">
                    <label className="qs-switch">
                      <input
                        type="checkbox"
                        checked={config.workbuddy_share_sessions_on_switch ?? false}
                        onChange={(event) =>
                          saveConfig({
                            workbuddy_share_sessions_on_switch: event.target.checked,
                          })
                        }
                      />
                      <span className="qs-switch-slider"></span>
                    </label>
                  </div>
                </div>
                <div className="qs-hint">
                  {t('settings.general.workbuddyShareSessionsOnSwitchDesc')}
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

                <div className="qs-codex-quick-config">
                  <div className="qs-row qs-row--top">
                    <div className="qs-row-label">
                      <Zap size={15} />
                      <span>
                        {t(
                          'quickSettings.codex.quickConfig.title',
                          '上下文与压缩阈值',
                        )}
                      </span>
                    </div>
                    <div className="qs-row-control">
                      <button
                        className="qs-btn"
                        onClick={() => void loadCodexQuickConfig()}
                        disabled={codexQuickConfigLoading || codexQuickConfigSaving}
                      >
                        {codexQuickConfigLoading
                          ? t('common.loading', '加载中...')
                          : t('common.refresh', '刷新')}
                      </button>
                      <button
                        className="qs-btn qs-btn--primary"
                        onClick={() => void handleSaveCodexQuickConfig()}
                        disabled={
                          codexQuickConfigLoading ||
                          codexQuickConfigSaving ||
                          !codexQuickConfigDirty ||
                          Boolean(codexQuickValidationError)
                        }
                      >
                        {codexQuickConfigSaving
                          ? t('common.saving', '保存中...')
                          : t('common.save', '保存')}
                      </button>
                    </div>
                  </div>

                  {codexQuickConfigLoading ? (
                    <div className="qs-hint">{t('common.loading', '加载中...')}</div>
                  ) : (
                    <>
                      <div
                        className="qs-codex-quick-preset-group"
                        role="radiogroup"
                        aria-label={t(
                          'quickSettings.codex.quickConfig.presetLabel',
                          '配置预设',
                        )}
                      >
                        {codexQuickPresetOptions.map((option) => (
                          <button
                            key={option.id}
                            type="button"
                            role="radio"
                            aria-checked={codexQuickConfigPresetId === option.id}
                            className={`qs-codex-quick-preset-btn ${
                              codexQuickConfigPresetId === option.id ? 'active' : ''
                            }`}
                            onClick={() => handleCodexQuickPresetChange(option.id)}
                            disabled={codexQuickConfigSaving}
                          >
                            <span className="qs-codex-quick-preset-btn__label">
                              {option.label}
                            </span>
                            <span className="qs-codex-quick-preset-btn__desc">
                              {option.desc}
                            </span>
                          </button>
                        ))}
                      </div>
                      <div className="qs-hint">
                        {t(
                          'quickSettings.codex.quickConfig.presetHint',
                          '可直接选择预设（默认 / 516K / 1M），或切到自定义手动填写两个字段。',
                        )}
                      </div>

                      <div className="qs-codex-quick-fields">
                        <div className="qs-codex-quick-field">
                          <label>
                            {t(
                              'quickSettings.codex.quickConfig.contextWindow',
                              '上下文窗口',
                            )}
                          </label>
                          <input
                            className="qs-select qs-select--input-mode"
                            type="text"
                            inputMode="numeric"
                            value={codexQuickContextWindowInput}
                            onChange={(event) => {
                              setCodexQuickConfigError(null);
                              setCodexQuickConfigNotice(null);
                              setCodexQuickContextWindowInput(event.target.value);
                            }}
                            disabled={!codexQuickIsCustomPreset || codexQuickConfigSaving}
                            placeholder={String(CONTEXT_WINDOW_1M)}
                          />
                          <div className="qs-hint">
                            {t(
                              'quickSettings.codex.quickConfig.contextWindowHint',
                              '写入 model_context_window。仅在“自定义”模式可编辑。',
                            )}
                          </div>
                          {codexQuickContextWindowError && (
                            <div className="qs-codex-quick-field-error">
                              {codexQuickContextWindowError}
                            </div>
                          )}
                        </div>

                        <div className="qs-codex-quick-field">
                          <label>
                            {t(
                              'quickSettings.codex.quickConfig.autoCompactLimit',
                              '自动压缩阈值',
                            )}
                          </label>
                          <input
                            className="qs-select qs-select--input-mode"
                            type="text"
                            inputMode="numeric"
                            value={codexQuickCompactLimitInput}
                            onChange={(event) => {
                              setCodexQuickConfigError(null);
                              setCodexQuickConfigNotice(null);
                              setCodexQuickCompactLimitInput(event.target.value);
                            }}
                            disabled={!codexQuickIsCustomPreset || codexQuickConfigSaving}
                            placeholder={String(DEFAULT_AUTO_COMPACT_TOKEN_LIMIT)}
                          />
                          <div className="qs-hint">
                            {t(
                              'quickSettings.codex.quickConfig.autoCompactLimitHint',
                              '写入 model_auto_compact_token_limit。仅在“自定义”模式可编辑。',
                            )}
                          </div>
                          {codexQuickCompactLimitError && (
                            <div className="qs-codex-quick-field-error">
                              {codexQuickCompactLimitError}
                            </div>
                          )}
                        </div>
                      </div>

                      {codexQuickConfigWarning && (
                        <div className="qs-codex-quick-warning">
                          {codexQuickConfigWarning}
                        </div>
                      )}
                    </>
                  )}

                  {(codexQuickConfigError || codexQuickConfigNotice) && (
                    <div
                      className={`qs-codex-quick-status ${
                        codexQuickConfigError ? 'error' : 'success'
                      }`}
                    >
                      {codexQuickConfigError || codexQuickConfigNotice}
                    </div>
                  )}
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
                        'settings.general.hermesAuthOverwrite',
                        '切换 Codex 时同步 Hermes'
                      )}
                    </span>
                  </div>
                  <div className="qs-row-control">
                    <label className="qs-switch">
                      <input
                        type="checkbox"
                        checked={Boolean(config.hermes_auth_overwrite_on_switch)}
                        onChange={(e) =>
                          saveConfig({ hermes_auth_overwrite_on_switch: e.target.checked })
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

                <div className="qs-row">
                  <div className="qs-row-label">
                    <Zap size={15} />
                    <span>{t('codex.list.showAdditionalQuota', '显示模型专属配额')}</span>
                  </div>
                  <div className="qs-row-control">
                    <label className="qs-switch">
                      <input
                        type="checkbox"
                        checked={codexShowAdditionalQuota}
                        onChange={(e) => handleCodexAdditionalQuotaToggle(e.target.checked)}
                      />
                      <span className="qs-switch-slider"></span>
                    </label>
                  </div>
                </div>

                <div className="qs-row">
                  <div className="qs-row-label">
                    <Zap size={15} />
                    <span>{t('codex.list.planBadgeStyle', '套餐徽章样式')}</span>
                  </div>
                  <div className="qs-row-control">
                    <select
                      className="qs-select"
                      value={codexPlanBadgeStyle}
                      onChange={(e) =>
                        handleCodexPlanBadgeStyleChange(e.target.value as CodexPlanBadgeStyle)
                      }
                    >
                      <option value="default">{t('codex.list.planBadgeStyleDefault', '默认')}</option>
                      <option value="outline">{t('codex.list.planBadgeStyleOutline', '描边')}</option>
                      <option value="soft">{t('codex.list.planBadgeStyleSoft', '柔和')}</option>
                      <option value="mono">{t('codex.list.planBadgeStyleMono', '单色')}</option>
                    </select>
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
                            onModeChange={(mode) => {
                              // all_accounts ignores selected IDs at runtime; clear
                              // the list so config does not keep a stale subset.
                              if (mode === AUTO_SWITCH_SCOPE_ALL_ACCOUNTS) {
                                saveConfig({
                                  codex_auto_switch_account_scope_mode: mode,
                                  codex_auto_switch_selected_account_ids: [],
                                });
                                return;
                              }
                              saveConfig({ codex_auto_switch_account_scope_mode: mode });
                            }}
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
                        '切号时同时执行本地落盘与扩展无感切号，不再自动重启 Antigravity IDE。'
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
                        <span>{t('quickSettings.autoSwitch.creditsEnable', '监控 Credits')}</span>
                      </div>
                      <div className="qs-row-control">
                        <label className="qs-switch">
                          <input
                            type="checkbox"
                            checked={creditsAutoSwitchEnabled}
                            onChange={(e) =>
                              saveConfig({ auto_switch_credits_enabled: e.target.checked })
                            }
                          />
                          <span className="qs-switch-slider"></span>
                        </label>
                      </div>
                    </div>

                    {creditsAutoSwitchEnabled && (
                      <div className="qs-row">
                        <div className="qs-row-label">
                          <span>{t('quickSettings.autoSwitch.creditsThreshold', 'Credits 阈值')}</span>
                        </div>
                        <div className="qs-row-control">
                          {showCreditsThresholdInput ? (
                            <div className="qs-inline-input">
                              <input
                                type="number"
                                min={0}
                                className="qs-select qs-select--input-mode"
                                value={customCreditsThreshold}
                                placeholder={t('quickSettings.inputCredits', '输入 Credits')}
                                onChange={(e) =>
                                  setCustomCreditsThreshold(e.target.value.replace(/[^\d]/g, ''))
                                }
                                onBlur={handleCustomCreditsThresholdApply}
                                onKeyDown={(e) => {
                                  if (e.key === 'Enter') {
                                    e.preventDefault();
                                    handleCustomCreditsThresholdApply();
                                  }
                                }}
                              />
                            </div>
                          ) : (
                            <select
                              className="qs-select"
                              value={String(creditsAutoSwitchThresholdValue)}
                              onChange={(e) => handleCreditsThresholdSelectChange(e.target.value)}
                            >
                              {!isCreditsThresholdPreset && (
                                <option value={String(creditsAutoSwitchThresholdValue)}>
                                  {creditsAutoSwitchThresholdValue}
                                </option>
                              )}
                              <option value="0">0</option>
                              <option value="5">5</option>
                              <option value="10">10</option>
                              <option value="20">20</option>
                              <option value="custom">{t('quickSettings.customInput', '自定义')}</option>
                            </select>
                          )}
                        </div>
                      </div>
                    )}

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
                          onModeChange={(mode) => {
                            if (mode === AUTO_SWITCH_SCOPE_ALL_ACCOUNTS) {
                              saveConfig({
                                auto_switch_account_scope_mode: mode,
                                auto_switch_selected_account_ids: [],
                              });
                              return;
                            }
                            saveConfig({ auto_switch_account_scope_mode: mode });
                          }}
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
                    '命中监控的模型分组阈值时会自动切号；启用 Credits 监控后，剩余 Credits 低于阈值时也会触发。'
                  )}
                </div>

                {renderQuotaAlertControls()}
              </div>
            )}

            {type !== 'antigravity' && type !== 'zcode' && (
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
