import { useState, useEffect, useMemo, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { openUrl } from '@tauri-apps/plugin-opener';
import { open } from '@tauri-apps/plugin-dialog';
import { invoke } from '@tauri-apps/api/core';
import { getVersion } from '@tauri-apps/api/app';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { changeLanguage, getCurrentLanguage, normalizeLanguage } from '../i18n';
import * as accountService from '../services/accountService';
import * as codexService from '../services/codexService';
import { getAccountGroups, type AccountGroup } from '../services/accountGroupService';
import {
  getCodexAccountGroups,
  type CodexAccountGroup,
} from '../services/codexAccountGroupService';
import { showFloatingCardWindow } from '../services/floatingCardService';
import { usePlatformRuntimeSupport } from '../hooks/usePlatformRuntimeSupport';
import { usePlatformLayoutStore } from '../stores/usePlatformLayoutStore';
import { SideNavLayoutMode, useSideNavLayoutStore } from '../stores/useSideNavLayoutStore';
import { UnlockFireworksOverlay } from '../components/UnlockFireworksOverlay';
import {
  AutoSwitchAccountScopeSelector,
  type AutoSwitchAccountScopeMode,
  type AutoSwitchScopeAccount,
} from '../components/AutoSwitchAccountScopeSelector';
import {
  buildAccountTierCounts,
  buildAccountTierFilterOptions,
} from '../utils/accountFilters';
import { getSubscriptionTier } from '../utils/account';
import type { Account } from '../types/account';
import type { CodexAccount } from '../types/codex';
import {
  FEATURE_UNLOCK_CHANGED_EVENT,
  type FeatureUnlockChangedDetail,
  isAntigravitySeamlessSwitchFeatureUnlocked,
  persistAntigravitySeamlessSwitchFeatureUnlocked,
} from '../utils/featureUnlocks';
import {
  buildDefaultCurrentAccountRefreshMinutesMap,
  CURRENT_ACCOUNT_REFRESH_PLATFORMS,
  type CurrentAccountRefreshMinutesMap,
  type CurrentAccountRefreshPlatform,
  loadCurrentAccountRefreshMinutesMap,
  saveCurrentAccountRefreshMinutesMap,
} from '../utils/currentAccountRefresh';
import { ALL_PLATFORM_IDS, PlatformId } from '../types/platform';
import { SettingsAccountTransferSection } from '../components/SettingsAccountTransferSection';
import './settings/Settings.css';
import { 
  Github, User, Rocket, Save, FolderOpen,
  AlertCircle, RefreshCw, Heart, MessageSquare
} from 'lucide-react';



/** 网络配置类型 */
interface NetworkConfig {
  ws_enabled: boolean;
  ws_port: number;
  actual_port: number | null;
  default_port: number;
  report_enabled: boolean;
  report_port: number;
  report_actual_port: number | null;
  report_default_port: number;
  report_token: string;
  global_proxy_enabled: boolean;
  global_proxy_url: string;
  global_proxy_no_proxy: string;
}

/** 通用配置类型 */
interface GeneralConfig {
  language: string;
  default_terminal: string;
  theme: string;
  ui_scale: number;
  auto_refresh_minutes: number;
  codex_auto_refresh_minutes: number;
  ghcp_auto_refresh_minutes: number;
  windsurf_auto_refresh_minutes: number;
  kiro_auto_refresh_minutes: number;
  cursor_auto_refresh_minutes: number;
  gemini_auto_refresh_minutes: number;
  close_behavior: 'ask' | 'minimize' | 'quit';
  minimize_behavior?: 'dock_and_tray' | 'tray_only';
  hide_dock_icon?: boolean;
  floating_card_show_on_startup?: boolean;
  floating_card_always_on_top?: boolean;
  app_auto_launch_enabled?: boolean;
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
  codebuddy_auto_refresh_minutes: number;
  codebuddy_cn_auto_refresh_minutes: number;
  workbuddy_auto_refresh_minutes: number;
  qoder_auto_refresh_minutes: number;
  trae_auto_refresh_minutes: number;
  zed_auto_refresh_minutes: number;
  codebuddy_quota_alert_enabled: boolean;
  codebuddy_quota_alert_threshold: number;
  codebuddy_cn_quota_alert_enabled: boolean;
  codebuddy_cn_quota_alert_threshold: number;
  qoder_quota_alert_enabled: boolean;
  qoder_quota_alert_threshold: number;
  trae_quota_alert_enabled: boolean;
  trae_quota_alert_threshold: number;
  zed_quota_alert_enabled: boolean;
  zed_quota_alert_threshold: number;
  workbuddy_quota_alert_enabled: boolean;
  workbuddy_quota_alert_threshold: number;
  opencode_sync_on_switch: boolean;
  opencode_auth_overwrite_on_switch: boolean;
  openclaw_auth_overwrite_on_switch: boolean;
  codex_launch_on_switch: boolean;
  antigravity_dual_switch_no_restart_enabled: boolean;
  auto_switch_enabled: boolean;
  auto_switch_threshold: number;
  auto_switch_account_scope_mode?: string;
  auto_switch_selected_account_ids?: string[];
  codex_auto_switch_enabled?: boolean;
  codex_auto_switch_primary_threshold?: number;
  codex_auto_switch_secondary_threshold?: number;
  codex_auto_switch_account_scope_mode?: string;
  codex_auto_switch_selected_account_ids?: string[];
  quota_alert_enabled: boolean;
  quota_alert_threshold: number;
  codex_quota_alert_enabled: boolean;
  codex_quota_alert_threshold: number;
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
}

type AppPathTarget =
  | 'antigravity'
  | 'codex'
  | 'vscode'
  | 'opencode'
  | 'windsurf'
  | 'kiro'
  | 'cursor'
  | 'codebuddy'
  | 'codebuddy_cn'
  | 'qoder'
  | 'trae'
  | 'workbuddy'
  | 'zed';
const REFRESH_PRESET_VALUES = ['-1', '2', '5', '10', '15'];
const CURRENT_ACCOUNT_REFRESH_PRESET_VALUES = ['1', '2', '5', '10', '15'];
const THRESHOLD_PRESET_VALUES = ['0', '20', '40', '60'];
const UI_SCALE_OPTIONS = ['0.9', '1', '1.1', '1.25', '1.5'] as const;
const ANTIGRAVITY_SEAMLESS_SWITCH_UNLOCK_REQUIRED_TAPS = 10;
const UNLOCK_FIREWORKS_VISIBLE_MS = 6000;
const AUTO_SWITCH_SCOPE_ALL_ACCOUNTS: AutoSwitchAccountScopeMode = 'all_accounts';
const AUTO_SWITCH_SCOPE_SELECTED_ACCOUNTS: AutoSwitchAccountScopeMode = 'selected_accounts';
const FALLBACK_PLATFORM_SETTINGS_ORDER: Record<PlatformId, number> = {
  antigravity: 0,
  codex: 1,
  'github-copilot': 2,
  windsurf: 3,
  kiro: 4,
  cursor: 5,
  gemini: 6,
  codebuddy: 7,
  codebuddy_cn: 8,
  qoder: 9,
  trae: 10,
  workbuddy: 11,
  zed: 12,
};
type UpdateCheckSource = 'auto' | 'manual';
type UpdateCheckFinishedDetail = {
  source: UpdateCheckSource;
  status: 'has_update' | 'up_to_date' | 'failed';
  currentVersion?: string;
  latestVersion?: string;
  error?: string;
};

const generateReportToken = () => {
  const bytes = new Uint8Array(12);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
};

const normalizeAutoSwitchAccountScopeMode = (
  value?: string | null,
): AutoSwitchAccountScopeMode =>
  value === AUTO_SWITCH_SCOPE_SELECTED_ACCOUNTS
    ? AUTO_SWITCH_SCOPE_SELECTED_ACCOUNTS
    : AUTO_SWITCH_SCOPE_ALL_ACCOUNTS;

const toCurrentAccountRefreshMinutesStringMap = (
  map: CurrentAccountRefreshMinutesMap,
): Record<CurrentAccountRefreshPlatform, string> => {
  return CURRENT_ACCOUNT_REFRESH_PLATFORMS.reduce((result, platform) => {
    result[platform] = String(map[platform]);
    return result;
  }, {} as Record<CurrentAccountRefreshPlatform, string>);
};

const buildDefaultCurrentAccountRefreshCustomModeMap = (): Record<
  CurrentAccountRefreshPlatform,
  boolean
> => {
  return CURRENT_ACCOUNT_REFRESH_PLATFORMS.reduce((result, platform) => {
    result[platform] = false;
    return result;
  }, {} as Record<CurrentAccountRefreshPlatform, boolean>);
};

export function SettingsPage() {
  const { t } = useTranslation();
  const isMacOS = usePlatformRuntimeSupport('macos-only');
  const isWindows = usePlatformRuntimeSupport('windows-only');
  const isLinux = usePlatformRuntimeSupport('linux-only');
  const sideNavLayoutMode = useSideNavLayoutStore((state) => state.mode);
  const setSideNavLayoutMode = useSideNavLayoutStore((state) => state.setMode);
  const [activeTab, setActiveTab] = useState<'general' | 'network' | 'about'>('general');
  const [availableTerminals, setAvailableTerminals] = useState<string[]>(['system']);

  useEffect(() => {
    invoke<string[]>('get_available_terminals')
      .then(setAvailableTerminals)
      .catch(err => console.error('获取可用终端失败:', err));
  }, []);

  const terminalOptions = useMemo(() => {
    const common = [{ value: 'system', label: t('settings.general.terminalSystem', '系统默认') }];
    const allOptions = isMacOS ? [
      { value: 'Terminal', label: 'Terminal.app' },
      { value: 'iTerm2', label: 'iTerm2' },
      { value: 'Warp', label: 'Warp' },
      { value: 'Ghostty', label: 'Ghostty' },
      { value: 'WezTerm', label: 'WezTerm' },
      { value: 'Kitty', label: 'Kitty' },
      { value: 'Alacritty', label: 'Alacritty' },
    ] : isWindows ? [
      { value: 'cmd', label: 'Command Prompt (cmd)' },
      { value: 'PowerShell', label: 'PowerShell' },
      { value: 'pwsh', label: 'PowerShell Core (pwsh)' },
      { value: 'wt', label: 'Windows Terminal (wt)' },
    ] : isLinux ? [
      { value: 'x-terminal-emulator', label: 'x-terminal-emulator' },
      { value: 'gnome-terminal', label: 'gnome-terminal' },
      { value: 'konsole', label: 'konsole' },
      { value: 'xfce4-terminal', label: 'xfce4-terminal' },
      { value: 'xterm', label: 'xterm' },
      { value: 'alacritty', label: 'Alacritty' },
      { value: 'kitty', label: 'Kitty' },
    ] : [];

    return [
      ...common,
      ...allOptions.filter(opt => availableTerminals.includes(opt.value))
    ];
  }, [isMacOS, isWindows, isLinux, availableTerminals, t]);

  const orderedPlatformIds = usePlatformLayoutStore((state) => state.orderedPlatformIds);
  const platformSettingsOrder = useMemo<Record<PlatformId, number>>(() => {
    const next: Record<PlatformId, number> = { ...FALLBACK_PLATFORM_SETTINGS_ORDER };
    let order = 0;
    for (const id of orderedPlatformIds) {
      if (!ALL_PLATFORM_IDS.includes(id)) continue;
      next[id] = order;
      order += 1;
    }
    return next;
  }, [orderedPlatformIds]);

  const languageOptions = [
    { value: 'zh-cn', label: '简体中文' },
    { value: 'zh-tw', label: '繁體中文' },
    { value: 'en', label: 'English' },
    { value: 'ja', label: '日本語' },
    { value: 'ko', label: '한국어' },
    { value: 'de', label: 'Deutsch' },
    { value: 'fr', label: 'Français' },
    { value: 'es', label: 'Español' },
    { value: 'pt-br', label: 'Português (Brasil)' },
    { value: 'ru', label: 'Русский' },
    { value: 'it', label: 'Italiano' },
    { value: 'tr', label: 'Türkçe' },
    { value: 'pl', label: 'Polski' },
    { value: 'cs', label: 'Čeština' },
    { value: 'vi', label: 'Tiếng Việt' },
    { value: 'ar', label: 'العربية' },
    { value: 'id', label: 'Bahasa Indonesia' },
  ];
  
  // General Settings States
  const [language, setLanguage] = useState(getCurrentLanguage());
  const [defaultTerminal, setDefaultTerminal] = useState('system');
  const [theme, setTheme] = useState('system');
  const [uiScale, setUiScale] = useState('1');
  const [autoRefresh, setAutoRefresh] = useState('5');
  const [codexAutoRefresh, setCodexAutoRefresh] = useState('10');
  const [ghcpAutoRefresh, setGhcpAutoRefresh] = useState('10');
  const [windsurfAutoRefresh, setWindsurfAutoRefresh] = useState('10');
  const [kiroAutoRefresh, setKiroAutoRefresh] = useState('10');
  const [cursorAutoRefresh, setCursorAutoRefresh] = useState('10');
  const [geminiAutoRefresh, setGeminiAutoRefresh] = useState('10');
  const [closeBehavior, setCloseBehavior] = useState<'ask' | 'minimize' | 'quit'>('ask');
  const [minimizeBehavior, setMinimizeBehavior] = useState<'dock_and_tray' | 'tray_only'>('dock_and_tray');
  const [hideDockIcon, setHideDockIcon] = useState(false);
  const [floatingCardShowOnStartup, setFloatingCardShowOnStartup] = useState(true);
  const [floatingCardAlwaysOnTop, setFloatingCardAlwaysOnTop] = useState(false);
  const [appAutoLaunchEnabled, setAppAutoLaunchEnabled] = useState(false);
  const [opencodeAppPath, setOpencodeAppPath] = useState('');
  const [antigravityAppPath, setAntigravityAppPath] = useState('');
  const [codexAppPath, setCodexAppPath] = useState('');
  const [vscodeAppPath, setVscodeAppPath] = useState('');
  const [windsurfAppPath, setWindsurfAppPath] = useState('');
  const [kiroAppPath, setKiroAppPath] = useState('');
  const [cursorAppPath, setCursorAppPath] = useState('');
  const [codebuddyAppPath, setCodebuddyAppPath] = useState('');
  const [codebuddyCnAppPath, setCodebuddyCnAppPath] = useState('');
  const [qoderAppPath, setQoderAppPath] = useState('');
  const [traeAppPath, setTraeAppPath] = useState('');
  const [workbuddyAppPath, setWorkbuddyAppPath] = useState('');
  const [zedAppPath, setZedAppPath] = useState('');
  const [codebuddyAutoRefresh, setCodebuddyAutoRefresh] = useState('10');
  const [codebuddyCnAutoRefresh, setCodebuddyCnAutoRefresh] = useState('10');
  const [workbuddyAutoRefresh, setWorkbuddyAutoRefresh] = useState('10');
  const [qoderAutoRefresh, setQoderAutoRefresh] = useState('10');
  const [traeAutoRefresh, setTraeAutoRefresh] = useState('10');
  const [zedAutoRefresh, setZedAutoRefresh] = useState('10');
  const [currentAccountRefreshMinutes, setCurrentAccountRefreshMinutes] = useState<
    Record<CurrentAccountRefreshPlatform, string>
  >(() =>
    toCurrentAccountRefreshMinutesStringMap(buildDefaultCurrentAccountRefreshMinutesMap()),
  );
  const [currentAccountRefreshCustomMode, setCurrentAccountRefreshCustomMode] = useState<
    Record<CurrentAccountRefreshPlatform, boolean>
  >(() => buildDefaultCurrentAccountRefreshCustomModeMap());
  const [codebuddyQuotaAlertEnabled, setCodebuddyQuotaAlertEnabled] = useState(false);
  const [codebuddyQuotaAlertThreshold, setCodebuddyQuotaAlertThreshold] = useState('20');
  const [codebuddyCnQuotaAlertEnabled, setCodebuddyCnQuotaAlertEnabled] = useState(false);
  const [codebuddyCnQuotaAlertThreshold, setCodebuddyCnQuotaAlertThreshold] = useState('20');
  const [qoderQuotaAlertEnabled, setQoderQuotaAlertEnabled] = useState(false);
  const [qoderQuotaAlertThreshold, setQoderQuotaAlertThreshold] = useState('20');
  const [traeQuotaAlertEnabled, setTraeQuotaAlertEnabled] = useState(false);
  const [traeQuotaAlertThreshold, setTraeQuotaAlertThreshold] = useState('20');
  const [zedQuotaAlertEnabled, setZedQuotaAlertEnabled] = useState(false);
  const [zedQuotaAlertThreshold, setZedQuotaAlertThreshold] = useState('20');
  const [workbuddyQuotaAlertEnabled, setWorkbuddyQuotaAlertEnabled] = useState(false);
  const [workbuddyQuotaAlertThreshold, setWorkbuddyQuotaAlertThreshold] = useState('20');
  const [codebuddyAutoRefreshCustomMode, setCodebuddyAutoRefreshCustomMode] = useState(false);
  const [codebuddyCnAutoRefreshCustomMode, setCodebuddyCnAutoRefreshCustomMode] = useState(false);
  const [workbuddyAutoRefreshCustomMode, setWorkbuddyAutoRefreshCustomMode] = useState(false);
  const [codebuddyQuotaAlertThresholdCustomMode, setCodebuddyQuotaAlertThresholdCustomMode] = useState(false);
  const [qoderAutoRefreshCustomMode, setQoderAutoRefreshCustomMode] = useState(false);
  const [qoderQuotaAlertThresholdCustomMode, setQoderQuotaAlertThresholdCustomMode] = useState(false);
  const [traeAutoRefreshCustomMode, setTraeAutoRefreshCustomMode] = useState(false);
  const [traeQuotaAlertThresholdCustomMode, setTraeQuotaAlertThresholdCustomMode] = useState(false);
  const [zedAutoRefreshCustomMode, setZedAutoRefreshCustomMode] = useState(false);
  const [zedQuotaAlertThresholdCustomMode, setZedQuotaAlertThresholdCustomMode] = useState(false);
  const [codebuddyCnQuotaAlertThresholdCustomMode, setCodebuddyCnQuotaAlertThresholdCustomMode] = useState(false);
  const [workbuddyQuotaAlertThresholdCustomMode, setWorkbuddyQuotaAlertThresholdCustomMode] = useState(false);
  const [appPathResetDetectingTargets, setAppPathResetDetectingTargets] = useState<Set<AppPathTarget>>(new Set());
  const [opencodeSyncOnSwitch, setOpencodeSyncOnSwitch] = useState(false);
  const [opencodeAuthOverwriteOnSwitch, setOpencodeAuthOverwriteOnSwitch] = useState(false);
  const [openclawAuthOverwriteOnSwitch, setOpenclawAuthOverwriteOnSwitch] = useState(false);
  const [codexLaunchOnSwitch, setCodexLaunchOnSwitch] = useState(true);
  const [antigravityDualSwitchNoRestartEnabled, setAntigravityDualSwitchNoRestartEnabled] = useState(false);
  const [autoSwitchEnabled, setAutoSwitchEnabled] = useState(false);
  const [autoSwitchThreshold, setAutoSwitchThreshold] = useState('20');
  const [autoSwitchAccountScopeMode, setAutoSwitchAccountScopeMode] =
    useState<AutoSwitchAccountScopeMode>(AUTO_SWITCH_SCOPE_ALL_ACCOUNTS);
  const [autoSwitchSelectedAccountIds, setAutoSwitchSelectedAccountIds] = useState<string[]>([]);
  const [codexAutoSwitchEnabled, setCodexAutoSwitchEnabled] = useState(false);
  const [codexAutoSwitchAccountScopeMode, setCodexAutoSwitchAccountScopeMode] =
    useState<AutoSwitchAccountScopeMode>(AUTO_SWITCH_SCOPE_ALL_ACCOUNTS);
  const [codexAutoSwitchSelectedAccountIds, setCodexAutoSwitchSelectedAccountIds] = useState<
    string[]
  >([]);
  const [quotaAlertEnabled, setQuotaAlertEnabled] = useState(false);
  const [quotaAlertThreshold, setQuotaAlertThreshold] = useState('20');
  const [codexQuotaAlertEnabled, setCodexQuotaAlertEnabled] = useState(false);
  const [codexQuotaAlertThreshold, setCodexQuotaAlertThreshold] = useState('20');
  const [ghcpQuotaAlertEnabled, setGhcpQuotaAlertEnabled] = useState(false);
  const [ghcpQuotaAlertThreshold, setGhcpQuotaAlertThreshold] = useState('20');
  const [windsurfQuotaAlertEnabled, setWindsurfQuotaAlertEnabled] = useState(false);
  const [windsurfQuotaAlertThreshold, setWindsurfQuotaAlertThreshold] = useState('20');
  const [kiroQuotaAlertEnabled, setKiroQuotaAlertEnabled] = useState(false);
  const [kiroQuotaAlertThreshold, setKiroQuotaAlertThreshold] = useState('20');
  const [cursorQuotaAlertEnabled, setCursorQuotaAlertEnabled] = useState(false);
  const [cursorQuotaAlertThreshold, setCursorQuotaAlertThreshold] = useState('20');
  const [geminiQuotaAlertEnabled, setGeminiQuotaAlertEnabled] = useState(false);
  const [geminiQuotaAlertThreshold, setGeminiQuotaAlertThreshold] = useState('20');
  const [autoRefreshCustomMode, setAutoRefreshCustomMode] = useState(false);
  const [codexAutoRefreshCustomMode, setCodexAutoRefreshCustomMode] = useState(false);
  const [ghcpAutoRefreshCustomMode, setGhcpAutoRefreshCustomMode] = useState(false);
  const [windsurfAutoRefreshCustomMode, setWindsurfAutoRefreshCustomMode] = useState(false);
  const [kiroAutoRefreshCustomMode, setKiroAutoRefreshCustomMode] = useState(false);
  const [cursorAutoRefreshCustomMode, setCursorAutoRefreshCustomMode] = useState(false);
  const [geminiAutoRefreshCustomMode, setGeminiAutoRefreshCustomMode] = useState(false);
  const [autoSwitchThresholdCustomMode, setAutoSwitchThresholdCustomMode] = useState(false);
  const [quotaAlertThresholdCustomMode, setQuotaAlertThresholdCustomMode] = useState(false);
  const [codexQuotaAlertThresholdCustomMode, setCodexQuotaAlertThresholdCustomMode] = useState(false);
  const [ghcpQuotaAlertThresholdCustomMode, setGhcpQuotaAlertThresholdCustomMode] = useState(false);
  const [windsurfQuotaAlertThresholdCustomMode, setWindsurfQuotaAlertThresholdCustomMode] = useState(false);
  const [kiroQuotaAlertThresholdCustomMode, setKiroQuotaAlertThresholdCustomMode] = useState(false);
  const [cursorQuotaAlertThresholdCustomMode, setCursorQuotaAlertThresholdCustomMode] = useState(false);
  const [geminiQuotaAlertThresholdCustomMode, setGeminiQuotaAlertThresholdCustomMode] = useState(false);
  const [antigravitySeamlessSwitchUnlocked, setAntigravitySeamlessSwitchUnlocked] = useState(
    isAntigravitySeamlessSwitchFeatureUnlocked,
  );
  const [, setAboutAvatarTapCount] = useState(0);
  const [showUnlockFireworks, setShowUnlockFireworks] = useState(false);
  const unlockFireworksTimerRef = useRef<number | null>(null);
  const [generalLoaded, setGeneralLoaded] = useState(false);
  const generalSaveTimerRef = useRef<number | null>(null);
  const suppressGeneralSaveRef = useRef(false);
  const currentAccountRefreshPersistReadyRef = useRef(false);
  
  const [appVersion, setAppVersion] = useState('');
  const [updateChecking, setUpdateChecking] = useState(false);
  const [updateCheckMessage, setUpdateCheckMessage] = useState<{
    text: string;
    tone?: 'error' | 'success';
  } | null>(null);
  const [autoInstall, setAutoInstall] = useState(false);
  const [autoInstallLoaded, setAutoInstallLoaded] = useState(false);
  const autoInstallTouchedRef = useRef(false);
  const [updateRemindersEnabled, setUpdateRemindersEnabled] = useState(true);
  const [updateRemindersLoaded, setUpdateRemindersLoaded] = useState(false);
  const updateRemindersTouchedRef = useRef(false);
  const [antigravityAccounts, setAntigravityAccounts] = useState<Account[]>([]);
  const [antigravityAccountGroups, setAntigravityAccountGroups] = useState<AccountGroup[]>([]);
  const [codexAccounts, setCodexAccounts] = useState<CodexAccount[]>([]);
  const [codexGroups, setCodexGroups] = useState<CodexAccountGroup[]>([]);

  const antigravityScopeTypeOptions = useMemo(
    () => buildAccountTierFilterOptions(t, buildAccountTierCounts(antigravityAccounts, {})),
    [antigravityAccounts, t],
  );
  const antigravityScopeAccounts = useMemo<AutoSwitchScopeAccount[]>(
    () =>
      antigravityAccounts.map((account) => {
        const disabledReason = account.disabled_reason || '';
        const type =
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
          type,
        };
      }),
    [antigravityAccounts],
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

  useEffect(() => {
    let mounted = true;
    const loadAutoSwitchScopeData = async () => {
      try {
        const [nextAntigravityAccounts, nextAntigravityGroups, nextCodexAccounts, nextCodexGroups] =
          await Promise.all([
            accountService.listAccounts(),
            getAccountGroups(),
            codexService.listCodexAccounts(),
            getCodexAccountGroups(),
          ]);
        if (!mounted) return;
        setAntigravityAccounts(nextAntigravityAccounts || []);
        setAntigravityAccountGroups(nextAntigravityGroups || []);
        setCodexAccounts(nextCodexAccounts || []);
        setCodexGroups(nextCodexGroups || []);
      } catch (error) {
        console.error('加载自动切号账号范围数据失败:', error);
        if (!mounted) return;
        setAntigravityAccounts([]);
        setAntigravityAccountGroups([]);
        setCodexAccounts([]);
        setCodexGroups([]);
      }
    };

    if (activeTab === 'general') {
      void loadAutoSwitchScopeData();
    }

    return () => {
      mounted = false;
    };
  }, [activeTab]);

  useEffect(() => {
    getVersion().then(ver => setAppVersion(`v${ver}`));
    // Load auto_install setting first to avoid overwriting existing value on initial render
    invoke<{
      auto_check: boolean;
      last_check_time: number;
      check_interval_hours: number;
      auto_install?: boolean;
      last_run_version?: string;
      remind_on_update?: boolean;
      skipped_version?: string;
    }>('get_update_settings')
      .then((s) => {
        setAutoInstall(Boolean(s?.auto_install));
        setUpdateRemindersEnabled(s?.remind_on_update ?? true);
        setAutoInstallLoaded(true);
        setUpdateRemindersLoaded(true);
      })
      .catch((err) => {
        console.error('加载自动更新设置失败:', err);
      });
  }, []);

  useEffect(() => {
    const handleStarted = (event: Event) => {
      const detail = (event as CustomEvent<{ source?: UpdateCheckSource }>).detail;
      if (detail?.source !== 'manual') {
        return;
      }
      setUpdateChecking(true);
      setUpdateCheckMessage(null);
    };

    const handleFinished = (event: Event) => {
      const detail = (event as CustomEvent<UpdateCheckFinishedDetail>).detail;
      if (!detail || detail.source !== 'manual') {
        return;
      }

      setUpdateChecking(false);

      if (detail.status === 'up_to_date') {
        const version = detail.latestVersion || detail.currentVersion;
        const upToDateText = t('settings.about.upToDate');
        setUpdateCheckMessage({
          text: version ? `${upToDateText} v${version}` : upToDateText,
          tone: 'success',
        });
        return;
      }

      if (detail.status === 'failed') {
        setUpdateCheckMessage({
          text: t('settings.about.checkFailed'),
          tone: 'error',
        });
        return;
      }

      setUpdateCheckMessage(null);
    };

    window.addEventListener('update-check-started', handleStarted as EventListener);
    window.addEventListener('update-check-finished', handleFinished as EventListener);
    return () => {
      window.removeEventListener('update-check-started', handleStarted as EventListener);
      window.removeEventListener('update-check-finished', handleFinished as EventListener);
    };
  }, [t]);

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

  useEffect(() => {
    return () => {
      if (unlockFireworksTimerRef.current !== null) {
        window.clearTimeout(unlockFireworksTimerRef.current);
      }
    };
  }, []);
  
  // Network States
  const [wsEnabled, setWsEnabled] = useState(true);
  const [wsPort, setWsPort] = useState('19528');
  const [actualPort, setActualPort] = useState<number | null>(null);
  const [defaultPort, setDefaultPort] = useState(19528);
  const [reportEnabled, setReportEnabled] = useState(false);
  const [reportPort, setReportPort] = useState('18081');
  const [reportActualPort, setReportActualPort] = useState<number | null>(null);
  const [reportDefaultPort, setReportDefaultPort] = useState(18081);
  const [reportToken, setReportToken] = useState('');
  const [globalProxyEnabled, setGlobalProxyEnabled] = useState(false);
  const [globalProxyUrl, setGlobalProxyUrl] = useState('');
  const [globalProxyNoProxy, setGlobalProxyNoProxy] = useState('');
  const reportPreviewPort = reportActualPort ?? (parseInt(reportPort, 10) || reportDefaultPort);
  const reportPreviewToken = encodeURIComponent((reportToken || 'your-token').trim() || 'your-token');
  const reportRawPreviewUrl = `http://<当前IP>:${reportPreviewPort}/report?token=${reportPreviewToken}`;
  const reportRenderedPreviewUrl = `${reportRawPreviewUrl}&render=true`;
  const [needsRestart, setNeedsRestart] = useState(false);
  const [networkSaving, setNetworkSaving] = useState(false);
  
  // 检测配额重置任务状态
  const [hasActiveResetTasks, setHasActiveResetTasks] = useState(false);
  
  // 加载配置
  useEffect(() => {
    loadGeneralConfig();
    loadNetworkConfig();
  }, []);
  
  useEffect(() => {
    if (!generalLoaded) {
      return;
    }
    changeLanguage(language);
    applyTheme(theme);
  }, [generalLoaded, language, theme]);

  useEffect(() => {
    if (!generalLoaded) {
      return;
    }
    void applyUiScale(uiScale);
  }, [generalLoaded, uiScale]);

  useEffect(() => {
    if (!generalLoaded) {
      return;
    }

    if (generalSaveTimerRef.current) {
      window.clearTimeout(generalSaveTimerRef.current);
    }

    if (
      !autoRefresh.trim() ||
      !codexAutoRefresh.trim() ||
      !ghcpAutoRefresh.trim() ||
      !windsurfAutoRefresh.trim() ||
      !kiroAutoRefresh.trim() ||
      !codebuddyAutoRefresh.trim() ||
      !codebuddyCnAutoRefresh.trim() ||
      !workbuddyAutoRefresh.trim() ||
      !qoderAutoRefresh.trim() ||
      !traeAutoRefresh.trim() ||
      !zedAutoRefresh.trim() ||
      !cursorAutoRefresh.trim() ||
      !geminiAutoRefresh.trim()
    ) {
      return;
    }

    const autoRefreshNum = parseInt(autoRefresh, 10) || -1;
    const codexAutoRefreshNum = parseInt(codexAutoRefresh, 10) || -1;
    const ghcpAutoRefreshNum = parseInt(ghcpAutoRefresh, 10) || -1;
    const windsurfAutoRefreshNum = parseInt(windsurfAutoRefresh, 10) || -1;
    const kiroAutoRefreshNum = parseInt(kiroAutoRefresh, 10) || -1;
    const codebuddyAutoRefreshNum = parseInt(codebuddyAutoRefresh, 10) || -1;
    const codebuddyCnAutoRefreshNum = parseInt(codebuddyCnAutoRefresh, 10) || -1;
    const workbuddyAutoRefreshNum = parseInt(workbuddyAutoRefresh, 10) || -1;
    const qoderAutoRefreshNum = parseInt(qoderAutoRefresh, 10) || -1;
    const traeAutoRefreshNum = parseInt(traeAutoRefresh, 10) || -1;
    const zedAutoRefreshNum = parseInt(zedAutoRefresh, 10) || -1;
    const cursorAutoRefreshNum = parseInt(cursorAutoRefresh, 10) || -1;
    const geminiAutoRefreshNum = parseInt(geminiAutoRefresh, 10) || -1;
    const parsedUiScale = Number.parseFloat(uiScale);
    const normalizedUiScale = Number.isFinite(parsedUiScale)
      ? Math.min(2, Math.max(0.8, parsedUiScale))
      : 1;
    const parsedAutoSwitchThreshold = Number.parseInt(autoSwitchThreshold, 10);
    const parsedQuotaAlertThreshold = Number.parseInt(quotaAlertThreshold, 10);
    const parsedCodexQuotaAlertThreshold = Number.parseInt(codexQuotaAlertThreshold, 10);
    const parsedGhcpQuotaAlertThreshold = Number.parseInt(ghcpQuotaAlertThreshold, 10);
    const parsedWindsurfQuotaAlertThreshold = Number.parseInt(windsurfQuotaAlertThreshold, 10);
    const parsedKiroQuotaAlertThreshold = Number.parseInt(kiroQuotaAlertThreshold, 10);
    const parsedCodebuddyQuotaAlertThreshold = Number.parseInt(codebuddyQuotaAlertThreshold, 10);
    const parsedCodebuddyCnQuotaAlertThreshold = Number.parseInt(codebuddyCnQuotaAlertThreshold, 10);
    const parsedWorkbuddyQuotaAlertThreshold = Number.parseInt(workbuddyQuotaAlertThreshold, 10);
    const parsedQoderQuotaAlertThreshold = Number.parseInt(qoderQuotaAlertThreshold, 10);
    const parsedTraeQuotaAlertThreshold = Number.parseInt(traeQuotaAlertThreshold, 10);
    const parsedZedQuotaAlertThreshold = Number.parseInt(zedQuotaAlertThreshold, 10);
    const parsedCursorQuotaAlertThreshold = Number.parseInt(cursorQuotaAlertThreshold, 10);
    const parsedGeminiQuotaAlertThreshold = Number.parseInt(geminiQuotaAlertThreshold, 10);
    if (suppressGeneralSaveRef.current) {
      suppressGeneralSaveRef.current = false;
      return;
    }

    generalSaveTimerRef.current = window.setTimeout(async () => {
      try {
        await invoke('save_general_config', {
          language,
          defaultTerminal,
          theme,
          uiScale: normalizedUiScale,
          autoRefreshMinutes: autoRefreshNum,
          codexAutoRefreshMinutes: codexAutoRefreshNum,
          ghcpAutoRefreshMinutes: ghcpAutoRefreshNum,
          windsurfAutoRefreshMinutes: windsurfAutoRefreshNum,
          kiroAutoRefreshMinutes: kiroAutoRefreshNum,
          codebuddyAutoRefreshMinutes: codebuddyAutoRefreshNum,
          codebuddyCnAutoRefreshMinutes: codebuddyCnAutoRefreshNum,
          workbuddyAutoRefreshMinutes: workbuddyAutoRefreshNum,
          qoderAutoRefreshMinutes: qoderAutoRefreshNum,
          traeAutoRefreshMinutes: traeAutoRefreshNum,
          zedAutoRefreshMinutes: zedAutoRefreshNum,
          cursorAutoRefreshMinutes: cursorAutoRefreshNum,
          geminiAutoRefreshMinutes: geminiAutoRefreshNum,
          closeBehavior,
          minimizeBehavior,
          hideDockIcon,
          floatingCardShowOnStartup,
          floatingCardAlwaysOnTop,
          appAutoLaunchEnabled,
          opencodeAppPath,
          antigravityAppPath,
          codexAppPath,
          vscodeAppPath,
          windsurfAppPath,
          kiroAppPath,
          cursorAppPath,
          codebuddyAppPath,
          codebuddyCnAppPath,
          qoderAppPath,
          traeAppPath,
          workbuddyAppPath,
          zedAppPath,
          opencodeSyncOnSwitch,
          opencodeAuthOverwriteOnSwitch,
          openclawAuthOverwriteOnSwitch,
          codexLaunchOnSwitch,
          antigravityDualSwitchNoRestartEnabled,
          autoSwitchEnabled,
          autoSwitchThreshold: Number.isNaN(parsedAutoSwitchThreshold) ? 20 : parsedAutoSwitchThreshold,
          autoSwitchAccountScopeMode,
          autoSwitchSelectedAccountIds,
          codexAutoSwitchAccountScopeMode,
          codexAutoSwitchSelectedAccountIds,
          quotaAlertEnabled,
          quotaAlertThreshold: Number.isNaN(parsedQuotaAlertThreshold) ? 20 : parsedQuotaAlertThreshold,
          codexQuotaAlertEnabled,
          codexQuotaAlertThreshold: Number.isNaN(parsedCodexQuotaAlertThreshold)
            ? 20
            : parsedCodexQuotaAlertThreshold,
          ghcpQuotaAlertEnabled,
          ghcpQuotaAlertThreshold: Number.isNaN(parsedGhcpQuotaAlertThreshold)
            ? 20
            : parsedGhcpQuotaAlertThreshold,
          windsurfQuotaAlertEnabled,
          windsurfQuotaAlertThreshold: Number.isNaN(parsedWindsurfQuotaAlertThreshold)
            ? 20
            : parsedWindsurfQuotaAlertThreshold,
          kiroQuotaAlertEnabled,
          kiroQuotaAlertThreshold: Number.isNaN(parsedKiroQuotaAlertThreshold)
            ? 20
            : parsedKiroQuotaAlertThreshold,
          codebuddyQuotaAlertEnabled,
          codebuddyQuotaAlertThreshold: Number.isNaN(parsedCodebuddyQuotaAlertThreshold)
            ? 20
            : parsedCodebuddyQuotaAlertThreshold,
          codebuddyCnQuotaAlertEnabled,
          codebuddyCnQuotaAlertThreshold: Number.isNaN(parsedCodebuddyCnQuotaAlertThreshold)
            ? 20
            : parsedCodebuddyCnQuotaAlertThreshold,
          workbuddyQuotaAlertEnabled,
          workbuddyQuotaAlertThreshold: Number.isNaN(parsedWorkbuddyQuotaAlertThreshold)
            ? 20
            : parsedWorkbuddyQuotaAlertThreshold,
          qoderQuotaAlertEnabled,
          qoderQuotaAlertThreshold: Number.isNaN(parsedQoderQuotaAlertThreshold)
            ? 20
            : parsedQoderQuotaAlertThreshold,
          traeQuotaAlertEnabled,
          traeQuotaAlertThreshold: Number.isNaN(parsedTraeQuotaAlertThreshold)
            ? 20
            : parsedTraeQuotaAlertThreshold,
          zedQuotaAlertEnabled,
          zedQuotaAlertThreshold: Number.isNaN(parsedZedQuotaAlertThreshold)
            ? 20
            : parsedZedQuotaAlertThreshold,
          cursorQuotaAlertEnabled,
          cursorQuotaAlertThreshold: Number.isNaN(parsedCursorQuotaAlertThreshold)
            ? 20
            : parsedCursorQuotaAlertThreshold,
          geminiQuotaAlertEnabled,
          geminiQuotaAlertThreshold: Number.isNaN(parsedGeminiQuotaAlertThreshold)
            ? 20
            : parsedGeminiQuotaAlertThreshold,
        });
        window.dispatchEvent(new Event('config-updated'));
      } catch (err) {
        console.error('保存通用配置失败:', err);
        alert(`${t('settings.network.saveFailed').replace('{error}', String(err))}`);
      }
    }, 300);

    return () => {
      if (generalSaveTimerRef.current) {
        window.clearTimeout(generalSaveTimerRef.current);
      }
    };
  }, [
    autoRefresh,
    codexAutoRefresh,
    ghcpAutoRefresh,
    windsurfAutoRefresh,
    kiroAutoRefresh,
    traeAutoRefresh,
    zedAutoRefresh,
    workbuddyAutoRefresh,
    qoderAutoRefresh,
    cursorAutoRefresh,
    geminiAutoRefresh,
    closeBehavior,
    minimizeBehavior,
    hideDockIcon,
    floatingCardShowOnStartup,
    floatingCardAlwaysOnTop,
    appAutoLaunchEnabled,
    generalLoaded,
    language,
    defaultTerminal,
    theme,
    uiScale,
    opencodeAppPath,
    antigravityAppPath,
    codexAppPath,
    vscodeAppPath,
    windsurfAppPath,
    kiroAppPath,
    cursorAppPath,
    codebuddyAppPath,
    codebuddyCnAppPath,
    qoderAppPath,
    traeAppPath,
    workbuddyAppPath,
    zedAppPath,
    opencodeSyncOnSwitch,
    opencodeAuthOverwriteOnSwitch,
    openclawAuthOverwriteOnSwitch,
    codexLaunchOnSwitch,
    antigravityDualSwitchNoRestartEnabled,
    autoSwitchEnabled,
    autoSwitchThreshold,
    autoSwitchAccountScopeMode,
    autoSwitchSelectedAccountIds,
    codexAutoSwitchAccountScopeMode,
    codexAutoSwitchSelectedAccountIds,
    quotaAlertEnabled,
    quotaAlertThreshold,
    codexQuotaAlertEnabled,
    codexQuotaAlertThreshold,
    ghcpQuotaAlertEnabled,
    ghcpQuotaAlertThreshold,
    windsurfQuotaAlertEnabled,
    windsurfQuotaAlertThreshold,
    kiroQuotaAlertEnabled,
    kiroQuotaAlertThreshold,
    codebuddyAutoRefresh,
    codebuddyCnAutoRefresh,
    codebuddyQuotaAlertEnabled,
    codebuddyQuotaAlertThreshold,
    codebuddyCnQuotaAlertEnabled,
    codebuddyCnQuotaAlertThreshold,
    workbuddyQuotaAlertEnabled,
    workbuddyQuotaAlertThreshold,
    qoderQuotaAlertEnabled,
    qoderQuotaAlertThreshold,
    traeQuotaAlertEnabled,
    traeQuotaAlertThreshold,
    zedQuotaAlertEnabled,
    zedQuotaAlertThreshold,
    cursorQuotaAlertEnabled,
    cursorQuotaAlertThreshold,
    geminiQuotaAlertEnabled,
    geminiQuotaAlertThreshold,
    t,
  ]);

  useEffect(() => {
    if (!generalLoaded) {
      return;
    }

    if (!currentAccountRefreshPersistReadyRef.current) {
      currentAccountRefreshPersistReadyRef.current = true;
      return;
    }

    const payload = CURRENT_ACCOUNT_REFRESH_PLATFORMS.reduce((result, platform) => {
      const raw = Number.parseInt(currentAccountRefreshMinutes[platform], 10);
      result[platform] = Number.isNaN(raw) ? 1 : raw;
      return result;
    }, {} as Partial<Record<CurrentAccountRefreshPlatform, number>>);
    saveCurrentAccountRefreshMinutesMap(payload);
    window.dispatchEvent(new Event('config-updated'));
  }, [generalLoaded, currentAccountRefreshMinutes]);

  useEffect(() => {
    const handleLanguageUpdated = (event: Event) => {
      const detail = (event as CustomEvent<{ language?: string }>).detail;
      if (!detail?.language) {
        return;
      }
      suppressGeneralSaveRef.current = true;
      setLanguage(detail.language);
    };

    window.addEventListener('general-language-updated', handleLanguageUpdated);
    return () => {
      window.removeEventListener('general-language-updated', handleLanguageUpdated);
    };
  }, []);

  // 监听外部配置更新（如 QuickSettingsPopover 保存后同步）
  useEffect(() => {
    const handleConfigUpdated = () => {
      suppressGeneralSaveRef.current = true;
      loadGeneralConfig();
    };
    window.addEventListener('config-updated', handleConfigUpdated);
    return () => {
      window.removeEventListener('config-updated', handleConfigUpdated);
    };
  }, []);

  // Save auto_install setting when changed
  useEffect(() => {
    if (!autoInstallLoaded && !autoInstallTouchedRef.current) {
      return;
    }

    invoke<{
      auto_check: boolean;
      last_check_time: number;
      check_interval_hours: number;
      auto_install?: boolean;
      last_run_version?: string;
      remind_on_update?: boolean;
      skipped_version?: string;
    }>('get_update_settings')
      .then((s) => {
        if (Boolean(s?.auto_install) === autoInstall) {
          return;
        }
        invoke('save_update_settings', {
          settings: { ...s, auto_install: autoInstall },
        }).catch((err: unknown) =>
          console.error('Failed to save auto_install setting:', err),
        );
      })
      .catch(() => {});
  }, [autoInstall, autoInstallLoaded]);

  // Save update reminder setting when changed
  useEffect(() => {
    if (!updateRemindersLoaded && !updateRemindersTouchedRef.current) {
      return;
    }

    invoke<{
      auto_check: boolean;
      last_check_time: number;
      check_interval_hours: number;
      auto_install?: boolean;
      last_run_version?: string;
      remind_on_update?: boolean;
      skipped_version?: string;
    }>('get_update_settings')
      .then((s) => {
        if ((s?.remind_on_update ?? true) === updateRemindersEnabled) {
          return;
        }
        invoke('save_update_settings', {
          settings: { ...s, remind_on_update: updateRemindersEnabled },
        }).then(() => {
          window.dispatchEvent(
            new CustomEvent('update-reminder-changed', { detail: { enabled: updateRemindersEnabled } }),
          );
        }).catch((err: unknown) =>
          console.error('Failed to save update reminder setting:', err),
        );
      })
      .catch(() => {});
  }, [updateRemindersEnabled, updateRemindersLoaded]);
  
  // 检测配额重置任务状态
  useEffect(() => {
    const checkResetTasks = () => {
      try {
        // 检查唤醒总开关
        const wakeupEnabledRaw = localStorage.getItem('agtools.wakeup.enabled');
        const wakeupEnabled = wakeupEnabledRaw === 'true';
        
        // 如果总开关关闭，不需要限制
        if (!wakeupEnabled) {
          setHasActiveResetTasks(false);
          return;
        }
        
        // 检查是否有启用的配额重置任务
        const tasksJson = localStorage.getItem('agtools.wakeup.tasks');
        if (!tasksJson) {
          setHasActiveResetTasks(false);
          return;
        }
        
        const tasks = JSON.parse(tasksJson);
        const hasReset = Array.isArray(tasks) && tasks.some(
          (task: any) => task.enabled && task.schedule?.wakeOnReset
        );
        setHasActiveResetTasks(hasReset);
      } catch (error) {
        console.error('检测配额重置任务失败:', error);
        setHasActiveResetTasks(false);
      }
    };
    
    // 初始检测
    checkResetTasks();
    
    // 监听存储变化
    const handleStorageChange = (e: StorageEvent) => {
      if (e.key === 'agtools.wakeup.tasks' || e.key === 'agtools.wakeup.enabled') {
        checkResetTasks();
      }
    };
    
    window.addEventListener('storage', handleStorageChange);
    
    // 监听自定义事件（同一窗口内的任务变更）
    const handleTasksUpdated = () => checkResetTasks();
    window.addEventListener('wakeup-tasks-updated', handleTasksUpdated);
    
    return () => {
      window.removeEventListener('storage', handleStorageChange);
      window.removeEventListener('wakeup-tasks-updated', handleTasksUpdated);
    };
  }, []);
  
  const applyTheme = (newTheme: string) => {
    if (newTheme === 'system') {
      const isDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
      document.documentElement.setAttribute('data-theme', isDark ? 'dark' : 'light');
    } else {
      document.documentElement.setAttribute('data-theme', newTheme);
    }
  };

  const applyUiScale = async (rawScale: string) => {
    const parsed = Number.parseFloat(rawScale);
    const normalized = Number.isFinite(parsed) ? Math.min(2, Math.max(0.8, parsed)) : 1;
    try {
      await getCurrentWebview().setZoom(normalized);
    } catch (error) {
      console.error('应用界面缩放失败:', error);
    }
  };

  useEffect(() => {
    if (theme !== 'system') {
      return;
    }

    const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)');
    const handleChange = () => applyTheme('system');

    if (mediaQuery.addEventListener) {
      mediaQuery.addEventListener('change', handleChange);
    } else {
      mediaQuery.addListener(handleChange);
    }

    return () => {
      if (mediaQuery.removeEventListener) {
        mediaQuery.removeEventListener('change', handleChange);
      } else {
        mediaQuery.removeListener(handleChange);
      }
    };
  }, [theme]);
  
  const loadGeneralConfig = async () => {
    try {
      const config = await invoke<GeneralConfig>('get_general_config');
      setLanguage(normalizeLanguage(config.language));
      setDefaultTerminal(config.default_terminal || 'system');
      setTheme(config.theme);
      setUiScale(String(config.ui_scale ?? 1));
      setAutoRefresh(String(config.auto_refresh_minutes));
      setCodexAutoRefresh(String(config.codex_auto_refresh_minutes ?? 10));
      setGhcpAutoRefresh(String(config.ghcp_auto_refresh_minutes ?? 10));
      setWindsurfAutoRefresh(String(config.windsurf_auto_refresh_minutes ?? 10));
      setKiroAutoRefresh(String(config.kiro_auto_refresh_minutes ?? 10));
      setCursorAutoRefresh(String(config.cursor_auto_refresh_minutes ?? 10));
      setGeminiAutoRefresh(String(config.gemini_auto_refresh_minutes ?? 10));
      setCloseBehavior(config.close_behavior || 'ask');
      setMinimizeBehavior(config.minimize_behavior || 'dock_and_tray');
      setHideDockIcon(Boolean(config.hide_dock_icon));
      setFloatingCardShowOnStartup(config.floating_card_show_on_startup ?? true);
      setFloatingCardAlwaysOnTop(config.floating_card_always_on_top ?? false);
      setAppAutoLaunchEnabled(config.app_auto_launch_enabled ?? false);
      setOpencodeAppPath(config.opencode_app_path || '');
      setAntigravityAppPath(config.antigravity_app_path || '');
      setCodexAppPath(config.codex_app_path || '');
      setVscodeAppPath(config.vscode_app_path || '');
      setWindsurfAppPath(config.windsurf_app_path || '');
      setKiroAppPath(config.kiro_app_path || '');
      setCursorAppPath(config.cursor_app_path || '');
      setCodebuddyAppPath(config.codebuddy_app_path || '');
      setCodebuddyCnAppPath(config.codebuddy_cn_app_path || '');
      setQoderAppPath(config.qoder_app_path || '');
      setTraeAppPath(config.trae_app_path || '');
      setWorkbuddyAppPath(config.workbuddy_app_path || '');
      setZedAppPath(config.zed_app_path || '');
      setCodebuddyAutoRefresh(String(config.codebuddy_auto_refresh_minutes ?? 10));
      setCodebuddyCnAutoRefresh(String(config.codebuddy_cn_auto_refresh_minutes ?? 10));
      setWorkbuddyAutoRefresh(String(config.workbuddy_auto_refresh_minutes ?? 10));
      setQoderAutoRefresh(String(config.qoder_auto_refresh_minutes ?? 10));
      setTraeAutoRefresh(String(config.trae_auto_refresh_minutes ?? 10));
      setZedAutoRefresh(String(config.zed_auto_refresh_minutes ?? 10));
      setCurrentAccountRefreshMinutes(
        toCurrentAccountRefreshMinutesStringMap(loadCurrentAccountRefreshMinutesMap()),
      );
      setCodebuddyQuotaAlertEnabled(config.codebuddy_quota_alert_enabled ?? false);
      setCodebuddyQuotaAlertThreshold(String(config.codebuddy_quota_alert_threshold ?? 20));
      setCodebuddyCnQuotaAlertEnabled(config.codebuddy_cn_quota_alert_enabled ?? false);
      setCodebuddyCnQuotaAlertThreshold(String(config.codebuddy_cn_quota_alert_threshold ?? 20));
      setWorkbuddyQuotaAlertEnabled(config.workbuddy_quota_alert_enabled ?? false);
      setWorkbuddyQuotaAlertThreshold(String(config.workbuddy_quota_alert_threshold ?? 20));
      setQoderQuotaAlertEnabled(config.qoder_quota_alert_enabled ?? false);
      setQoderQuotaAlertThreshold(String(config.qoder_quota_alert_threshold ?? 20));
      setTraeQuotaAlertEnabled(config.trae_quota_alert_enabled ?? false);
      setTraeQuotaAlertThreshold(String(config.trae_quota_alert_threshold ?? 20));
      setZedQuotaAlertEnabled(config.zed_quota_alert_enabled ?? false);
      setZedQuotaAlertThreshold(String(config.zed_quota_alert_threshold ?? 20));
      setOpencodeSyncOnSwitch(config.opencode_sync_on_switch ?? false);
      setOpencodeAuthOverwriteOnSwitch(config.opencode_auth_overwrite_on_switch ?? false);
      setOpenclawAuthOverwriteOnSwitch(config.openclaw_auth_overwrite_on_switch ?? false);
      setCodexLaunchOnSwitch(config.codex_launch_on_switch ?? true);
      setAntigravityDualSwitchNoRestartEnabled(
        config.antigravity_dual_switch_no_restart_enabled ?? false
      );
      setAutoSwitchEnabled(config.auto_switch_enabled ?? false);
      setAutoSwitchThreshold(String(config.auto_switch_threshold ?? 20));
      setAutoSwitchAccountScopeMode(
        normalizeAutoSwitchAccountScopeMode(config.auto_switch_account_scope_mode),
      );
      setAutoSwitchSelectedAccountIds(config.auto_switch_selected_account_ids ?? []);
      setCodexAutoSwitchEnabled(config.codex_auto_switch_enabled ?? false);
      setCodexAutoSwitchAccountScopeMode(
        normalizeAutoSwitchAccountScopeMode(config.codex_auto_switch_account_scope_mode),
      );
      setCodexAutoSwitchSelectedAccountIds(config.codex_auto_switch_selected_account_ids ?? []);
      setQuotaAlertEnabled(config.quota_alert_enabled ?? false);
      setQuotaAlertThreshold(String(config.quota_alert_threshold ?? 20));
      setCodexQuotaAlertEnabled(config.codex_quota_alert_enabled ?? false);
      setCodexQuotaAlertThreshold(String(config.codex_quota_alert_threshold ?? 20));
      setGhcpQuotaAlertEnabled(config.ghcp_quota_alert_enabled ?? false);
      setGhcpQuotaAlertThreshold(String(config.ghcp_quota_alert_threshold ?? 20));
      setWindsurfQuotaAlertEnabled(config.windsurf_quota_alert_enabled ?? false);
      setWindsurfQuotaAlertThreshold(String(config.windsurf_quota_alert_threshold ?? 20));
      setKiroQuotaAlertEnabled(config.kiro_quota_alert_enabled ?? false);
      setKiroQuotaAlertThreshold(String(config.kiro_quota_alert_threshold ?? 20));
      setCursorQuotaAlertEnabled(config.cursor_quota_alert_enabled ?? false);
      setCursorQuotaAlertThreshold(String(config.cursor_quota_alert_threshold ?? 20));
      setGeminiQuotaAlertEnabled(config.gemini_quota_alert_enabled ?? false);
      setGeminiQuotaAlertThreshold(String(config.gemini_quota_alert_threshold ?? 20));
      setAutoRefreshCustomMode(false);
      setCodexAutoRefreshCustomMode(false);
      setGhcpAutoRefreshCustomMode(false);
      setWindsurfAutoRefreshCustomMode(false);
      setKiroAutoRefreshCustomMode(false);
      setCodebuddyAutoRefreshCustomMode(false);
      setCodebuddyCnAutoRefreshCustomMode(false);
      setWorkbuddyAutoRefreshCustomMode(false);
      setQoderAutoRefreshCustomMode(false);
      setTraeAutoRefreshCustomMode(false);
      setZedAutoRefreshCustomMode(false);
      setCursorAutoRefreshCustomMode(false);
      setGeminiAutoRefreshCustomMode(false);
      setAutoSwitchThresholdCustomMode(false);
      setQuotaAlertThresholdCustomMode(false);
      setCodexQuotaAlertThresholdCustomMode(false);
      setGhcpQuotaAlertThresholdCustomMode(false);
      setWindsurfQuotaAlertThresholdCustomMode(false);
      setKiroQuotaAlertThresholdCustomMode(false);
      setCodebuddyQuotaAlertThresholdCustomMode(false);
      setCodebuddyCnQuotaAlertThresholdCustomMode(false);
      setWorkbuddyQuotaAlertThresholdCustomMode(false);
      setQoderQuotaAlertThresholdCustomMode(false);
      setTraeQuotaAlertThresholdCustomMode(false);
      setZedQuotaAlertThresholdCustomMode(false);
      setCursorQuotaAlertThresholdCustomMode(false);
      setGeminiQuotaAlertThresholdCustomMode(false);
      setCurrentAccountRefreshCustomMode(buildDefaultCurrentAccountRefreshCustomModeMap());
      currentAccountRefreshPersistReadyRef.current = false;
      // 同步语言
      changeLanguage(config.language);
      applyTheme(config.theme);
      setGeneralLoaded(true);
    } catch (err) {
      console.error('加载通用配置失败:', err);
    }
  };
  
  const loadNetworkConfig = async () => {
    try {
      const config = await invoke<NetworkConfig>('get_network_config');
      setWsEnabled(config.ws_enabled);
      setWsPort(String(config.ws_port));
      setActualPort(config.actual_port);
      setDefaultPort(config.default_port);
      setReportEnabled(config.report_enabled);
      setReportPort(String(config.report_port));
      setReportActualPort(config.report_actual_port);
      setReportDefaultPort(config.report_default_port);
      setReportToken(config.report_token || '');
      setGlobalProxyEnabled(Boolean(config.global_proxy_enabled));
      setGlobalProxyUrl(config.global_proxy_url || '');
      setGlobalProxyNoProxy(config.global_proxy_no_proxy || '');
      setNeedsRestart(false);
    } catch (err) {
      console.error('加载网络配置失败:', err);
    }
  };
  
  // 保存网络配置
  const handleSaveNetworkConfig = async () => {
    setNetworkSaving(true);
    try {
      const portNum = parseInt(wsPort, 10) || defaultPort;
      const reportPortNum = parseInt(reportPort, 10) || reportDefaultPort;
      const normalizedToken = reportToken.trim();

      if (reportEnabled && !normalizedToken) {
        alert(t('settings.network.reportTokenRequired'));
        return;
      }
      const normalizedGlobalProxyUrl = globalProxyUrl.trim();
      const normalizedGlobalProxyNoProxy = globalProxyNoProxy.trim();
      if (globalProxyEnabled && !normalizedGlobalProxyUrl) {
        alert(t('settings.network.proxyUrlRequired'));
        return;
      }

      const result = await invoke<boolean>('save_network_config', {
        wsEnabled,
        wsPort: portNum,
        reportEnabled,
        reportPort: reportPortNum,
        reportToken: normalizedToken,
        globalProxyEnabled,
        globalProxyUrl: normalizedGlobalProxyUrl,
        globalProxyNoProxy: normalizedGlobalProxyNoProxy,
      });
      
      if (result) {
        setNeedsRestart(true);
        alert(t('settings.network.saveSuccessRestart'));
      } else {
        alert(t('settings.network.saveSuccess'));
      }
    } catch (err) {
      alert(t('settings.network.saveFailed').replace('{error}', String(err)));
    } finally {
      setNetworkSaving(false);
    }
  };

  const openLink = (url: string) => {
    openUrl(url);
  };

  const isAppPathResetDetecting = (target: AppPathTarget) => appPathResetDetectingTargets.has(target);

  const setAppPathForTarget = (target: AppPathTarget, path: string) => {
    if (target === 'antigravity') {
      setAntigravityAppPath(path);
    } else if (target === 'codex') {
      setCodexAppPath(path);
    } else if (target === 'vscode') {
      setVscodeAppPath(path);
    } else if (target === 'windsurf') {
      setWindsurfAppPath(path);
    } else if (target === 'kiro') {
      setKiroAppPath(path);
    } else if (target === 'cursor') {
      setCursorAppPath(path);
    } else if (target === 'codebuddy') {
      setCodebuddyAppPath(path);
    } else if (target === 'codebuddy_cn') {
      setCodebuddyCnAppPath(path);
    } else if (target === 'qoder') {
      setQoderAppPath(path);
    } else if (target === 'trae') {
      setTraeAppPath(path);
    } else if (target === 'workbuddy') {
      setWorkbuddyAppPath(path);
    } else if (target === 'zed') {
      setZedAppPath(path);
    } else {
      setOpencodeAppPath(path);
    }
  };

  const getResetLabelByTarget = (target: AppPathTarget) => {
    if (target === 'vscode') {
      return t('settings.general.vscodePathReset', '重置默认');
    }
    if (target === 'windsurf') {
      return t('settings.general.windsurfPathReset', '重置默认');
    }
    if (target === 'kiro') {
      return t('settings.general.kiroPathReset', '重置默认');
    }
    if (target === 'cursor') {
      return t('settings.general.cursorPathReset', '重置默认');
    }
    if (target === 'codebuddy') {
      return t('settings.general.codebuddyPathReset', '重置默认');
    }
    if (target === 'codebuddy_cn') {
      return t('settings.general.codebuddyCnPathReset', '重置默认');
    }
    if (target === 'qoder') {
      return t('settings.general.qoderPathReset', '重置默认');
    }
    if (target === 'trae') {
      return t('settings.general.traePathReset', '重置默认');
    }
    if (target === 'workbuddy') {
      return t('settings.general.workbuddyPathReset', '重置默认');
    }
    if (target === 'zed') {
      return t('settings.general.zedPathReset', '重置默认');
    }
    if (target === 'opencode') {
      return t('settings.general.opencodePathReset', '重置默认');
    }
    return t('settings.general.codexPathReset', '重置默认');
  };

  const handlePickAppPath = async (target: AppPathTarget) => {
    try {
      const selected = await open({
        multiple: false,
        directory: false,
      });

      const path = Array.isArray(selected) ? selected[0] : selected;
      if (!path) return;

      setAppPathForTarget(target, path);
    } catch (err) {
      console.error('选择启动路径失败:', err);
    }
  };

  const handleResetAppPath = async (target: AppPathTarget) => {
    if (isAppPathResetDetecting(target)) return;
    setAppPathResetDetectingTargets((prev) => {
      const next = new Set(prev);
      next.add(target);
      return next;
    });
    try {
      const detected = await invoke<string | null>('detect_app_path', { app: target, force: true });
      setAppPathForTarget(target, detected || '');
    } catch (err) {
      console.error('重置启动路径失败:', err);
      setAppPathForTarget(target, '');
    } finally {
      setAppPathResetDetectingTargets((prev) => {
        const next = new Set(prev);
        next.delete(target);
        return next;
      });
    }
  };

  const sanitizeNumberInput = (value: string) => value.replace(/[^\d]/g, '');

  const normalizeNumberInput = (value: string, min: number, max?: number): string => {
    const parsed = Number.parseInt(value, 10);
    if (Number.isNaN(parsed)) {
      return String(min);
    }
    const bounded = Math.max(min, max ? Math.min(parsed, max) : parsed);
    return String(bounded);
  };

  const setCurrentAccountRefreshValue = (
    platform: CurrentAccountRefreshPlatform,
    value: string,
  ) => {
    setCurrentAccountRefreshMinutes((prev) => ({
      ...prev,
      [platform]: value,
    }));
  };

  const setCurrentAccountRefreshCustomModeValue = (
    platform: CurrentAccountRefreshPlatform,
    enabled: boolean,
  ) => {
    setCurrentAccountRefreshCustomMode((prev) => ({
      ...prev,
      [platform]: enabled,
    }));
  };

  const isCurrentAccountRefreshAvailable = (
    platform: CurrentAccountRefreshPlatform,
  ): boolean => {
    const parseRefresh = (value: string): number => Number.parseInt(value, 10) || -1;
    switch (platform) {
      case 'antigravity':
        return parseRefresh(autoRefresh) > 0;
      case 'codex':
        return parseRefresh(codexAutoRefresh) > 0;
      case 'ghcp':
        return parseRefresh(ghcpAutoRefresh) > 0;
      case 'windsurf':
        return parseRefresh(windsurfAutoRefresh) > 0;
      case 'kiro':
        return parseRefresh(kiroAutoRefresh) > 0;
      case 'cursor':
        return parseRefresh(cursorAutoRefresh) > 0;
      case 'gemini':
        return parseRefresh(geminiAutoRefresh) > 0;
      case 'codebuddy':
        return parseRefresh(codebuddyAutoRefresh) > 0;
      case 'codebuddy_cn':
        return parseRefresh(codebuddyCnAutoRefresh) > 0;
      case 'workbuddy':
        return parseRefresh(workbuddyAutoRefresh) > 0;
      case 'qoder':
        return parseRefresh(qoderAutoRefresh) > 0;
      case 'trae':
        return parseRefresh(traeAutoRefresh) > 0;
      case 'zed':
        return parseRefresh(zedAutoRefresh) > 0;
    }
  };

  const renderCurrentAccountRefreshRow = (platform: CurrentAccountRefreshPlatform) => {
    const value = currentAccountRefreshMinutes[platform];
    const currentRefreshAvailable = isCurrentAccountRefreshAvailable(platform);
    const customMode = currentAccountRefreshCustomMode[platform] && currentRefreshAvailable;
    const isPreset = CURRENT_ACCOUNT_REFRESH_PRESET_VALUES.includes(value);
    const displayValue = currentRefreshAvailable ? value : '-1';

    return (
      <div className="settings-row">
        <div className="row-label">
          <div className="row-title">{t('settings.general.currentAccountRefreshTitle')}</div>
          <div className="row-desc">
            {currentRefreshAvailable
              ? t('settings.general.currentAccountRefreshItemDesc')
              : t(
                'settings.general.currentAccountRefreshRequiresAutoRefresh',
                '需先开启“配额自动刷新”后，才能设置当前账号刷新。',
              )}
          </div>
        </div>
        <div className="row-control">
          <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
            {customMode ? (
              <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                <input
                  type="number"
                  min={1}
                  max={999}
                  className="settings-select settings-select--input-mode settings-select--with-unit"
                  value={value}
                  placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                  onChange={(event) =>
                    setCurrentAccountRefreshValue(platform, sanitizeNumberInput(event.target.value))
                  }
                  onBlur={() => {
                    const normalized = normalizeNumberInput(value, 1, 999);
                    setCurrentAccountRefreshValue(platform, normalized);
                    setCurrentAccountRefreshCustomModeValue(platform, false);
                  }}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter') {
                      event.preventDefault();
                      const normalized = normalizeNumberInput(value, 1, 999);
                      setCurrentAccountRefreshValue(platform, normalized);
                      setCurrentAccountRefreshCustomModeValue(platform, false);
                    }
                  }}
                  disabled={!currentRefreshAvailable}
                />
                <span className="settings-input-unit">{t('settings.general.minutes')}</span>
              </div>
            ) : (
              <select
                className="settings-select"
                style={{ minWidth: '120px', width: 'auto' }}
                value={displayValue}
                onChange={(event) => {
                  if (!currentRefreshAvailable) {
                    return;
                  }
                  const nextValue = event.target.value;
                  if (nextValue === 'custom') {
                    setCurrentAccountRefreshCustomModeValue(platform, true);
                    setCurrentAccountRefreshValue(
                      platform,
                      value || String(CURRENT_ACCOUNT_REFRESH_PRESET_VALUES[0]),
                    );
                    return;
                  }
                  setCurrentAccountRefreshCustomModeValue(platform, false);
                  setCurrentAccountRefreshValue(platform, nextValue);
                }}
                disabled={!currentRefreshAvailable}
              >
                {!currentRefreshAvailable && (
                  <option value="-1">{t('settings.general.autoRefreshDisabled')}</option>
                )}
                {!isPreset && (
                  <option value={value}>
                    {value} {t('settings.general.minutes')}
                  </option>
                )}
                <option value="1">1 {t('settings.general.minutes')}</option>
                <option value="2">2 {t('settings.general.minutes')}</option>
                <option value="5">5 {t('settings.general.minutes')}</option>
                <option value="10">10 {t('settings.general.minutes')}</option>
                <option value="15">15 {t('settings.general.minutes')}</option>
                <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
              </select>
            )}
          </div>
        </div>
      </div>
    );
  };

  const autoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(autoRefresh);
  const codexAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(codexAutoRefresh);
  const ghcpAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(ghcpAutoRefresh);
  const windsurfAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(windsurfAutoRefresh);
  const kiroAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(kiroAutoRefresh);
  const codebuddyAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(codebuddyAutoRefresh);
  const codebuddyCnAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(codebuddyCnAutoRefresh);
  const workbuddyAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(workbuddyAutoRefresh);
  const qoderAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(qoderAutoRefresh);
  const traeAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(traeAutoRefresh);
  const zedAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(zedAutoRefresh);
  const cursorAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(cursorAutoRefresh);
  const geminiAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(geminiAutoRefresh);
  const autoSwitchThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(autoSwitchThreshold);
  const quotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(quotaAlertThreshold);
  const codexQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(codexQuotaAlertThreshold);
  const ghcpQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(ghcpQuotaAlertThreshold);
  const windsurfQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(windsurfQuotaAlertThreshold);
  const kiroQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(kiroQuotaAlertThreshold);
  const codebuddyQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(codebuddyQuotaAlertThreshold);
  const codebuddyCnQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(codebuddyCnQuotaAlertThreshold);
  const workbuddyQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(workbuddyQuotaAlertThreshold);
  const qoderQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(qoderQuotaAlertThreshold);
  const traeQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(traeQuotaAlertThreshold);
  const zedQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(zedQuotaAlertThreshold);
  const cursorQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(cursorQuotaAlertThreshold);
  const geminiQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(geminiQuotaAlertThreshold);

  // 检查更新
  const handleCheckUpdate = () => {
    if (updateChecking) {
      return;
    }
    window.dispatchEvent(
      new CustomEvent('update-check-requested', {
        detail: { source: 'manual' as UpdateCheckSource },
      }),
    );
  };

  const triggerUnlockFireworks = () => {
    if (unlockFireworksTimerRef.current !== null) {
      window.clearTimeout(unlockFireworksTimerRef.current);
      unlockFireworksTimerRef.current = null;
    }
    setShowUnlockFireworks(true);
    unlockFireworksTimerRef.current = window.setTimeout(() => {
      setShowUnlockFireworks(false);
      unlockFireworksTimerRef.current = null;
    }, UNLOCK_FIREWORKS_VISIBLE_MS);
  };

  const handleAboutAvatarTap = () => {
    setAboutAvatarTapCount((prev) => {
      const next = prev + 1;
      if (next % ANTIGRAVITY_SEAMLESS_SWITCH_UNLOCK_REQUIRED_TAPS === 0) {
        if (!antigravitySeamlessSwitchUnlocked) {
          persistAntigravitySeamlessSwitchFeatureUnlocked(true);
          setAntigravitySeamlessSwitchUnlocked(true);
        }
        triggerUnlockFireworks();
      }
      return next;
    });
  };

  return (
    <main className="main-content">
      <div className="page-tabs-row">
        <div className="page-tabs-label">{t('settings.title')}</div>
        <div className="page-tabs filter-tabs">
          <button 
            className={`filter-tab ${activeTab === 'general' ? 'active' : ''}`}
            onClick={() => setActiveTab('general')}
          >
            {t('settings.tabs.general')}
          </button>
          <button 
            className={`filter-tab ${activeTab === 'network' ? 'active' : ''}`}
            onClick={() => setActiveTab('network')}
          >
            {t('settings.tabs.network')}
          </button>
          <button 
            className={`filter-tab ${activeTab === 'about' ? 'active' : ''}`}
            onClick={() => setActiveTab('about')}
          >
            {t('settings.tabs.about')}
          </button>
        </div>
      </div>

      {/* 2. Content Area */}
      <div className="settings-container">
        <div className="settings-content">
        {/* === General Tab === */}
        {activeTab === 'general' && (
          <>
            <div className="group-title">{t('settings.general.commonTitle', '通用')}</div>
            <div className="settings-group">
              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.language')}</div>
                  <div className="row-desc">{t('settings.general.languageDesc')}</div>
                </div>
                <div className="row-control">
                  <select 
                    className="settings-select" 
                    value={language} 
                    onChange={(e) => setLanguage(normalizeLanguage(e.target.value))}
                  >
                    {languageOptions.map((option) => (
                      <option key={option.value} value={option.value}>{option.label}</option>
                    ))}
                  </select>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.theme')}</div>
                  <div className="row-desc">{t('settings.general.themeDesc')}</div>
                </div>
                <div className="row-control">
                  <select 
                    className="settings-select" 
                    value={theme} 
                    onChange={(e) => setTheme(e.target.value)}
                  >
                    <option value="light">{t('settings.general.themeLight')}</option>
                    <option value="dark">{t('settings.general.themeDark')}</option>
                    <option value="system">{t('settings.general.themeSystem')}</option>
                  </select>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.defaultTerminal', '默认终端')}</div>
                  <div className="row-desc">{t('settings.general.defaultTerminalDesc', 'Gemini CLI 打开时使用的终端')}</div>
                </div>
                <div className="row-control">
                  <select 
                    className="settings-select" 
                    value={defaultTerminal} 
                    onChange={(e) => setDefaultTerminal(e.target.value)}
                  >
                    {terminalOptions.map((option) => (
                      <option key={option.value} value={option.value}>{option.label}</option>
                    ))}
                  </select>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.sideNavLayout', '侧边栏布局')}</div>
                  <div className="row-desc">{t('settings.general.sideNavLayoutDesc', '切换原始布局或经典布局')}</div>
                </div>
                <div className="row-control">
                  <select
                    className="settings-select"
                    value={sideNavLayoutMode}
                    onChange={(e) => setSideNavLayoutMode(e.target.value as SideNavLayoutMode)}
                  >
                    <option value="original">{t('settings.general.sideNavLayoutOriginal', '原始布局')}</option>
                    <option value="classic">{t('settings.general.sideNavLayoutClassic', '经典布局')}</option>
                  </select>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.uiScale')}</div>
                  <div className="row-desc">{t('settings.general.uiScaleDesc')}</div>
                </div>
                <div className="row-control">
                  <select
                    className="settings-select"
                    value={uiScale}
                    onChange={(e) => setUiScale(e.target.value)}
                  >
                    {UI_SCALE_OPTIONS.map((value) => (
                      <option key={value} value={value}>{`${Math.round(Number.parseFloat(value) * 100)}%`}</option>
                    ))}
                  </select>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.closeBehavior')}</div>
                  <div className="row-desc">{t('settings.general.closeBehaviorDesc')}</div>
                </div>
                <div className="row-control">
                  <select 
                    className="settings-select" 
                    value={closeBehavior} 
                    onChange={(e) => setCloseBehavior(e.target.value as 'ask' | 'minimize' | 'quit')}
                  >
                    <option value="ask">{t('settings.general.closeBehaviorAsk')}</option>
                    <option value="minimize">{t('settings.general.closeBehaviorMinimize')}</option>
                    <option value="quit">{t('settings.general.closeBehaviorQuit')}</option>
                  </select>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.autoUpdate')}</div>
                  <div className="row-desc">{t('settings.general.autoUpdateDesc')}</div>
                </div>
                <div className="row-control">
                  <select
                    className="settings-select"
                    value={autoInstall ? 'true' : 'false'}
                    onChange={(e) => {
                      autoInstallTouchedRef.current = true;
                      setAutoInstall(e.target.value === 'true');
                    }}
                  >
                    <option value="false">{t('settings.general.autoUpdateOff')}</option>
                    <option value="true">{t('settings.general.autoUpdateOn')}</option>
                  </select>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.updateReminder')}</div>
                  <div className="row-desc">{t('settings.general.updateReminderDesc')}</div>
                </div>
                <div className="row-control">
                  <select
                    className="settings-select"
                    value={updateRemindersEnabled ? 'true' : 'false'}
                    onChange={(e) => {
                      updateRemindersTouchedRef.current = true;
                      setUpdateRemindersEnabled(e.target.value === 'true');
                    }}
                  >
                    <option value="true">{t('settings.general.updateReminderOn')}</option>
                    <option value="false">{t('settings.general.updateReminderOff')}</option>
                  </select>
                </div>
              </div>

              {isMacOS && (
                <div className="settings-row">
                  <div className="row-label">
                    <div className="row-title">
                      {t('settings.general.hideDockIcon', '是否隐藏Dock图标（仅 macOS）')}
                    </div>
                    <div className="row-desc">
                      {t(
                        'settings.general.hideDockIconDesc',
                        '独立控制程序坞图标显示状态，不受窗口最小化行为影响'
                      )}
                    </div>
                  </div>
                  <div className="row-control">
                    <select
                      className="settings-select"
                      value={hideDockIcon ? 'true' : 'false'}
                      onChange={(e) => setHideDockIcon(e.target.value === 'true')}
                    >
                      <option value="false">
                        {t('settings.general.hideDockIconOff', '否（显示Dock图标）')}
                      </option>
                      <option value="true">
                        {t('settings.general.hideDockIconOn', '是（隐藏Dock图标）')}
                      </option>
                    </select>
                  </div>
                </div>
              )}

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.floatingCardStartup', '启动时显示悬浮卡片')}</div>
                  <div className="row-desc">{t('settings.general.floatingCardStartupDesc', '应用启动后默认展示悬浮账号卡片')}</div>
                </div>
                <div className="row-control">
                  <select
                    className="settings-select"
                    value={floatingCardShowOnStartup ? 'true' : 'false'}
                    onChange={(e) => setFloatingCardShowOnStartup(e.target.value === 'true')}
                  >
                    <option value="true">{t('common.enable', '启用')}</option>
                    <option value="false">{t('common.disable', '停用')}</option>
                  </select>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.floatingCardAlwaysOnTop', '悬浮卡片默认置顶')}</div>
                  <div className="row-desc">{t('settings.general.floatingCardAlwaysOnTopDesc', '新打开的悬浮卡片窗口默认保持置顶')}</div>
                </div>
                <div className="row-control">
                  <select
                    className="settings-select"
                    value={floatingCardAlwaysOnTop ? 'true' : 'false'}
                    onChange={(e) => setFloatingCardAlwaysOnTop(e.target.value === 'true')}
                  >
                    <option value="false">{t('common.disable', '停用')}</option>
                    <option value="true">{t('common.enable', '启用')}</option>
                  </select>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.appAutoLaunch')}</div>
                  <div className="row-desc">{t('settings.general.appAutoLaunchDesc')}</div>
                </div>
                <div className="row-control">
                  <select
                    className="settings-select"
                    value={appAutoLaunchEnabled ? 'true' : 'false'}
                    onChange={(e) => setAppAutoLaunchEnabled(e.target.value === 'true')}
                  >
                    <option value="false">{t('common.disable', '停用')}</option>
                    <option value="true">{t('common.enable', '启用')}</option>
                  </select>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.floatingCardShowNow', '立即显示悬浮卡片')}</div>
                  <div className="row-desc">{t('settings.general.floatingCardShowNowDesc', '关闭后可在这里或托盘菜单中重新打开')}</div>
                </div>
                <div className="row-control">
                  <button className="btn btn-secondary" onClick={() => void showFloatingCardWindow()}>
                    {t('settings.general.floatingCardShowNowAction', '显示悬浮卡片')}
                  </button>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.dataDir')}</div>
                  <div className="row-desc">{t('settings.general.dataDirDesc')}</div>
                </div>
                <div className="row-control">
                  <button className="btn btn-secondary" onClick={() => accountService.openDataFolder()}>
                    <FolderOpen size={16} />{t('common.open')}
                  </button>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.fpDir')}</div>
                  <div className="row-desc">{t('settings.general.fpDirDesc')}</div>
                </div>
                <div className="row-control">
                  <button className="btn btn-secondary" onClick={() => accountService.openDeviceFolder()}>
                    <FolderOpen size={16} />{t('common.open')}
                  </button>
                </div>
              </div>
            </div>

            <SettingsAccountTransferSection />

            <div style={{ display: 'flex', flexDirection: 'column' }}>
              <div style={{ order: platformSettingsOrder.antigravity }}>
                <div className="group-title">{t('settings.general.antigravitySettingsTitle', 'Antigravity 设置')}</div>
                <div className="settings-group">
              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.autoRefresh')}</div>
                  <div className="row-desc">{t('settings.general.autoRefreshDesc')}</div>
                </div>
                <div className="row-control">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                    {autoRefreshCustomMode ? (
                      <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                        <input
                          type="number"
                          min={1}
                          max={999}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={autoRefresh}
                          placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                          onChange={(e) => setAutoRefresh(sanitizeNumberInput(e.target.value))}
                        onBlur={() => {
                          const normalized = normalizeNumberInput(autoRefresh, 1, 999);
                          setAutoRefresh(normalized);
                          setAutoRefreshCustomMode(false);
                        }}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') {
                            e.preventDefault();
                            const normalized = normalizeNumberInput(autoRefresh, 1, 999);
                            setAutoRefresh(normalized);
                            setAutoRefreshCustomMode(false);
                          }
                        }}
                      />
                        <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                      </div>
                    ) : (
                      <select
                        className="settings-select"
                        style={{ minWidth: '120px', width: 'auto' }}
                        value={autoRefresh}
                        onChange={(e) => {
                          const val = e.target.value;
                          if (val === 'custom') {
                            setAutoRefreshCustomMode(true);
                            setAutoRefresh(autoRefresh !== '-1' ? autoRefresh : '1');
                            return;
                          }
                          setAutoRefreshCustomMode(false);
                          setAutoRefresh(val);
                        }}
                      >
                        {!autoRefreshIsPreset && (
                          <option value={autoRefresh}>
                            {autoRefresh} {t('settings.general.minutes')}
                          </option>
                        )}
                        <option value="-1" disabled={hasActiveResetTasks}>{t('settings.general.autoRefreshDisabled')}</option>
                        <option value="2">2 {t('settings.general.minutes')}</option>
                        <option value="5" disabled={hasActiveResetTasks}>5 {t('settings.general.minutes')}</option>
                        <option value="10" disabled={hasActiveResetTasks}>10 {t('settings.general.minutes')}</option>
                        <option value="15" disabled={hasActiveResetTasks}>15 {t('settings.general.minutes')}</option>
                        <option value="custom" disabled={hasActiveResetTasks}>{t('settings.general.autoRefreshCustom')}</option>
                      </select>
                    )}
                  </div>
                  
                  {hasActiveResetTasks && (
                    <div style={{
                      display: 'flex',
                      alignItems: 'flex-start',
                      gap: '8px',
                      padding: '12px',
                      marginTop: '8px',
                      background: 'rgba(59, 130, 246, 0.1)',
                      borderRadius: '8px',
                      fontSize: '13px',
                      color: 'var(--accent)',
                      lineHeight: '1.5'
                    }}>
                      <AlertCircle size={16} style={{ marginTop: '2px', flexShrink: 0 }} />
                      <span>{t('settings.general.refreshIntervalLimited')}</span>
                    </div>
                  )}
                </div>
              </div>

              {renderCurrentAccountRefreshRow('antigravity')}

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.antigravityAppPath', 'Antigravity 启动路径')}</div>
                  <div className="row-desc">{t('settings.general.codexAppPathDesc', '留空则使用默认路径')}</div>
                </div>
                <div className="row-control row-control--grow">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                    <input
                      type="text"
                      className="settings-input settings-input--path"
                      value={antigravityAppPath}
                      placeholder={t('settings.general.codexAppPathPlaceholder', '默认路径')}
                      onChange={(e) => setAntigravityAppPath(e.target.value)}
                    />
                    <button
                      className="btn btn-secondary"
                      onClick={() => handlePickAppPath('antigravity')}
                      disabled={isAppPathResetDetecting('antigravity')}
                    >
                      {t('settings.general.codexPathSelect', '选择')}
                    </button>
                    <button
                      className="btn btn-secondary"
                      onClick={() => handleResetAppPath('antigravity')}
                      disabled={isAppPathResetDetecting('antigravity')}
                    >
                      <RefreshCw size={16} className={isAppPathResetDetecting('antigravity') ? 'spin' : undefined} />
                      {isAppPathResetDetecting('antigravity')
                        ? t('common.loading', '加载中...')
                        : getResetLabelByTarget('antigravity')}
                    </button>
                  </div>
                </div>
              </div>

              {antigravitySeamlessSwitchUnlocked && (
                <div className="settings-row">
                  <div className="row-label">
                    <div className="row-title">
                      {t(
                        'settings.general.antigravityDualSwitchNoRestart',
                        '无感双通道切号（不重启）'
                      )}
                    </div>
                    <div className="row-desc">
                      {t(
                        'settings.general.antigravityDualSwitchNoRestartDesc',
                        '切号时同时执行本地落盘与扩展无感切号，不再自动重启 Antigravity。'
                      )}
                    </div>
                  </div>
                  <div className="row-control">
                    <label className="switch">
                      <input
                        type="checkbox"
                        checked={antigravityDualSwitchNoRestartEnabled}
                        onChange={(e) =>
                          setAntigravityDualSwitchNoRestartEnabled(e.target.checked)
                        }
                      />
                      <span className="slider"></span>
                    </label>
                  </div>
                </div>
              )}

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('quickSettings.autoSwitch.enable', '自动切号')}</div>
                  <div className="row-desc">{t('quickSettings.autoSwitch.hint', '当任意模型配额低于阈值时，自动切换到配额最高的账号。')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={autoSwitchEnabled}
                      onChange={(e) => setAutoSwitchEnabled(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>
              {autoSwitchEnabled && (
                <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                  <div className="row-label">
                    <div className="row-title">{t('quickSettings.autoSwitch.threshold', '切号阈值')}</div>
                    <div className="row-desc">{t('quickSettings.autoSwitch.thresholdDesc', '任意模型配额低于此百分比时触发自动切号')}</div>
                  </div>
                  <div className="row-control">
                    {autoSwitchThresholdCustomMode ? (
                      <div className="settings-inline-input">
                        <input
                          type="number"
                          min={0}
                          max={100}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={autoSwitchThreshold}
                          placeholder={t('quickSettings.inputPercent', '输入百分比')}
                          onChange={(e) => setAutoSwitchThreshold(sanitizeNumberInput(e.target.value))}
                        onBlur={() => {
                          const normalized = normalizeNumberInput(autoSwitchThreshold, 0, 100);
                          setAutoSwitchThreshold(normalized);
                          setAutoSwitchThresholdCustomMode(false);
                        }}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') {
                            e.preventDefault();
                            const normalized = normalizeNumberInput(autoSwitchThreshold, 0, 100);
                            setAutoSwitchThreshold(normalized);
                            setAutoSwitchThresholdCustomMode(false);
                          }
                        }}
                      />
                        <span className="settings-input-unit">%</span>
                      </div>
                    ) : (
                      <select
                        className="settings-select"
                        value={autoSwitchThreshold}
                        onChange={(e) => {
                          const val = e.target.value;
                          if (val === 'custom') {
                            setAutoSwitchThresholdCustomMode(true);
                            setAutoSwitchThreshold(autoSwitchThreshold || '20');
                            return;
                          }
                          setAutoSwitchThresholdCustomMode(false);
                          setAutoSwitchThreshold(val);
                        }}
                      >
                        {!autoSwitchThresholdIsPreset && (
                          <option value={autoSwitchThreshold}>{autoSwitchThreshold}%</option>
                        )}
                        <option value="0">0%</option>
                        <option value="20">20%</option>
                        <option value="40">40%</option>
                        <option value="60">60%</option>
                        <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                      </select>
                    )}
                  </div>
                </div>
              )}
              {autoSwitchEnabled && (
                <div className="settings-row settings-row--align-start" style={{ animation: 'fadeUp 0.3s ease both' }}>
                  <div className="row-label">
                    <div className="row-title">
                      {t('settings.general.autoSwitchAccountScope')}
                    </div>
                    <div className="row-desc">
                      {t('settings.general.autoSwitchAccountScopeDesc')}
                    </div>
                  </div>
                  <div className="row-control row-control--grow">
                    <AutoSwitchAccountScopeSelector
                      mode={autoSwitchAccountScopeMode}
                      onModeChange={setAutoSwitchAccountScopeMode}
                      selectedAccountIds={autoSwitchSelectedAccountIds}
                      onSelectedAccountIdsChange={setAutoSwitchSelectedAccountIds}
                      accounts={antigravityScopeAccounts}
                      groups={antigravityAccountGroups}
                      typeOptions={antigravityScopeTypeOptions}
                      useDialog
                    />
                  </div>
                </div>
              )}

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
                  <div className="row-desc">{t('quickSettings.quotaAlert.hint', '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={quotaAlertEnabled}
                      onChange={(e) => setQuotaAlertEnabled(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>
              {quotaAlertEnabled && (
                <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                  <div className="row-label">
                    <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                    <div className="row-desc">{t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}</div>
                  </div>
                  <div className="row-control">
                    {quotaAlertThresholdCustomMode ? (
                      <div className="settings-inline-input">
                        <input
                          type="number"
                          min={0}
                          max={100}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={quotaAlertThreshold}
                          placeholder={t('quickSettings.inputPercent', '输入百分比')}
                          onChange={(e) => setQuotaAlertThreshold(sanitizeNumberInput(e.target.value))}
                          onBlur={() => {
                            const normalized = normalizeNumberInput(quotaAlertThreshold, 0, 100);
                            setQuotaAlertThreshold(normalized);
                            setQuotaAlertThresholdCustomMode(false);
                          }}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter') {
                              e.preventDefault();
                              const normalized = normalizeNumberInput(quotaAlertThreshold, 0, 100);
                              setQuotaAlertThreshold(normalized);
                              setQuotaAlertThresholdCustomMode(false);
                            }
                          }}
                        />
                        <span className="settings-input-unit">%</span>
                      </div>
                    ) : (
                      <select
                        className="settings-select"
                        value={quotaAlertThreshold}
                        onChange={(e) => {
                          const val = e.target.value;
                          if (val === 'custom') {
                            setQuotaAlertThresholdCustomMode(true);
                            setQuotaAlertThreshold(quotaAlertThreshold || '20');
                            return;
                          }
                          setQuotaAlertThresholdCustomMode(false);
                          setQuotaAlertThreshold(val);
                        }}
                      >
                        {!quotaAlertThresholdIsPreset && (
                          <option value={quotaAlertThreshold}>{quotaAlertThreshold}%</option>
                        )}
                        <option value="0">0%</option>
                        <option value="20">20%</option>
                        <option value="40">40%</option>
                        <option value="60">60%</option>
                        <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                      </select>
                    )}
                  </div>
                </div>
              )}
            </div>

              </div>

              <div style={{ order: platformSettingsOrder.codex }}>
                <div className="group-title">{t('settings.general.codexSettingsTitle', 'Codex 设置')}</div>
                <div className="settings-group">
              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.codexAutoRefresh')}</div>
                  <div className="row-desc">{t('settings.general.codexAutoRefreshDesc')}</div>
                </div>
                <div className="row-control">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                    {codexAutoRefreshCustomMode ? (
                      <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                        <input
                          type="number"
                          min={1}
                          max={999}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={codexAutoRefresh}
                          placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                          onChange={(e) => setCodexAutoRefresh(sanitizeNumberInput(e.target.value))}
                        onBlur={() => {
                          const normalized = normalizeNumberInput(codexAutoRefresh, 1, 999);
                          setCodexAutoRefresh(normalized);
                          setCodexAutoRefreshCustomMode(false);
                        }}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') {
                            e.preventDefault();
                            const normalized = normalizeNumberInput(codexAutoRefresh, 1, 999);
                            setCodexAutoRefresh(normalized);
                            setCodexAutoRefreshCustomMode(false);
                          }
                        }}
                      />
                        <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                      </div>
                    ) : (
                      <select
                        className="settings-select"
                        style={{ minWidth: '120px', width: 'auto' }}
                        value={codexAutoRefresh}
                        onChange={(e) => {
                          const val = e.target.value;
                          if (val === 'custom') {
                            setCodexAutoRefreshCustomMode(true);
                            setCodexAutoRefresh(codexAutoRefresh !== '-1' ? codexAutoRefresh : '1');
                            return;
                          }
                          setCodexAutoRefreshCustomMode(false);
                          setCodexAutoRefresh(val);
                        }}
                      >
                        {!codexAutoRefreshIsPreset && (
                          <option value={codexAutoRefresh}>
                            {codexAutoRefresh} {t('settings.general.minutes')}
                          </option>
                        )}
                        <option value="-1">{t('settings.general.autoRefreshDisabled')}</option>
                        <option value="2">2 {t('settings.general.minutes')}</option>
                        <option value="5">5 {t('settings.general.minutes')}</option>
                        <option value="10">10 {t('settings.general.minutes')}</option>
                        <option value="15">15 {t('settings.general.minutes')}</option>
                        <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                      </select>
                    )}
                  </div>
                </div>
              </div>

              {renderCurrentAccountRefreshRow('codex')}

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.codexAppPath', 'Codex 启动路径')}</div>
                  <div className="row-desc">{t('settings.general.codexAppPathDesc', '留空则使用默认路径')}</div>
                </div>
                <div className="row-control row-control--grow">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                    <input
                      type="text"
                      className="settings-input settings-input--path"
                      value={codexAppPath}
                      placeholder={t('settings.general.codexAppPathPlaceholder', '默认路径')}
                      onChange={(e) => setCodexAppPath(e.target.value)}
                    />
                    <button
                      className="btn btn-secondary"
                      onClick={() => handlePickAppPath('codex')}
                      disabled={isAppPathResetDetecting('codex')}
                    >
                      {t('settings.general.codexPathSelect', '选择')}
                    </button>
                    <button
                      className="btn btn-secondary"
                      onClick={() => handleResetAppPath('codex')}
                      disabled={isAppPathResetDetecting('codex')}
                    >
                      <RefreshCw size={16} className={isAppPathResetDetecting('codex') ? 'spin' : undefined} />
                      {isAppPathResetDetecting('codex')
                        ? t('common.loading', '加载中...')
                        : getResetLabelByTarget('codex')}
                    </button>
                  </div>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.codexLaunchOnSwitch', '切换 Codex 时自动启动 Codex App')}</div>
                  <div className="row-desc">{t('settings.general.codexLaunchOnSwitchDesc', '切换账号后自动启动或重启 Codex App')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={codexLaunchOnSwitch}
                      onChange={(e) => setCodexLaunchOnSwitch(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>
              <div className="settings-row settings-row--align-start">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.codexAutoSwitchAccountScope')}</div>
                  <div className="row-desc">
                    {t('settings.general.codexAutoSwitchAccountScopeDesc', {
                      status: codexAutoSwitchEnabled ? t('common.enabled') : t('common.disabled'),
                    })}
                  </div>
                </div>
                <div className="row-control row-control--grow">
                  <AutoSwitchAccountScopeSelector
                    mode={codexAutoSwitchAccountScopeMode}
                    onModeChange={setCodexAutoSwitchAccountScopeMode}
                    selectedAccountIds={codexAutoSwitchSelectedAccountIds}
                    onSelectedAccountIdsChange={setCodexAutoSwitchSelectedAccountIds}
                    accounts={codexScopeAccounts}
                    groups={codexGroups}
                    useDialog
                  />
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.opencodeAuthOverwrite')}</div>
                  <div className="row-desc">{t('settings.general.opencodeAuthOverwriteDesc')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={opencodeAuthOverwriteOnSwitch}
                      onChange={(e) => {
                        const enabled = e.target.checked;
                        setOpencodeAuthOverwriteOnSwitch(enabled);
                        if (!enabled) {
                          setOpencodeSyncOnSwitch(false);
                        }
                      }}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.openclawAuthOverwrite')}</div>
                  <div className="row-desc">{t('settings.general.openclawAuthOverwriteDesc')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={openclawAuthOverwriteOnSwitch}
                      onChange={(e) => setOpenclawAuthOverwriteOnSwitch(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.opencodeRestart')}</div>
                  <div className="row-desc">
                    {opencodeAuthOverwriteOnSwitch
                      ? t('settings.general.opencodeRestartDesc')
                      : t('settings.general.opencodeRestartRequiresOverwrite')}
                  </div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={opencodeSyncOnSwitch}
                      onChange={(e) => setOpencodeSyncOnSwitch(e.target.checked)}
                      disabled={!opencodeAuthOverwriteOnSwitch}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.opencodeAppPath')}</div>
                  <div className="row-desc">
                    {t('settings.general.opencodeAppPathDesc')}
                  </div>
                </div>
                <div className="row-control row-control--grow">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                    <input
                      type="text"
                      className="settings-input settings-input--path"
                      value={opencodeAppPath}
                      placeholder={t('settings.general.opencodeAppPathPlaceholder')}
                      onChange={(e) => setOpencodeAppPath(e.target.value)}
                    />
                    <button
                      className="btn btn-secondary"
                      onClick={() => handlePickAppPath('opencode')}
                      disabled={isAppPathResetDetecting('opencode')}
                    >
                      {t('settings.general.opencodePathSelect', '选择')}
                    </button>
                    <button
                      className="btn btn-secondary"
                      onClick={() => handleResetAppPath('opencode')}
                      disabled={isAppPathResetDetecting('opencode')}
                    >
                      <RefreshCw size={16} className={isAppPathResetDetecting('opencode') ? 'spin' : undefined} />
                      {isAppPathResetDetecting('opencode')
                        ? t('common.loading', '加载中...')
                        : getResetLabelByTarget('opencode')}
                    </button>
                  </div>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
                  <div className="row-desc">{t('quickSettings.quotaAlert.hint', '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={codexQuotaAlertEnabled}
                      onChange={(e) => setCodexQuotaAlertEnabled(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>
              {codexQuotaAlertEnabled && (
                <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                  <div className="row-label">
                    <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                    <div className="row-desc">{t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}</div>
                  </div>
                  <div className="row-control">
                    {codexQuotaAlertThresholdCustomMode ? (
                      <div className="settings-inline-input">
                        <input
                          type="number"
                          min={0}
                          max={100}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={codexQuotaAlertThreshold}
                          placeholder={t('quickSettings.inputPercent', '输入百分比')}
                          onChange={(e) => setCodexQuotaAlertThreshold(sanitizeNumberInput(e.target.value))}
                          onBlur={() => {
                            const normalized = normalizeNumberInput(codexQuotaAlertThreshold, 0, 100);
                            setCodexQuotaAlertThreshold(normalized);
                            setCodexQuotaAlertThresholdCustomMode(false);
                          }}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter') {
                              e.preventDefault();
                              const normalized = normalizeNumberInput(codexQuotaAlertThreshold, 0, 100);
                              setCodexQuotaAlertThreshold(normalized);
                              setCodexQuotaAlertThresholdCustomMode(false);
                            }
                          }}
                        />
                        <span className="settings-input-unit">%</span>
                      </div>
                    ) : (
                      <select
                        className="settings-select"
                        value={codexQuotaAlertThreshold}
                        onChange={(e) => {
                          const val = e.target.value;
                          if (val === 'custom') {
                            setCodexQuotaAlertThresholdCustomMode(true);
                            setCodexQuotaAlertThreshold(codexQuotaAlertThreshold || '20');
                            return;
                          }
                          setCodexQuotaAlertThresholdCustomMode(false);
                          setCodexQuotaAlertThreshold(val);
                        }}
                      >
                        {!codexQuotaAlertThresholdIsPreset && (
                          <option value={codexQuotaAlertThreshold}>{codexQuotaAlertThreshold}%</option>
                        )}
                        <option value="0">0%</option>
                        <option value="20">20%</option>
                        <option value="40">40%</option>
                        <option value="60">60%</option>
                        <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                      </select>
                    )}
                  </div>
                </div>
              )}
            </div>

              </div>

              <div style={{ order: platformSettingsOrder['github-copilot'] }}>
                <div className="group-title">{t('settings.general.githubCopilotSettingsTitle', 'GitHub Copilot 设置')}</div>
                <div className="settings-group">
              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.ghcpAutoRefresh', 'GitHub Copilot 自动刷新配额')}</div>
                  <div className="row-desc">{t('settings.general.ghcpAutoRefreshDesc', '后台自动更新频率')}</div>
                </div>
                <div className="row-control">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                    {ghcpAutoRefreshCustomMode ? (
                      <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                        <input
                          type="number"
                          min={1}
                          max={999}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={ghcpAutoRefresh}
                          placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                          onChange={(e) => setGhcpAutoRefresh(sanitizeNumberInput(e.target.value))}
                        onBlur={() => {
                          const normalized = normalizeNumberInput(ghcpAutoRefresh, 1, 999);
                          setGhcpAutoRefresh(normalized);
                          setGhcpAutoRefreshCustomMode(false);
                        }}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') {
                            e.preventDefault();
                            const normalized = normalizeNumberInput(ghcpAutoRefresh, 1, 999);
                            setGhcpAutoRefresh(normalized);
                            setGhcpAutoRefreshCustomMode(false);
                          }
                        }}
                      />
                        <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                      </div>
                    ) : (
                      <select
                        className="settings-select"
                        style={{ minWidth: '120px', width: 'auto' }}
                        value={ghcpAutoRefresh}
                        onChange={(e) => {
                          const val = e.target.value;
                          if (val === 'custom') {
                            setGhcpAutoRefreshCustomMode(true);
                            setGhcpAutoRefresh(ghcpAutoRefresh !== '-1' ? ghcpAutoRefresh : '1');
                            return;
                          }
                          setGhcpAutoRefreshCustomMode(false);
                          setGhcpAutoRefresh(val);
                        }}
                      >
                        {!ghcpAutoRefreshIsPreset && (
                          <option value={ghcpAutoRefresh}>
                            {ghcpAutoRefresh} {t('settings.general.minutes')}
                          </option>
                        )}
                        <option value="-1">{t('settings.general.autoRefreshDisabled')}</option>
                        <option value="2">2 {t('settings.general.minutes')}</option>
                        <option value="5">5 {t('settings.general.minutes')}</option>
                        <option value="10">10 {t('settings.general.minutes')}</option>
                        <option value="15">15 {t('settings.general.minutes')}</option>
                        <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                      </select>
                    )}
                  </div>
                </div>
              </div>

              {renderCurrentAccountRefreshRow('ghcp')}

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.vscodeAppPath', 'VS Code 启动路径')}</div>
                  <div className="row-desc">{t('settings.general.vscodeAppPathDesc', '留空则使用默认路径')}</div>
                </div>
                <div className="row-control row-control--grow">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                    <input
                      type="text"
                      className="settings-input settings-input--path"
                      value={vscodeAppPath}
                      placeholder={t('settings.general.vscodeAppPathPlaceholder', '默认路径')}
                      onChange={(e) => setVscodeAppPath(e.target.value)}
                    />
                    <button
                      className="btn btn-secondary"
                      onClick={() => handlePickAppPath('vscode')}
                      disabled={isAppPathResetDetecting('vscode')}
                    >
                      {t('settings.general.vscodePathSelect', '选择')}
                    </button>
                    <button
                      className="btn btn-secondary"
                      onClick={() => handleResetAppPath('vscode')}
                      disabled={isAppPathResetDetecting('vscode')}
                    >
                      <RefreshCw size={16} className={isAppPathResetDetecting('vscode') ? 'spin' : undefined} />
                      {isAppPathResetDetecting('vscode')
                        ? t('common.loading', '加载中...')
                        : getResetLabelByTarget('vscode')}
                    </button>
                  </div>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
                  <div className="row-desc">{t('quickSettings.quotaAlert.hint', '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={ghcpQuotaAlertEnabled}
                      onChange={(e) => setGhcpQuotaAlertEnabled(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>
              {ghcpQuotaAlertEnabled && (
                <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                  <div className="row-label">
                    <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                    <div className="row-desc">{t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}</div>
                  </div>
                  <div className="row-control">
                    {ghcpQuotaAlertThresholdCustomMode ? (
                      <div className="settings-inline-input">
                        <input
                          type="number"
                          min={0}
                          max={100}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={ghcpQuotaAlertThreshold}
                          placeholder={t('quickSettings.inputPercent', '输入百分比')}
                          onChange={(e) => setGhcpQuotaAlertThreshold(sanitizeNumberInput(e.target.value))}
                          onBlur={() => {
                            const normalized = normalizeNumberInput(ghcpQuotaAlertThreshold, 0, 100);
                            setGhcpQuotaAlertThreshold(normalized);
                            setGhcpQuotaAlertThresholdCustomMode(false);
                          }}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter') {
                              e.preventDefault();
                              const normalized = normalizeNumberInput(ghcpQuotaAlertThreshold, 0, 100);
                              setGhcpQuotaAlertThreshold(normalized);
                              setGhcpQuotaAlertThresholdCustomMode(false);
                            }
                          }}
                        />
                        <span className="settings-input-unit">%</span>
                      </div>
                    ) : (
                      <select
                        className="settings-select"
                        value={ghcpQuotaAlertThreshold}
                        onChange={(e) => {
                          const val = e.target.value;
                          if (val === 'custom') {
                            setGhcpQuotaAlertThresholdCustomMode(true);
                            setGhcpQuotaAlertThreshold(ghcpQuotaAlertThreshold || '20');
                            return;
                          }
                          setGhcpQuotaAlertThresholdCustomMode(false);
                          setGhcpQuotaAlertThreshold(val);
                        }}
                      >
                        {!ghcpQuotaAlertThresholdIsPreset && (
                          <option value={ghcpQuotaAlertThreshold}>{ghcpQuotaAlertThreshold}%</option>
                        )}
                        <option value="0">0%</option>
                        <option value="20">20%</option>
                        <option value="40">40%</option>
                        <option value="60">60%</option>
                        <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                      </select>
                    )}
                  </div>
                </div>
              )}
            </div>

              </div>

              <div style={{ order: platformSettingsOrder.windsurf }}>
                <div className="group-title">{t('settings.general.windsurfSettingsTitle', 'Windsurf 设置')}</div>
                <div className="settings-group">
              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.windsurfAutoRefresh', 'Windsurf 自动刷新配额')}</div>
                  <div className="row-desc">{t('settings.general.windsurfAutoRefreshDesc', '后台自动更新频率')}</div>
                </div>
                <div className="row-control">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                    {windsurfAutoRefreshCustomMode ? (
                      <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                        <input
                          type="number"
                          min={1}
                          max={999}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={windsurfAutoRefresh}
                          placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                          onChange={(e) => setWindsurfAutoRefresh(sanitizeNumberInput(e.target.value))}
                        onBlur={() => {
                          const normalized = normalizeNumberInput(windsurfAutoRefresh, 1, 999);
                          setWindsurfAutoRefresh(normalized);
                          setWindsurfAutoRefreshCustomMode(false);
                        }}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') {
                            e.preventDefault();
                            const normalized = normalizeNumberInput(windsurfAutoRefresh, 1, 999);
                            setWindsurfAutoRefresh(normalized);
                            setWindsurfAutoRefreshCustomMode(false);
                          }
                        }}
                      />
                        <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                      </div>
                    ) : (
                      <select
                        className="settings-select"
                        style={{ minWidth: '120px', width: 'auto' }}
                        value={windsurfAutoRefresh}
                        onChange={(e) => {
                          const val = e.target.value;
                          if (val === 'custom') {
                            setWindsurfAutoRefreshCustomMode(true);
                            setWindsurfAutoRefresh(windsurfAutoRefresh !== '-1' ? windsurfAutoRefresh : '1');
                            return;
                          }
                          setWindsurfAutoRefreshCustomMode(false);
                          setWindsurfAutoRefresh(val);
                        }}
                      >
                        {!windsurfAutoRefreshIsPreset && (
                          <option value={windsurfAutoRefresh}>
                            {windsurfAutoRefresh} {t('settings.general.minutes')}
                          </option>
                        )}
                        <option value="-1">{t('settings.general.autoRefreshDisabled')}</option>
                        <option value="2">2 {t('settings.general.minutes')}</option>
                        <option value="5">5 {t('settings.general.minutes')}</option>
                        <option value="10">10 {t('settings.general.minutes')}</option>
                        <option value="15">15 {t('settings.general.minutes')}</option>
                        <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                      </select>
                    )}
                  </div>
                </div>
              </div>

              {renderCurrentAccountRefreshRow('windsurf')}

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.windsurfAppPath', 'Windsurf 启动路径')}</div>
                  <div className="row-desc">{t('settings.general.windsurfAppPathDesc', '留空则使用默认路径')}</div>
                </div>
                <div className="row-control row-control--grow">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                    <input
                      type="text"
                      className="settings-input settings-input--path"
                      value={windsurfAppPath}
                      placeholder={t('settings.general.windsurfAppPathPlaceholder', '默认路径')}
                      onChange={(e) => setWindsurfAppPath(e.target.value)}
                    />
                    <button
                      className="btn btn-secondary"
                      onClick={() => handlePickAppPath('windsurf')}
                      disabled={isAppPathResetDetecting('windsurf')}
                    >
                      {t('settings.general.windsurfPathSelect', '选择')}
                    </button>
                    <button
                      className="btn btn-secondary"
                      onClick={() => handleResetAppPath('windsurf')}
                      disabled={isAppPathResetDetecting('windsurf')}
                    >
                      <RefreshCw size={16} className={isAppPathResetDetecting('windsurf') ? 'spin' : undefined} />
                      {isAppPathResetDetecting('windsurf')
                        ? t('common.loading', '加载中...')
                        : getResetLabelByTarget('windsurf')}
                    </button>
                  </div>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
                  <div className="row-desc">{t('quickSettings.quotaAlert.hint', '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={windsurfQuotaAlertEnabled}
                      onChange={(e) => setWindsurfQuotaAlertEnabled(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>
              {windsurfQuotaAlertEnabled && (
                <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                  <div className="row-label">
                    <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                    <div className="row-desc">{t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}</div>
                  </div>
                  <div className="row-control">
                    {windsurfQuotaAlertThresholdCustomMode ? (
                      <div className="settings-inline-input">
                        <input
                          type="number"
                          min={0}
                          max={100}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={windsurfQuotaAlertThreshold}
                          placeholder={t('quickSettings.inputPercent', '输入百分比')}
                          onChange={(e) => setWindsurfQuotaAlertThreshold(sanitizeNumberInput(e.target.value))}
                          onBlur={() => {
                            const normalized = normalizeNumberInput(windsurfQuotaAlertThreshold, 0, 100);
                            setWindsurfQuotaAlertThreshold(normalized);
                            setWindsurfQuotaAlertThresholdCustomMode(false);
                          }}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter') {
                              e.preventDefault();
                              const normalized = normalizeNumberInput(windsurfQuotaAlertThreshold, 0, 100);
                              setWindsurfQuotaAlertThreshold(normalized);
                              setWindsurfQuotaAlertThresholdCustomMode(false);
                            }
                          }}
                        />
                        <span className="settings-input-unit">%</span>
                      </div>
                    ) : (
                      <select
                        className="settings-select"
                        value={windsurfQuotaAlertThreshold}
                        onChange={(e) => {
                          const val = e.target.value;
                          if (val === 'custom') {
                            setWindsurfQuotaAlertThresholdCustomMode(true);
                            setWindsurfQuotaAlertThreshold(windsurfQuotaAlertThreshold || '20');
                            return;
                          }
                          setWindsurfQuotaAlertThresholdCustomMode(false);
                          setWindsurfQuotaAlertThreshold(val);
                        }}
                      >
                        {!windsurfQuotaAlertThresholdIsPreset && (
                          <option value={windsurfQuotaAlertThreshold}>{windsurfQuotaAlertThreshold}%</option>
                        )}
                        <option value="0">0%</option>
                        <option value="20">20%</option>
                        <option value="40">40%</option>
                        <option value="60">60%</option>
                        <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                      </select>
                    )}
                  </div>
                </div>
              )}
            </div>

              </div>

              <div style={{ order: platformSettingsOrder.kiro }}>
                <div className="group-title">{t('settings.general.kiroSettingsTitle', 'Kiro 设置')}</div>
                <div className="settings-group">
              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.kiroAutoRefresh', 'Kiro 自动刷新配额')}</div>
                  <div className="row-desc">{t('settings.general.kiroAutoRefreshDesc', '后台自动更新频率')}</div>
                </div>
                <div className="row-control">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                    {kiroAutoRefreshCustomMode ? (
                      <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                        <input
                          type="number"
                          min={1}
                          max={999}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={kiroAutoRefresh}
                          placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                          onChange={(e) => setKiroAutoRefresh(sanitizeNumberInput(e.target.value))}
                          onBlur={() => {
                            const normalized = normalizeNumberInput(kiroAutoRefresh, 1, 999);
                            setKiroAutoRefresh(normalized);
                            setKiroAutoRefreshCustomMode(false);
                          }}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter') {
                              e.preventDefault();
                              const normalized = normalizeNumberInput(kiroAutoRefresh, 1, 999);
                              setKiroAutoRefresh(normalized);
                              setKiroAutoRefreshCustomMode(false);
                            }
                          }}
                        />
                        <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                      </div>
                    ) : (
                      <select
                        className="settings-select"
                        style={{ minWidth: '120px', width: 'auto' }}
                        value={kiroAutoRefresh}
                        onChange={(e) => {
                          const val = e.target.value;
                          if (val === 'custom') {
                            setKiroAutoRefreshCustomMode(true);
                            setKiroAutoRefresh(kiroAutoRefresh !== '-1' ? kiroAutoRefresh : '1');
                            return;
                          }
                          setKiroAutoRefreshCustomMode(false);
                          setKiroAutoRefresh(val);
                        }}
                      >
                        {!kiroAutoRefreshIsPreset && (
                          <option value={kiroAutoRefresh}>
                            {kiroAutoRefresh} {t('settings.general.minutes')}
                          </option>
                        )}
                        <option value="-1">{t('settings.general.autoRefreshDisabled')}</option>
                        <option value="2">2 {t('settings.general.minutes')}</option>
                        <option value="5">5 {t('settings.general.minutes')}</option>
                        <option value="10">10 {t('settings.general.minutes')}</option>
                        <option value="15">15 {t('settings.general.minutes')}</option>
                        <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                      </select>
                    )}
                  </div>
                </div>
              </div>

              {renderCurrentAccountRefreshRow('kiro')}

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.kiroAppPath', 'Kiro 启动路径')}</div>
                  <div className="row-desc">{t('settings.general.kiroAppPathDesc', '留空则使用默认路径')}</div>
                </div>
                <div className="row-control row-control--grow">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                    <input
                      type="text"
                      className="settings-input settings-input--path"
                      value={kiroAppPath}
                      placeholder={t('settings.general.kiroAppPathPlaceholder', '默认路径')}
                      onChange={(e) => setKiroAppPath(e.target.value)}
                    />
                    <button
                      className="btn btn-secondary"
                      onClick={() => handlePickAppPath('kiro')}
                      disabled={isAppPathResetDetecting('kiro')}
                    >
                      {t('settings.general.kiroPathSelect', '选择')}
                    </button>
                    <button
                      className="btn btn-secondary"
                      onClick={() => handleResetAppPath('kiro')}
                      disabled={isAppPathResetDetecting('kiro')}
                    >
                      <RefreshCw size={16} className={isAppPathResetDetecting('kiro') ? 'spin' : undefined} />
                      {isAppPathResetDetecting('kiro')
                        ? t('common.loading', '加载中...')
                        : getResetLabelByTarget('kiro')}
                    </button>
                  </div>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
                  <div className="row-desc">{t('quickSettings.quotaAlert.hint', '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={kiroQuotaAlertEnabled}
                      onChange={(e) => setKiroQuotaAlertEnabled(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>
              {kiroQuotaAlertEnabled && (
                <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                  <div className="row-label">
                    <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                    <div className="row-desc">{t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}</div>
                  </div>
                  <div className="row-control">
                    {kiroQuotaAlertThresholdCustomMode ? (
                      <div className="settings-inline-input">
                        <input
                          type="number"
                          min={0}
                          max={100}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={kiroQuotaAlertThreshold}
                          placeholder={t('quickSettings.inputPercent', '输入百分比')}
                          onChange={(e) => setKiroQuotaAlertThreshold(sanitizeNumberInput(e.target.value))}
                          onBlur={() => {
                            const normalized = normalizeNumberInput(kiroQuotaAlertThreshold, 0, 100);
                            setKiroQuotaAlertThreshold(normalized);
                            setKiroQuotaAlertThresholdCustomMode(false);
                          }}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter') {
                              e.preventDefault();
                              const normalized = normalizeNumberInput(kiroQuotaAlertThreshold, 0, 100);
                              setKiroQuotaAlertThreshold(normalized);
                              setKiroQuotaAlertThresholdCustomMode(false);
                            }
                          }}
                        />
                        <span className="settings-input-unit">%</span>
                      </div>
                    ) : (
                      <select
                        className="settings-select"
                        value={kiroQuotaAlertThreshold}
                        onChange={(e) => {
                          const val = e.target.value;
                          if (val === 'custom') {
                            setKiroQuotaAlertThresholdCustomMode(true);
                            setKiroQuotaAlertThreshold(kiroQuotaAlertThreshold || '20');
                            return;
                          }
                          setKiroQuotaAlertThresholdCustomMode(false);
                          setKiroQuotaAlertThreshold(val);
                        }}
                      >
                        {!kiroQuotaAlertThresholdIsPreset && (
                          <option value={kiroQuotaAlertThreshold}>{kiroQuotaAlertThreshold}%</option>
                        )}
                        <option value="0">0%</option>
                        <option value="20">20%</option>
                        <option value="40">40%</option>
                        <option value="60">60%</option>
                        <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                      </select>
                    )}
                  </div>
                </div>
              )}
            </div>
              </div>

              <div style={{ order: platformSettingsOrder.codebuddy }}>
                <div className="group-title">{t('settings.general.codebuddySettingsTitle', 'CodeBuddy 设置')}</div>
                <div className="settings-group">
              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.codebuddyAutoRefresh', 'CodeBuddy 自动刷新配额')}</div>
                  <div className="row-desc">{t('settings.general.codebuddyAutoRefreshDesc', '后台自动更新频率')}</div>
                </div>
                <div className="row-control">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                    {codebuddyAutoRefreshCustomMode ? (
                      <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                        <input
                          type="number"
                          min={1}
                          max={999}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={codebuddyAutoRefresh}
                          placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                          onChange={(e) => setCodebuddyAutoRefresh(sanitizeNumberInput(e.target.value))}
                          onBlur={() => {
                            const normalized = normalizeNumberInput(codebuddyAutoRefresh, 1, 999);
                            if (REFRESH_PRESET_VALUES.includes(normalized)) {
                              setCodebuddyAutoRefreshCustomMode(false);
                            }
                            setCodebuddyAutoRefresh(normalized);
                          }}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter') {
                              e.preventDefault();
                              const normalized = normalizeNumberInput(codebuddyAutoRefresh, 1, 999);
                              setCodebuddyAutoRefreshCustomMode(false);
                              setCodebuddyAutoRefresh(normalized);
                            }
                          }}
                        />
                        <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                      </div>
                    ) : (
                      <select
                        className="settings-select"
                        style={{ minWidth: '120px', width: 'auto' }}
                        value={codebuddyAutoRefresh}
                        onChange={(e) => {
                          const val = e.target.value;
                          if (val === 'custom') {
                            setCodebuddyAutoRefreshCustomMode(true);
                            setCodebuddyAutoRefresh(codebuddyAutoRefresh !== '-1' ? codebuddyAutoRefresh : '1');
                            return;
                          }
                          setCodebuddyAutoRefreshCustomMode(false);
                          setCodebuddyAutoRefresh(val);
                        }}
                      >
                        {!codebuddyAutoRefreshIsPreset && (
                          <option value={codebuddyAutoRefresh}>
                            {codebuddyAutoRefresh} {t('settings.general.minutes')}
                          </option>
                        )}
                        <option value="-1">{t('settings.general.autoRefreshDisabled')}</option>
                        <option value="2">2 {t('settings.general.minutes')}</option>
                        <option value="5">5 {t('settings.general.minutes')}</option>
                        <option value="10">10 {t('settings.general.minutes')}</option>
                        <option value="15">15 {t('settings.general.minutes')}</option>
                        <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                      </select>
                    )}
                  </div>
                </div>
              </div>

              {renderCurrentAccountRefreshRow('codebuddy')}

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.general.codebuddyAppPath', 'CodeBuddy 启动路径')}</div>
                  <div className="row-desc">{t('settings.general.codebuddyAppPathDesc', '留空则使用默认路径')}</div>
                </div>
                <div className="row-control row-control--grow">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                    <input
                      type="text"
                      className="settings-input settings-input--path"
                      value={codebuddyAppPath}
                      placeholder={t('settings.general.codebuddyAppPathPlaceholder', '默认路径')}
                      onChange={(e) => setCodebuddyAppPath(e.target.value)}
                    />
                    <button
                      className="btn btn-secondary"
                      onClick={() => handlePickAppPath('codebuddy')}
                      disabled={isAppPathResetDetecting('codebuddy')}
                    >
                      {t('settings.general.codebuddyPathSelect', '选择')}
                    </button>
                    <button
                      className="btn btn-secondary"
                      onClick={() => handleResetAppPath('codebuddy')}
                      disabled={isAppPathResetDetecting('codebuddy')}
                    >
                      <RefreshCw size={16} className={isAppPathResetDetecting('codebuddy') ? 'spin' : undefined} />
                      {isAppPathResetDetecting('codebuddy')
                        ? t('common.loading', '加载中...')
                        : getResetLabelByTarget('codebuddy')}
                    </button>
                  </div>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
                  <div className="row-desc">{t('quickSettings.quotaAlert.hint', '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={codebuddyQuotaAlertEnabled}
                      onChange={(e) => setCodebuddyQuotaAlertEnabled(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>
              {codebuddyQuotaAlertEnabled && (
                <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                  <div className="row-label">
                    <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                    <div className="row-desc">{t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}</div>
                  </div>
                  <div className="row-control">
                    {codebuddyQuotaAlertThresholdCustomMode ? (
                      <div className="settings-inline-input">
                        <input
                          type="number"
                          min={0}
                          max={100}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={codebuddyQuotaAlertThreshold}
                          placeholder={t('quickSettings.inputPercent', '输入百分比')}
                          onChange={(e) => setCodebuddyQuotaAlertThreshold(sanitizeNumberInput(e.target.value))}
                          onBlur={() => {
                            const normalized = normalizeNumberInput(codebuddyQuotaAlertThreshold, 0, 100);
                            if (THRESHOLD_PRESET_VALUES.includes(normalized)) {
                              setCodebuddyQuotaAlertThresholdCustomMode(false);
                            }
                            setCodebuddyQuotaAlertThreshold(normalized);
                          }}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter') {
                              e.preventDefault();
                              const normalized = normalizeNumberInput(codebuddyQuotaAlertThreshold, 0, 100);
                              setCodebuddyQuotaAlertThresholdCustomMode(false);
                              setCodebuddyQuotaAlertThreshold(normalized);
                            }
                          }}
                        />
                        <span className="settings-input-unit">%</span>
                      </div>
                    ) : (
                      <select
                        className="settings-select"
                        value={codebuddyQuotaAlertThreshold}
                        onChange={(e) => {
                          const val = e.target.value;
                          if (val === 'custom') {
                            setCodebuddyQuotaAlertThresholdCustomMode(true);
                            setCodebuddyQuotaAlertThreshold(codebuddyQuotaAlertThreshold || '20');
                            return;
                          }
                          setCodebuddyQuotaAlertThresholdCustomMode(false);
                          setCodebuddyQuotaAlertThreshold(val);
                        }}
                      >
                        {!codebuddyQuotaAlertThresholdIsPreset && (
                          <option value={codebuddyQuotaAlertThreshold}>{codebuddyQuotaAlertThreshold}%</option>
                        )}
                        <option value="0">0%</option>
                        <option value="20">20%</option>
                        <option value="40">40%</option>
                        <option value="60">60%</option>
                        <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                      </select>
                    )}
                  </div>
                </div>
              )}
            </div>
              </div>

              <div style={{ order: platformSettingsOrder.codebuddy_cn }}>
                <div className="group-title">{t('settings.general.codebuddyCnSettingsTitle', 'CodeBuddy CN 设置')}</div>
                <div className="settings-group">
                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('settings.general.codebuddyCnAutoRefresh', 'CodeBuddy CN 自动刷新配额')}</div>
                      <div className="row-desc">{t('settings.general.codebuddyCnAutoRefreshDesc', '后台自动更新频率')}</div>
                    </div>
                    <div className="row-control">
                      <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                        {codebuddyCnAutoRefreshCustomMode ? (
                          <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                            <input
                              type="number"
                              min={1}
                              max={999}
                              className="settings-select settings-select--input-mode settings-select--with-unit"
                              value={codebuddyCnAutoRefresh}
                              placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                              onChange={(e) => setCodebuddyCnAutoRefresh(sanitizeNumberInput(e.target.value))}
                              onBlur={() => {
                                const normalized = normalizeNumberInput(codebuddyCnAutoRefresh, 1, 999);
                                if (REFRESH_PRESET_VALUES.includes(normalized)) {
                                  setCodebuddyCnAutoRefreshCustomMode(false);
                                }
                                setCodebuddyCnAutoRefresh(normalized);
                              }}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  const normalized = normalizeNumberInput(codebuddyCnAutoRefresh, 1, 999);
                                  setCodebuddyCnAutoRefreshCustomMode(false);
                                  setCodebuddyCnAutoRefresh(normalized);
                                }
                              }}
                            />
                            <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                          </div>
                        ) : (
                          <select
                            className="settings-select"
                            style={{ minWidth: '120px', width: 'auto' }}
                            value={codebuddyCnAutoRefresh}
                            onChange={(e) => {
                              const val = e.target.value;
                              if (val === 'custom') {
                                setCodebuddyCnAutoRefreshCustomMode(true);
                                setCodebuddyCnAutoRefresh(codebuddyCnAutoRefresh !== '-1' ? codebuddyCnAutoRefresh : '1');
                                return;
                              }
                              setCodebuddyCnAutoRefreshCustomMode(false);
                              setCodebuddyCnAutoRefresh(val);
                            }}
                          >
                            {!codebuddyCnAutoRefreshIsPreset && (
                              <option value={codebuddyCnAutoRefresh}>
                                {codebuddyCnAutoRefresh} {t('settings.general.minutes')}
                              </option>
                            )}
                            <option value="-1">{t('settings.general.autoRefreshDisabled')}</option>
                            <option value="2">2 {t('settings.general.minutes')}</option>
                            <option value="5">5 {t('settings.general.minutes')}</option>
                            <option value="10">10 {t('settings.general.minutes')}</option>
                            <option value="15">15 {t('settings.general.minutes')}</option>
                            <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                          </select>
                        )}
                      </div>
                    </div>
                  </div>

                  {renderCurrentAccountRefreshRow('codebuddy_cn')}

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('settings.general.codebuddyCnAppPath', 'CodeBuddy CN 启动路径')}</div>
                      <div className="row-desc">{t('settings.general.codebuddyCnAppPathDesc', '留空则使用默认路径')}</div>
                    </div>
                    <div className="row-control row-control--grow">
                      <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                        <input
                          type="text"
                          className="settings-input settings-input--path"
                          value={codebuddyCnAppPath}
                          placeholder={t('settings.general.codebuddyCnAppPathPlaceholder', '默认路径')}
                          onChange={(e) => setCodebuddyCnAppPath(e.target.value)}
                        />
                        <button
                          className="btn btn-secondary"
                          onClick={() => handlePickAppPath('codebuddy_cn')}
                          disabled={isAppPathResetDetecting('codebuddy_cn')}
                        >
                          {t('settings.general.codebuddyCnPathSelect', '选择')}
                        </button>
                        <button
                          className="btn btn-secondary"
                          onClick={() => handleResetAppPath('codebuddy_cn')}
                          disabled={isAppPathResetDetecting('codebuddy_cn')}
                        >
                          <RefreshCw size={16} className={isAppPathResetDetecting('codebuddy_cn') ? 'spin' : undefined} />
                          {isAppPathResetDetecting('codebuddy_cn')
                            ? t('common.loading', '加载中...')
                            : getResetLabelByTarget('codebuddy_cn')}
                        </button>
                      </div>
                    </div>
                  </div>

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
                      <div className="row-desc">{t('quickSettings.quotaAlert.hint', '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。')}</div>
                    </div>
                    <div className="row-control">
                      <label className="switch">
                        <input
                          type="checkbox"
                          checked={codebuddyCnQuotaAlertEnabled}
                          onChange={(e) => setCodebuddyCnQuotaAlertEnabled(e.target.checked)}
                        />
                        <span className="slider"></span>
                      </label>
                    </div>
                  </div>
                  {codebuddyCnQuotaAlertEnabled && (
                    <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                      <div className="row-label">
                        <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                        <div className="row-desc">{t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}</div>
                      </div>
                      <div className="row-control">
                        {codebuddyCnQuotaAlertThresholdCustomMode ? (
                          <div className="settings-inline-input">
                            <input
                              type="number"
                              min={0}
                              max={100}
                              className="settings-select settings-select--input-mode settings-select--with-unit"
                              value={codebuddyCnQuotaAlertThreshold}
                              placeholder={t('quickSettings.inputPercent', '输入百分比')}
                              onChange={(e) => setCodebuddyCnQuotaAlertThreshold(sanitizeNumberInput(e.target.value))}
                              onBlur={() => {
                                const normalized = normalizeNumberInput(codebuddyCnQuotaAlertThreshold, 0, 100);
                                if (THRESHOLD_PRESET_VALUES.includes(normalized)) {
                                  setCodebuddyCnQuotaAlertThresholdCustomMode(false);
                                }
                                setCodebuddyCnQuotaAlertThreshold(normalized);
                              }}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  const normalized = normalizeNumberInput(codebuddyCnQuotaAlertThreshold, 0, 100);
                                  setCodebuddyCnQuotaAlertThresholdCustomMode(false);
                                  setCodebuddyCnQuotaAlertThreshold(normalized);
                                }
                              }}
                            />
                            <span className="settings-input-unit">%</span>
                          </div>
                        ) : (
                          <select
                            className="settings-select"
                            value={codebuddyCnQuotaAlertThreshold}
                            onChange={(e) => {
                              const val = e.target.value;
                              if (val === 'custom') {
                                setCodebuddyCnQuotaAlertThresholdCustomMode(true);
                                setCodebuddyCnQuotaAlertThreshold(codebuddyCnQuotaAlertThreshold || '20');
                                return;
                              }
                              setCodebuddyCnQuotaAlertThresholdCustomMode(false);
                              setCodebuddyCnQuotaAlertThreshold(val);
                            }}
                          >
                            {!codebuddyCnQuotaAlertThresholdIsPreset && (
                              <option value={codebuddyCnQuotaAlertThreshold}>{codebuddyCnQuotaAlertThreshold}%</option>
                            )}
                            <option value="0">0%</option>
                            <option value="20">20%</option>
                            <option value="40">40%</option>
                            <option value="60">60%</option>
                            <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                          </select>
                        )}
                      </div>
                    </div>
                  )}
                </div>
              </div>

              <div style={{ order: platformSettingsOrder.qoder }}>
                <div className="group-title">{t('quickSettings.qoder.title', 'Qoder 设置')}</div>
                <div className="settings-group">
                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('settings.general.qoderAutoRefresh', 'Qoder 自动刷新配额')}</div>
                      <div className="row-desc">{t('settings.general.qoderAutoRefreshDesc', '后台自动更新频率')}</div>
                    </div>
                    <div className="row-control">
                      <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                        {qoderAutoRefreshCustomMode ? (
                          <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                            <input
                              type="number"
                              min={1}
                              max={999}
                              className="settings-select settings-select--input-mode settings-select--with-unit"
                              value={qoderAutoRefresh}
                              placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                              onChange={(e) => setQoderAutoRefresh(sanitizeNumberInput(e.target.value))}
                              onBlur={() => {
                                const normalized = normalizeNumberInput(qoderAutoRefresh, 1, 999);
                                if (REFRESH_PRESET_VALUES.includes(normalized)) {
                                  setQoderAutoRefreshCustomMode(false);
                                }
                                setQoderAutoRefresh(normalized);
                              }}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  const normalized = normalizeNumberInput(qoderAutoRefresh, 1, 999);
                                  setQoderAutoRefreshCustomMode(false);
                                  setQoderAutoRefresh(normalized);
                                }
                              }}
                            />
                            <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                          </div>
                        ) : (
                          <select
                            className="settings-select"
                            style={{ minWidth: '120px', width: 'auto' }}
                            value={qoderAutoRefresh}
                            onChange={(e) => {
                              const val = e.target.value;
                              if (val === 'custom') {
                                setQoderAutoRefreshCustomMode(true);
                                setQoderAutoRefresh(qoderAutoRefresh !== '-1' ? qoderAutoRefresh : '1');
                                return;
                              }
                              setQoderAutoRefreshCustomMode(false);
                              setQoderAutoRefresh(val);
                            }}
                          >
                            {!qoderAutoRefreshIsPreset && (
                              <option value={qoderAutoRefresh}>
                                {qoderAutoRefresh} {t('settings.general.minutes')}
                              </option>
                            )}
                            <option value="-1">{t('settings.general.autoRefreshDisabled')}</option>
                            <option value="2">2 {t('settings.general.minutes')}</option>
                            <option value="5">5 {t('settings.general.minutes')}</option>
                            <option value="10">10 {t('settings.general.minutes')}</option>
                            <option value="15">15 {t('settings.general.minutes')}</option>
                            <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                          </select>
                        )}
                      </div>
                    </div>
                  </div>

                  {renderCurrentAccountRefreshRow('qoder')}

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('settings.general.qoderAppPath', 'Qoder 启动路径')}</div>
                      <div className="row-desc">{t('settings.general.qoderAppPathDesc', '留空则使用默认路径')}</div>
                    </div>
                    <div className="row-control row-control--grow">
                      <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                        <input
                          type="text"
                          className="settings-input settings-input--path"
                          value={qoderAppPath}
                          placeholder={t('settings.general.qoderAppPathPlaceholder', '默认路径')}
                          onChange={(e) => setQoderAppPath(e.target.value)}
                        />
                        <button
                          className="btn btn-secondary"
                          onClick={() => handlePickAppPath('qoder')}
                          disabled={isAppPathResetDetecting('qoder')}
                        >
                          {t('settings.general.qoderPathSelect', '选择')}
                        </button>
                        <button
                          className="btn btn-secondary"
                          onClick={() => handleResetAppPath('qoder')}
                          disabled={isAppPathResetDetecting('qoder')}
                        >
                          <RefreshCw size={16} className={isAppPathResetDetecting('qoder') ? 'spin' : undefined} />
                          {isAppPathResetDetecting('qoder')
                            ? t('common.loading', '加载中...')
                            : getResetLabelByTarget('qoder')}
                        </button>
                      </div>
                    </div>
                  </div>

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
                      <div className="row-desc">{t('quickSettings.quotaAlert.hint', '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。')}</div>
                    </div>
                    <div className="row-control">
                      <label className="switch">
                        <input
                          type="checkbox"
                          checked={qoderQuotaAlertEnabled}
                          onChange={(e) => setQoderQuotaAlertEnabled(e.target.checked)}
                        />
                        <span className="slider"></span>
                      </label>
                    </div>
                  </div>
                  {qoderQuotaAlertEnabled && (
                    <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                      <div className="row-label">
                        <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                        <div className="row-desc">{t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}</div>
                      </div>
                      <div className="row-control">
                        {qoderQuotaAlertThresholdCustomMode ? (
                          <div className="settings-inline-input">
                            <input
                              type="number"
                              min={0}
                              max={100}
                              className="settings-select settings-select--input-mode settings-select--with-unit"
                              value={qoderQuotaAlertThreshold}
                              placeholder={t('quickSettings.inputPercent', '输入百分比')}
                              onChange={(e) => setQoderQuotaAlertThreshold(sanitizeNumberInput(e.target.value))}
                              onBlur={() => {
                                const normalized = normalizeNumberInput(qoderQuotaAlertThreshold, 0, 100);
                                if (THRESHOLD_PRESET_VALUES.includes(normalized)) {
                                  setQoderQuotaAlertThresholdCustomMode(false);
                                }
                                setQoderQuotaAlertThreshold(normalized);
                              }}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  const normalized = normalizeNumberInput(qoderQuotaAlertThreshold, 0, 100);
                                  setQoderQuotaAlertThresholdCustomMode(false);
                                  setQoderQuotaAlertThreshold(normalized);
                                }
                              }}
                            />
                            <span className="settings-input-unit">%</span>
                          </div>
                        ) : (
                          <select
                            className="settings-select"
                            value={qoderQuotaAlertThreshold}
                            onChange={(e) => {
                              const val = e.target.value;
                              if (val === 'custom') {
                                setQoderQuotaAlertThresholdCustomMode(true);
                                setQoderQuotaAlertThreshold(qoderQuotaAlertThreshold || '20');
                                return;
                              }
                              setQoderQuotaAlertThresholdCustomMode(false);
                              setQoderQuotaAlertThreshold(val);
                            }}
                          >
                            {!qoderQuotaAlertThresholdIsPreset && (
                              <option value={qoderQuotaAlertThreshold}>{qoderQuotaAlertThreshold}%</option>
                            )}
                            <option value="0">0%</option>
                            <option value="20">20%</option>
                            <option value="40">40%</option>
                            <option value="60">60%</option>
                            <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                          </select>
                        )}
                      </div>
                    </div>
                  )}
                </div>
              </div>

              <div style={{ order: platformSettingsOrder.trae }}>
                <div className="group-title">{t('quickSettings.trae.title', 'Trae 设置')}</div>
                <div className="settings-group">
                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('settings.general.traeAutoRefresh', 'Trae 自动刷新配额')}</div>
                      <div className="row-desc">{t('settings.general.traeAutoRefreshDesc', '后台自动更新频率')}</div>
                    </div>
                    <div className="row-control">
                      <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                        {traeAutoRefreshCustomMode ? (
                          <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                            <input
                              type="number"
                              min={1}
                              max={999}
                              className="settings-select settings-select--input-mode settings-select--with-unit"
                              value={traeAutoRefresh}
                              placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                              onChange={(e) => setTraeAutoRefresh(sanitizeNumberInput(e.target.value))}
                              onBlur={() => {
                                const normalized = normalizeNumberInput(traeAutoRefresh, 1, 999);
                                if (REFRESH_PRESET_VALUES.includes(normalized)) {
                                  setTraeAutoRefreshCustomMode(false);
                                }
                                setTraeAutoRefresh(normalized);
                              }}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  const normalized = normalizeNumberInput(traeAutoRefresh, 1, 999);
                                  setTraeAutoRefreshCustomMode(false);
                                  setTraeAutoRefresh(normalized);
                                }
                              }}
                            />
                            <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                          </div>
                        ) : (
                          <select
                            className="settings-select"
                            style={{ minWidth: '120px', width: 'auto' }}
                            value={traeAutoRefresh}
                            onChange={(e) => {
                              const val = e.target.value;
                              if (val === 'custom') {
                                setTraeAutoRefreshCustomMode(true);
                                setTraeAutoRefresh(traeAutoRefresh !== '-1' ? traeAutoRefresh : '1');
                                return;
                              }
                              setTraeAutoRefreshCustomMode(false);
                              setTraeAutoRefresh(val);
                            }}
                          >
                            {!traeAutoRefreshIsPreset && (
                              <option value={traeAutoRefresh}>
                                {traeAutoRefresh} {t('settings.general.minutes')}
                              </option>
                            )}
                            <option value="-1">{t('settings.general.autoRefreshDisabled')}</option>
                            <option value="2">2 {t('settings.general.minutes')}</option>
                            <option value="5">5 {t('settings.general.minutes')}</option>
                            <option value="10">10 {t('settings.general.minutes')}</option>
                            <option value="15">15 {t('settings.general.minutes')}</option>
                            <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                          </select>
                        )}
                      </div>
                    </div>
                  </div>

                  {renderCurrentAccountRefreshRow('trae')}

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('settings.general.traeAppPath', 'Trae 启动路径')}</div>
                      <div className="row-desc">{t('settings.general.traeAppPathDesc', '留空则使用默认路径')}</div>
                    </div>
                    <div className="row-control row-control--grow">
                      <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                        <input
                          type="text"
                          className="settings-input settings-input--path"
                          value={traeAppPath}
                          placeholder={t('settings.general.traeAppPathPlaceholder', '默认路径')}
                          onChange={(e) => setTraeAppPath(e.target.value)}
                        />
                        <button
                          className="btn btn-secondary"
                          onClick={() => handlePickAppPath('trae')}
                          disabled={isAppPathResetDetecting('trae')}
                        >
                          {t('settings.general.traePathSelect', '选择')}
                        </button>
                        <button
                          className="btn btn-secondary"
                          onClick={() => handleResetAppPath('trae')}
                          disabled={isAppPathResetDetecting('trae')}
                        >
                          <RefreshCw size={16} className={isAppPathResetDetecting('trae') ? 'spin' : undefined} />
                          {isAppPathResetDetecting('trae')
                            ? t('common.loading', '加载中...')
                            : getResetLabelByTarget('trae')}
                        </button>
                      </div>
                    </div>
                  </div>

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
                      <div className="row-desc">{t('quickSettings.quotaAlert.hint', '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。')}</div>
                    </div>
                    <div className="row-control">
                      <label className="switch">
                        <input
                          type="checkbox"
                          checked={traeQuotaAlertEnabled}
                          onChange={(e) => setTraeQuotaAlertEnabled(e.target.checked)}
                        />
                        <span className="slider"></span>
                      </label>
                    </div>
                  </div>
                  {traeQuotaAlertEnabled && (
                    <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                      <div className="row-label">
                        <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                        <div className="row-desc">{t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}</div>
                      </div>
                      <div className="row-control">
                        {traeQuotaAlertThresholdCustomMode ? (
                          <div className="settings-inline-input">
                            <input
                              type="number"
                              min={0}
                              max={100}
                              className="settings-select settings-select--input-mode settings-select--with-unit"
                              value={traeQuotaAlertThreshold}
                              placeholder={t('quickSettings.inputPercent', '输入百分比')}
                              onChange={(e) => setTraeQuotaAlertThreshold(sanitizeNumberInput(e.target.value))}
                              onBlur={() => {
                                const normalized = normalizeNumberInput(traeQuotaAlertThreshold, 0, 100);
                                if (THRESHOLD_PRESET_VALUES.includes(normalized)) {
                                  setTraeQuotaAlertThresholdCustomMode(false);
                                }
                                setTraeQuotaAlertThreshold(normalized);
                              }}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  const normalized = normalizeNumberInput(traeQuotaAlertThreshold, 0, 100);
                                  setTraeQuotaAlertThresholdCustomMode(false);
                                  setTraeQuotaAlertThreshold(normalized);
                                }
                              }}
                            />
                            <span className="settings-input-unit">%</span>
                          </div>
                        ) : (
                          <select
                            className="settings-select"
                            value={traeQuotaAlertThreshold}
                            onChange={(e) => {
                              const val = e.target.value;
                              if (val === 'custom') {
                                setTraeQuotaAlertThresholdCustomMode(true);
                                setTraeQuotaAlertThreshold(traeQuotaAlertThreshold || '20');
                                return;
                              }
                              setTraeQuotaAlertThresholdCustomMode(false);
                              setTraeQuotaAlertThreshold(val);
                            }}
                          >
                            {!traeQuotaAlertThresholdIsPreset && (
                              <option value={traeQuotaAlertThreshold}>{traeQuotaAlertThreshold}%</option>
                            )}
                            <option value="0">0%</option>
                            <option value="20">20%</option>
                            <option value="40">40%</option>
                            <option value="60">60%</option>
                            <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                          </select>
                        )}
                      </div>
                    </div>
                  )}
                </div>
              </div>

              <div style={{ order: platformSettingsOrder.workbuddy }}>
                <div className="group-title">{t('quickSettings.workbuddy.title', 'WorkBuddy 设置')}</div>
                <div className="settings-group">
                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('settings.general.workbuddyAutoRefresh', 'WorkBuddy 自动刷新配额')}</div>
                      <div className="row-desc">{t('settings.general.workbuddyAutoRefreshDesc', '后台自动更新频率')}</div>
                    </div>
                    <div className="row-control">
                      <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                        {workbuddyAutoRefreshCustomMode ? (
                          <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                            <input
                              type="number"
                              min={1}
                              max={999}
                              className="settings-select settings-select--input-mode settings-select--with-unit"
                              value={workbuddyAutoRefresh}
                              placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                              onChange={(e) => setWorkbuddyAutoRefresh(sanitizeNumberInput(e.target.value))}
                              onBlur={() => {
                                const normalized = normalizeNumberInput(workbuddyAutoRefresh, 1, 999);
                                if (REFRESH_PRESET_VALUES.includes(normalized)) {
                                  setWorkbuddyAutoRefreshCustomMode(false);
                                }
                                setWorkbuddyAutoRefresh(normalized);
                              }}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  const normalized = normalizeNumberInput(workbuddyAutoRefresh, 1, 999);
                                  setWorkbuddyAutoRefreshCustomMode(false);
                                  setWorkbuddyAutoRefresh(normalized);
                                }
                              }}
                            />
                            <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                          </div>
                        ) : (
                          <select
                            className="settings-select"
                            style={{ minWidth: '120px', width: 'auto' }}
                            value={workbuddyAutoRefresh}
                            onChange={(e) => {
                              const val = e.target.value;
                              if (val === 'custom') {
                                setWorkbuddyAutoRefreshCustomMode(true);
                                setWorkbuddyAutoRefresh(workbuddyAutoRefresh !== '-1' ? workbuddyAutoRefresh : '1');
                                return;
                              }
                              setWorkbuddyAutoRefreshCustomMode(false);
                              setWorkbuddyAutoRefresh(val);
                            }}
                          >
                            {!workbuddyAutoRefreshIsPreset && (
                              <option value={workbuddyAutoRefresh}>
                                {workbuddyAutoRefresh} {t('settings.general.minutes')}
                              </option>
                            )}
                            <option value="-1">{t('settings.general.autoRefreshDisabled')}</option>
                            <option value="2">2 {t('settings.general.minutes')}</option>
                            <option value="5">5 {t('settings.general.minutes')}</option>
                            <option value="10">10 {t('settings.general.minutes')}</option>
                            <option value="15">15 {t('settings.general.minutes')}</option>
                            <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                          </select>
                        )}
                      </div>
                    </div>
                  </div>

                  {renderCurrentAccountRefreshRow('workbuddy')}

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('settings.general.workbuddyAppPath', 'WorkBuddy 启动路径')}</div>
                      <div className="row-desc">{t('settings.general.workbuddyAppPathDesc', '留空则使用默认路径')}</div>
                    </div>
                    <div className="row-control row-control--grow">
                      <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                        <input
                          type="text"
                          className="settings-input settings-input--path"
                          value={workbuddyAppPath}
                          placeholder={t('settings.general.workbuddyAppPathPlaceholder', '默认路径')}
                          onChange={(e) => setWorkbuddyAppPath(e.target.value)}
                        />
                        <button
                          className="btn btn-secondary"
                          onClick={() => handlePickAppPath('workbuddy')}
                          disabled={isAppPathResetDetecting('workbuddy')}
                        >
                          {t('settings.general.workbuddyPathSelect', '选择')}
                        </button>
                        <button
                          className="btn btn-secondary"
                          onClick={() => handleResetAppPath('workbuddy')}
                          disabled={isAppPathResetDetecting('workbuddy')}
                        >
                          <RefreshCw size={16} className={isAppPathResetDetecting('workbuddy') ? 'spin' : undefined} />
                          {isAppPathResetDetecting('workbuddy')
                            ? t('common.loading', '加载中...')
                            : getResetLabelByTarget('workbuddy')}
                        </button>
                      </div>
                    </div>
                  </div>

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
                      <div className="row-desc">{t('quickSettings.quotaAlert.hint', '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。')}</div>
                    </div>
                    <div className="row-control">
                      <label className="switch">
                        <input
                          type="checkbox"
                          checked={workbuddyQuotaAlertEnabled}
                          onChange={(e) => setWorkbuddyQuotaAlertEnabled(e.target.checked)}
                        />
                        <span className="slider"></span>
                      </label>
                    </div>
                  </div>
                  {workbuddyQuotaAlertEnabled && (
                    <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                      <div className="row-label">
                        <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                        <div className="row-desc">{t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}</div>
                      </div>
                      <div className="row-control">
                        {workbuddyQuotaAlertThresholdCustomMode ? (
                          <div className="settings-inline-input">
                            <input
                              type="number"
                              min={0}
                              max={100}
                              className="settings-select settings-select--input-mode settings-select--with-unit"
                              value={workbuddyQuotaAlertThreshold}
                              placeholder={t('quickSettings.inputPercent', '输入百分比')}
                              onChange={(e) => setWorkbuddyQuotaAlertThreshold(sanitizeNumberInput(e.target.value))}
                              onBlur={() => {
                                const normalized = normalizeNumberInput(workbuddyQuotaAlertThreshold, 0, 100);
                                if (THRESHOLD_PRESET_VALUES.includes(normalized)) {
                                  setWorkbuddyQuotaAlertThresholdCustomMode(false);
                                }
                                setWorkbuddyQuotaAlertThreshold(normalized);
                              }}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  const normalized = normalizeNumberInput(workbuddyQuotaAlertThreshold, 0, 100);
                                  setWorkbuddyQuotaAlertThresholdCustomMode(false);
                                  setWorkbuddyQuotaAlertThreshold(normalized);
                                }
                              }}
                            />
                            <span className="settings-input-unit">%</span>
                          </div>
                        ) : (
                          <select
                            className="settings-select"
                            value={workbuddyQuotaAlertThreshold}
                            onChange={(e) => {
                              const val = e.target.value;
                              if (val === 'custom') {
                                setWorkbuddyQuotaAlertThresholdCustomMode(true);
                                setWorkbuddyQuotaAlertThreshold(workbuddyQuotaAlertThreshold || '20');
                                return;
                              }
                              setWorkbuddyQuotaAlertThresholdCustomMode(false);
                              setWorkbuddyQuotaAlertThreshold(val);
                            }}
                          >
                            {!workbuddyQuotaAlertThresholdIsPreset && (
                              <option value={workbuddyQuotaAlertThreshold}>{workbuddyQuotaAlertThreshold}%</option>
                            )}
                            <option value="0">0%</option>
                            <option value="20">20%</option>
                            <option value="40">40%</option>
                            <option value="60">60%</option>
                            <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                          </select>
                        )}
                      </div>
                    </div>
                  )}
                </div>
              </div>

              <div style={{ order: platformSettingsOrder.zed }}>
                <div className="group-title">{t('quickSettings.zed.title', 'Zed 设置')}</div>
                <div className="settings-group">
                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('settings.general.zedAutoRefresh', 'Zed 自动刷新配额')}</div>
                      <div className="row-desc">{t('settings.general.zedAutoRefreshDesc', '后台自动更新频率')}</div>
                    </div>
                    <div className="row-control">
                      <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                        {zedAutoRefreshCustomMode ? (
                          <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                            <input
                              type="number"
                              min={1}
                              max={999}
                              className="settings-select settings-select--input-mode settings-select--with-unit"
                              value={zedAutoRefresh}
                              placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                              onChange={(e) => setZedAutoRefresh(sanitizeNumberInput(e.target.value))}
                              onBlur={() => {
                                const normalized = normalizeNumberInput(zedAutoRefresh, 1, 999);
                                setZedAutoRefresh(normalized);
                                setZedAutoRefreshCustomMode(false);
                              }}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  const normalized = normalizeNumberInput(zedAutoRefresh, 1, 999);
                                  setZedAutoRefresh(normalized);
                                  setZedAutoRefreshCustomMode(false);
                                }
                              }}
                            />
                            <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                          </div>
                        ) : (
                          <select
                            className="settings-select"
                            style={{ minWidth: '120px', width: 'auto' }}
                            value={zedAutoRefresh}
                            onChange={(e) => {
                              const val = e.target.value;
                              if (val === 'custom') {
                                setZedAutoRefreshCustomMode(true);
                                setZedAutoRefresh(zedAutoRefresh !== '-1' ? zedAutoRefresh : '1');
                                return;
                              }
                              setZedAutoRefreshCustomMode(false);
                              setZedAutoRefresh(val);
                            }}
                          >
                            {!zedAutoRefreshIsPreset && (
                              <option value={zedAutoRefresh}>
                                {zedAutoRefresh} {t('settings.general.minutes')}
                              </option>
                            )}
                            <option value="-1">{t('settings.general.autoRefreshDisabled')}</option>
                            <option value="2">2 {t('settings.general.minutes')}</option>
                            <option value="5">5 {t('settings.general.minutes')}</option>
                            <option value="10">10 {t('settings.general.minutes')}</option>
                            <option value="15">15 {t('settings.general.minutes')}</option>
                            <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                          </select>
                        )}
                      </div>
                    </div>
                  </div>

                  {renderCurrentAccountRefreshRow('zed')}

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('settings.general.zedAppPath', 'Zed 启动路径')}</div>
                      <div className="row-desc">{t('settings.general.zedAppPathDesc', '留空则使用默认路径')}</div>
                    </div>
                    <div className="row-control row-control--grow">
                      <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                        <input
                          type="text"
                          className="settings-input settings-input--path"
                          value={zedAppPath}
                          placeholder={t('settings.general.zedAppPathPlaceholder', '默认路径')}
                          onChange={(e) => setZedAppPath(e.target.value)}
                        />
                        <button
                          className="btn btn-secondary"
                          onClick={() => handlePickAppPath('zed')}
                          disabled={isAppPathResetDetecting('zed')}
                        >
                          {t('settings.general.zedPathSelect', '选择')}
                        </button>
                        <button
                          className="btn btn-secondary"
                          onClick={() => handleResetAppPath('zed')}
                          disabled={isAppPathResetDetecting('zed')}
                        >
                          <RefreshCw size={16} className={isAppPathResetDetecting('zed') ? 'spin' : undefined} />
                          {isAppPathResetDetecting('zed')
                            ? t('common.loading', '加载中...')
                            : getResetLabelByTarget('zed')}
                        </button>
                      </div>
                    </div>
                  </div>

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
                      <div className="row-desc">{t('quickSettings.quotaAlert.hint', '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。')}</div>
                    </div>
                    <div className="row-control">
                      <label className="switch">
                        <input
                          type="checkbox"
                          checked={zedQuotaAlertEnabled}
                          onChange={(e) => setZedQuotaAlertEnabled(e.target.checked)}
                        />
                        <span className="slider"></span>
                      </label>
                    </div>
                  </div>
                  {zedQuotaAlertEnabled && (
                    <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                      <div className="row-label">
                        <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                        <div className="row-desc">{t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}</div>
                      </div>
                      <div className="row-control">
                        {zedQuotaAlertThresholdCustomMode ? (
                          <div className="settings-inline-input">
                            <input
                              type="number"
                              min={0}
                              max={100}
                              className="settings-select settings-select--input-mode settings-select--with-unit"
                              value={zedQuotaAlertThreshold}
                              placeholder={t('quickSettings.inputPercent', '输入百分比')}
                              onChange={(e) => setZedQuotaAlertThreshold(sanitizeNumberInput(e.target.value))}
                              onBlur={() => {
                                const normalized = normalizeNumberInput(zedQuotaAlertThreshold, 0, 100);
                                setZedQuotaAlertThreshold(normalized);
                                setZedQuotaAlertThresholdCustomMode(false);
                              }}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  const normalized = normalizeNumberInput(zedQuotaAlertThreshold, 0, 100);
                                  setZedQuotaAlertThreshold(normalized);
                                  setZedQuotaAlertThresholdCustomMode(false);
                                }
                              }}
                            />
                            <span className="settings-input-unit">%</span>
                          </div>
                        ) : (
                          <select
                            className="settings-select"
                            value={zedQuotaAlertThreshold}
                            onChange={(e) => {
                              const val = e.target.value;
                              if (val === 'custom') {
                                setZedQuotaAlertThresholdCustomMode(true);
                                setZedQuotaAlertThreshold(zedQuotaAlertThreshold || '20');
                                return;
                              }
                              setZedQuotaAlertThresholdCustomMode(false);
                              setZedQuotaAlertThreshold(val);
                            }}
                          >
                            {!zedQuotaAlertThresholdIsPreset && (
                              <option value={zedQuotaAlertThreshold}>{zedQuotaAlertThreshold}%</option>
                            )}
                            <option value="0">0%</option>
                            <option value="20">20%</option>
                            <option value="40">40%</option>
                            <option value="60">60%</option>
                            <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                          </select>
                        )}
                      </div>
                    </div>
                  )}
                </div>
              </div>

              <div style={{ order: platformSettingsOrder.cursor }}>
                <div className="group-title">{t('quickSettings.cursor.title', 'Cursor 设置')}</div>
                <div className="settings-group">
              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('quickSettings.cursorRefreshInterval', '配额自动刷新')}</div>
                  <div className="row-desc">{t('settings.general.windsurfAutoRefreshDesc', '后台自动更新频率')}</div>
                </div>
                <div className="row-control">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                    {cursorAutoRefreshCustomMode ? (
                      <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                        <input
                          type="number"
                          min={1}
                          max={999}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={cursorAutoRefresh}
                          placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                          onChange={(e) => setCursorAutoRefresh(sanitizeNumberInput(e.target.value))}
                          onBlur={() => {
                            const normalized = normalizeNumberInput(cursorAutoRefresh, 1, 999);
                            setCursorAutoRefresh(normalized);
                            setCursorAutoRefreshCustomMode(false);
                          }}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter') {
                              e.preventDefault();
                              const normalized = normalizeNumberInput(cursorAutoRefresh, 1, 999);
                              setCursorAutoRefresh(normalized);
                              setCursorAutoRefreshCustomMode(false);
                            }
                          }}
                        />
                        <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                      </div>
                    ) : (
                      <select
                        className="settings-select"
                        style={{ minWidth: '120px', width: 'auto' }}
                        value={cursorAutoRefresh}
                        onChange={(e) => {
                          const val = e.target.value;
                          if (val === 'custom') {
                            setCursorAutoRefreshCustomMode(true);
                            setCursorAutoRefresh(cursorAutoRefresh !== '-1' ? cursorAutoRefresh : '1');
                            return;
                          }
                          setCursorAutoRefreshCustomMode(false);
                          setCursorAutoRefresh(val);
                        }}
                      >
                        {!cursorAutoRefreshIsPreset && (
                          <option value={cursorAutoRefresh}>
                            {cursorAutoRefresh} {t('settings.general.minutes')}
                          </option>
                        )}
                        <option value="-1">{t('settings.general.autoRefreshDisabled')}</option>
                        <option value="2">2 {t('settings.general.minutes')}</option>
                        <option value="5">5 {t('settings.general.minutes')}</option>
                        <option value="10">10 {t('settings.general.minutes')}</option>
                        <option value="15">15 {t('settings.general.minutes')}</option>
                        <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                      </select>
                    )}
                  </div>
                </div>
              </div>

              {renderCurrentAccountRefreshRow('cursor')}

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('quickSettings.cursor.appPath', 'Cursor 路径')}</div>
                  <div className="row-desc">{t('settings.general.codexAppPathDesc', '留空则使用默认路径')}</div>
                </div>
                <div className="row-control row-control--grow">
                  <div style={{ display: 'flex', gap: '8px', alignItems: 'center', flex: 1 }}>
                    <input
                      type="text"
                      className="settings-input settings-input--path"
                      value={cursorAppPath}
                      placeholder={t('settings.general.codexAppPathPlaceholder', '默认路径')}
                      onChange={(e) => setCursorAppPath(e.target.value)}
                    />
                    <button
                      className="btn btn-secondary"
                      onClick={() => handlePickAppPath('cursor')}
                      disabled={isAppPathResetDetecting('cursor')}
                    >
                      {t('settings.general.codexPathSelect', '选择')}
                    </button>
                    <button
                      className="btn btn-secondary"
                      onClick={() => handleResetAppPath('cursor')}
                      disabled={isAppPathResetDetecting('cursor')}
                    >
                      <RefreshCw size={16} className={isAppPathResetDetecting('cursor') ? 'spin' : undefined} />
                      {isAppPathResetDetecting('cursor')
                        ? t('common.loading', '加载中...')
                        : getResetLabelByTarget('cursor')}
                    </button>
                  </div>
                </div>
              </div>

              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
                  <div className="row-desc">{t('quickSettings.quotaAlert.hint', '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={cursorQuotaAlertEnabled}
                      onChange={(e) => setCursorQuotaAlertEnabled(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>
              {cursorQuotaAlertEnabled && (
                <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                  <div className="row-label">
                    <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                    <div className="row-desc">{t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}</div>
                  </div>
                  <div className="row-control">
                    {cursorQuotaAlertThresholdCustomMode ? (
                      <div className="settings-inline-input">
                        <input
                          type="number"
                          min={0}
                          max={100}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={cursorQuotaAlertThreshold}
                          placeholder={t('quickSettings.inputPercent', '输入百分比')}
                          onChange={(e) => setCursorQuotaAlertThreshold(sanitizeNumberInput(e.target.value))}
                          onBlur={() => {
                            const normalized = normalizeNumberInput(cursorQuotaAlertThreshold, 0, 100);
                            setCursorQuotaAlertThreshold(normalized);
                            setCursorQuotaAlertThresholdCustomMode(false);
                          }}
                          onKeyDown={(e) => {
                            if (e.key === 'Enter') {
                              e.preventDefault();
                              const normalized = normalizeNumberInput(cursorQuotaAlertThreshold, 0, 100);
                              setCursorQuotaAlertThreshold(normalized);
                              setCursorQuotaAlertThresholdCustomMode(false);
                            }
                          }}
                        />
                        <span className="settings-input-unit">%</span>
                      </div>
                    ) : (
                      <select
                        className="settings-select"
                        value={cursorQuotaAlertThreshold}
                        onChange={(e) => {
                          const val = e.target.value;
                          if (val === 'custom') {
                            setCursorQuotaAlertThresholdCustomMode(true);
                            setCursorQuotaAlertThreshold(cursorQuotaAlertThreshold || '20');
                            return;
                          }
                          setCursorQuotaAlertThresholdCustomMode(false);
                          setCursorQuotaAlertThreshold(val);
                        }}
                      >
                        {!cursorQuotaAlertThresholdIsPreset && (
                          <option value={cursorQuotaAlertThreshold}>{cursorQuotaAlertThreshold}%</option>
                        )}
                        <option value="0">0%</option>
                        <option value="20">20%</option>
                        <option value="40">40%</option>
                        <option value="60">60%</option>
                        <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                      </select>
                    )}
                  </div>
                </div>
              )}
            </div>
              </div>

              <div style={{ order: platformSettingsOrder.gemini }}>
                <div className="group-title">{t('quickSettings.gemini.title', 'Gemini Cli 设置')}</div>
                <div className="settings-group">
                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('quickSettings.geminiRefreshInterval', '配额自动刷新')}</div>
                      <div className="row-desc">{t('settings.general.windsurfAutoRefreshDesc', '后台自动更新频率')}</div>
                    </div>
                    <div className="row-control">
                      <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
                        {geminiAutoRefreshCustomMode ? (
                          <div className="settings-inline-input" style={{ minWidth: '120px', width: 'auto' }}>
                            <input
                              type="number"
                              min={1}
                              max={999}
                              className="settings-select settings-select--input-mode settings-select--with-unit"
                              value={geminiAutoRefresh}
                              placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                              onChange={(e) => setGeminiAutoRefresh(sanitizeNumberInput(e.target.value))}
                              onBlur={() => {
                                const normalized = normalizeNumberInput(geminiAutoRefresh, 1, 999);
                                setGeminiAutoRefresh(normalized);
                                setGeminiAutoRefreshCustomMode(false);
                              }}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  const normalized = normalizeNumberInput(geminiAutoRefresh, 1, 999);
                                  setGeminiAutoRefresh(normalized);
                                  setGeminiAutoRefreshCustomMode(false);
                                }
                              }}
                            />
                            <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                          </div>
                        ) : (
                          <select
                            className="settings-select"
                            style={{ minWidth: '120px', width: 'auto' }}
                            value={geminiAutoRefresh}
                            onChange={(e) => {
                              const val = e.target.value;
                              if (val === 'custom') {
                                setGeminiAutoRefreshCustomMode(true);
                                setGeminiAutoRefresh(geminiAutoRefresh !== '-1' ? geminiAutoRefresh : '1');
                                return;
                              }
                              setGeminiAutoRefreshCustomMode(false);
                              setGeminiAutoRefresh(val);
                            }}
                          >
                            {!geminiAutoRefreshIsPreset && (
                              <option value={geminiAutoRefresh}>
                                {geminiAutoRefresh} {t('settings.general.minutes')}
                              </option>
                            )}
                            <option value="-1">{t('settings.general.autoRefreshDisabled')}</option>
                            <option value="2">2 {t('settings.general.minutes')}</option>
                            <option value="5">5 {t('settings.general.minutes')}</option>
                            <option value="10">10 {t('settings.general.minutes')}</option>
                            <option value="15">15 {t('settings.general.minutes')}</option>
                            <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                          </select>
                        )}
                      </div>
                    </div>
                  </div>

                  {renderCurrentAccountRefreshRow('gemini')}

                  <div className="settings-row">
                    <div className="row-label">
                      <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
                      <div className="row-desc">{t('quickSettings.quotaAlert.hint', '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。')}</div>
                    </div>
                    <div className="row-control">
                      <label className="switch">
                        <input
                          type="checkbox"
                          checked={geminiQuotaAlertEnabled}
                          onChange={(e) => setGeminiQuotaAlertEnabled(e.target.checked)}
                        />
                        <span className="slider"></span>
                      </label>
                    </div>
                  </div>
                  {geminiQuotaAlertEnabled && (
                    <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                      <div className="row-label">
                        <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
                        <div className="row-desc">{t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}</div>
                      </div>
                      <div className="row-control">
                        {geminiQuotaAlertThresholdCustomMode ? (
                          <div className="settings-inline-input">
                            <input
                              type="number"
                              min={0}
                              max={100}
                              className="settings-select settings-select--input-mode settings-select--with-unit"
                              value={geminiQuotaAlertThreshold}
                              placeholder={t('quickSettings.inputPercent', '输入百分比')}
                              onChange={(e) => setGeminiQuotaAlertThreshold(sanitizeNumberInput(e.target.value))}
                              onBlur={() => {
                                const normalized = normalizeNumberInput(geminiQuotaAlertThreshold, 0, 100);
                                setGeminiQuotaAlertThreshold(normalized);
                                setGeminiQuotaAlertThresholdCustomMode(false);
                              }}
                              onKeyDown={(e) => {
                                if (e.key === 'Enter') {
                                  e.preventDefault();
                                  const normalized = normalizeNumberInput(geminiQuotaAlertThreshold, 0, 100);
                                  setGeminiQuotaAlertThreshold(normalized);
                                  setGeminiQuotaAlertThresholdCustomMode(false);
                                }
                              }}
                            />
                            <span className="settings-input-unit">%</span>
                          </div>
                        ) : (
                          <select
                            className="settings-select"
                            value={geminiQuotaAlertThreshold}
                            onChange={(e) => {
                              const val = e.target.value;
                              if (val === 'custom') {
                                setGeminiQuotaAlertThresholdCustomMode(true);
                                setGeminiQuotaAlertThreshold(geminiQuotaAlertThreshold || '20');
                                return;
                              }
                              setGeminiQuotaAlertThresholdCustomMode(false);
                              setGeminiQuotaAlertThreshold(val);
                            }}
                          >
                            {!geminiQuotaAlertThresholdIsPreset && (
                              <option value={geminiQuotaAlertThreshold}>{geminiQuotaAlertThreshold}%</option>
                            )}
                            <option value="0">0%</option>
                            <option value="20">20%</option>
                            <option value="40">40%</option>
                            <option value="60">60%</option>
                            <option value="custom">{t('settings.general.autoRefreshCustom')}</option>
                          </select>
                        )}
                      </div>
                    </div>
                  )}
                </div>
              </div>
            </div>

          </>
        )}

        {/* === Network Tab === */}
        {activeTab === 'network' && (
          <>
            <div className="group-title">Antigravity Cockpit API</div>
            <div className="settings-group">
              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.network.wsService')}</div>
                  <div className="row-desc">{t('settings.network.wsServiceDesc')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input 
                      type="checkbox" 
                      checked={wsEnabled} 
                      onChange={(e) => setWsEnabled(e.target.checked)} 
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>

              {wsEnabled && (
                <>
                  <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                    <div className="row-label">
                      <div className="row-title">{t('settings.network.preferredPort')}</div>
                      <div className="row-desc">
                        {t('settings.network.preferredPortDesc').replace('{port}', String(defaultPort))}
                      </div>
                    </div>
                    <div className="row-control">
                      <input 
                        type="number" 
                        className="settings-input"
                        value={wsPort}
                        onChange={(e) => setWsPort(e.target.value)}
                        placeholder={String(defaultPort)}
                        min="1024"
                        max="65535"
                      />
                    </div>
                  </div>
                  
                  {actualPort && (
                    <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                      <div className="row-label">
                        <div className="row-title">{t('settings.network.currentPort')}</div>
                        <div className="row-desc">
                          {actualPort === parseInt(wsPort, 10) 
                            ? t('settings.network.portNormal')
                            : t('settings.network.portFallback')
                                .replace('{configured}', wsPort)
                                .replace('{actual}', String(actualPort))}
                        </div>
                      </div>
                      <div className="row-control">
                        <span style={{ 
                          fontFamily: 'var(--font-mono)', 
                          fontSize: '14px',
                          color: actualPort === parseInt(wsPort, 10) ? 'var(--accent)' : 'var(--warning, #f59e0b)'
                        }}>
                          ws://127.0.0.1:{actualPort}
                        </span>
                      </div>
                    </div>
                  )}
                </>
              )}
            </div>

            <div className="group-title">{t('settings.network.reportTitle')}</div>
            <div className="settings-group">
              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.network.reportService')}</div>
                  <div className="row-desc">{t('settings.network.reportServiceDesc')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={reportEnabled}
                      onChange={(e) => setReportEnabled(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>

              {reportEnabled && (
                <>
                  <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                    <div className="row-label">
                      <div className="row-title">{t('settings.network.reportPort')}</div>
                      <div className="row-desc">
                        {t('settings.network.reportPortDesc').replace('{port}', String(reportDefaultPort))}
                      </div>
                    </div>
                    <div className="row-control">
                      <input
                        type="number"
                        className="settings-input"
                        value={reportPort}
                        onChange={(e) => setReportPort(e.target.value)}
                        placeholder={String(reportDefaultPort)}
                        min="1024"
                        max="65535"
                      />
                    </div>
                  </div>

                  <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                    <div className="row-label">
                      <div className="row-title">{t('settings.network.reportToken')}</div>
                      <div className="row-desc">{t('settings.network.reportTokenDesc')}</div>
                    </div>
                    <div className="row-control" style={{ minWidth: '260px', display: 'flex', gap: '8px', alignItems: 'center' }}>
                      <input
                        type="text"
                        className="settings-input"
                        value={reportToken}
                        onChange={(e) => setReportToken(e.target.value)}
                        placeholder="change-this-token"
                      />
                      <button
                        className="btn btn-secondary"
                        onClick={() => setReportToken(generateReportToken())}
                        type="button"
                      >
                        {t('settings.network.generateToken')}
                      </button>
                    </div>
                  </div>

                  {reportActualPort && (
                    <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                      <div className="row-label">
                        <div className="row-title">{t('settings.network.currentPort')}</div>
                        <div className="row-desc">
                          {reportActualPort === parseInt(reportPort, 10)
                            ? t('settings.network.portNormal')
                            : t('settings.network.portFallback')
                                .replace('{configured}', reportPort)
                                .replace('{actual}', String(reportActualPort))}
                        </div>
                      </div>
                      <div className="row-control">
                        <span style={{
                          fontFamily: 'var(--font-mono)',
                          fontSize: '14px',
                          color: reportActualPort === parseInt(reportPort, 10) ? 'var(--accent)' : 'var(--warning, #f59e0b)',
                        }}>
                          http://0.0.0.0:{reportActualPort}
                        </span>
                      </div>
                    </div>
                  )}

                  <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                    <div className="row-label">
                      <div className="row-title">{t('settings.network.reportUrlPreview')}</div>
                      <div className="row-desc">
                        {t('settings.network.reportUrlPreviewDesc')}
                      </div>
                    </div>
                    <div className="row-control">
                      <div style={{
                        display: 'flex',
                        flexDirection: 'column',
                        gap: '6px',
                        alignItems: 'flex-start',
                        fontFamily: 'var(--font-mono)',
                        fontSize: '12px',
                        color: 'var(--text-secondary)',
                        wordBreak: 'break-all',
                      }}>
                        <span>{`${t('settings.network.reportUrlRaw')}: ${reportRawPreviewUrl}`}</span>
                        <span>{`${t('settings.network.reportUrlRendered')}: ${reportRenderedPreviewUrl}`}</span>
                      </div>
                    </div>
                  </div>

                  <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                    <div className="row-label">
                      <div className="row-title">{t('settings.network.firewallHintTitle')}</div>
                      <div className="row-desc">{t('settings.network.firewallHint')}</div>
                    </div>
                  </div>
                </>
              )}
            </div>

            <div className="group-title">{t('settings.network.proxyTitle')}</div>
            <div className="settings-group">
              <div className="settings-row">
                <div className="row-label">
                  <div className="row-title">{t('settings.network.proxyEnabled')}</div>
                  <div className="row-desc">{t('settings.network.proxyEnabledDesc')}</div>
                </div>
                <div className="row-control">
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={globalProxyEnabled}
                      onChange={(e) => setGlobalProxyEnabled(e.target.checked)}
                    />
                    <span className="slider"></span>
                  </label>
                </div>
              </div>

              {globalProxyEnabled && (
                <>
                  <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                    <div className="row-label">
                      <div className="row-title">{t('settings.network.proxyUrl')}</div>
                      <div className="row-desc">{t('settings.network.proxyUrlDesc')}</div>
                    </div>
                    <div className="row-control">
                      <input
                        type="text"
                        className="settings-input"
                        value={globalProxyUrl}
                        onChange={(e) => setGlobalProxyUrl(e.target.value)}
                        placeholder={t('settings.network.proxyUrlPlaceholder')}
                      />
                    </div>
                  </div>

                  <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
                    <div className="row-label">
                      <div className="row-title">{t('settings.network.proxyNoProxy')}</div>
                      <div className="row-desc">{t('settings.network.proxyNoProxyDesc')}</div>
                    </div>
                    <div className="row-control">
                      <input
                        type="text"
                        className="settings-input"
                        value={globalProxyNoProxy}
                        onChange={(e) => setGlobalProxyNoProxy(e.target.value)}
                        placeholder={t('settings.network.proxyNoProxyPlaceholder')}
                      />
                    </div>
                  </div>
                </>
              )}
            </div>
            
            {needsRestart && (
              <div style={{ 
                display: 'flex', 
                alignItems: 'center', 
                gap: '8px', 
                padding: '12px 16px',
                marginTop: '12px',
                background: 'rgba(245, 158, 11, 0.1)',
                borderRadius: '8px',
                color: 'var(--warning, #f59e0b)',
                fontSize: '14px'
              }}>
                <AlertCircle size={18} />
                {t('settings.network.restartRequired')}
              </div>
            )}

            <div style={{ display: 'flex', justifyContent: 'flex-end', marginTop: '12px' }}>
                <button 
                  className="btn btn-primary" 
                  onClick={handleSaveNetworkConfig}
                  disabled={networkSaving}
                >
                    <Save size={16} /> {networkSaving ? t('common.saving') : t('settings.saveSettings')}
                </button>
            </div>
          </>
        )}

        {/* === About Tab === */}
        {activeTab === 'about' && (
          <div className="about-container">
            <div className="about-logo-section">
              <div
                className={`app-icon-squircle${showUnlockFireworks ? ' unlock-fireworks-active' : ''}`}
                onClick={handleAboutAvatarTap}
                onMouseDown={(event) => event.preventDefault()}
              >
                <Rocket size={40} />
              </div>
              <div className="app-info">
                <h2>{t('settings.about.appName')}</h2>
                <div style={{ display: 'flex', alignItems: 'center', gap: '12px' }}>
                  <div className="version-tag">{appVersion}</div>
                  <button 
                    className="btn btn-sm btn-ghost"
                    onClick={handleCheckUpdate}
                    disabled={updateChecking}
                    style={{ 
                      fontSize: '12px', 
                      padding: '4px 10px',
                      display: 'flex',
                      alignItems: 'center',
                      gap: '4px'
                    }}
                  >
                    <>
                      <RefreshCw size={14} className={updateChecking ? 'spin' : undefined} />
                      {updateChecking ? t('settings.about.checking') : t('settings.about.checkUpdate')}
                    </>
                  </button>
                </div>
                {updateCheckMessage && (
                  <div
                    className={`action-message${updateCheckMessage.tone ? ` ${updateCheckMessage.tone}` : ''}`}
                    style={{ marginTop: '10px', marginBottom: 0 }}
                  >
                    <span className="action-message-text">{updateCheckMessage.text}</span>
                  </div>
                )}
              </div>
              <p style={{ color: 'var(--text-secondary)', fontSize: '14px' }}>
                {t('settings.about.slogan')}
              </p>
            </div>

            <div className="credits-list">
              <button className="credit-item" onClick={() => openLink('https://github.com/jlcodes99')}>
                <div className="credit-icon"><User size={24} /></div>
                <h3>{t('settings.about.author')}</h3>
                <p>jlcodes99</p>
              </button>
              
              
              <button className="credit-item" onClick={() => openLink('https://github.com/jlcodes99/cockpit-tools')}>
                <div className="credit-icon" style={{ color: '#0f172a' }}><Github size={24} /></div>
                <h3>{t('settings.about.github')}</h3>
                <p>cockpit-tools</p>
              </button>

              <button className="credit-item" onClick={() => openLink('https://github.com/jlcodes99/cockpit-tools/blob/main/docs/DONATE.md')}>
                <div className="credit-icon" style={{ color: '#ef4444' }}><Heart size={24} /></div>
                <h3>{t('settings.about.sponsor')}</h3>
                <p>{t('settings.about.sponsorDesc', 'Donate')}</p>
              </button>

              <button className="credit-item" onClick={() => openLink('https://github.com/jlcodes99/cockpit-tools/issues')}>
                <div className="credit-icon" style={{ color: '#3b82f6' }}><MessageSquare size={24} /></div>
                <h3>{t('settings.about.feedback', '意见反馈')}</h3>
                <p>{t('settings.about.feedbackDesc', 'Issues')}</p>
              </button>
            </div>
          </div>
        )}
        </div>
      </div>
      {showUnlockFireworks && (
        <UnlockFireworksOverlay />
      )}
    </main>
  );
}
