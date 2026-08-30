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
import { usePlatformRuntimeSupport } from '../hooks/usePlatformRuntimeSupport';
import { usePlatformLayoutStore } from '../stores/usePlatformLayoutStore';
import { useSideNavLayoutStore } from '../stores/useSideNavLayoutStore';
import {
  type AutoSwitchAccountScopeMode,
  type AutoSwitchScopeAccount,
} from '../components/AutoSwitchAccountScopeSelector';
import {
  buildAccountTierCounts,
  buildAccountTierFilterOptions,
} from '../utils/accountFilters';
import {
  getUpdaterReleaseHighlightLines,
  resolveUpdaterDownloadUrl,
} from '../utils/updaterReleaseNotes';
import { applyReducedMotion } from '../utils/reducedMotion';
import {
  setClaudeQuotaDisplayRemainingEnabled,
} from '../utils/claudeQuotaDisplayPreference';
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
  loadAccountRefreshOverrides,
  setAccountRefreshMinutes,
  removeAccountRefreshOverride,
  type AccountRefreshOverrides,
} from '../utils/currentAccountRefresh';
import { useGitHubCopilotAccountStore } from '../stores/useGitHubCopilotAccountStore';
import { useWindsurfAccountStore } from '../stores/useWindsurfAccountStore';
import { useKiroAccountStore } from '../stores/useKiroAccountStore';
import { useCursorAccountStore } from '../stores/useCursorAccountStore';
import { useGrokAccountStore } from '../stores/useGrokAccountStore';
import { useClaudeAccountStore } from '../stores/useClaudeAccountStore';
import { useCodebuddyAccountStore } from '../stores/useCodebuddyAccountStore';
import { useCodebuddyCnAccountStore } from '../stores/useCodebuddyCnAccountStore';
import { useWorkbuddyAccountStore } from '../stores/useWorkbuddyAccountStore';
import { useQoderAccountStore } from '../stores/useQoderAccountStore';
import { useZcodeAccountStore } from '../stores/useZcodeAccountStore';
import { useTraeAccountStore } from '../stores/useTraeAccountStore';
import { useZedAccountStore } from '../stores/useZedAccountStore';
import { getGitHubCopilotAccountDisplayEmail } from '../types/githubCopilot';
import { getWindsurfAccountDisplayEmail } from '../types/windsurf';
import { getKiroAccountDisplayEmail } from '../types/kiro';
import { getCursorAccountDisplayEmail } from '../types/cursor';
import { getGrokAccountDisplayEmail } from '../types/grok';
import { getClaudeAccountDisplayEmail } from '../types/claude';
import { getCodebuddyAccountDisplayEmail } from '../types/codebuddy';
import { getWorkbuddyAccountDisplayEmail } from '../types/workbuddy';
import { getQoderAccountDisplayEmail } from '../types/qoder';
import { getZcodeAccountDisplayEmail } from '../types/zcode';
import {
  getTraeAccountDisplayEmail,
  getTraeAccountPlatformId,
} from '../types/trae';
import { getZedAccountDisplayEmail } from '../types/zed';
import { ALL_PLATFORM_IDS, PlatformId } from '../types/platform';
import { useEscClose } from '../hooks/useEscClose';
import './settings/Settings.css';
import { RefreshCw } from 'lucide-react';
import { SettingsPageView } from "./SettingsPageView";




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

interface DiagnosticsConfig {
  errorReportingEnabled: boolean;
  errorReportingDebug: boolean;
  endpointConfigured: boolean;
}

interface GrokCliStatus {
  available: boolean;
  binaryPath?: string | null;
  configuredPath?: string | null;
  version?: string | null;
  source?: string | null;
  message?: string | null;
}

/** 通用配置类型 */
interface GeneralConfig {
  language: string;
  default_terminal: string;
  theme: string;
  theme_color?: string;
  external_network_enabled?: boolean;
  webdav_allowed_domains?: string;
  reduced_motion_enabled: boolean;
  ui_scale: number;
  auto_refresh_minutes: number;
  codex_auto_refresh_minutes: number;
  claude_auto_refresh_minutes: number;
  codex_sync_wsl: boolean;
  codex_app_ui_injection_enabled?: boolean;
  codex_oauth_app_version?: string;
  codex_wsl_config_dir: string;
  ghcp_auto_refresh_minutes: number;
  windsurf_auto_refresh_minutes: number;
  kiro_auto_refresh_minutes: number;
  cursor_auto_refresh_minutes: number;
  grok_auto_refresh_minutes: number;
  grok_sync_official_auth_on_switch: boolean;
  grok_opencode_sync_on_switch?: boolean;
  grok_opencode_auth_overwrite_on_switch?: boolean;
  close_behavior: 'ask' | 'minimize' | 'quit';
  minimize_behavior?: 'dock_and_tray' | 'tray_only';
  hide_dock_icon?: boolean;
  tray_icon_style?: 'template' | 'color';
  menu_bar_quota_enabled?: boolean;
  menu_bar_show_account_prefix?: boolean;
  menu_bar_quota_platform?: PlatformId;
  floating_card_show_on_startup?: boolean;
  startup_minimized?: boolean;
  remember_main_window_state?: boolean;
  /** `last` = restore previous page; otherwise a page id like `dashboard` / `codex` */
  startup_page?: string;
  floating_card_always_on_top?: boolean;
  app_auto_launch_enabled?: boolean;
  token_keeper_enabled?: boolean;
  auto_import_from_local_enabled?: boolean;
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
  codebuddy_cn_share_sessions_on_switch: boolean;
  qoder_app_path: string;
  zcode_app_path: string;
  trae_app_path: string;
  trae_solo_app_path: string;
  trae_cn_app_path: string;
  trae_solo_cn_app_path: string;
  trae_share_sessions_on_switch: boolean;
  trae_solo_share_sessions_on_switch: boolean;
  trae_cn_share_sessions_on_switch: boolean;
  trae_solo_cn_share_sessions_on_switch: boolean;
  trae_app_scan_roots: string;
  trae_solo_app_scan_roots: string;
  trae_cn_app_scan_roots: string;
  trae_solo_cn_app_scan_roots: string;
  workbuddy_app_path: string;
  workbuddy_share_sessions_on_switch: boolean;
  zed_app_path: string;
  codebuddy_auto_refresh_minutes: number;
  codebuddy_cn_auto_refresh_minutes: number;
  workbuddy_auto_refresh_minutes: number;
  qoder_auto_refresh_minutes: number;
  zcode_auto_refresh_minutes: number;
  trae_auto_refresh_minutes: number;
  trae_solo_auto_refresh_minutes: number;
  trae_cn_auto_refresh_minutes: number;
  trae_solo_cn_auto_refresh_minutes: number;
  zed_auto_refresh_minutes: number;
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
  zed_quota_alert_enabled: boolean;
  zed_quota_alert_threshold: number;
  workbuddy_quota_alert_enabled: boolean;
  workbuddy_quota_alert_threshold: number;
  opencode_sync_on_switch: boolean;
  opencode_auth_overwrite_on_switch: boolean;
  openclaw_auth_overwrite_on_switch: boolean;
  hermes_auth_overwrite_on_switch?: boolean;
  codex_launch_on_switch: boolean;
  antigravity_launch_on_switch: boolean;
  codex_restart_specified_app_on_switch: boolean;
  codex_local_access_entry_visible: boolean;
  codex_hide_relay_quota?: boolean;
  top_right_ad_visible?: boolean;
  antigravity_dual_switch_no_restart_enabled: boolean;
  auto_switch_enabled: boolean;
  auto_switch_threshold: number;
  auto_switch_credits_enabled?: boolean;
  auto_switch_credits_threshold?: number;
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
  claude_quota_alert_enabled: boolean;
  claude_quota_alert_threshold: number;
  claude_quota_display_remaining?: boolean;
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
}

type AppPathTarget =
  | 'antigravity'
  | 'codex'
  | 'claude'
  | 'vscode'
  | 'opencode'
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

type TraeAppPathTarget = 'trae' | 'trae_solo' | 'trae_cn' | 'trae_solo_cn';

type ClaudeDesktopLaunchCandidate = {
  target_type: string;
  label: string;
  target: string;
  source: string;
  supports_multi_instance: boolean;
};
type AppLaunchCandidate = ClaudeDesktopLaunchCandidate;
const REFRESH_PRESET_VALUES = ['-1', '2', '5', '10', '15'];
const CURRENT_ACCOUNT_REFRESH_PRESET_VALUES = ['1', '2', '5', '10', '15'];
const THRESHOLD_PRESET_VALUES = ['0', '20', '40', '60'];
const CREDITS_THRESHOLD_PRESET_VALUES = ['0', '5', '10', '20'];
const ANTIGRAVITY_SEAMLESS_SWITCH_UNLOCK_REQUIRED_TAPS = 10;
const UNLOCK_FIREWORKS_VISIBLE_MS = 6000;
const AUTO_SWITCH_SCOPE_ALL_ACCOUNTS: AutoSwitchAccountScopeMode = 'all_accounts';
const AUTO_SWITCH_SCOPE_SELECTED_ACCOUNTS: AutoSwitchAccountScopeMode = 'selected_accounts';
const SETTINGS_PAGE_CONFIG_UPDATE_SOURCE_PREFIX = 'settings-page';
const FALLBACK_PLATFORM_SETTINGS_ORDER: Record<PlatformId, number> = {
  antigravity: 0,
  antigravity_ide: 1,
  codex: 2,
  codex_api_service: 3,
  claude_manager: 4,
  'github-copilot': 5,
  windsurf: 6,
  kiro: 7,
  cursor: 8,
  grok: 9,
  codebuddy: 10,
  codebuddy_cn: 11,
  qoder: 12,
  zcode: 13,
  trae: 14,
  trae_solo: 15,
  trae_cn: 16,
  trae_solo_cn: 17,
  workbuddy: 18,
  zed: 19,
};
type ConfigUpdatedEventDetail = {
  source?: string;
};
type UpdateCheckSource = 'auto' | 'manual';
type UpdateCheckFinishedDetail = {
  source: UpdateCheckSource;
  status: 'has_update' | 'up_to_date' | 'failed';
  currentVersion?: string;
  latestVersion?: string;
  error?: string;
};

type ReleaseHistorySectionKey = 'highlights' | 'added' | 'changed' | 'fixed' | 'removed';

interface ReleaseHistoryItem {
  version: string;
  date: string;
  highlights?: string[];
  added: string[];
  changed: string[];
  fixed: string[];
  removed: string[];
}

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

const dispatchSettingsConfigUpdated = (source: string) => {
  window.dispatchEvent(
    new CustomEvent<ConfigUpdatedEventDetail>('config-updated', {
      detail: { source },
    }),
  );
};

const areGeneralConfigPayloadValuesEqual = (left: unknown, right: unknown): boolean => {
  if (Object.is(left, right)) return true;
  if (!Array.isArray(left) || !Array.isArray(right) || left.length !== right.length) {
    return false;
  }
  return left.every((value, index) => Object.is(value, right[index]));
};

export function useSettingsPageController() {
  const { t } = useTranslation();
  const configUpdateSource = useMemo(
    () => `${SETTINGS_PAGE_CONFIG_UPDATE_SOURCE_PREFIX}:${generateReportToken()}`,
    [],
  );
  const isMacOS = usePlatformRuntimeSupport('macos-only');
  const isWindows = usePlatformRuntimeSupport('windows-only');
  const isLinux = usePlatformRuntimeSupport('linux-only');
  const sideNavLayoutMode = useSideNavLayoutStore((state) => state.mode);
  const setSideNavLayoutMode = useSideNavLayoutStore((state) => state.setMode);
  const [activeTab, setActiveTab] = useState<'general' | 'network' | 'data' | 'about'>('general');
  const [availableTerminals, setAvailableTerminals] = useState<string[]>(['system']);

  useEffect(() => {
    invoke<string[]>('get_available_terminals')
      .then(setAvailableTerminals)
      .catch(err => console.error('获取可用终端失败:', err));
  }, []);

  const terminalOptions = useMemo(() => {
    const common = [{
      value: 'system',
      label: isWindows
        ? t(
            'settings.general.terminalSystemWindowsCompatibility',
            '系统兼容（PowerShell）',
          )
        : t('settings.general.terminalSystem', '系统默认'),
    }];
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

  const menuBarQuotaPlatformOptions: Array<{ value: PlatformId; label: string }> = [
    { value: 'codex', label: 'Codex' },
    { value: 'claude_manager', label: 'Claude' },
    { value: 'antigravity', label: 'Antigravity' },
    { value: 'github-copilot', label: 'GitHub Copilot' },
    { value: 'windsurf', label: 'Windsurf' },
    { value: 'kiro', label: 'Kiro' },
    { value: 'cursor', label: 'Cursor' },
    { value: 'grok', label: 'Grok' },
    { value: 'codebuddy', label: 'CodeBuddy' },
    { value: 'codebuddy_cn', label: 'CodeBuddy CN' },
    { value: 'qoder', label: 'Qoder' },
    { value: 'zcode', label: 'ZCode' },
    { value: 'trae', label: 'Trae' },
    { value: 'trae_solo', label: 'TRAE SOLO' },
    { value: 'trae_cn', label: 'Trae CN' },
    { value: 'trae_solo_cn', label: 'TRAE SOLO CN' },
    { value: 'workbuddy', label: 'WorkBuddy' },
    { value: 'zed', label: 'Zed' },
  ];
  
  // General Settings States
  const [language, setLanguage] = useState(getCurrentLanguage());
  const [defaultTerminal, setDefaultTerminal] = useState('system');
  const [theme, setTheme] = useState('system');
  const [themeColor, setThemeColor] = useState('default');
  const [externalNetworkEnabled, setExternalNetworkEnabled] = useState(true);
  const [webdavAllowedDomains, setWebdavAllowedDomains] = useState('');
  const [reducedMotionEnabled, setReducedMotionEnabled] = useState(false);
  const [uiScale, setUiScale] = useState('1');
  const [autoRefresh, setAutoRefresh] = useState('5');
  const [codexAutoRefresh, setCodexAutoRefresh] = useState('10');
  const [claudeAutoRefresh, setClaudeAutoRefresh] = useState('10');
  const [codexSyncWsl, setCodexSyncWsl] = useState(false);
  const [codexAppUiInjectionEnabled, setCodexAppUiInjectionEnabled] = useState(true);
  const [codexWslConfigDir, setCodexWslConfigDir] = useState('');
  const [ghcpAutoRefresh, setGhcpAutoRefresh] = useState('10');
  const [windsurfAutoRefresh, setWindsurfAutoRefresh] = useState('10');
  const [kiroAutoRefresh, setKiroAutoRefresh] = useState('10');
  const [cursorAutoRefresh, setCursorAutoRefresh] = useState('10');
  const [grokAutoRefresh, setGrokAutoRefresh] = useState('10');
  const [grokSyncOfficialAuthOnSwitch, setGrokSyncOfficialAuthOnSwitch] = useState(false);
  const [grokOpencodeAuthOverwriteOnSwitch, setGrokOpencodeAuthOverwriteOnSwitch] = useState(false);
  const [grokOpencodeSyncOnSwitch, setGrokOpencodeSyncOnSwitch] = useState(false);
  const [grokCliPath, setGrokCliPath] = useState('');
  const [grokCliStatus, setGrokCliStatus] = useState<GrokCliStatus | null>(null);
  const [grokCliStatusError, setGrokCliStatusError] = useState<string | null>(null);
  const [grokCliSaving, setGrokCliSaving] = useState(false);
  const [closeBehavior, setCloseBehavior] = useState<'ask' | 'minimize' | 'quit'>('ask');
  const [minimizeBehavior, setMinimizeBehavior] = useState<'dock_and_tray' | 'tray_only'>('dock_and_tray');
  const [hideDockIcon, setHideDockIcon] = useState(false);
  const [trayIconStyle, setTrayIconStyle] = useState<'template' | 'color'>('template');
  const [menuBarQuotaEnabled, setMenuBarQuotaEnabled] = useState(false);
  const [menuBarShowAccountPrefix, setMenuBarShowAccountPrefix] = useState(true);
  const [menuBarQuotaPlatform, setMenuBarQuotaPlatform] = useState<PlatformId>('codex');
  const [menuBarQuotaModalOpen, setMenuBarQuotaModalOpen] = useState(false);
  const [menuBarQuotaModalMode, setMenuBarQuotaModalMode] = useState<'enable' | 'edit'>('enable');
  const [menuBarQuotaDraftPlatform, setMenuBarQuotaDraftPlatform] =
    useState<PlatformId>('codex');
  const [menuBarQuotaDraftShowPrefix, setMenuBarQuotaDraftShowPrefix] = useState(true);
  const [floatingCardShowOnStartup, setFloatingCardShowOnStartup] = useState(false);
  const [startupMinimized, setStartupMinimized] = useState(false);
  const [rememberMainWindowState, setRememberMainWindowState] = useState(false);
  const [startupPage, setStartupPage] = useState('last');
  const [floatingCardAlwaysOnTop, setFloatingCardAlwaysOnTop] = useState(false);
  const [appAutoLaunchEnabled, setAppAutoLaunchEnabled] = useState(false);
  const [tokenKeeperEnabled, setTokenKeeperEnabled] = useState(true);
  const [autoImportFromLocalEnabled, setAutoImportFromLocalEnabled] = useState(false);
  const [autoImportScanStatus, setAutoImportScanStatus] = useState('');
  const [autoImportScanBusy, setAutoImportScanBusy] = useState(false);
  const autoImportScanSeqRef = useRef(0);
  const [errorReportingEnabled, setErrorReportingEnabled] = useState(true);
  const [errorReportingSaving, setErrorReportingSaving] = useState(false);
  const [opencodeAppPath, setOpencodeAppPath] = useState('');
  const [antigravityAppPath, setAntigravityAppPath] = useState('');
  const [codexAppPath, setCodexAppPath] = useState('');
  const [codexOAuthAppVersion, setCodexOAuthAppVersion] = useState('');
  const [codexLaunchCandidates, setCodexLaunchCandidates] = useState<AppLaunchCandidate[]>([]);
  const [codexAppScanError, setCodexAppScanError] = useState('');
  const [claudeAppPath, setClaudeAppPath] = useState('');
  const [claudeAppScanRoots, setClaudeAppScanRoots] = useState('');
  const [codexSpecifiedAppPath, setCodexSpecifiedAppPath] = useState('');
  const [vscodeAppPath, setVscodeAppPath] = useState('');
  const [windsurfAppPath, setWindsurfAppPath] = useState('');
  const [kiroAppPath, setKiroAppPath] = useState('');
  const [cursorAppPath, setCursorAppPath] = useState('');
  const [codebuddyAppPath, setCodebuddyAppPath] = useState('');
  const [codebuddyShareSessionsOnSwitch, setCodebuddyShareSessionsOnSwitch] = useState(false);
  const [codebuddyCnAppPath, setCodebuddyCnAppPath] = useState('');
  const [codebuddyCnShareSessionsOnSwitch, setCodebuddyCnShareSessionsOnSwitch] = useState(false);
  const [qoderAppPath, setQoderAppPath] = useState('');
  const [zcodeAppPath, setZcodeAppPath] = useState('');
  const [traeAppPath, setTraeAppPath] = useState('');
  const [traeSoloAppPath, setTraeSoloAppPath] = useState('');
  const [traeCnAppPath, setTraeCnAppPath] = useState('');
  const [traeSoloCnAppPath, setTraeSoloCnAppPath] = useState('');
  const [workbuddyAppPath, setWorkbuddyAppPath] = useState('');
  const [workbuddyShareSessionsOnSwitch, setWorkbuddyShareSessionsOnSwitch] = useState(false);
  const [zedAppPath, setZedAppPath] = useState('');
  const [codebuddyAutoRefresh, setCodebuddyAutoRefresh] = useState('10');
  const [codebuddyCnAutoRefresh, setCodebuddyCnAutoRefresh] = useState('10');
  const [workbuddyAutoRefresh, setWorkbuddyAutoRefresh] = useState('10');
  const [qoderAutoRefresh, setQoderAutoRefresh] = useState('10');
  const [zcodeAutoRefresh, setZcodeAutoRefresh] = useState('10');
  const [traeAutoRefresh, setTraeAutoRefresh] = useState('10');
  const [traeSoloAutoRefresh, setTraeSoloAutoRefresh] = useState('10');
  const [traeCnAutoRefresh, setTraeCnAutoRefresh] = useState('10');
  const [traeSoloCnAutoRefresh, setTraeSoloCnAutoRefresh] = useState('10');
  const [zedAutoRefresh, setZedAutoRefresh] = useState('10');
  const [currentAccountRefreshMinutes, setCurrentAccountRefreshMinutes] = useState<
    Record<CurrentAccountRefreshPlatform, string>
  >(() =>
    toCurrentAccountRefreshMinutesStringMap(buildDefaultCurrentAccountRefreshMinutesMap()),
  );
  const [currentAccountRefreshCustomMode, setCurrentAccountRefreshCustomMode] = useState<
    Record<CurrentAccountRefreshPlatform, boolean>
  >(() => buildDefaultCurrentAccountRefreshCustomModeMap());
  const [accountOverrides, setAccountOverrides] = useState<AccountRefreshOverrides>(
    loadAccountRefreshOverrides(),
  );
  const [accountLevelRefreshCustomMode, setAccountLevelRefreshCustomMode] = useState<
    Record<string, boolean>
  >({});
  const [codebuddyQuotaAlertEnabled, setCodebuddyQuotaAlertEnabled] = useState(false);
  const [codebuddyQuotaAlertThreshold, setCodebuddyQuotaAlertThreshold] = useState('20');
  const [codebuddyCnQuotaAlertEnabled, setCodebuddyCnQuotaAlertEnabled] = useState(false);
  const [codebuddyCnQuotaAlertThreshold, setCodebuddyCnQuotaAlertThreshold] = useState('20');
  const [qoderQuotaAlertEnabled, setQoderQuotaAlertEnabled] = useState(false);
  const [qoderQuotaAlertThreshold, setQoderQuotaAlertThreshold] = useState('20');
  const [traeQuotaAlertEnabled, setTraeQuotaAlertEnabled] = useState(false);
  const [traeQuotaAlertThreshold, setTraeQuotaAlertThreshold] = useState('20');
  const [traeSoloQuotaAlertEnabled, setTraeSoloQuotaAlertEnabled] = useState(false);
  const [traeSoloQuotaAlertThreshold, setTraeSoloQuotaAlertThreshold] = useState('20');
  const [traeCnQuotaAlertEnabled, setTraeCnQuotaAlertEnabled] = useState(false);
  const [traeCnQuotaAlertThreshold, setTraeCnQuotaAlertThreshold] = useState('20');
  const [traeSoloCnQuotaAlertEnabled, setTraeSoloCnQuotaAlertEnabled] = useState(false);
  const [traeSoloCnQuotaAlertThreshold, setTraeSoloCnQuotaAlertThreshold] = useState('20');
  const [zedQuotaAlertEnabled, setZedQuotaAlertEnabled] = useState(false);
  const [zedQuotaAlertThreshold, setZedQuotaAlertThreshold] = useState('20');
  const [workbuddyQuotaAlertEnabled, setWorkbuddyQuotaAlertEnabled] = useState(false);
  const [workbuddyQuotaAlertThreshold, setWorkbuddyQuotaAlertThreshold] = useState('20');
  const [codebuddyAutoRefreshCustomMode, setCodebuddyAutoRefreshCustomMode] = useState(false);
  const [codebuddyCnAutoRefreshCustomMode, setCodebuddyCnAutoRefreshCustomMode] = useState(false);
  const [workbuddyAutoRefreshCustomMode, setWorkbuddyAutoRefreshCustomMode] = useState(false);
  const [codebuddyQuotaAlertThresholdCustomMode, setCodebuddyQuotaAlertThresholdCustomMode] = useState(false);
  const [qoderAutoRefreshCustomMode, setQoderAutoRefreshCustomMode] = useState(false);
  const [zcodeAutoRefreshCustomMode, setZcodeAutoRefreshCustomMode] = useState(false);
  const [qoderQuotaAlertThresholdCustomMode, setQoderQuotaAlertThresholdCustomMode] = useState(false);
  const [traeAutoRefreshCustomMode, setTraeAutoRefreshCustomMode] = useState(false);
  const [traeQuotaAlertThresholdCustomMode, setTraeQuotaAlertThresholdCustomMode] = useState(false);
  const [traeSoloAutoRefreshCustomMode, setTraeSoloAutoRefreshCustomMode] = useState(false);
  const [traeSoloQuotaAlertThresholdCustomMode, setTraeSoloQuotaAlertThresholdCustomMode] = useState(false);
  const [traeCnAutoRefreshCustomMode, setTraeCnAutoRefreshCustomMode] = useState(false);
  const [traeCnQuotaAlertThresholdCustomMode, setTraeCnQuotaAlertThresholdCustomMode] = useState(false);
  const [traeSoloCnAutoRefreshCustomMode, setTraeSoloCnAutoRefreshCustomMode] = useState(false);
  const [traeSoloCnQuotaAlertThresholdCustomMode, setTraeSoloCnQuotaAlertThresholdCustomMode] = useState(false);
  const [zedAutoRefreshCustomMode, setZedAutoRefreshCustomMode] = useState(false);
  const [zedQuotaAlertThresholdCustomMode, setZedQuotaAlertThresholdCustomMode] = useState(false);
  const [codebuddyCnQuotaAlertThresholdCustomMode, setCodebuddyCnQuotaAlertThresholdCustomMode] = useState(false);
  const [workbuddyQuotaAlertThresholdCustomMode, setWorkbuddyQuotaAlertThresholdCustomMode] = useState(false);
  const [appPathResetDetectingTargets, setAppPathResetDetectingTargets] = useState<Set<AppPathTarget>>(new Set());
  const [claudeLaunchCandidates, setClaudeLaunchCandidates] = useState<ClaudeDesktopLaunchCandidate[]>([]);
  const [traeAppScanRoots, setTraeAppScanRoots] = useState('');
  const [traeSoloAppScanRoots, setTraeSoloAppScanRoots] = useState('');
  const [traeCnAppScanRoots, setTraeCnAppScanRoots] = useState('');
  const [traeSoloCnAppScanRoots, setTraeSoloCnAppScanRoots] = useState('');
  const [traeLaunchCandidatesTarget, setTraeLaunchCandidatesTarget] = useState<TraeAppPathTarget>('trae');
  const [traeLaunchCandidates, setTraeLaunchCandidates] = useState<AppLaunchCandidate[]>([]);
  const [opencodeSyncOnSwitch, setOpencodeSyncOnSwitch] = useState(false);
  const [opencodeAuthOverwriteOnSwitch, setOpencodeAuthOverwriteOnSwitch] = useState(false);
  const [openclawAuthOverwriteOnSwitch, setOpenclawAuthOverwriteOnSwitch] = useState(false);
  const [hermesAuthOverwriteOnSwitch, setHermesAuthOverwriteOnSwitch] = useState(false);
  const [codexLaunchOnSwitch, setCodexLaunchOnSwitch] = useState(true);
  const [antigravityLaunchOnSwitch, setAntigravityLaunchOnSwitch] = useState(true);
  const [codexRestartSpecifiedAppOnSwitch, setCodexRestartSpecifiedAppOnSwitch] = useState(false);
  const [codexLocalAccessEntryVisible, setCodexLocalAccessEntryVisible] = useState(true);
  const [codexHideRelayQuota, setCodexHideRelayQuota] = useState(false);
  const [topRightAdVisible, setTopRightAdVisible] = useState(true);
  const [antigravityDualSwitchNoRestartEnabled, setAntigravityDualSwitchNoRestartEnabled] = useState(false);
  const [autoSwitchEnabled, setAutoSwitchEnabled] = useState(false);
  const [autoSwitchThreshold, setAutoSwitchThreshold] = useState('20');
  const [autoSwitchCreditsEnabled, setAutoSwitchCreditsEnabled] = useState(false);
  const [autoSwitchCreditsThreshold, setAutoSwitchCreditsThreshold] = useState('5');
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
  const [claudeQuotaAlertEnabled, setClaudeQuotaAlertEnabled] = useState(false);
  const [claudeQuotaAlertThreshold, setClaudeQuotaAlertThreshold] = useState('20');
  const [claudeQuotaDisplayRemaining, setClaudeQuotaDisplayRemaining] = useState(false);
  const [ghcpQuotaAlertEnabled, setGhcpQuotaAlertEnabled] = useState(false);
  const [ghcpQuotaAlertThreshold, setGhcpQuotaAlertThreshold] = useState('20');
  const [windsurfQuotaAlertEnabled, setWindsurfQuotaAlertEnabled] = useState(false);
  const [windsurfQuotaAlertThreshold, setWindsurfQuotaAlertThreshold] = useState('20');
  const [kiroQuotaAlertEnabled, setKiroQuotaAlertEnabled] = useState(false);
  const [kiroQuotaAlertThreshold, setKiroQuotaAlertThreshold] = useState('20');
  const [cursorQuotaAlertEnabled, setCursorQuotaAlertEnabled] = useState(false);
  const [cursorQuotaAlertThreshold, setCursorQuotaAlertThreshold] = useState('20');
  const [grokQuotaAlertEnabled, setGrokQuotaAlertEnabled] = useState(false);
  const [grokQuotaAlertThreshold, setGrokQuotaAlertThreshold] = useState('20');
  const [autoRefreshCustomMode, setAutoRefreshCustomMode] = useState(false);
  const [codexAutoRefreshCustomMode, setCodexAutoRefreshCustomMode] = useState(false);
  const [claudeAutoRefreshCustomMode, setClaudeAutoRefreshCustomMode] = useState(false);
  const [ghcpAutoRefreshCustomMode, setGhcpAutoRefreshCustomMode] = useState(false);
  const [windsurfAutoRefreshCustomMode, setWindsurfAutoRefreshCustomMode] = useState(false);
  const [kiroAutoRefreshCustomMode, setKiroAutoRefreshCustomMode] = useState(false);
  const [cursorAutoRefreshCustomMode, setCursorAutoRefreshCustomMode] = useState(false);
  const [autoSwitchThresholdCustomMode, setAutoSwitchThresholdCustomMode] = useState(false);
  const [autoSwitchCreditsThresholdCustomMode, setAutoSwitchCreditsThresholdCustomMode] = useState(false);
  const [quotaAlertThresholdCustomMode, setQuotaAlertThresholdCustomMode] = useState(false);
  const [codexQuotaAlertThresholdCustomMode, setCodexQuotaAlertThresholdCustomMode] = useState(false);
  const [claudeQuotaAlertThresholdCustomMode, setClaudeQuotaAlertThresholdCustomMode] = useState(false);
  const [ghcpQuotaAlertThresholdCustomMode, setGhcpQuotaAlertThresholdCustomMode] = useState(false);
  const [windsurfQuotaAlertThresholdCustomMode, setWindsurfQuotaAlertThresholdCustomMode] = useState(false);
  const [kiroQuotaAlertThresholdCustomMode, setKiroQuotaAlertThresholdCustomMode] = useState(false);
  const [cursorQuotaAlertThresholdCustomMode, setCursorQuotaAlertThresholdCustomMode] = useState(false);
  const [antigravitySeamlessSwitchUnlocked, setAntigravitySeamlessSwitchUnlocked] = useState(
    isAntigravitySeamlessSwitchFeatureUnlocked,
  );
  const [, setAboutAvatarTapCount] = useState(0);
  const [showUnlockFireworks, setShowUnlockFireworks] = useState(false);
  const unlockFireworksTimerRef = useRef<number | null>(null);
  const [generalLoaded, setGeneralLoaded] = useState(false);
  const [generalLoadFailed, setGeneralLoadFailed] = useState(false);
  const [generalConfigHydrationRevision, setGeneralConfigHydrationRevision] = useState(0);
  const generalSaveTimerRef = useRef<number | null>(null);
  const generalSaveQueueRef = useRef<Promise<void>>(Promise.resolve());
  const generalSaveInFlightRef = useRef(false);
  const pendingExternalConfigReloadRef = useRef(false);
  const skipNextGeneralSaveRef = useRef(false);
  const generalStateRevisionRef = useRef(0);
  const generalConfigLoadVersionRef = useRef(0);
  const generalConfigLoadInFlightRef = useRef(false);
  const hasHydratedGeneralConfigRef = useRef(false);
  const persistedGeneralPayloadRef = useRef<Record<string, unknown> | null>(null);
  const currentAccountRefreshPersistReadyRef = useRef(false);
  
  const [appVersion, setAppVersion] = useState('');
  const [updateChecking, setUpdateChecking] = useState(false);
  const [updateCheckMessage, setUpdateCheckMessage] = useState<{
    text: string;
    tone?: 'error' | 'success';
  } | null>(null);
  const [releaseHistoryOpen, setReleaseHistoryOpen] = useState(false);
  const [releaseHistoryLoading, setReleaseHistoryLoading] = useState(false);
  const [releaseHistoryError, setReleaseHistoryError] = useState('');
  const [releaseHistoryItems, setReleaseHistoryItems] = useState<ReleaseHistoryItem[]>([]);
  const [autoInstall, setAutoInstall] = useState(false);
  const [autoInstallLoaded, setAutoInstallLoaded] = useState(false);
  const autoInstallTouchedRef = useRef(false);
  const [updateRemindersEnabled, setUpdateRemindersEnabled] = useState(true);
  const [updateRemindersLoaded, setUpdateRemindersLoaded] = useState(false);
  const [updateSettingsLoadFailed, setUpdateSettingsLoadFailed] = useState(false);
  const updateRemindersTouchedRef = useRef(false);
  const updateSettingsSaveQueueRef = useRef<Promise<void>>(Promise.resolve());
  const updateSettingsSaveVersionRef = useRef(0);
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

  const loadUpdateSettings = async () => {
    setUpdateSettingsLoadFailed(false);
    try {
      const settings = await invoke<{
        auto_check: boolean;
        last_check_time: number;
        check_interval_hours: number;
        auto_install?: boolean;
        last_run_version?: string;
        remind_on_update?: boolean;
        skipped_version?: string;
      }>('get_update_settings');
      if (!autoInstallTouchedRef.current) {
        setAutoInstall(Boolean(settings?.auto_install));
      }
      if (!updateRemindersTouchedRef.current) {
        setUpdateRemindersEnabled(settings?.remind_on_update ?? true);
      }
      setAutoInstallLoaded(true);
      setUpdateRemindersLoaded(true);
    } catch (err) {
      console.error('加载自动更新设置失败:', err);
      setUpdateSettingsLoadFailed(true);
    }
  };

  useEffect(() => {
    getVersion().then(ver => setAppVersion(`v${ver}`));
    // Load updater preferences before enabling their controls.
    void loadUpdateSettings();
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
    loadDiagnosticsConfig();
    loadGrokCliStatus();
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
    applyReducedMotion(reducedMotionEnabled);
  }, [generalLoaded, reducedMotionEnabled]);

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
      generalSaveTimerRef.current = null;
    }

    if (
      !autoRefresh.trim() ||
      !codexAutoRefresh.trim() ||
      !claudeAutoRefresh.trim() ||
      !ghcpAutoRefresh.trim() ||
      !windsurfAutoRefresh.trim() ||
      !kiroAutoRefresh.trim() ||
      !codebuddyAutoRefresh.trim() ||
      !codebuddyCnAutoRefresh.trim() ||
      !workbuddyAutoRefresh.trim() ||
      !qoderAutoRefresh.trim() ||
      !zcodeAutoRefresh.trim() ||
      !traeAutoRefresh.trim() ||
      !traeSoloAutoRefresh.trim() ||
      !traeCnAutoRefresh.trim() ||
      !traeSoloCnAutoRefresh.trim() ||
      !zedAutoRefresh.trim() ||
      !cursorAutoRefresh.trim() ||
      !grokAutoRefresh.trim()
    ) {
      return;
    }

    const autoRefreshNum = parseInt(autoRefresh, 10) || -1;
    const codexAutoRefreshNum = parseInt(codexAutoRefresh, 10) || -1;
    const claudeAutoRefreshNum = parseInt(claudeAutoRefresh, 10) || -1;
    const ghcpAutoRefreshNum = parseInt(ghcpAutoRefresh, 10) || -1;
    const windsurfAutoRefreshNum = parseInt(windsurfAutoRefresh, 10) || -1;
    const kiroAutoRefreshNum = parseInt(kiroAutoRefresh, 10) || -1;
    const codebuddyAutoRefreshNum = parseInt(codebuddyAutoRefresh, 10) || -1;
    const codebuddyCnAutoRefreshNum = parseInt(codebuddyCnAutoRefresh, 10) || -1;
    const workbuddyAutoRefreshNum = parseInt(workbuddyAutoRefresh, 10) || -1;
    const qoderAutoRefreshNum = parseInt(qoderAutoRefresh, 10) || -1;
    const zcodeAutoRefreshNum = parseInt(zcodeAutoRefresh, 10) || -1;
    const traeAutoRefreshNum = parseInt(traeAutoRefresh, 10) || -1;
    const traeSoloAutoRefreshNum = parseInt(traeSoloAutoRefresh, 10) || -1;
    const traeCnAutoRefreshNum = parseInt(traeCnAutoRefresh, 10) || -1;
    const traeSoloCnAutoRefreshNum = parseInt(traeSoloCnAutoRefresh, 10) || -1;
    const zedAutoRefreshNum = parseInt(zedAutoRefresh, 10) || -1;
    const cursorAutoRefreshNum = parseInt(cursorAutoRefresh, 10) || -1;
    const grokAutoRefreshNum = parseInt(grokAutoRefresh, 10) || -1;
    const parsedUiScale = Number.parseFloat(uiScale);
    const normalizedUiScale = Number.isFinite(parsedUiScale)
      ? Math.min(2, Math.max(0.3, parsedUiScale))
      : 1;
    const parsedAutoSwitchThreshold = Number.parseInt(autoSwitchThreshold, 10);
    const parsedAutoSwitchCreditsThreshold = Number.parseInt(autoSwitchCreditsThreshold, 10);
    const parsedQuotaAlertThreshold = Number.parseInt(quotaAlertThreshold, 10);
    const parsedCodexQuotaAlertThreshold = Number.parseInt(codexQuotaAlertThreshold, 10);
    const parsedClaudeQuotaAlertThreshold = Number.parseInt(claudeQuotaAlertThreshold, 10);
    const parsedGhcpQuotaAlertThreshold = Number.parseInt(ghcpQuotaAlertThreshold, 10);
    const parsedWindsurfQuotaAlertThreshold = Number.parseInt(windsurfQuotaAlertThreshold, 10);
    const parsedKiroQuotaAlertThreshold = Number.parseInt(kiroQuotaAlertThreshold, 10);
    const parsedCodebuddyQuotaAlertThreshold = Number.parseInt(codebuddyQuotaAlertThreshold, 10);
    const parsedCodebuddyCnQuotaAlertThreshold = Number.parseInt(codebuddyCnQuotaAlertThreshold, 10);
    const parsedWorkbuddyQuotaAlertThreshold = Number.parseInt(workbuddyQuotaAlertThreshold, 10);
    const parsedQoderQuotaAlertThreshold = Number.parseInt(qoderQuotaAlertThreshold, 10);
    const parsedTraeQuotaAlertThreshold = Number.parseInt(traeQuotaAlertThreshold, 10);
    const parsedTraeSoloQuotaAlertThreshold = Number.parseInt(traeSoloQuotaAlertThreshold, 10);
    const parsedTraeCnQuotaAlertThreshold = Number.parseInt(traeCnQuotaAlertThreshold, 10);
    const parsedTraeSoloCnQuotaAlertThreshold = Number.parseInt(traeSoloCnQuotaAlertThreshold, 10);
    const parsedZedQuotaAlertThreshold = Number.parseInt(zedQuotaAlertThreshold, 10);
    const parsedCursorQuotaAlertThreshold = Number.parseInt(cursorQuotaAlertThreshold, 10);
    const parsedGrokQuotaAlertThreshold = Number.parseInt(grokQuotaAlertThreshold, 10);
    const payload: Record<string, unknown> = {
      language,
      default_terminal: defaultTerminal,
      theme,
      theme_color: themeColor || 'default',
      external_network_enabled: externalNetworkEnabled,
      webdav_allowed_domains: webdavAllowedDomains,
      reduced_motion_enabled: reducedMotionEnabled,
      ui_scale: normalizedUiScale,
      auto_refresh_minutes: autoRefreshNum,
      codex_auto_refresh_minutes: codexAutoRefreshNum,
      claude_auto_refresh_minutes: claudeAutoRefreshNum,
      codex_sync_wsl: codexSyncWsl,
      codex_app_ui_injection_enabled: codexAppUiInjectionEnabled,
      codex_wsl_config_dir: codexWslConfigDir,
      ghcp_auto_refresh_minutes: ghcpAutoRefreshNum,
      windsurf_auto_refresh_minutes: windsurfAutoRefreshNum,
      kiro_auto_refresh_minutes: kiroAutoRefreshNum,
      codebuddy_auto_refresh_minutes: codebuddyAutoRefreshNum,
      codebuddy_cn_auto_refresh_minutes: codebuddyCnAutoRefreshNum,
      workbuddy_auto_refresh_minutes: workbuddyAutoRefreshNum,
      qoder_auto_refresh_minutes: qoderAutoRefreshNum,
      zcode_auto_refresh_minutes: zcodeAutoRefreshNum,
      trae_auto_refresh_minutes: traeAutoRefreshNum,
      trae_solo_auto_refresh_minutes: traeSoloAutoRefreshNum,
      trae_cn_auto_refresh_minutes: traeCnAutoRefreshNum,
      trae_solo_cn_auto_refresh_minutes: traeSoloCnAutoRefreshNum,
      zed_auto_refresh_minutes: zedAutoRefreshNum,
      cursor_auto_refresh_minutes: cursorAutoRefreshNum,
      grok_auto_refresh_minutes: grokAutoRefreshNum,
      grok_sync_official_auth_on_switch: grokSyncOfficialAuthOnSwitch,
      grok_opencode_auth_overwrite_on_switch: grokOpencodeAuthOverwriteOnSwitch,
      grok_opencode_sync_on_switch: grokOpencodeAuthOverwriteOnSwitch && grokOpencodeSyncOnSwitch,
      close_behavior: closeBehavior,
      minimize_behavior: minimizeBehavior,
      hide_dock_icon: hideDockIcon,
      tray_icon_style: isMacOS ? trayIconStyle : undefined,
      menu_bar_quota_enabled: isMacOS ? menuBarQuotaEnabled : undefined,
      menu_bar_show_account_prefix: isMacOS ? menuBarShowAccountPrefix : undefined,
      menu_bar_quota_platform: isMacOS ? menuBarQuotaPlatform : undefined,
      floating_card_show_on_startup: floatingCardShowOnStartup,
      startup_minimized: startupMinimized,
      remember_main_window_state: rememberMainWindowState,
      startup_page: startupPage || 'last',
      floating_card_always_on_top: floatingCardAlwaysOnTop,
      app_auto_launch_enabled: appAutoLaunchEnabled,
      token_keeper_enabled: tokenKeeperEnabled,
      auto_import_from_local_enabled: autoImportFromLocalEnabled,
      opencode_app_path: opencodeAppPath,
      antigravity_app_path: antigravityAppPath,
      codex_app_path: codexAppPath,
      codex_oauth_app_version: codexOAuthAppVersion.trim(),
      claude_app_path: claudeAppPath,
      claude_app_scan_roots: claudeAppScanRoots,
      codex_specified_app_path: codexSpecifiedAppPath,
      vscode_app_path: vscodeAppPath,
      windsurf_app_path: windsurfAppPath,
      kiro_app_path: kiroAppPath,
      cursor_app_path: cursorAppPath,
      codebuddy_app_path: codebuddyAppPath,
      codebuddy_share_sessions_on_switch: codebuddyShareSessionsOnSwitch,
      codebuddy_cn_app_path: codebuddyCnAppPath,
      codebuddy_cn_share_sessions_on_switch: codebuddyCnShareSessionsOnSwitch,
      qoder_app_path: qoderAppPath,
      zcode_app_path: zcodeAppPath,
      trae_app_path: traeAppPath,
      trae_solo_app_path: traeSoloAppPath,
      trae_cn_app_path: traeCnAppPath,
      trae_solo_cn_app_path: traeSoloCnAppPath,
      // Trae session sharing is disabled for this release (no effective cross-account history).
      trae_share_sessions_on_switch: false,
      trae_solo_share_sessions_on_switch: false,
      trae_cn_share_sessions_on_switch: false,
      trae_solo_cn_share_sessions_on_switch: false,
      trae_app_scan_roots: traeAppScanRoots,
      trae_solo_app_scan_roots: traeSoloAppScanRoots,
      trae_cn_app_scan_roots: traeCnAppScanRoots,
      trae_solo_cn_app_scan_roots: traeSoloCnAppScanRoots,
      workbuddy_app_path: workbuddyAppPath,
      workbuddy_share_sessions_on_switch: workbuddyShareSessionsOnSwitch,
      zed_app_path: zedAppPath,
      opencode_sync_on_switch: opencodeSyncOnSwitch,
      opencode_auth_overwrite_on_switch: opencodeAuthOverwriteOnSwitch,
      openclaw_auth_overwrite_on_switch: openclawAuthOverwriteOnSwitch,
      hermes_auth_overwrite_on_switch: hermesAuthOverwriteOnSwitch,
      codex_launch_on_switch: codexLaunchOnSwitch,
      antigravity_launch_on_switch: antigravityLaunchOnSwitch,
      codex_restart_specified_app_on_switch: codexRestartSpecifiedAppOnSwitch,
      codex_local_access_entry_visible: codexLocalAccessEntryVisible,
      codex_hide_relay_quota: codexHideRelayQuota,
      top_right_ad_visible: topRightAdVisible,
      antigravity_dual_switch_no_restart_enabled: antigravityDualSwitchNoRestartEnabled,
      auto_switch_enabled: autoSwitchEnabled,
      auto_switch_threshold: Number.isNaN(parsedAutoSwitchThreshold)
        ? 20
        : parsedAutoSwitchThreshold,
      auto_switch_credits_enabled: autoSwitchCreditsEnabled,
      auto_switch_credits_threshold: Number.isNaN(parsedAutoSwitchCreditsThreshold)
        ? 5
        : parsedAutoSwitchCreditsThreshold,
      auto_switch_account_scope_mode: autoSwitchAccountScopeMode,
      auto_switch_selected_account_ids: autoSwitchSelectedAccountIds,
      codex_auto_switch_account_scope_mode: codexAutoSwitchAccountScopeMode,
      codex_auto_switch_selected_account_ids: codexAutoSwitchSelectedAccountIds,
      quota_alert_enabled: quotaAlertEnabled,
      quota_alert_threshold: Number.isNaN(parsedQuotaAlertThreshold)
        ? 20
        : parsedQuotaAlertThreshold,
      codex_quota_alert_enabled: codexQuotaAlertEnabled,
      codex_quota_alert_threshold: Number.isNaN(parsedCodexQuotaAlertThreshold)
        ? 20
        : parsedCodexQuotaAlertThreshold,
      claude_quota_alert_enabled: claudeQuotaAlertEnabled,
      claude_quota_alert_threshold: Number.isNaN(parsedClaudeQuotaAlertThreshold)
        ? 20
        : parsedClaudeQuotaAlertThreshold,
      claude_quota_display_remaining: (() => {
        setClaudeQuotaDisplayRemainingEnabled(claudeQuotaDisplayRemaining);
        return claudeQuotaDisplayRemaining;
      })(),
      ghcp_quota_alert_enabled: ghcpQuotaAlertEnabled,
      ghcp_quota_alert_threshold: Number.isNaN(parsedGhcpQuotaAlertThreshold)
        ? 20
        : parsedGhcpQuotaAlertThreshold,
      windsurf_quota_alert_enabled: windsurfQuotaAlertEnabled,
      windsurf_quota_alert_threshold: Number.isNaN(parsedWindsurfQuotaAlertThreshold)
        ? 20
        : parsedWindsurfQuotaAlertThreshold,
      kiro_quota_alert_enabled: kiroQuotaAlertEnabled,
      kiro_quota_alert_threshold: Number.isNaN(parsedKiroQuotaAlertThreshold)
        ? 20
        : parsedKiroQuotaAlertThreshold,
      codebuddy_quota_alert_enabled: codebuddyQuotaAlertEnabled,
      codebuddy_quota_alert_threshold: Number.isNaN(parsedCodebuddyQuotaAlertThreshold)
        ? 20
        : parsedCodebuddyQuotaAlertThreshold,
      codebuddy_cn_quota_alert_enabled: codebuddyCnQuotaAlertEnabled,
      codebuddy_cn_quota_alert_threshold: Number.isNaN(parsedCodebuddyCnQuotaAlertThreshold)
        ? 20
        : parsedCodebuddyCnQuotaAlertThreshold,
      workbuddy_quota_alert_enabled: workbuddyQuotaAlertEnabled,
      workbuddy_quota_alert_threshold: Number.isNaN(parsedWorkbuddyQuotaAlertThreshold)
        ? 20
        : parsedWorkbuddyQuotaAlertThreshold,
      qoder_quota_alert_enabled: qoderQuotaAlertEnabled,
      qoder_quota_alert_threshold: Number.isNaN(parsedQoderQuotaAlertThreshold)
        ? 20
        : parsedQoderQuotaAlertThreshold,
      trae_quota_alert_enabled: traeQuotaAlertEnabled,
      trae_quota_alert_threshold: Number.isNaN(parsedTraeQuotaAlertThreshold)
        ? 20
        : parsedTraeQuotaAlertThreshold,
      trae_solo_quota_alert_enabled: traeSoloQuotaAlertEnabled,
      trae_solo_quota_alert_threshold: Number.isNaN(parsedTraeSoloQuotaAlertThreshold)
        ? 20
        : parsedTraeSoloQuotaAlertThreshold,
      trae_cn_quota_alert_enabled: traeCnQuotaAlertEnabled,
      trae_cn_quota_alert_threshold: Number.isNaN(parsedTraeCnQuotaAlertThreshold)
        ? 20
        : parsedTraeCnQuotaAlertThreshold,
      trae_solo_cn_quota_alert_enabled: traeSoloCnQuotaAlertEnabled,
      trae_solo_cn_quota_alert_threshold: Number.isNaN(parsedTraeSoloCnQuotaAlertThreshold)
        ? 20
        : parsedTraeSoloCnQuotaAlertThreshold,
      zed_quota_alert_enabled: zedQuotaAlertEnabled,
      zed_quota_alert_threshold: Number.isNaN(parsedZedQuotaAlertThreshold)
        ? 20
        : parsedZedQuotaAlertThreshold,
      cursor_quota_alert_enabled: cursorQuotaAlertEnabled,
      cursor_quota_alert_threshold: Number.isNaN(parsedCursorQuotaAlertThreshold)
        ? 20
        : parsedCursorQuotaAlertThreshold,      grok_quota_alert_enabled: grokQuotaAlertEnabled,
      grok_quota_alert_threshold: Number.isNaN(parsedGrokQuotaAlertThreshold)
        ? 20
        : parsedGrokQuotaAlertThreshold,
    };
    Object.keys(payload).forEach((key) => {
      if (payload[key] === undefined) delete payload[key];
    });
    if (skipNextGeneralSaveRef.current) {
      skipNextGeneralSaveRef.current = false;
      persistedGeneralPayloadRef.current = payload;
      return;
    }

    const persistedPayload = persistedGeneralPayloadRef.current;
    if (!persistedPayload) {
      persistedGeneralPayloadRef.current = payload;
      return;
    }
    const updates = Object.fromEntries(
      Object.entries(payload).filter(
        ([key, value]) =>
          !areGeneralConfigPayloadValuesEqual(value, persistedPayload[key]),
      ),
    );
    if (Object.keys(updates).length === 0) {
      return;
    }
    generalStateRevisionRef.current += 1;

    generalSaveTimerRef.current = window.setTimeout(async () => {
      generalSaveTimerRef.current = null;
      generalSaveInFlightRef.current = true;
      const operation = generalSaveQueueRef.current.then(async () => {
        try {
          await invoke('patch_general_config', { updates });
          persistedGeneralPayloadRef.current = {
            ...(persistedGeneralPayloadRef.current ?? {}),
            ...updates,
          };
          dispatchSettingsConfigUpdated(configUpdateSource);
        } catch (err) {
          console.error('保存通用配置失败:', err);
          alert(`${t('settings.network.saveFailed').replace('{error}', String(err))}`);
          if (generalSaveQueueRef.current === operation) {
            await loadGeneralConfig();
          }
        } finally {
          if (generalSaveQueueRef.current === operation) {
            generalSaveInFlightRef.current = false;
            if (
              pendingExternalConfigReloadRef.current &&
              generalSaveTimerRef.current === null &&
              !generalConfigLoadInFlightRef.current
            ) {
              pendingExternalConfigReloadRef.current = false;
              void loadGeneralConfig();
            }
          }
        }
      });
      generalSaveQueueRef.current = operation;
      await operation;
    }, 300);

    return undefined;
  }, [
    autoRefresh,
    codexAutoRefresh,
    claudeAutoRefresh,
    codexSyncWsl,
    codexAppUiInjectionEnabled,
    codexWslConfigDir,
    ghcpAutoRefresh,
    windsurfAutoRefresh,
    kiroAutoRefresh,
    traeAutoRefresh,
    traeSoloAutoRefresh,
    traeCnAutoRefresh,
    traeSoloCnAutoRefresh,
    zedAutoRefresh,
    workbuddyAutoRefresh,
    qoderAutoRefresh,
    zcodeAutoRefresh,
    cursorAutoRefresh,
    grokAutoRefresh,
    grokSyncOfficialAuthOnSwitch,
    grokOpencodeAuthOverwriteOnSwitch,
    grokOpencodeSyncOnSwitch,
    closeBehavior,
    minimizeBehavior,
    hideDockIcon,
    trayIconStyle,
    menuBarQuotaEnabled,
    menuBarShowAccountPrefix,
    menuBarQuotaPlatform,
    isMacOS,
    floatingCardShowOnStartup,
    startupMinimized,
    rememberMainWindowState,
    startupPage,
    floatingCardAlwaysOnTop,
    appAutoLaunchEnabled,
    tokenKeeperEnabled,
    autoImportFromLocalEnabled,
    generalLoaded,
    generalConfigHydrationRevision,
    language,
    defaultTerminal,
    theme,
    themeColor,
    externalNetworkEnabled,
    webdavAllowedDomains,
    reducedMotionEnabled,
    uiScale,
    opencodeAppPath,
    antigravityAppPath,
    codexAppPath,
    claudeAppPath,
    claudeAppScanRoots,
    codexSpecifiedAppPath,
    vscodeAppPath,
    windsurfAppPath,
    kiroAppPath,
    cursorAppPath,
    codebuddyAppPath,
    codebuddyShareSessionsOnSwitch,
    codebuddyCnAppPath,
    codebuddyCnShareSessionsOnSwitch,
    qoderAppPath,
    zcodeAppPath,
    traeAppPath,
    traeSoloAppPath,
    traeCnAppPath,
    traeSoloCnAppPath,
    traeAppScanRoots,
    traeSoloAppScanRoots,
    traeCnAppScanRoots,
    traeSoloCnAppScanRoots,
    workbuddyAppPath,
    workbuddyShareSessionsOnSwitch,
    zedAppPath,
    opencodeSyncOnSwitch,
    opencodeAuthOverwriteOnSwitch,
    openclawAuthOverwriteOnSwitch,
    hermesAuthOverwriteOnSwitch,
    codexLaunchOnSwitch,
    antigravityLaunchOnSwitch,
    codexRestartSpecifiedAppOnSwitch,
    codexLocalAccessEntryVisible,
    codexHideRelayQuota,
    topRightAdVisible,
    antigravityDualSwitchNoRestartEnabled,
    autoSwitchEnabled,
    autoSwitchThreshold,
    autoSwitchCreditsEnabled,
    autoSwitchCreditsThreshold,
    autoSwitchAccountScopeMode,
    autoSwitchSelectedAccountIds,
    codexAutoSwitchAccountScopeMode,
    codexAutoSwitchSelectedAccountIds,
    quotaAlertEnabled,
    quotaAlertThreshold,
    codexQuotaAlertEnabled,
    codexQuotaAlertThreshold,
    claudeQuotaAlertEnabled,
    claudeQuotaAlertThreshold,
    claudeQuotaDisplayRemaining,
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
    traeSoloQuotaAlertEnabled,
    traeSoloQuotaAlertThreshold,
    traeCnQuotaAlertEnabled,
    traeCnQuotaAlertThreshold,
    traeSoloCnQuotaAlertEnabled,
    traeSoloCnQuotaAlertThreshold,
    zedQuotaAlertEnabled,
    zedQuotaAlertThreshold,
    cursorQuotaAlertEnabled,
    cursorQuotaAlertThreshold,
    grokQuotaAlertEnabled,
    grokQuotaAlertThreshold,
    configUpdateSource,
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
    dispatchSettingsConfigUpdated(configUpdateSource);
  }, [configUpdateSource, generalLoaded, currentAccountRefreshMinutes]);

  useEffect(() => {
    const handleLanguageUpdated = (event: Event) => {
      const detail = (event as CustomEvent<{ language?: string }>).detail;
      if (!detail?.language) {
        return;
      }
      setLanguage(normalizeLanguage(detail.language));
    };

    window.addEventListener('general-language-updated', handleLanguageUpdated);
    return () => {
      window.removeEventListener('general-language-updated', handleLanguageUpdated);
    };
  }, []);

  // 监听外部配置更新（如 QuickSettingsPopover 保存后同步）
  useEffect(() => {
    const handleConfigUpdated = (event: Event) => {
      const detail = (event as CustomEvent<ConfigUpdatedEventDetail>).detail;
      if (detail?.source === configUpdateSource) {
        return;
      }
      if (
        generalSaveTimerRef.current !== null ||
        generalSaveInFlightRef.current ||
        generalConfigLoadInFlightRef.current
      ) {
        pendingExternalConfigReloadRef.current = true;
        return;
      }
      void loadGeneralConfig();
    };
    window.addEventListener('config-updated', handleConfigUpdated);
    return () => {
      window.removeEventListener('config-updated', handleConfigUpdated);
    };
  }, [configUpdateSource]);

  // Serialize updater preference saves so the two toggles cannot overwrite each other.
  useEffect(() => {
    if (!autoInstallLoaded || !updateRemindersLoaded) {
      return;
    }

    const saveVersion = updateSettingsSaveVersionRef.current + 1;
    updateSettingsSaveVersionRef.current = saveVersion;
    const operation = updateSettingsSaveQueueRef.current.then(async () => {
      await invoke('patch_update_settings', {
        autoInstall,
        remindOnUpdate: updateRemindersEnabled,
      });
      window.dispatchEvent(
        new CustomEvent('update-reminder-changed', {
          detail: { enabled: updateRemindersEnabled },
        }),
      );
    }).catch(async (error: unknown) => {
      console.error('Failed to save update settings:', error);
      if (saveVersion !== updateSettingsSaveVersionRef.current) {
        return;
      }
      try {
        const settings = await invoke<{
          auto_install?: boolean;
          remind_on_update?: boolean;
        }>('get_update_settings');
        setAutoInstall(Boolean(settings.auto_install));
        setUpdateRemindersEnabled(settings.remind_on_update ?? true);
      } catch (reloadError) {
        console.error('Failed to reload update settings:', reloadError);
      }
    });
    updateSettingsSaveQueueRef.current = operation;
  }, [
    autoInstall,
    autoInstallLoaded,
    updateRemindersEnabled,
    updateRemindersLoaded,
  ]);

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
    const normalized = Number.isFinite(parsed) ? Math.min(2, Math.max(0.3, parsed)) : 1;
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
    const loadVersion = generalConfigLoadVersionRef.current + 1;
    generalConfigLoadVersionRef.current = loadVersion;
    const stateRevisionAtStart = generalStateRevisionRef.current;
    generalConfigLoadInFlightRef.current = true;
    setGeneralLoadFailed(false);
    if (hasHydratedGeneralConfigRef.current) {
      setGeneralLoaded(false);
    }
    try {
      const config = await invoke<GeneralConfig>('get_general_config');
      if (loadVersion !== generalConfigLoadVersionRef.current) {
        return;
      }
      if (
        hasHydratedGeneralConfigRef.current &&
        stateRevisionAtStart !== generalStateRevisionRef.current
      ) {
        pendingExternalConfigReloadRef.current = true;
        setGeneralLoaded(true);
        return;
      }
      skipNextGeneralSaveRef.current = true;
      setGeneralConfigHydrationRevision((revision) => revision + 1);
      setLanguage(normalizeLanguage(config.language));
      const configuredTerminal = config.default_terminal || 'system';
      setDefaultTerminal(
        configuredTerminal.toLowerCase() === 'powershell'
          ? 'PowerShell'
          : configuredTerminal,
      );
      setTheme(config.theme);
      setThemeColor((config.theme_color || 'default').trim() || 'default');
      setExternalNetworkEnabled(config.external_network_enabled ?? true);
      setWebdavAllowedDomains(config.webdav_allowed_domains || '');
      setReducedMotionEnabled(Boolean(config.reduced_motion_enabled ?? false));
      setUiScale(String(config.ui_scale ?? 1));
      setAutoRefresh(String(config.auto_refresh_minutes));
      setCodexAutoRefresh(String(config.codex_auto_refresh_minutes ?? 10));
      setClaudeAutoRefresh(String(config.claude_auto_refresh_minutes ?? 10));
      setCodexSyncWsl(Boolean(config.codex_sync_wsl ?? false));
      setCodexAppUiInjectionEnabled(Boolean(config.codex_app_ui_injection_enabled ?? false));
      setCodexWslConfigDir(config.codex_wsl_config_dir || '');
      setGhcpAutoRefresh(String(config.ghcp_auto_refresh_minutes ?? 10));
      setWindsurfAutoRefresh(String(config.windsurf_auto_refresh_minutes ?? 10));
      setKiroAutoRefresh(String(config.kiro_auto_refresh_minutes ?? 10));
      setCursorAutoRefresh(String(config.cursor_auto_refresh_minutes ?? 10));
      setGrokAutoRefresh(String(config.grok_auto_refresh_minutes ?? 10));
      setGrokSyncOfficialAuthOnSwitch(Boolean(config.grok_sync_official_auth_on_switch));
      setGrokOpencodeAuthOverwriteOnSwitch(Boolean(config.grok_opencode_auth_overwrite_on_switch));
      setGrokOpencodeSyncOnSwitch(Boolean(config.grok_opencode_sync_on_switch));
      setCloseBehavior(config.close_behavior || 'ask');
      setMinimizeBehavior(config.minimize_behavior || 'dock_and_tray');
      setHideDockIcon(Boolean(config.hide_dock_icon));
      setTrayIconStyle(config.tray_icon_style === 'color' ? 'color' : 'template');
      setMenuBarQuotaEnabled(config.menu_bar_quota_enabled ?? false);
      setMenuBarShowAccountPrefix(config.menu_bar_show_account_prefix ?? true);
      setMenuBarQuotaPlatform(config.menu_bar_quota_platform ?? 'codex');
      setFloatingCardShowOnStartup(config.floating_card_show_on_startup ?? false);
      setStartupMinimized(config.startup_minimized ?? false);
      setRememberMainWindowState(config.remember_main_window_state ?? false);
      setStartupPage((config.startup_page || 'last').trim() || 'last');
      setFloatingCardAlwaysOnTop(config.floating_card_always_on_top ?? false);
      setAppAutoLaunchEnabled(config.app_auto_launch_enabled ?? false);
      setTokenKeeperEnabled(config.token_keeper_enabled ?? true);
      setAutoImportFromLocalEnabled(config.auto_import_from_local_enabled ?? false);
      setOpencodeAppPath(config.opencode_app_path || '');
      setAntigravityAppPath(config.antigravity_app_path || '');
      setCodexAppPath(config.codex_app_path || '');
      setCodexOAuthAppVersion(config.codex_oauth_app_version || '');
      setClaudeAppPath(config.claude_app_path || '');
      setClaudeAppScanRoots(config.claude_app_scan_roots || '');
      setClaudeLaunchCandidates([]);
      setCodexSpecifiedAppPath(config.codex_specified_app_path || '');
      setVscodeAppPath(config.vscode_app_path || '');
      setWindsurfAppPath(config.windsurf_app_path || '');
      setKiroAppPath(config.kiro_app_path || '');
      setCursorAppPath(config.cursor_app_path || '');
      setCodebuddyAppPath(config.codebuddy_app_path || '');
      setCodebuddyShareSessionsOnSwitch(config.codebuddy_share_sessions_on_switch ?? false);
      setCodebuddyCnAppPath(config.codebuddy_cn_app_path || '');
      setCodebuddyCnShareSessionsOnSwitch(config.codebuddy_cn_share_sessions_on_switch ?? false);
      setQoderAppPath(config.qoder_app_path || '');
      setZcodeAppPath(config.zcode_app_path || '');
      setTraeAppPath(config.trae_app_path || '');
      setTraeSoloAppPath(config.trae_solo_app_path || '');
      setTraeCnAppPath(config.trae_cn_app_path || '');
      setTraeSoloCnAppPath(config.trae_solo_cn_app_path || '');
      setTraeAppScanRoots(config.trae_app_scan_roots || '');
      setTraeSoloAppScanRoots(config.trae_solo_app_scan_roots || '');
      setTraeCnAppScanRoots(config.trae_cn_app_scan_roots || '');
      setTraeSoloCnAppScanRoots(config.trae_solo_cn_app_scan_roots || '');
      setTraeLaunchCandidatesTarget('trae');
      setTraeLaunchCandidates([]);
      setWorkbuddyAppPath(config.workbuddy_app_path || '');
      setWorkbuddyShareSessionsOnSwitch(config.workbuddy_share_sessions_on_switch ?? false);
      setZedAppPath(config.zed_app_path || '');
      setCodebuddyAutoRefresh(String(config.codebuddy_auto_refresh_minutes ?? 10));
      setCodebuddyCnAutoRefresh(String(config.codebuddy_cn_auto_refresh_minutes ?? 10));
      setWorkbuddyAutoRefresh(String(config.workbuddy_auto_refresh_minutes ?? 10));
      setQoderAutoRefresh(String(config.qoder_auto_refresh_minutes ?? 10));
      setZcodeAutoRefresh(String(config.zcode_auto_refresh_minutes ?? 10));
      setTraeAutoRefresh(String(config.trae_auto_refresh_minutes ?? 10));
      setTraeSoloAutoRefresh(String(config.trae_solo_auto_refresh_minutes ?? 10));
      setTraeCnAutoRefresh(String(config.trae_cn_auto_refresh_minutes ?? 10));
      setTraeSoloCnAutoRefresh(String(config.trae_solo_cn_auto_refresh_minutes ?? 10));
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
      setTraeSoloQuotaAlertEnabled(config.trae_solo_quota_alert_enabled ?? false);
      setTraeSoloQuotaAlertThreshold(String(config.trae_solo_quota_alert_threshold ?? 20));
      setTraeCnQuotaAlertEnabled(config.trae_cn_quota_alert_enabled ?? false);
      setTraeCnQuotaAlertThreshold(String(config.trae_cn_quota_alert_threshold ?? 20));
      setTraeSoloCnQuotaAlertEnabled(config.trae_solo_cn_quota_alert_enabled ?? false);
      setTraeSoloCnQuotaAlertThreshold(String(config.trae_solo_cn_quota_alert_threshold ?? 20));
      setZedQuotaAlertEnabled(config.zed_quota_alert_enabled ?? false);
      setZedQuotaAlertThreshold(String(config.zed_quota_alert_threshold ?? 20));
      setOpencodeSyncOnSwitch(config.opencode_sync_on_switch ?? false);
      setOpencodeAuthOverwriteOnSwitch(config.opencode_auth_overwrite_on_switch ?? false);
      setOpenclawAuthOverwriteOnSwitch(config.openclaw_auth_overwrite_on_switch ?? false);
      setHermesAuthOverwriteOnSwitch(config.hermes_auth_overwrite_on_switch ?? false);
      setCodexLaunchOnSwitch(config.codex_launch_on_switch ?? true);
      setAntigravityLaunchOnSwitch(config.antigravity_launch_on_switch ?? true);
      setCodexRestartSpecifiedAppOnSwitch(
        config.codex_restart_specified_app_on_switch ?? false,
      );
      setCodexLocalAccessEntryVisible(config.codex_local_access_entry_visible ?? true);
      setCodexHideRelayQuota(config.codex_hide_relay_quota ?? false);
      setTopRightAdVisible(config.top_right_ad_visible ?? true);
      setAntigravityDualSwitchNoRestartEnabled(
        config.antigravity_dual_switch_no_restart_enabled ?? false
      );
      setAutoSwitchEnabled(config.auto_switch_enabled ?? false);
      setAutoSwitchThreshold(String(config.auto_switch_threshold ?? 20));
      setAutoSwitchCreditsEnabled(config.auto_switch_credits_enabled ?? false);
      setAutoSwitchCreditsThreshold(String(config.auto_switch_credits_threshold ?? 5));
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
      setClaudeQuotaAlertEnabled(config.claude_quota_alert_enabled ?? false);
      setClaudeQuotaAlertThreshold(String(config.claude_quota_alert_threshold ?? 20));
      const claudeRemainingDisplay = config.claude_quota_display_remaining ?? false;
      setClaudeQuotaDisplayRemaining(claudeRemainingDisplay);
      setClaudeQuotaDisplayRemainingEnabled(claudeRemainingDisplay);
      setGhcpQuotaAlertEnabled(config.ghcp_quota_alert_enabled ?? false);
      setGhcpQuotaAlertThreshold(String(config.ghcp_quota_alert_threshold ?? 20));
      setWindsurfQuotaAlertEnabled(config.windsurf_quota_alert_enabled ?? false);
      setWindsurfQuotaAlertThreshold(String(config.windsurf_quota_alert_threshold ?? 20));
      setKiroQuotaAlertEnabled(config.kiro_quota_alert_enabled ?? false);
      setKiroQuotaAlertThreshold(String(config.kiro_quota_alert_threshold ?? 20));
      setCursorQuotaAlertEnabled(config.cursor_quota_alert_enabled ?? false);
      setCursorQuotaAlertThreshold(String(config.cursor_quota_alert_threshold ?? 20));      setGrokQuotaAlertEnabled(config.grok_quota_alert_enabled ?? false);
      setGrokQuotaAlertThreshold(String(config.grok_quota_alert_threshold ?? 20));
      setAutoRefreshCustomMode(false);
      setCodexAutoRefreshCustomMode(false);
      setClaudeAutoRefreshCustomMode(false);
      setGhcpAutoRefreshCustomMode(false);
      setWindsurfAutoRefreshCustomMode(false);
      setKiroAutoRefreshCustomMode(false);
      setCodebuddyAutoRefreshCustomMode(false);
      setCodebuddyCnAutoRefreshCustomMode(false);
      setWorkbuddyAutoRefreshCustomMode(false);
      setQoderAutoRefreshCustomMode(false);
      setZcodeAutoRefreshCustomMode(false);
      setTraeAutoRefreshCustomMode(false);
      setTraeSoloAutoRefreshCustomMode(false);
      setTraeCnAutoRefreshCustomMode(false);
      setTraeSoloCnAutoRefreshCustomMode(false);
      setZedAutoRefreshCustomMode(false);
      setCursorAutoRefreshCustomMode(false);      setAutoSwitchThresholdCustomMode(false);
      setAutoSwitchCreditsThresholdCustomMode(false);
      setQuotaAlertThresholdCustomMode(false);
      setCodexQuotaAlertThresholdCustomMode(false);
      setClaudeQuotaAlertThresholdCustomMode(false);
      setGhcpQuotaAlertThresholdCustomMode(false);
      setWindsurfQuotaAlertThresholdCustomMode(false);
      setKiroQuotaAlertThresholdCustomMode(false);
      setCodebuddyQuotaAlertThresholdCustomMode(false);
      setCodebuddyCnQuotaAlertThresholdCustomMode(false);
      setWorkbuddyQuotaAlertThresholdCustomMode(false);
      setQoderQuotaAlertThresholdCustomMode(false);
      setTraeQuotaAlertThresholdCustomMode(false);
      setTraeSoloQuotaAlertThresholdCustomMode(false);
      setTraeCnQuotaAlertThresholdCustomMode(false);
      setTraeSoloCnQuotaAlertThresholdCustomMode(false);
      setZedQuotaAlertThresholdCustomMode(false);
      setCursorQuotaAlertThresholdCustomMode(false);      setCurrentAccountRefreshCustomMode(buildDefaultCurrentAccountRefreshCustomModeMap());
      currentAccountRefreshPersistReadyRef.current = false;
      // 同步语言
      changeLanguage(config.language);
      applyTheme(config.theme);
      hasHydratedGeneralConfigRef.current = true;
      setGeneralLoadFailed(false);
      setGeneralLoaded(true);
    } catch (err) {
      if (loadVersion !== generalConfigLoadVersionRef.current) {
        return;
      }
      console.error('加载通用配置失败:', err);
      setGeneralLoadFailed(true);
      if (hasHydratedGeneralConfigRef.current) {
        setGeneralLoaded(true);
      }
    } finally {
      if (loadVersion !== generalConfigLoadVersionRef.current) {
        return;
      }
      generalConfigLoadInFlightRef.current = false;
      if (
        pendingExternalConfigReloadRef.current &&
        generalSaveTimerRef.current === null &&
        !generalSaveInFlightRef.current
      ) {
        pendingExternalConfigReloadRef.current = false;
        void loadGeneralConfig();
      }
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

  const loadGrokCliStatus = async () => {
    try {
      const status = await invoke<GrokCliStatus>('grok_get_cli_status');
      setGrokCliStatus(status);
      setGrokCliPath(status.configuredPath || '');
      setGrokCliStatusError(null);
    } catch (error) {
      setGrokCliStatusError(String(error));
    }
  };

  const saveGrokCliPath = async () => {
    setGrokCliSaving(true);
    setGrokCliStatusError(null);
    try {
      const status = await invoke<GrokCliStatus>('grok_update_cli_runtime_config', {
        grokCliPath: grokCliPath.trim() || null,
      });
      setGrokCliStatus(status);
      setGrokCliPath(status.configuredPath || '');
    } catch (error) {
      setGrokCliStatusError(String(error));
    } finally {
      setGrokCliSaving(false);
    }
  };

  const loadDiagnosticsConfig = async () => {
    try {
      const config = await invoke<DiagnosticsConfig>('get_diagnostics_config');
      setErrorReportingEnabled(config.errorReportingEnabled);
    } catch (err) {
      console.warn('加载诊断配置失败:', err);
    }
  };

  const handleErrorReportingEnabledChange = async (enabled: boolean) => {
    const previous = errorReportingEnabled;
    setErrorReportingEnabled(enabled);
    setErrorReportingSaving(true);
    try {
      await invoke('save_diagnostics_config', {
        errorReportingEnabled: enabled,
        errorReportingDebug: false,
      });
    } catch (err) {
      setErrorReportingEnabled(previous);
      console.error('保存诊断配置失败:', err);
    } finally {
      setErrorReportingSaving(false);
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

  const isTraeAppPathTarget = (target: AppPathTarget): target is TraeAppPathTarget =>
    target === 'trae' || target === 'trae_solo' || target === 'trae_cn' || target === 'trae_solo_cn';

  const getTraeAppPathValue = (target: TraeAppPathTarget) => {
    switch (target) {
      case 'trae_solo':
        return traeSoloAppPath;
      case 'trae_cn':
        return traeCnAppPath;
      case 'trae_solo_cn':
        return traeSoloCnAppPath;
      case 'trae':
      default:
        return traeAppPath;
    }
  };

  const setTraeAppPathValue = (target: TraeAppPathTarget, path: string) => {
    switch (target) {
      case 'trae_solo':
        setTraeSoloAppPath(path);
        break;
      case 'trae_cn':
        setTraeCnAppPath(path);
        break;
      case 'trae_solo_cn':
        setTraeSoloCnAppPath(path);
        break;
      case 'trae':
      default:
        setTraeAppPath(path);
        break;
    }
  };

  const getTraeAppDisplayName = (target: TraeAppPathTarget) => {
    switch (target) {
      case 'trae_solo':
        return 'TRAE SOLO';
      case 'trae_cn':
        return 'Trae CN';
      case 'trae_solo_cn':
        return 'TRAE SOLO CN';
      case 'trae':
      default:
        return 'Trae';
    }
  };

  const setAppPathForTarget = (target: AppPathTarget, path: string) => {
    if (target === 'antigravity') {
      setAntigravityAppPath(path);
    } else if (target === 'codex') {
      setCodexAppPath(path);
      setCodexLaunchCandidates([]);
      setCodexAppScanError('');
    } else if (target === 'claude') {
      setClaudeAppPath(path);
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
    } else if (target === 'zcode') {
      setZcodeAppPath(path);
    } else if (isTraeAppPathTarget(target)) {
      setTraeAppPathValue(target, path);
      setTraeLaunchCandidatesTarget(target);
      setTraeLaunchCandidates([]);
    } else if (target === 'workbuddy') {
      setWorkbuddyAppPath(path);
    } else if (target === 'zed') {
      setZedAppPath(path);
    } else {
      setOpencodeAppPath(path);
    }
  };

  const getAppPathDisplayName = (target: AppPathTarget) => {
    switch (target) {
      case 'antigravity':
        return 'Antigravity';
      case 'codex':
        return 'ChatGPT / Codex';
      case 'claude':
        return 'Claude Desktop';
      case 'vscode':
        return 'Visual Studio Code';
      case 'windsurf':
        return 'Devin';
      case 'kiro':
        return 'Kiro';
      case 'cursor':
        return 'Cursor';
      case 'codebuddy':
        return 'CodeBuddy';
      case 'codebuddy_cn':
        return 'CodeBuddy CN';
      case 'qoder':
        return 'Qoder';
      case 'zcode':
        return 'ZCode';
      case 'trae':
      case 'trae_solo':
      case 'trae_cn':
      case 'trae_solo_cn':
        return getTraeAppDisplayName(target);
      case 'workbuddy':
        return 'WorkBuddy';
      case 'zed':
        return 'Zed';
      case 'opencode':
        return 'OpenCode';
    }
  };

  const getResetLabelByTarget = (target: AppPathTarget) => {
    if (isWindows) {
      return t('appPath.missing.scanApps', '检测运行中应用');
    }
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
    if (target === 'zcode') {
      return t('settings.general.codexPathReset', '重置默认');
    }
    if (isTraeAppPathTarget(target)) {
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
    if (target === 'claude') {
      return t('settings.general.codexPathReset', '重置默认');
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

  const handlePickCodexSpecifiedAppPath = async () => {
    try {
      const selected = await open({
        multiple: false,
        directory: false,
      });
      const path = Array.isArray(selected) ? selected[0] : selected;
      if (!path) return;
      setCodexSpecifiedAppPath(path);
    } catch (err) {
      console.error('选择指定应用路径失败:', err);
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
      if (isWindows) {
        const candidates = await invoke<AppLaunchCandidate[]>('scan_app_launch_targets', {
          app: target,
        });
        if (target === 'codex') {
          setCodexAppScanError('');
          setCodexLaunchCandidates(candidates);
        } else if (target === 'claude') {
          setClaudeLaunchCandidates(candidates);
        } else if (isTraeAppPathTarget(target)) {
          setTraeLaunchCandidatesTarget(target);
          setTraeLaunchCandidates(candidates);
        }

        if (candidates.length > 0) {
          if (target !== 'codex' && target !== 'claude') {
            setAppPathForTarget(target, candidates[0].target);
          }
        } else {
          const message = t(
            'appPath.missing.scanEmptyGeneric',
            '未检测到正在运行的 {{app}}，请先启动应用后重试，或手动选择路径。',
            { app: getAppPathDisplayName(target) },
          );
          if (target === 'codex') {
            setCodexAppScanError(message);
          } else {
            alert(message);
          }
        }
        return;
      }
      const detected = await invoke<string | null>('detect_app_path', { app: target, force: true });
      setAppPathForTarget(target, detected || '');
    } catch (err) {
      console.error('重置启动路径失败:', err);
      if (target === 'codex' && isWindows) {
        setCodexAppScanError(String(err));
      } else {
        setAppPathForTarget(target, '');
      }
    } finally {
      setAppPathResetDetectingTargets((prev) => {
        const next = new Set(prev);
        next.delete(target);
        return next;
      });
    }
  };

  const handleSelectClaudeLaunchCandidate = (candidate: ClaudeDesktopLaunchCandidate) => {
    setClaudeAppPath(candidate.target);
  };

  const handleSelectCodexLaunchCandidate = (candidate: AppLaunchCandidate) => {
    setCodexAppScanError('');
    setCodexAppPath(candidate.target);
  };

  const handleSelectTraeLaunchCandidate = (target: TraeAppPathTarget, candidate: AppLaunchCandidate) => {
    setTraeAppPathValue(target, candidate.target);
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

  const renderTraeAppPathRow = (
    target: TraeAppPathTarget,
    titleKey: string,
    titleDefault: string,
  ) => {
    const appPath = getTraeAppPathValue(target);
    const displayName = getTraeAppDisplayName(target);
    const showCandidates =
      isWindows && traeLaunchCandidatesTarget === target && traeLaunchCandidates.length > 0;

    return (
      <div className="settings-row" key={target}>
        <div className="row-label">
          <div className="row-title">{t(titleKey, titleDefault)}</div>
          <div className="row-desc">{t('settings.general.traeAppPathDesc', '留空则使用默认路径')}</div>
        </div>
        <div className="row-control row-control--grow settings-claude-launch-control">
          <div className="settings-claude-launch-row">
            <input
              type="text"
              className="settings-input settings-input--path"
              value={appPath}
              placeholder={t('settings.general.traeAppPathPlaceholder', '默认路径')}
              onChange={(e) => setTraeAppPathValue(target, e.target.value)}
            />
            <button
              className="btn btn-secondary"
              onClick={() => handlePickAppPath(target)}
              disabled={isAppPathResetDetecting(target)}
            >
              {t('settings.general.traePathSelect', '选择')}
            </button>
            <button
              className="btn btn-secondary"
              onClick={() => handleResetAppPath(target)}
              disabled={isAppPathResetDetecting(target)}
            >
              <RefreshCw size={16} className={isAppPathResetDetecting(target) ? 'spin' : undefined} />
              {isAppPathResetDetecting(target)
                ? t('common.loading', '加载中...')
                : getResetLabelByTarget(target)}
            </button>
          </div>
          {showCandidates ? (
            <div className="settings-claude-candidate-list">
              {traeLaunchCandidates.map((candidate) => (
                <button
                  key={`${target}:${candidate.target_type}:${candidate.target}`}
                  type="button"
                  className={`settings-claude-candidate-item${
                    appPath.trim() === candidate.target ? ' selected' : ''
                  }`}
                  onClick={() => handleSelectTraeLaunchCandidate(target, candidate)}
                >
                  <div className="settings-claude-candidate-main">
                    <span>{candidate.label || displayName}</span>
                    <span className="settings-claude-candidate-badge">EXE</span>
                  </div>
                  <div className="settings-claude-candidate-target">
                    {candidate.target}
                  </div>
                </button>
              ))}
            </div>
          ) : null}
        </div>
      </div>
    );
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
      case 'claude':
        return parseRefresh(claudeAutoRefresh) > 0;
      case 'ghcp':
        return parseRefresh(ghcpAutoRefresh) > 0;
      case 'windsurf':
        return parseRefresh(windsurfAutoRefresh) > 0;
      case 'kiro':
        return parseRefresh(kiroAutoRefresh) > 0;
      case 'cursor':
        return parseRefresh(cursorAutoRefresh) > 0;
      case 'grok':
        return parseRefresh(grokAutoRefresh) > 0;
      case 'codebuddy':
        return parseRefresh(codebuddyAutoRefresh) > 0;
      case 'codebuddy_cn':
        return parseRefresh(codebuddyCnAutoRefresh) > 0;
      case 'workbuddy':
        return parseRefresh(workbuddyAutoRefresh) > 0;
      case 'qoder':
        return parseRefresh(qoderAutoRefresh) > 0;
      case 'zcode':
        return parseRefresh(zcodeAutoRefresh) > 0;
      case 'trae':
        return parseRefresh(traeAutoRefresh) > 0;
      case 'trae_solo':
        return parseRefresh(traeSoloAutoRefresh) > 0;
      case 'trae_cn':
        return parseRefresh(traeCnAutoRefresh) > 0;
      case 'trae_solo_cn':
        return parseRefresh(traeSoloCnAutoRefresh) > 0;
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

  const getAccountsForPlatform = (
    platform: CurrentAccountRefreshPlatform,
  ): Array<{ id: string; email: string }> => {
    const getProviderAccounts = <T extends { id: string; email?: string | null }>(
      store: { getState: () => { accounts: T[] } },
      getDisplayEmail: (account: T) => string,
    ): Array<{ id: string; email: string }> =>
      store.getState().accounts.map((a) => ({
        id: a.id,
        email: getDisplayEmail(a),
      }));
    const getTraeAccounts = (target: TraeAppPathTarget) =>
      useTraeAccountStore
        .getState()
        .accounts.filter((account) => getTraeAccountPlatformId(account) === target)
        .map((account) => ({
          id: account.id,
          email: account.email || getTraeAccountDisplayEmail(account),
        }));

    switch (platform) {
      case 'antigravity':
        return antigravityAccounts.map((a) => ({ id: a.id, email: a.email }));
      case 'codex':
        return codexAccounts.map((a) => ({ id: a.id, email: a.email }));
      case 'claude':
        return getProviderAccounts(useClaudeAccountStore, getClaudeAccountDisplayEmail);
      case 'ghcp':
        return getProviderAccounts(useGitHubCopilotAccountStore, getGitHubCopilotAccountDisplayEmail);
      case 'windsurf':
        return getProviderAccounts(useWindsurfAccountStore, getWindsurfAccountDisplayEmail);
      case 'kiro':
        return getProviderAccounts(useKiroAccountStore, getKiroAccountDisplayEmail);
      case 'cursor':
        return getProviderAccounts(useCursorAccountStore, getCursorAccountDisplayEmail);
      case 'grok':
        return getProviderAccounts(useGrokAccountStore, getGrokAccountDisplayEmail);
      case 'codebuddy':
        return getProviderAccounts(useCodebuddyAccountStore, getCodebuddyAccountDisplayEmail);
      case 'codebuddy_cn':
        return getProviderAccounts(useCodebuddyCnAccountStore, getCodebuddyAccountDisplayEmail);
      case 'workbuddy':
        return getProviderAccounts(useWorkbuddyAccountStore, getWorkbuddyAccountDisplayEmail);
      case 'qoder':
        return getProviderAccounts(useQoderAccountStore, getQoderAccountDisplayEmail);
      case 'zcode':
        return getProviderAccounts(useZcodeAccountStore, getZcodeAccountDisplayEmail);
      case 'trae':
        return getTraeAccounts('trae');
      case 'trae_solo':
        return getTraeAccounts('trae_solo');
      case 'trae_cn':
        return getTraeAccounts('trae_cn');
      case 'trae_solo_cn':
        return getTraeAccounts('trae_solo_cn');
      case 'zed':
        return getProviderAccounts(useZedAccountStore, getZedAccountDisplayEmail);
      default:
        return [];
    }
  };

  const handleAccountOverrideChange = (
    platform: CurrentAccountRefreshPlatform,
    email: string,
    value: string,
  ) => {
    if (value === 'inherit') {
      removeAccountRefreshOverride(platform, email);
      setAccountLevelRefreshCustomMode((prev) => {
        const next = { ...prev };
        delete next[`${platform}:${email}`];
        return next;
      });
    } else if (value === 'custom') {
      setAccountLevelRefreshCustomMode((prev) => ({
        ...prev,
        [`${platform}:${email}`]: true,
      }));
      const currentValue = accountOverrides[platform]?.[email];
      if (currentValue !== undefined) {
        setAccountRefreshMinutes(platform, email, currentValue);
      } else {
        setAccountRefreshMinutes(platform, email, 1);
      }
    } else {
      setAccountRefreshMinutes(platform, email, Number(value));
      setAccountLevelRefreshCustomMode((prev) => {
        const next = { ...prev };
        delete next[`${platform}:${email}`];
        return next;
      });
    }
    setAccountOverrides(loadAccountRefreshOverrides());
    dispatchSettingsConfigUpdated(configUpdateSource);
  };

  const renderAccountLevelRefreshConfig = (platform: CurrentAccountRefreshPlatform) => {
    const accounts = getAccountsForPlatform(platform);
    if (accounts.length === 0) {
      return null;
    }

    const platformOverrides = accountOverrides[platform] ?? {};
    const hasAnyOverride = Object.keys(platformOverrides).length > 0;

    return (
      <div className="settings-row">
        <div className="row-label">
          <div className="row-title">
            {t('settings.general.accountLevelRefreshTitle', '账号级刷新配置')}
          </div>
          <div className="row-desc">
            {t(
              'settings.general.accountLevelRefreshDesc',
              '为不同账号设置不同的自动刷新间隔，覆盖平台级默认值。',
            )}
          </div>
        </div>
        <div className="row-control">
          <details>
            <summary style={{ cursor: 'pointer', fontSize: '13px', color: 'var(--text-secondary)' }}>
              {hasAnyOverride
                ? t('settings.general.accountLevelRefreshSummaryActive', '已配置 {{count}} 个账号', {
                    count: Object.keys(platformOverrides).length,
                  })
                : t('settings.general.accountLevelRefreshSummary', '展开配置')}
            </summary>
            <div style={{ marginTop: '8px', display: 'flex', flexDirection: 'column', gap: '6px' }}>
              {accounts.map((account) => {
                const overrideValue = platformOverrides[account.email];
                const isCustomMode = accountLevelRefreshCustomMode[`${platform}:${account.email}`];
                const isPreset = overrideValue !== undefined && [1, 2, 5, 10, 15, -1].includes(overrideValue);
                const selectValue = isCustomMode
                  ? 'custom'
                  : (overrideValue !== undefined ? String(overrideValue) : 'inherit');
                return (
                  <div
                    key={account.id}
                    style={{ display: 'flex', alignItems: 'center', gap: '8px' }}
                  >
                    <span
                      style={{
                        flex: 1,
                        fontSize: '13px',
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                        whiteSpace: 'nowrap',
                      }}
                      title={account.email}
                    >
                      {account.email}
                    </span>
                    {isCustomMode ? (
                      <div className="settings-inline-input" style={{ minWidth: '100px', width: 'auto' }}>
                        <input
                          type="number"
                          min={1}
                          max={999}
                          className="settings-select settings-select--input-mode settings-select--with-unit"
                          value={overrideValue !== undefined ? String(overrideValue) : '1'}
                          placeholder={t('quickSettings.inputMinutes', '输入分钟数')}
                          onChange={(e) => {
                            const sanitized = sanitizeNumberInput(e.target.value);
                            if (sanitized) {
                              setAccountRefreshMinutes(platform, account.email, Number(sanitized));
                              setAccountOverrides(loadAccountRefreshOverrides());
                            }
                          }}
                          onBlur={() => {
                            const currentValue = overrideValue !== undefined ? String(overrideValue) : '1';
                            const normalized = normalizeNumberInput(currentValue, 1, 999);
                            setAccountRefreshMinutes(platform, account.email, Number(normalized));
                            setAccountOverrides(loadAccountRefreshOverrides());
                            setAccountLevelRefreshCustomMode((prev) => {
                              const next = { ...prev };
                              delete next[`${platform}:${account.email}`];
                              return next;
                            });
                            dispatchSettingsConfigUpdated(configUpdateSource);
                          }}
                          onKeyDown={(event) => {
                            if (event.key === 'Enter') {
                              event.preventDefault();
                              const currentValue = overrideValue !== undefined ? String(overrideValue) : '1';
                              const normalized = normalizeNumberInput(currentValue, 1, 999);
                              setAccountRefreshMinutes(platform, account.email, Number(normalized));
                              setAccountOverrides(loadAccountRefreshOverrides());
                              setAccountLevelRefreshCustomMode((prev) => {
                                const next = { ...prev };
                                delete next[`${platform}:${account.email}`];
                                return next;
                              });
                              dispatchSettingsConfigUpdated(configUpdateSource);
                            }
                          }}
                        />
                        <span className="settings-input-unit">{t('settings.general.minutes')}</span>
                      </div>
                    ) : (
                      <select
                        className="settings-select"
                        style={{ minWidth: '100px', width: 'auto', fontSize: '12px' }}
                        value={selectValue}
                        onChange={(e) =>
                          handleAccountOverrideChange(platform, account.email, e.target.value)
                        }
                      >
                        <option value="inherit">
                          {t('settings.general.accountLevelRefreshInherit', '继承平台设置')}
                        </option>
                        <option value="-1">
                          {t('settings.general.accountLevelRefreshDisabled', '禁用')}
                        </option>
                        <option value="1">1 {t('settings.general.minutes')}</option>
                        <option value="2">2 {t('settings.general.minutes')}</option>
                        <option value="5">5 {t('settings.general.minutes')}</option>
                        <option value="10">10 {t('settings.general.minutes')}</option>
                        <option value="15">15 {t('settings.general.minutes')}</option>
                        {!isPreset && overrideValue !== undefined && overrideValue > 0 && (
                          <option value={String(overrideValue)}>
                            {overrideValue} {t('settings.general.minutes')}
                          </option>
                        )}
                        <option value="custom">{t('settings.general.autoRefreshCustom', '自定义')}</option>
                      </select>
                    )}
                  </div>
                );
              })}
            </div>
          </details>
        </div>
      </div>
    );
  };

  const renderPlatformAutoRefreshRow = ({
    title,
    description,
    value,
    setValue,
    customMode,
    setCustomMode,
    isPreset,
  }: {
    title: string;
    description: string;
    value: string;
    setValue: (value: string) => void;
    customMode: boolean;
    setCustomMode: (enabled: boolean) => void;
    isPreset: boolean;
  }) => (
    <div className="settings-row">
      <div className="row-label">
        <div className="row-title">{title}</div>
        <div className="row-desc">{description}</div>
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
                onChange={(event) => setValue(sanitizeNumberInput(event.target.value))}
                onBlur={() => {
                  setValue(normalizeNumberInput(value, 1, 999));
                  setCustomMode(false);
                }}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') {
                    event.preventDefault();
                    setValue(normalizeNumberInput(value, 1, 999));
                    setCustomMode(false);
                  }
                }}
              />
              <span className="settings-input-unit">{t('settings.general.minutes')}</span>
            </div>
          ) : (
            <select
              className="settings-select"
              style={{ minWidth: '120px', width: 'auto' }}
              value={value}
              onChange={(event) => {
                const nextValue = event.target.value;
                if (nextValue === 'custom') {
                  setCustomMode(true);
                  setValue(value !== '-1' ? value : '1');
                  return;
                }
                setCustomMode(false);
                setValue(nextValue);
              }}
            >
              {!isPreset && (
                <option value={value}>
                  {value} {t('settings.general.minutes')}
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
  );

  const renderPlatformQuotaAlertRows = ({
    enabled,
    setEnabled,
    threshold,
    setThreshold,
    customMode,
    setCustomMode,
    isPreset,
  }: {
    enabled: boolean;
    setEnabled: (enabled: boolean) => void;
    threshold: string;
    setThreshold: (value: string) => void;
    customMode: boolean;
    setCustomMode: (enabled: boolean) => void;
    isPreset: boolean;
  }) => (
    <>
      <div className="settings-row">
        <div className="row-label">
          <div className="row-title">{t('quickSettings.quotaAlert.enable', '超额预警')}</div>
          <div className="row-desc">
            {t(
              'quickSettings.quotaAlert.hint',
              '当当前账号任意模型配额低于阈值时，发送原生通知并在页面提示快捷切号。',
            )}
          </div>
        </div>
        <div className="row-control">
          <label className="switch">
            <input
              type="checkbox"
              checked={enabled}
              onChange={(event) => setEnabled(event.target.checked)}
            />
            <span className="slider"></span>
          </label>
        </div>
      </div>
      {enabled && (
        <div className="settings-row" style={{ animation: 'fadeUp 0.3s ease both' }}>
          <div className="row-label">
            <div className="row-title">{t('quickSettings.quotaAlert.threshold', '预警阈值')}</div>
            <div className="row-desc">
              {t('quickSettings.quotaAlert.thresholdDesc', '任意模型配额低于此百分比时触发预警')}
            </div>
          </div>
          <div className="row-control">
            {customMode ? (
              <div className="settings-inline-input">
                <input
                  type="number"
                  min={0}
                  max={100}
                  className="settings-select settings-select--input-mode settings-select--with-unit"
                  value={threshold}
                  placeholder={t('quickSettings.inputPercent', '输入百分比')}
                  onChange={(event) => setThreshold(sanitizeNumberInput(event.target.value))}
                  onBlur={() => {
                    setThreshold(normalizeNumberInput(threshold, 0, 100));
                    setCustomMode(false);
                  }}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter') {
                      event.preventDefault();
                      setThreshold(normalizeNumberInput(threshold, 0, 100));
                      setCustomMode(false);
                    }
                  }}
                />
                <span className="settings-input-unit">%</span>
              </div>
            ) : (
              <select
                className="settings-select"
                value={threshold}
                onChange={(event) => {
                  const nextValue = event.target.value;
                  if (nextValue === 'custom') {
                    setCustomMode(true);
                    setThreshold(threshold || '20');
                    return;
                  }
                  setCustomMode(false);
                  setThreshold(nextValue);
                }}
              >
                {!isPreset && <option value={threshold}>{threshold}%</option>}
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
    </>
  );

  const renderSessionSharingRow = (
    platform: string,
    enabled: boolean,
    setEnabled: (enabled: boolean) => void,
    fullSessionContent: boolean,
  ) => (
    <div className="settings-row">
      <div className="row-label">
        <div className="row-title">
          {t('common.sessionSharing.title', { platform })}
        </div>
        <div className="row-desc">
          {t(
            fullSessionContent
              ? 'common.sessionSharing.fullDesc'
              : 'common.sessionSharing.workspaceDesc',
            { platform },
          )}
        </div>
      </div>
      <div className="row-control">
        <label className="switch">
          <input
            type="checkbox"
            checked={enabled}
            onChange={(event) => setEnabled(event.target.checked)}
          />
          <span className="slider"></span>
        </label>
      </div>
    </div>
  );

  const renderTraeVariantSettingsGroup = ({
    target,
    order,
    titleKey,
    titleDefault,
    appPathTitleKey,
    appPathTitleDefault,
    autoRefresh,
    setAutoRefresh,
    autoRefreshCustomMode,
    setAutoRefreshCustomMode,
    autoRefreshIsPreset,
    quotaAlertEnabled,
    setQuotaAlertEnabled,
    quotaAlertThreshold,
    setQuotaAlertThreshold,
    quotaAlertThresholdCustomMode,
    setQuotaAlertThresholdCustomMode,
    quotaAlertThresholdIsPreset,
  }: {
    target: TraeAppPathTarget;
    order: number;
    titleKey: string;
    titleDefault: string;
    appPathTitleKey: string;
    appPathTitleDefault: string;
    autoRefresh: string;
    setAutoRefresh: (value: string) => void;
    autoRefreshCustomMode: boolean;
    setAutoRefreshCustomMode: (enabled: boolean) => void;
    autoRefreshIsPreset: boolean;
    quotaAlertEnabled: boolean;
    setQuotaAlertEnabled: (enabled: boolean) => void;
    quotaAlertThreshold: string;
    setQuotaAlertThreshold: (value: string) => void;
    quotaAlertThresholdCustomMode: boolean;
    setQuotaAlertThresholdCustomMode: (enabled: boolean) => void;
    quotaAlertThresholdIsPreset: boolean;
  }) => {
    const displayName = getTraeAppDisplayName(target);

    return (
      <div style={{ order }}>
        <div className="group-title">{t(titleKey, titleDefault)}</div>
        <div className="settings-group">
          {renderPlatformAutoRefreshRow({
            title: t('settings.general.platformAutoRefresh', {
              defaultValue: '{{platform}} Auto Refresh Quota',
              platform: displayName,
            }),
            description: t('settings.general.traeAutoRefreshDesc', 'Background auto-refresh interval'),
            value: autoRefresh,
            setValue: setAutoRefresh,
            customMode: autoRefreshCustomMode,
            setCustomMode: setAutoRefreshCustomMode,
            isPreset: autoRefreshIsPreset,
          })}
          {renderCurrentAccountRefreshRow(target)}
          {renderAccountLevelRefreshConfig(target)}
          {renderTraeAppPathRow(target, appPathTitleKey, appPathTitleDefault)}
          {renderPlatformQuotaAlertRows({
            enabled: quotaAlertEnabled,
            setEnabled: setQuotaAlertEnabled,
            threshold: quotaAlertThreshold,
            setThreshold: setQuotaAlertThreshold,
            customMode: quotaAlertThresholdCustomMode,
            setCustomMode: setQuotaAlertThresholdCustomMode,
            isPreset: quotaAlertThresholdIsPreset,
          })}
        </div>
      </div>
    );
  };

  const autoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(autoRefresh);
  const codexAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(codexAutoRefresh);
  const claudeAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(claudeAutoRefresh);
  const ghcpAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(ghcpAutoRefresh);
  const windsurfAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(windsurfAutoRefresh);
  const kiroAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(kiroAutoRefresh);
  const codebuddyAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(codebuddyAutoRefresh);
  const codebuddyCnAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(codebuddyCnAutoRefresh);
  const workbuddyAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(workbuddyAutoRefresh);
  const qoderAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(qoderAutoRefresh);
  const zcodeAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(zcodeAutoRefresh);
  const traeAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(traeAutoRefresh);
  const traeSoloAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(traeSoloAutoRefresh);
  const traeCnAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(traeCnAutoRefresh);
  const traeSoloCnAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(traeSoloCnAutoRefresh);
  const zedAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(zedAutoRefresh);
  const cursorAutoRefreshIsPreset = REFRESH_PRESET_VALUES.includes(cursorAutoRefresh);  const autoSwitchThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(autoSwitchThreshold);
  const autoSwitchCreditsThresholdIsPreset = CREDITS_THRESHOLD_PRESET_VALUES.includes(
    autoSwitchCreditsThreshold,
  );
  const quotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(quotaAlertThreshold);
  const codexQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(codexQuotaAlertThreshold);
  const claudeQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(claudeQuotaAlertThreshold);
  const ghcpQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(ghcpQuotaAlertThreshold);
  const windsurfQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(windsurfQuotaAlertThreshold);
  const kiroQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(kiroQuotaAlertThreshold);
  const codebuddyQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(codebuddyQuotaAlertThreshold);
  const codebuddyCnQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(codebuddyCnQuotaAlertThreshold);
  const workbuddyQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(workbuddyQuotaAlertThreshold);
  const qoderQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(qoderQuotaAlertThreshold);
  const traeQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(traeQuotaAlertThreshold);
  const traeSoloQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(traeSoloQuotaAlertThreshold);
  const traeCnQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(traeCnQuotaAlertThreshold);
  const traeSoloCnQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(traeSoloCnQuotaAlertThreshold);
  const zedQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(zedQuotaAlertThreshold);
  const cursorQuotaAlertThresholdIsPreset = THRESHOLD_PRESET_VALUES.includes(cursorQuotaAlertThreshold);
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

  const releaseHistorySections = useMemo<
    Array<{ key: ReleaseHistorySectionKey; label: string }>
  >(
    () => [
      { key: 'highlights', label: t('settings.about.releaseHistorySectionHighlights', '重要更新') },
      { key: 'added', label: t('settings.about.releaseHistorySectionAdded', '新增') },
      { key: 'changed', label: t('settings.about.releaseHistorySectionChanged', '变更') },
      { key: 'fixed', label: t('settings.about.releaseHistorySectionFixed', '修复') },
      { key: 'removed', label: t('settings.about.releaseHistorySectionRemoved', '移除') },
    ],
    [t],
  );

  const loadReleaseHistory = async () => {
    setReleaseHistoryLoading(true);
    setReleaseHistoryError('');
    try {
      const items = await invoke<ReleaseHistoryItem[]>('get_release_history', {
        locale: language,
        limit: 30,
      });
      setReleaseHistoryItems(
        Array.isArray(items)
          ? items.map((item) => ({
              ...item,
              highlights: getUpdaterReleaseHighlightLines(item.version, language),
            }))
          : [],
      );
    } catch (error) {
      console.error('加载更新记录失败:', error);
      setReleaseHistoryItems([]);
      setReleaseHistoryError(
        error instanceof Error ? error.message : String(error || ''),
      );
    } finally {
      setReleaseHistoryLoading(false);
    }
  };

  const handleOpenReleaseHistory = () => {
    setReleaseHistoryOpen(true);
    void loadReleaseHistory();
  };

  const handleCloseReleaseHistory = () => {
    setReleaseHistoryOpen(false);
  };

  useEscClose(releaseHistoryOpen, handleCloseReleaseHistory);

  const openMenuBarQuotaModal = (mode: 'enable' | 'edit') => {
    setMenuBarQuotaDraftPlatform(menuBarQuotaPlatform);
    setMenuBarQuotaDraftShowPrefix(menuBarShowAccountPrefix);
    setMenuBarQuotaModalMode(mode);
    setMenuBarQuotaModalOpen(true);
  };

  const handleCloseMenuBarQuotaModal = () => {
    setMenuBarQuotaModalOpen(false);
  };

  const handleConfirmMenuBarQuotaModal = () => {
    setMenuBarQuotaPlatform(menuBarQuotaDraftPlatform);
    setMenuBarShowAccountPrefix(menuBarQuotaDraftShowPrefix);
    setMenuBarQuotaEnabled(true);
    setMenuBarQuotaModalOpen(false);
  };

  useEscClose(menuBarQuotaModalOpen, handleCloseMenuBarQuotaModal);

  const handleDownloadReleaseVersion = async (version: string) => {
    const targetVersion = String(version || '').trim();
    if (!targetVersion) {
      return;
    }

    const releaseUrl = resolveUpdaterDownloadUrl(targetVersion);
    try {
      await openUrl(releaseUrl);
    } catch {
      window.open(releaseUrl, '_blank', 'noopener,noreferrer');
    }
  };

  const renderReleaseHistoryLine = (line: string) => {
    const parts = line.split(/\*\*(.*?)\*\*/g);
    return parts.map((part, index) =>
      index % 2 === 1 ? (
        <strong key={index}>{part}</strong>
      ) : (
        <span key={index}>{part}</span>
      ),
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

  return {
    activeTab,
    actualPort,
    antigravityAccountGroups,
    antigravityAppPath,
    antigravityDualSwitchNoRestartEnabled,
    antigravityLaunchOnSwitch,
    antigravityScopeAccounts,
    antigravityScopeTypeOptions,
    antigravitySeamlessSwitchUnlocked,
    appAutoLaunchEnabled,
    appVersion,
    autoImportFromLocalEnabled,
    autoImportScanBusy,
    autoImportScanSeqRef,
    autoImportScanStatus,
    autoInstall,
    autoInstallLoaded,
    autoInstallTouchedRef,
    autoRefresh,
    autoRefreshCustomMode,
    autoRefreshIsPreset,
    autoSwitchAccountScopeMode,
    autoSwitchCreditsEnabled,
    autoSwitchCreditsThreshold,
    autoSwitchCreditsThresholdCustomMode,
    autoSwitchCreditsThresholdIsPreset,
    autoSwitchEnabled,
    autoSwitchSelectedAccountIds,
    autoSwitchThreshold,
    autoSwitchThresholdCustomMode,
    autoSwitchThresholdIsPreset,
    claudeAppPath,
    claudeAutoRefresh,
    claudeAutoRefreshCustomMode,
    claudeAutoRefreshIsPreset,
    claudeLaunchCandidates,
    claudeQuotaAlertEnabled,
    claudeQuotaAlertThreshold,
    claudeQuotaAlertThresholdCustomMode,
    claudeQuotaAlertThresholdIsPreset,
    claudeQuotaDisplayRemaining,
    closeBehavior,
    codebuddyAppPath,
    codebuddyAutoRefresh,
    codebuddyAutoRefreshCustomMode,
    codebuddyAutoRefreshIsPreset,
    codebuddyCnAppPath,
    codebuddyCnAutoRefresh,
    codebuddyCnAutoRefreshCustomMode,
    codebuddyCnAutoRefreshIsPreset,
    codebuddyCnQuotaAlertEnabled,
    codebuddyCnQuotaAlertThreshold,
    codebuddyCnQuotaAlertThresholdCustomMode,
    codebuddyCnQuotaAlertThresholdIsPreset,
    codebuddyCnShareSessionsOnSwitch,
    codebuddyQuotaAlertEnabled,
    codebuddyQuotaAlertThreshold,
    codebuddyQuotaAlertThresholdCustomMode,
    codebuddyQuotaAlertThresholdIsPreset,
    codebuddyShareSessionsOnSwitch,
    codexAppPath,
    codexOAuthAppVersion,
    codexAppScanError,
    codexAppUiInjectionEnabled,
    codexAutoRefresh,
    codexAutoRefreshCustomMode,
    codexAutoRefreshIsPreset,
    codexAutoSwitchAccountScopeMode,
    codexAutoSwitchEnabled,
    codexAutoSwitchSelectedAccountIds,
    codexGroups,
    codexHideRelayQuota,
    codexLaunchCandidates,
    codexLaunchOnSwitch,
    codexLocalAccessEntryVisible,
    codexQuotaAlertEnabled,
    codexQuotaAlertThreshold,
    codexQuotaAlertThresholdCustomMode,
    codexQuotaAlertThresholdIsPreset,
    codexRestartSpecifiedAppOnSwitch,
    codexScopeAccounts,
    codexSpecifiedAppPath,
    codexSyncWsl,
    codexWslConfigDir,
    cursorAppPath,
    cursorAutoRefresh,
    cursorAutoRefreshCustomMode,
    cursorAutoRefreshIsPreset,
    cursorQuotaAlertEnabled,
    cursorQuotaAlertThreshold,
    cursorQuotaAlertThresholdCustomMode,
    cursorQuotaAlertThresholdIsPreset,
    defaultPort,
    defaultTerminal,
    errorReportingEnabled,
    errorReportingSaving,
    externalNetworkEnabled,
    floatingCardAlwaysOnTop,
    floatingCardShowOnStartup,
    generalLoaded,
    generalLoadFailed,
    generateReportToken,
    getResetLabelByTarget,
    ghcpAutoRefresh,
    ghcpAutoRefreshCustomMode,
    ghcpAutoRefreshIsPreset,
    ghcpQuotaAlertEnabled,
    ghcpQuotaAlertThreshold,
    ghcpQuotaAlertThresholdCustomMode,
    ghcpQuotaAlertThresholdIsPreset,
    globalProxyEnabled,
    globalProxyNoProxy,
    globalProxyUrl,
    grokAutoRefresh,
    grokCliPath,
    grokCliSaving,
    grokCliStatus,
    grokCliStatusError,
    grokOpencodeAuthOverwriteOnSwitch,
    grokOpencodeSyncOnSwitch,
    grokQuotaAlertEnabled,
    grokQuotaAlertThreshold,
    grokSyncOfficialAuthOnSwitch,
    handleAboutAvatarTap,
    handleCheckUpdate,
    handleCloseMenuBarQuotaModal,
    handleCloseReleaseHistory,
    handleConfirmMenuBarQuotaModal,
    handleDownloadReleaseVersion,
    handleErrorReportingEnabledChange,
    handleOpenReleaseHistory,
    handlePickAppPath,
    handlePickCodexSpecifiedAppPath,
    handleResetAppPath,
    handleSaveNetworkConfig,
    handleSelectClaudeLaunchCandidate,
    handleSelectCodexLaunchCandidate,
    handleSelectTraeLaunchCandidate,
    hasActiveResetTasks,
    hermesAuthOverwriteOnSwitch,
    hideDockIcon,
    isAppPathResetDetecting,
    isMacOS,
    isWindows,
    kiroAppPath,
    kiroAutoRefresh,
    kiroAutoRefreshCustomMode,
    kiroAutoRefreshIsPreset,
    kiroQuotaAlertEnabled,
    kiroQuotaAlertThreshold,
    kiroQuotaAlertThresholdCustomMode,
    kiroQuotaAlertThresholdIsPreset,
    language,
    languageOptions,
    loadGeneralConfig,
    loadUpdateSettings,
    menuBarQuotaDraftPlatform,
    menuBarQuotaDraftShowPrefix,
    menuBarQuotaEnabled,
    menuBarQuotaModalMode,
    menuBarQuotaModalOpen,
    menuBarQuotaPlatform,
    menuBarQuotaPlatformOptions,
    needsRestart,
    networkSaving,
    normalizeNumberInput,
    openclawAuthOverwriteOnSwitch,
    opencodeAppPath,
    opencodeAuthOverwriteOnSwitch,
    opencodeSyncOnSwitch,
    openLink,
    openMenuBarQuotaModal,
    platformSettingsOrder,
    qoderAppPath,
    qoderAutoRefresh,
    qoderAutoRefreshCustomMode,
    qoderAutoRefreshIsPreset,
    qoderQuotaAlertEnabled,
    qoderQuotaAlertThreshold,
    qoderQuotaAlertThresholdCustomMode,
    qoderQuotaAlertThresholdIsPreset,
    quotaAlertEnabled,
    quotaAlertThreshold,
    quotaAlertThresholdCustomMode,
    quotaAlertThresholdIsPreset,
    reducedMotionEnabled,
    REFRESH_PRESET_VALUES,
    releaseHistoryError,
    releaseHistoryItems,
    releaseHistoryLoading,
    releaseHistoryOpen,
    releaseHistorySections,
    rememberMainWindowState,
    renderAccountLevelRefreshConfig,
    renderCurrentAccountRefreshRow,
    renderPlatformAutoRefreshRow,
    renderPlatformQuotaAlertRows,
    renderReleaseHistoryLine,
    renderSessionSharingRow,
    renderTraeVariantSettingsGroup,
    reportActualPort,
    reportDefaultPort,
    reportEnabled,
    reportPort,
    reportRawPreviewUrl,
    reportRenderedPreviewUrl,
    reportToken,
    sanitizeNumberInput,
    saveGrokCliPath,
    setActiveTab,
    setAntigravityAppPath,
    setAntigravityDualSwitchNoRestartEnabled,
    setAntigravityLaunchOnSwitch,
    setAppAutoLaunchEnabled,
    setAutoImportFromLocalEnabled,
    setAutoImportScanBusy,
    setAutoImportScanStatus,
    setAutoInstall,
    setAutoRefresh,
    setAutoRefreshCustomMode,
    setAutoSwitchAccountScopeMode,
    setAutoSwitchCreditsEnabled,
    setAutoSwitchCreditsThreshold,
    setAutoSwitchCreditsThresholdCustomMode,
    setAutoSwitchEnabled,
    setAutoSwitchSelectedAccountIds,
    setAutoSwitchThreshold,
    setAutoSwitchThresholdCustomMode,
    setClaudeAppPath,
    setClaudeAutoRefresh,
    setClaudeAutoRefreshCustomMode,
    setClaudeQuotaAlertEnabled,
    setClaudeQuotaAlertThreshold,
    setClaudeQuotaAlertThresholdCustomMode,
    setClaudeQuotaDisplayRemaining,
    setCloseBehavior,
    setCodebuddyAppPath,
    setCodebuddyAutoRefresh,
    setCodebuddyAutoRefreshCustomMode,
    setCodebuddyCnAppPath,
    setCodebuddyCnAutoRefresh,
    setCodebuddyCnAutoRefreshCustomMode,
    setCodebuddyCnQuotaAlertEnabled,
    setCodebuddyCnQuotaAlertThreshold,
    setCodebuddyCnQuotaAlertThresholdCustomMode,
    setCodebuddyCnShareSessionsOnSwitch,
    setCodebuddyQuotaAlertEnabled,
    setCodebuddyQuotaAlertThreshold,
    setCodebuddyQuotaAlertThresholdCustomMode,
    setCodebuddyShareSessionsOnSwitch,
    setCodexAppPath,
    setCodexOAuthAppVersion,
    setCodexAppScanError,
    setCodexAppUiInjectionEnabled,
    setCodexAutoRefresh,
    setCodexAutoRefreshCustomMode,
    setCodexAutoSwitchAccountScopeMode,
    setCodexAutoSwitchSelectedAccountIds,
    setCodexHideRelayQuota,
    setCodexLaunchCandidates,
    setCodexLaunchOnSwitch,
    setCodexLocalAccessEntryVisible,
    setCodexQuotaAlertEnabled,
    setCodexQuotaAlertThreshold,
    setCodexQuotaAlertThresholdCustomMode,
    setCodexRestartSpecifiedAppOnSwitch,
    setCodexSpecifiedAppPath,
    setCodexSyncWsl,
    setCodexWslConfigDir,
    setCursorAppPath,
    setCursorAutoRefresh,
    setCursorAutoRefreshCustomMode,
    setCursorQuotaAlertEnabled,
    setCursorQuotaAlertThreshold,
    setCursorQuotaAlertThresholdCustomMode,
    setDefaultTerminal,
    setExternalNetworkEnabled,
    setFloatingCardAlwaysOnTop,
    setFloatingCardShowOnStartup,
    setGhcpAutoRefresh,
    setGhcpAutoRefreshCustomMode,
    setGhcpQuotaAlertEnabled,
    setGhcpQuotaAlertThreshold,
    setGhcpQuotaAlertThresholdCustomMode,
    setGlobalProxyEnabled,
    setGlobalProxyNoProxy,
    setGlobalProxyUrl,
    setGrokAutoRefresh,
    setGrokCliPath,
    setGrokCliStatusError,
    setGrokOpencodeAuthOverwriteOnSwitch,
    setGrokOpencodeSyncOnSwitch,
    setGrokQuotaAlertEnabled,
    setGrokQuotaAlertThreshold,
    setGrokSyncOfficialAuthOnSwitch,
    setHermesAuthOverwriteOnSwitch,
    setHideDockIcon,
    setKiroAppPath,
    setKiroAutoRefresh,
    setKiroAutoRefreshCustomMode,
    setKiroQuotaAlertEnabled,
    setKiroQuotaAlertThreshold,
    setKiroQuotaAlertThresholdCustomMode,
    setLanguage,
    setMenuBarQuotaDraftPlatform,
    setMenuBarQuotaDraftShowPrefix,
    setMenuBarQuotaEnabled,
    setOpenclawAuthOverwriteOnSwitch,
    setOpencodeAppPath,
    setOpencodeAuthOverwriteOnSwitch,
    setOpencodeSyncOnSwitch,
    setQoderAppPath,
    setQoderAutoRefresh,
    setQoderAutoRefreshCustomMode,
    setQoderQuotaAlertEnabled,
    setQoderQuotaAlertThreshold,
    setQoderQuotaAlertThresholdCustomMode,
    setQuotaAlertEnabled,
    setQuotaAlertThreshold,
    setQuotaAlertThresholdCustomMode,
    setReducedMotionEnabled,
    setRememberMainWindowState,
    setReportEnabled,
    setReportPort,
    setReportToken,
    setSideNavLayoutMode,
    setStartupMinimized,
    setStartupPage,
    setTheme,
    setThemeColor,
    setTokenKeeperEnabled,
    setTopRightAdVisible,
    setTraeAppPath,
    setTraeAutoRefresh,
    setTraeAutoRefreshCustomMode,
    setTraeCnAutoRefresh,
    setTraeCnAutoRefreshCustomMode,
    setTraeCnQuotaAlertEnabled,
    setTraeCnQuotaAlertThreshold,
    setTraeCnQuotaAlertThresholdCustomMode,
    setTraeQuotaAlertEnabled,
    setTraeQuotaAlertThreshold,
    setTraeQuotaAlertThresholdCustomMode,
    setTraeSoloAutoRefresh,
    setTraeSoloAutoRefreshCustomMode,
    setTraeSoloCnAutoRefresh,
    setTraeSoloCnAutoRefreshCustomMode,
    setTraeSoloCnQuotaAlertEnabled,
    setTraeSoloCnQuotaAlertThreshold,
    setTraeSoloCnQuotaAlertThresholdCustomMode,
    setTraeSoloQuotaAlertEnabled,
    setTraeSoloQuotaAlertThreshold,
    setTraeSoloQuotaAlertThresholdCustomMode,
    setTrayIconStyle,
    setUiScale,
    setUpdateRemindersEnabled,
    setVscodeAppPath,
    setWebdavAllowedDomains,
    setWindsurfAppPath,
    setWindsurfAutoRefresh,
    setWindsurfAutoRefreshCustomMode,
    setWindsurfQuotaAlertEnabled,
    setWindsurfQuotaAlertThreshold,
    setWindsurfQuotaAlertThresholdCustomMode,
    setWorkbuddyAppPath,
    setWorkbuddyAutoRefresh,
    setWorkbuddyAutoRefreshCustomMode,
    setWorkbuddyQuotaAlertEnabled,
    setWorkbuddyQuotaAlertThreshold,
    setWorkbuddyQuotaAlertThresholdCustomMode,
    setWorkbuddyShareSessionsOnSwitch,
    setWsEnabled,
    setWsPort,
    setZcodeAppPath,
    setZcodeAutoRefresh,
    setZcodeAutoRefreshCustomMode,
    setZedAppPath,
    setZedAutoRefresh,
    setZedAutoRefreshCustomMode,
    setZedQuotaAlertEnabled,
    setZedQuotaAlertThreshold,
    setZedQuotaAlertThresholdCustomMode,
    showUnlockFireworks,
    sideNavLayoutMode,
    startupMinimized,
    startupPage,
    t,
    terminalOptions,
    theme,
    themeColor,
    THRESHOLD_PRESET_VALUES,
    tokenKeeperEnabled,
    topRightAdVisible,
    traeAppPath,
    traeAutoRefresh,
    traeAutoRefreshCustomMode,
    traeAutoRefreshIsPreset,
    traeCnAutoRefresh,
    traeCnAutoRefreshCustomMode,
    traeCnAutoRefreshIsPreset,
    traeCnQuotaAlertEnabled,
    traeCnQuotaAlertThreshold,
    traeCnQuotaAlertThresholdCustomMode,
    traeCnQuotaAlertThresholdIsPreset,
    traeLaunchCandidates,
    traeLaunchCandidatesTarget,
    traeQuotaAlertEnabled,
    traeQuotaAlertThreshold,
    traeQuotaAlertThresholdCustomMode,
    traeQuotaAlertThresholdIsPreset,
    traeSoloAutoRefresh,
    traeSoloAutoRefreshCustomMode,
    traeSoloAutoRefreshIsPreset,
    traeSoloCnAutoRefresh,
    traeSoloCnAutoRefreshCustomMode,
    traeSoloCnAutoRefreshIsPreset,
    traeSoloCnQuotaAlertEnabled,
    traeSoloCnQuotaAlertThreshold,
    traeSoloCnQuotaAlertThresholdCustomMode,
    traeSoloCnQuotaAlertThresholdIsPreset,
    traeSoloQuotaAlertEnabled,
    traeSoloQuotaAlertThreshold,
    traeSoloQuotaAlertThresholdCustomMode,
    traeSoloQuotaAlertThresholdIsPreset,
    trayIconStyle,
    uiScale,
    updateChecking,
    updateCheckMessage,
    updateRemindersEnabled,
    updateRemindersLoaded,
    updateRemindersTouchedRef,
    updateSettingsLoadFailed,
    vscodeAppPath,
    webdavAllowedDomains,
    windsurfAppPath,
    windsurfAutoRefresh,
    windsurfAutoRefreshCustomMode,
    windsurfAutoRefreshIsPreset,
    windsurfQuotaAlertEnabled,
    windsurfQuotaAlertThreshold,
    windsurfQuotaAlertThresholdCustomMode,
    windsurfQuotaAlertThresholdIsPreset,
    workbuddyAppPath,
    workbuddyAutoRefresh,
    workbuddyAutoRefreshCustomMode,
    workbuddyAutoRefreshIsPreset,
    workbuddyQuotaAlertEnabled,
    workbuddyQuotaAlertThreshold,
    workbuddyQuotaAlertThresholdCustomMode,
    workbuddyQuotaAlertThresholdIsPreset,
    workbuddyShareSessionsOnSwitch,
    wsEnabled,
    wsPort,
    zcodeAppPath,
    zcodeAutoRefresh,
    zcodeAutoRefreshCustomMode,
    zcodeAutoRefreshIsPreset,
    zedAppPath,
    zedAutoRefresh,
    zedAutoRefreshCustomMode,
    zedAutoRefreshIsPreset,
    zedQuotaAlertEnabled,
    zedQuotaAlertThreshold,
    zedQuotaAlertThresholdCustomMode,
    zedQuotaAlertThresholdIsPreset,
  };
}

/** 组合业务 Controller 与独立 View，保持原组件公开调用入口不变。 */
export function SettingsPage() {
  const controller = useSettingsPageController();
  return <SettingsPageView {...controller} />;
}
