import {
  Suspense,
  lazy,
  useCallback,
  useEffect,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
} from 'react';
import './App.css';
import { getVersion } from '@tauri-apps/api/app';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { openUrl } from '@tauri-apps/plugin-opener';
import { useTranslation } from 'react-i18next';
import { FileText, FolderOpen, RefreshCw, X } from 'lucide-react';
import { SideNav } from './components/layout/SideNav';
import { GlobalModal } from './components/GlobalModal';
import type { QuickSettingsType } from './components/QuickSettingsPopover';
import { Page } from './types/navigation';
import { useAutoRefresh } from './hooks/useAutoRefresh';
import { useEasterEggTrigger } from './hooks/useEasterEggTrigger';
import { useGlobalModal } from './hooks/useGlobalModal';
import { changeLanguage, getCurrentLanguage, normalizeLanguage } from './i18n';
import { useAccountStore } from './stores/useAccountStore';
import { useCodexAccountStore } from './stores/useCodexAccountStore';
import { useGitHubCopilotAccountStore } from './stores/useGitHubCopilotAccountStore';
import { useWindsurfAccountStore } from './stores/useWindsurfAccountStore';
import { useKiroAccountStore } from './stores/useKiroAccountStore';
import { useCursorAccountStore } from './stores/useCursorAccountStore';
import { useGeminiAccountStore } from './stores/useGeminiAccountStore';
import { useCodebuddyAccountStore } from './stores/useCodebuddyAccountStore';
import { useCodebuddyCnAccountStore } from './stores/useCodebuddyCnAccountStore';
import { useQoderAccountStore } from './stores/useQoderAccountStore';
import { useTraeAccountStore } from './stores/useTraeAccountStore';
import { useWorkbuddyAccountStore } from './stores/useWorkbuddyAccountStore';
import { useZedAccountStore } from './stores/useZedAccountStore';
import { useSideNavLayoutStore } from './stores/useSideNavLayoutStore';
import { usePlatformLayoutStore } from './stores/usePlatformLayoutStore';
import { useTopRightAdStore } from './stores/useTopRightAdStore';
import type { UpdateCheckResult, UpdateInfo } from './components/UpdateNotification';
import type { Update as UpdaterUpdate } from '@tauri-apps/plugin-updater';
import { parseUpdaterReleaseNotes, resolveUpdaterDownloadUrl } from './utils/updaterReleaseNotes';
import { FloatingCardWindow } from './pages/FloatingCardWindow';
import {
  createUpdaterCanceledError,
  isRetryableUpdaterError,
  isUpdaterCanceledError,
  retryWithBackoff,
  sanitizeUpdaterErrorMessage,
  UPDATE_CHECK_RETRY_DELAYS_MS,
  UPDATE_DOWNLOAD_RETRY_DELAYS_MS,
} from './utils/updaterRetry';
import { loadWakeupOfficialLsVersionMode } from './utils/wakeupOfficialLsVersion';
import {
  dispatchExternalProviderImportEvent,
  normalizeExternalProviderImportPayload,
  type ExternalProviderImportPayload,
} from './utils/externalProviderImport';
import { runAutoBackupCycle } from './services/scheduledBackupService';
import { prepareCodexLocalAccessForRestart } from './services/codexLocalAccessService';

const DashboardPage = lazy(() =>
  import('./pages/DashboardPage').then((module) => ({ default: module.DashboardPage })),
);
const AccountsPage = lazy(() =>
  import('./pages/AccountsPage').then((module) => ({ default: module.AccountsPage })),
);
const CodexAccountsPage = lazy(() =>
  import('./pages/CodexAccountsPage').then((module) => ({ default: module.CodexAccountsPage })),
);
const GitHubCopilotAccountsPage = lazy(() =>
  import('./pages/GitHubCopilotAccountsPage').then((module) => ({
    default: module.GitHubCopilotAccountsPage,
  })),
);
const WindsurfAccountsPage = lazy(() =>
  import('./pages/WindsurfAccountsPage').then((module) => ({ default: module.WindsurfAccountsPage })),
);
const KiroAccountsPage = lazy(() =>
  import('./pages/KiroAccountsPage').then((module) => ({ default: module.KiroAccountsPage })),
);
const CursorAccountsPage = lazy(() =>
  import('./pages/CursorAccountsPage').then((module) => ({ default: module.CursorAccountsPage })),
);
const GeminiAccountsPage = lazy(() =>
  import('./pages/GeminiAccountsPage').then((module) => ({ default: module.GeminiAccountsPage })),
);
const CodebuddyAccountsPage = lazy(() =>
  import('./pages/CodebuddyAccountsPage').then((module) => ({ default: module.CodebuddyAccountsPage })),
);
const CodebuddyCnAccountsPage = lazy(() =>
  import('./pages/CodebuddyCnAccountsPage').then((module) => ({ default: module.CodebuddyCnAccountsPage })),
);
const QoderAccountsPage = lazy(() =>
  import('./pages/QoderAccountsPage').then((module) => ({ default: module.QoderAccountsPage })),
);
const TraeAccountsPage = lazy(() =>
  import('./pages/TraeAccountsPage').then((module) => ({ default: module.TraeAccountsPage })),
);
const WorkbuddyAccountsPage = lazy(() =>
  import('./pages/WorkbuddyAccountsPage').then((module) => ({ default: module.WorkbuddyAccountsPage })),
);
const ZedAccountsPage = lazy(() =>
  import('./pages/ZedAccountsPage').then((module) => ({ default: module.ZedAccountsPage })),
);
const FingerprintsPage = lazy(() =>
  import('./pages/FingerprintsPage').then((module) => ({ default: module.FingerprintsPage })),
);
const WakeupTasksPage = lazy(() =>
  import('./pages/WakeupTasksPage').then((module) => ({ default: module.WakeupTasksPage })),
);
const WakeupVerificationPage = lazy(() =>
  import('./pages/WakeupVerificationPage').then((module) => ({
    default: module.WakeupVerificationPage,
  })),
);
const SettingsPage = lazy(() =>
  import('./pages/SettingsPage').then((module) => ({ default: module.SettingsPage })),
);
const TwoFactorAuthPage = lazy(() =>
  import('./pages/TwoFactorAuthPage').then((module) => ({ default: module.TwoFactorAuthPage })),
);
const ManualPage = lazy(() =>
  import('./pages/ManualPage').then((module) => ({ default: module.ManualPage })),
);
const InstancesPage = lazy(() =>
  import('./pages/InstancesPage').then((module) => ({ default: module.InstancesPage })),
);
const PlatformLayoutModal = lazy(() =>
  import('./components/PlatformLayoutModal').then((module) => ({
    default: module.PlatformLayoutModal,
  })),
);
const UpdateNotification = lazy(() =>
  import('./components/UpdateNotification').then((module) => ({ default: module.UpdateNotification })),
);
const VersionJumpNotification = lazy(() =>
  import('./components/VersionJumpNotification').then((module) => ({ default: module.VersionJumpNotification })),
);
const CloseConfirmDialog = lazy(() =>
  import('./components/CloseConfirmDialog').then((module) => ({ default: module.CloseConfirmDialog })),
);
const BreakoutModal = lazy(() =>
  import('./components/easter-egg/BreakoutModal').then((module) => ({ default: module.BreakoutModal })),
);
const LogViewerModal = lazy(() =>
  import('./components/LogViewerModal').then((module) => ({ default: module.LogViewerModal })),
);


interface GeneralConfigTheme {
  theme: string;
  ui_scale?: number;
}

interface GeneralConfig extends GeneralConfigTheme {
  opencode_app_path: string;
  antigravity_app_path: string;
  codex_app_path: string;
  codex_launch_on_switch: boolean;
  vscode_app_path: string;
  windsurf_app_path: string;
  kiro_app_path: string;
  cursor_app_path: string;
  codebuddy_app_path: string;
  codebuddy_cn_app_path: string;
  qoder_app_path: string;
  trae_app_path: string;
  zed_app_path: string;
}

type AppPathMissingDetail = {
  app:
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
    | 'zed';
  retry?:
    | { kind: 'default' }
    | { kind: 'instance'; instanceId?: string }
    | { kind: 'switchAccount'; accountId?: string };
};

const WAKEUP_ENABLED_KEY = 'agtools.wakeup.enabled';
const TASKS_STORAGE_KEY = 'agtools.wakeup.tasks';
const WAKEUP_FORCE_DISABLE_MIGRATION_KEY = 'agtools.wakeup.migration.force_disable_0_8_14';
const TOP_RIGHT_AD_REFRESH_INTERVAL_MS = 10 * 60 * 1000;
const EXTERNAL_IMPORT_DEDUPE_WINDOW_MS = 30 * 1000;

type WakeupHistoryRecord = {
  id: string;
  timestamp: number;
  triggerType: string;
  triggerSource: string;
  taskName?: string;
  accountEmail: string;
  modelId: string;
  prompt?: string;
  success: boolean;
  message?: string;
  duration?: number;
};

type WakeupTaskResultPayload = {
  taskId: string;
  lastRunAt: number;
  records: WakeupHistoryRecord[];
};

type QuotaAlertPayload = {
  platform?: string;
  current_account_id: string;
  current_email: string;
  threshold: number;
  threshold_display?: string | null;
  lowest_percentage: number;
  low_models: string[];
  recommended_account_id?: string | null;
  recommended_email?: string | null;
  triggered_at: number;
};

type QuotaAlertPlatform =
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
type UpdateCheckSource = 'auto' | 'manual';
type UpdateActionState = 'hidden' | 'available' | 'downloading' | 'installing' | 'ready';

type UpdateRuntimeInfo = {
  platform: string;
  linux_install_kind: string;
  linux_managed_install_supported: boolean;
  updater_target?: string | null;
};

type LinuxUpdateProgressPhase =
  | 'download_started'
  | 'downloading'
  | 'downloaded'
  | 'auth_required'
  | 'installing'
  | 'completed';

type LinuxUpdateProgressPayload = {
  version: string;
  phase: LinuxUpdateProgressPhase;
  progress?: number | null;
};

type UpdateAction = {
  state: UpdateActionState;
  version: string | null;
  progress: number;
  requiresInstall: boolean;
};

function buildExternalImportDedupeKey(payload: {
  providerId: string;
  page: string;
  token: string;
  importUrl?: string | null;
  apiBaseUrl?: string | null;
  minAppVersion?: string | null;
  rawUrl?: string | null;
}): string {
  return [
    payload.providerId,
    payload.page,
    payload.rawUrl ?? '',
    payload.importUrl ?? '',
    payload.apiBaseUrl ?? '',
    payload.minAppVersion ?? '',
    payload.token,
  ].join('|');
}

function parseVersionParts(value: string | null | undefined): number[] {
  if (!value) return [];
  return value
    .trim()
    .replace(/^v/i, '')
    .split(/[^\d]+/)
    .filter(Boolean)
    .map((part) => Number.parseInt(part, 10))
    .filter((part) => Number.isFinite(part) && part >= 0);
}

function isVersionLowerThan(currentVersion: string, minimumVersion: string): boolean {
  const currentParts = parseVersionParts(currentVersion);
  const minimumParts = parseVersionParts(minimumVersion);
  if (currentParts.length === 0 || minimumParts.length === 0) {
    return false;
  }
  const maxLength = Math.max(currentParts.length, minimumParts.length);
  for (let index = 0; index < maxLength; index += 1) {
    const current = currentParts[index] ?? 0;
    const minimum = minimumParts[index] ?? 0;
    if (current < minimum) return true;
    if (current > minimum) return false;
  }
  return false;
}

function normalizeQuotaAlertPlatform(platform: string | undefined): QuotaAlertPlatform {
  switch (platform) {
    case 'codex':
      return 'codex';
    case 'github_copilot':
      return 'github_copilot';
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
    case 'zed':
      return 'zed';
    default:
      return 'antigravity';
  }
}

function getQuotaAlertPlatformLabel(
  platform: QuotaAlertPlatform,
  t: (key: string, defaultValue: string) => string,
): string {
  switch (platform) {
    case 'codex':
      return t('nav.codex', 'Codex');
    case 'github_copilot':
      return t('nav.githubCopilot', 'GitHub Copilot');
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
      return t('nav.codebuddyCn', 'CodeBuddy CN');
    case 'qoder':
      return t('nav.qoder', 'Qoder');
    case 'trae':
      return t('nav.trae', 'Trae');
    case 'zed':
      return t('nav.zed', 'Zed');
    default:
      return t('nav.overview', 'Antigravity');
  }
}

function getQuotaAlertTargetPage(platform: QuotaAlertPlatform): Page {
  switch (platform) {
    case 'codex':
      return 'codex';
    case 'github_copilot':
      return 'github-copilot';
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
      return 'codebuddy-cn';
    case 'qoder':
      return 'qoder';
    case 'trae':
      return 'trae';
    case 'workbuddy':
      return 'workbuddy';
    case 'zed':
      return 'zed';
    default:
      return 'overview';
  }
}

function getQuotaAlertQuickSettingsType(platform: QuotaAlertPlatform): QuickSettingsType {
  switch (platform) {
    case 'codex':
      return 'codex';
    case 'github_copilot':
      return 'github_copilot';
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
    default:
      return 'antigravity';
  }
}

function isElementVisible(element: HTMLElement): boolean {
  return element.getClientRects().length > 0;
}

function triggerPageRefreshButton(): boolean {
  const buttons = Array.from(
    document.querySelectorAll<HTMLButtonElement>('button.btn.btn-secondary.icon-only:not(:disabled)'),
  );

  const target = buttons.find((button) => {
    if (!isElementVisible(button)) {
      return false;
    }
    return !!button.querySelector('svg.lucide-refresh-cw');
  });

  if (!target) {
    return false;
  }

  target.click();
  return true;
}

function isWindowsPlatform(): boolean {
  const navWithUAData = navigator as Navigator & { userAgentData?: { platform?: string } };
  const platform = navWithUAData.userAgentData?.platform || navigator.platform || '';
  return platform.toLowerCase().includes('win');
}

function MainApp() {
  const { t } = useTranslation();
  const sideNavLayoutMode = useSideNavLayoutStore((state) => state.mode);
  const sideNavClassicCollapsed = useSideNavLayoutStore((state) => state.classicCollapsed);
  const sideNavClassicFirstSyncDone = useSideNavLayoutStore((state) => state.classicFirstSyncDone);
  const markSideNavClassicFirstSyncDone = useSideNavLayoutStore((state) => state.markClassicFirstSyncDone);
  const syncSidebarEntriesFromDashboard = usePlatformLayoutStore((state) => state.syncSidebarEntriesFromDashboard);
  const [page, setPage] = useState<Page>('dashboard');
  const [showUpdateNotification, setShowUpdateNotification] = useState(false);
  const [updateNotificationKey, setUpdateNotificationKey] = useState(0);
  const [showCloseDialog, setShowCloseDialog] = useState(false);
  const [showLogViewer, setShowLogViewer] = useState(false);
  const [showPlatformLayoutModal, setShowPlatformLayoutModal] = useState(false);
  const [platformLayoutRequestedGroupId, setPlatformLayoutRequestedGroupId] = useState<string | null>(null);
  const [showBreakout, setShowBreakout] = useState(false);
  const [hasBreakoutSession, setHasBreakoutSession] = useState(false);
  const [appPathMissing, setAppPathMissing] = useState<AppPathMissingDetail | null>(null);
  const [appPathSetting, setAppPathSetting] = useState(false);
  const [appPathDetecting, setAppPathDetecting] = useState(false);
  const [appPathDraft, setAppPathDraft] = useState('');
  const [appPathActionError, setAppPathActionError] = useState('');
  const [appPathCodexLaunchOnSwitch, setAppPathCodexLaunchOnSwitch] = useState(true);
  const [appPathCodexLaunchSetting, setAppPathCodexLaunchSetting] = useState(false);
  const [versionJumpInfo, setVersionJumpInfo] = useState<{
    previous_version: string;
    current_version: string;
    release_notes: string;
    release_notes_zh: string;
  } | null>(null);
  const [updateRuntimeInfo, setUpdateRuntimeInfo] = useState<UpdateRuntimeInfo | null>(null);
  const [updateRuntimeInfoLoaded, setUpdateRuntimeInfoLoaded] = useState(false);
  const [updateNotificationInfo, setUpdateNotificationInfo] = useState<UpdateInfo | null>(null);
  const [updateNotificationChecking, setUpdateNotificationChecking] = useState(false);
  const [updateRemindersEnabled, setUpdateRemindersEnabled] = useState(true);
  const [updateSkipError, setUpdateSkipError] = useState('');
  const [silentUpdateVersion, setSilentUpdateVersion] = useState<string | null>(null);
  const [updateAction, setUpdateAction] = useState<UpdateAction>({
    state: 'hidden',
    version: null,
    progress: 0,
    requiresInstall: true,
  });
  const [updateRetryStatus, setUpdateRetryStatus] = useState('');
  const [updateDownloadError, setUpdateDownloadError] = useState('');
  const [updateErrorDetails, setUpdateErrorDetails] = useState('');
  const pendingSilentUpdateRef = useRef<UpdaterUpdate | null>(null);
  const activeUpdateDownloadRef = useRef<UpdaterUpdate | null>(null);
  const updateCancelRequestedRef = useRef(false);
  const updateDownloadTaskIdRef = useRef(0);
  const updateDownloadOwnerRef = useRef<'none' | 'shared' | 'silent'>('none');
  const updateCheckRequestIdRef = useRef(0);
  const externalImportHandledAtRef = useRef<Map<string, number>>(new Map());
  const { showModal, closeModal } = useGlobalModal();
  const topRightAdState = useTopRightAdStore((state) => state.state);
  const fetchTopRightAdState = useTopRightAdStore((state) => state.fetchState);
  const trayRefreshInFlightRef = useRef(false);
  const openPlatformLayoutModal = useCallback(() => {
    setPlatformLayoutRequestedGroupId(null);
    setShowPlatformLayoutModal(true);
  }, []);
  const handleTopRightAdClick = useCallback(async () => {
    const target = topRightAdState.ad?.ctaUrl?.trim();
    if (!target || !/^https?:\/\//i.test(target)) {
      return;
    }
    try {
      await openUrl(target);
    } catch {
      window.open(target, '_blank', 'noopener,noreferrer');
    }
  }, [topRightAdState.ad?.ctaUrl]);
  const openBreakout = useCallback(() => {
    setHasBreakoutSession(true);
    setShowBreakout(true);
  }, []);
  const ensureExternalImportVersionCompatible = useCallback(
    async (payload: ExternalProviderImportPayload): Promise<boolean> => {
      const requiredVersion = payload.minAppVersion?.trim().replace(/^v/i, '');
      if (!requiredVersion) return true;

      let currentVersion = '';
      try {
        currentVersion = await getVersion();
      } catch (error) {
        console.warn('[ExternalImport][App] 读取当前应用版本失败，已终止外部导入', error);
      }

      if (currentVersion && !isVersionLowerThan(currentVersion, requiredVersion)) {
        return true;
      }

      showModal({
        title: t('common.shared.externalImport.versionUnsupportedTitle', '应用版本过低'),
        description: t(
          'common.shared.externalImport.versionUnsupportedDesc',
          '暂不支持此方式，请下载最新版。',
        ),
        width: 'sm',
        actions: [
          {
            id: 'check-update',
            label: t('common.shared.externalImport.checkUpdate', '检查更新'),
            variant: 'primary',
            onClick: () => {
              window.dispatchEvent(
                new CustomEvent('update-check-requested', {
                  detail: { source: 'manual' satisfies UpdateCheckSource },
                }),
              );
            },
          },
          {
            id: 'close',
            label: t('common.close', '关闭'),
            variant: 'secondary',
          },
        ],
      });
      console.warn('[ExternalImport][App] 当前版本不支持外部导入方式，已终止导入', {
        currentVersion: currentVersion || null,
        requiredVersion,
        providerId: payload.providerId,
      });
      return false;
    },
    [showModal, t],
  );

  const handleExternalProviderImportRawPayload = useCallback(async (rawPayload: unknown) => {
    console.info('[ExternalImport][App] 收到原始 payload:', rawPayload);
    const normalized = normalizeExternalProviderImportPayload(rawPayload);
    if (!normalized) {
      console.warn('[ExternalImport][App] payload 归一化失败，已忽略');
      return;
    }
    if (!(await ensureExternalImportVersionCompatible(normalized))) {
      return;
    }
    const now = Date.now();
    for (const [key, handledAt] of externalImportHandledAtRef.current) {
      if (now - handledAt > EXTERNAL_IMPORT_DEDUPE_WINDOW_MS) {
        externalImportHandledAtRef.current.delete(key);
      }
    }
    const dedupeKey = buildExternalImportDedupeKey(normalized);
    if (externalImportHandledAtRef.current.has(dedupeKey)) {
      console.info('[ExternalImport][App] 重复外部导入 payload 已忽略');
      return;
    }
    externalImportHandledAtRef.current.set(dedupeKey, now);
    console.info('[ExternalImport][App] payload 归一化成功:', {
      providerId: normalized.providerId,
      page: normalized.page,
      autoImport: normalized.autoImport,
      tokenLength: normalized.token.length,
      hasImportUrl: Boolean(normalized.importUrl),
      apiBaseUrl: normalized.apiBaseUrl ?? null,
      minAppVersion: normalized.minAppVersion ?? null,
      source: normalized.source ?? null,
    });
    setPage(normalized.page);
    window.setTimeout(() => {
      console.info('[ExternalImport][App] 分发前端外部导入事件');
      dispatchExternalProviderImportEvent(normalized);
    }, 0);
  }, [ensureExternalImportVersionCompatible]);
  const handleBreakoutMinimize = useCallback(() => {
    setShowBreakout(false);
  }, []);
  const handleBreakoutTerminate = useCallback(() => {
    setShowBreakout(false);
    setHasBreakoutSession(false);
  }, []);
  const handleResumeBreakout = useCallback(() => {
    if (!hasBreakoutSession) return;
    setShowBreakout(true);
  }, [hasBreakoutSession]);

  const {
    count: easterEggClickCount,
    registerClick: handleEasterEggTriggerClick,
    reset: resetEasterEggTrigger,
  } = useEasterEggTrigger({
    threshold: 20,
    windowMs: 8000,
    onTrigger: openBreakout,
  });
  const handleBreakoutEntryTriggerClick = useCallback(() => {
    if (hasBreakoutSession) {
      resetEasterEggTrigger();
      handleResumeBreakout();
      return;
    }
    handleEasterEggTriggerClick();
  }, [handleEasterEggTriggerClick, handleResumeBreakout, hasBreakoutSession, resetEasterEggTrigger]);
  
  // 启用自动刷新 hook
  useAutoRefresh();

  useEffect(() => {
    const handleRefreshShortcut = (event: KeyboardEvent) => {
      const isRefreshKey = event.key.toLowerCase() === 'r';
      const isWindowsF5 = isWindowsPlatform() && event.key === 'F5';
      const hasMainModifier = event.metaKey || event.ctrlKey;
      const matchMainRefresh = isRefreshKey && hasMainModifier && !event.altKey && !event.shiftKey;
      const matchWindowsRefresh = isWindowsF5 && !event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey;
      if ((!matchMainRefresh && !matchWindowsRefresh) || event.repeat) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      triggerPageRefreshButton();
    };

    window.addEventListener('keydown', handleRefreshShortcut, true);
    return () => {
      window.removeEventListener('keydown', handleRefreshShortcut, true);
    };
  }, []);

  useEffect(() => {
    void fetchTopRightAdState();
  }, [fetchTopRightAdState]);

  useEffect(() => {
    const intervalId = window.setInterval(() => {
      void fetchTopRightAdState();
    }, TOP_RIGHT_AD_REFRESH_INTERVAL_MS);
    return () => {
      window.clearInterval(intervalId);
    };
  }, [fetchTopRightAdState]);

  useEffect(() => {
    const handleLanguageChanged = () => {
      void fetchTopRightAdState();
    };
    window.addEventListener('general-language-updated', handleLanguageChanged);
    return () => {
      window.removeEventListener('general-language-updated', handleLanguageChanged);
    };
  }, [fetchTopRightAdState]);

  useEffect(() => {
    if (sideNavLayoutMode !== 'classic' || sideNavClassicFirstSyncDone) {
      return;
    }
    syncSidebarEntriesFromDashboard();
    markSideNavClassicFirstSyncDone();
  }, [
    sideNavLayoutMode,
    sideNavClassicFirstSyncDone,
    syncSidebarEntriesFromDashboard,
    markSideNavClassicFirstSyncDone,
  ]);

  const openUpdateNotification = useCallback((source: UpdateCheckSource) => {
    if (source === 'manual') {
      window.dispatchEvent(new CustomEvent('update-check-started', { detail: { source } }));
    }
    setUpdateSkipError('');
    setUpdateNotificationKey(Date.now());
    setShowUpdateNotification(true);
  }, []);

  const closeUpdateNotification = useCallback(() => {
    setShowUpdateNotification(false);
    setUpdateSkipError('');
    if (updateAction.state === 'hidden') {
      setUpdateNotificationInfo(null);
      setUpdateRetryStatus('');
      setUpdateDownloadError('');
      setUpdateErrorDetails('');
    }
  }, [updateAction.state]);

  const writeUpdateLog = useCallback((level: 'info' | 'warn' | 'error', message: string) => {
    void invoke('update_log', { level, message }).catch(() => {});
  }, []);

  const prepareCodexLocalAccessBeforeRelaunch = useCallback(async () => {
    setUpdateRetryStatus(
      t('update_notification.stoppingApiService', '正在关闭 API 服务...'),
    );
    try {
      const state = await prepareCodexLocalAccessForRestart();
      writeUpdateLog(
        'info',
        `应用重启前已关闭 Codex API 服务监听: enabled=${Boolean(state.collection?.enabled)}, running=${state.running}`,
      );
    } catch (error) {
      writeUpdateLog(
        'warn',
        `应用重启前关闭 Codex API 服务监听失败，已中止本次重启: error=${sanitizeUpdaterErrorMessage(error)}`,
      );
      throw error;
    }
  }, [t, writeUpdateLog]);

  const restoreCodexLocalAccessAfterRelaunchFailure = useCallback(async () => {
    await invoke('codex_local_access_get_state').catch((error) => {
      writeUpdateLog(
        'warn',
        `应用重启失败后恢复 Codex API 服务状态失败: error=${sanitizeUpdaterErrorMessage(error)}`,
      );
    });
  }, [writeUpdateLog]);

  const prepareUpdateNotificationInfo = useCallback(async (update: UpdaterUpdate): Promise<UpdateInfo> => {
    const { releaseNotes, releaseNotesZh } = parseUpdaterReleaseNotes(update.body);
    const currentVersion = update.currentVersion || (await getVersion());
    return {
      current_version: currentVersion,
      latest_version: update.version,
      download_url: resolveUpdaterDownloadUrl(update.version, update.rawJson),
      release_notes: releaseNotes,
      release_notes_zh: releaseNotesZh,
    };
  }, []);

  const handleUpdateCheckResult = useCallback((result: UpdateCheckResult) => {
    const latestVersion = result.latestVersion;
    if (result.status === 'has_update' && latestVersion) {
      setUpdateAction((prev) => {
        if (prev.state === 'downloading' && prev.version === latestVersion) {
          return prev;
        }
        if (prev.state === 'installing' && prev.version === latestVersion) {
          return prev;
        }
        if (prev.state === 'ready' && prev.version === latestVersion) {
          return prev;
        }
        return {
          state: 'available',
          version: latestVersion,
          progress: 0,
          requiresInstall: true,
        };
      });
      setUpdateRetryStatus('');
    } else if (result.status === 'up_to_date') {
      setUpdateAction((prev) => {
        if (prev.state === 'ready' || prev.state === 'downloading' || prev.state === 'installing') {
          return prev;
        }
        return {
          state: 'hidden',
          version: null,
          progress: 0,
          requiresInstall: true,
        };
      });
      setUpdateRetryStatus('');
      setUpdateDownloadError('');
      setUpdateErrorDetails('');
    }

    if (result.source === 'manual') {
      window.dispatchEvent(new CustomEvent('update-check-finished', { detail: result }));
    }
  }, []);

  useEffect(() => {
    let cancelled = false;

    invoke<UpdateRuntimeInfo>('get_update_runtime_info')
      .then((info) => {
        if (cancelled) {
          return;
        }
        setUpdateRuntimeInfo(info);
      })
      .catch((error) => {
        if (cancelled) {
          return;
        }
        console.error('[App] Failed to load update runtime info:', error);
        writeUpdateLog('warn', `加载更新运行时信息失败: error=${sanitizeUpdaterErrorMessage(error)}`);
      })
      .finally(() => {
        if (!cancelled) {
          setUpdateRuntimeInfoLoaded(true);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [writeUpdateLog]);

  useEffect(() => {
    let cancelled = false;
    invoke<{
      auto_check?: boolean;
      check_interval_hours?: number;
      auto_install?: boolean;
      last_run_version?: string;
      remind_on_update?: boolean;
      skipped_version?: string;
    }>('get_update_settings')
      .then((settings) => {
        if (cancelled) {
          return;
        }
        setUpdateRemindersEnabled(settings?.remind_on_update ?? true);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    const handleUpdateReminderChanged = (event: Event) => {
      const detail = (event as CustomEvent<{ enabled?: boolean }>).detail;
      if (typeof detail?.enabled === 'boolean') {
        setUpdateRemindersEnabled(detail.enabled);
      }
    };
    window.addEventListener('update-reminder-changed', handleUpdateReminderChanged as EventListener);
    return () => {
      window.removeEventListener('update-reminder-changed', handleUpdateReminderChanged as EventListener);
    };
  }, []);

  const isLinuxManagedUpdate = updateRuntimeInfo?.platform === 'linux'
    && updateRuntimeInfo.linux_managed_install_supported;

  const getUpdaterCheckTarget = useCallback((): string | undefined => {
    if (updateRuntimeInfo?.platform !== 'windows') {
      return undefined;
    }
    if (typeof updateRuntimeInfo.updater_target !== 'string') {
      return undefined;
    }

    const target = updateRuntimeInfo.updater_target.trim();
    return target.length > 0 ? target : undefined;
  }, [updateRuntimeInfo]);

  const runUpdaterCheck = useCallback(async () => {
    const { check } = await import('@tauri-apps/plugin-updater');
    const target = getUpdaterCheckTarget();
    return target ? check({ target }) : check();
  }, [getUpdaterCheckTarget]);

  const closeUpdaterHandle = useCallback(async (handle: UpdaterUpdate | null | undefined) => {
    if (!handle) {
      return;
    }
    await handle.close().catch(() => {});
  }, []);

  const runModalUpdateCheck = useCallback(async (source: UpdateCheckSource) => {
    const requestId = Date.now();
    updateCheckRequestIdRef.current = requestId;
    setUpdateNotificationInfo(null);
    setUpdateNotificationChecking(true);
    setUpdateRetryStatus('');
    setUpdateDownloadError('');
    setUpdateErrorDetails('');
    openUpdateNotification(source);

    try {
      const update = await retryWithBackoff(
        async () => runUpdaterCheck(),
        {
          delaysMs: UPDATE_CHECK_RETRY_DELAYS_MS,
          shouldRetry: isRetryableUpdaterError,
          onRetry: ({ retryIndex, totalRetries, delayMs, error }) => {
            if (updateCheckRequestIdRef.current !== requestId) {
              return;
            }
            const compactError = sanitizeUpdaterErrorMessage(error);
            setUpdateRetryStatus(
              t('update_notification.checkRetrying', {
                attempt: retryIndex,
                total: totalRetries,
              }),
            );
            writeUpdateLog(
              'warn',
              `交互式更新检查失败，准备重试(${retryIndex}/${totalRetries})，delay=${delayMs}ms，error=${compactError}`,
            );
          },
        },
      );

      if (updateCheckRequestIdRef.current !== requestId) {
        await closeUpdaterHandle(update);
        return;
      }

      setUpdateRetryStatus('');
      if (update) {
        const info = await prepareUpdateNotificationInfo(update);
        if (updateCheckRequestIdRef.current !== requestId) {
          await closeUpdaterHandle(update);
          return;
        }
        setUpdateNotificationInfo(info);
        handleUpdateCheckResult({
          source,
          status: 'has_update',
          currentVersion: info.current_version,
          latestVersion: info.latest_version,
        });
        await closeUpdaterHandle(update);
        return;
      }

      const currentVersion = await getVersion();
      if (updateCheckRequestIdRef.current !== requestId) {
        return;
      }
      setShowUpdateNotification(false);
      setUpdateNotificationInfo(null);
      handleUpdateCheckResult({
        source,
        status: 'up_to_date',
        currentVersion,
        latestVersion: currentVersion,
      });
    } catch (error) {
      if (updateCheckRequestIdRef.current !== requestId) {
        return;
      }
      console.error('[App] Interactive update check failed:', error);
      writeUpdateLog(
        'warn',
        `交互式更新检查失败，关闭弹窗: error=${sanitizeUpdaterErrorMessage(error)}`,
      );
      setShowUpdateNotification(false);
      setUpdateNotificationInfo(null);
      handleUpdateCheckResult({
        source,
        status: 'failed',
        error: String(error),
      });
    } finally {
      if (updateCheckRequestIdRef.current === requestId) {
        setUpdateNotificationChecking(false);
      }
    }
  }, [
    closeUpdaterHandle,
    handleUpdateCheckResult,
    openUpdateNotification,
    prepareUpdateNotificationInfo,
    runUpdaterCheck,
    t,
    writeUpdateLog,
  ]);

  const handleApplyPendingUpdate = useCallback(async () => {
    const targetVersion = updateAction.version || silentUpdateVersion || '';
    const shouldInstall = updateAction.state === 'ready'
      ? updateAction.requiresInstall
      : Boolean(pendingSilentUpdateRef.current);
    let failureStage: 'prepare' | 'install' | 'relaunch' = 'prepare';
    try {
      writeUpdateLog(
        'info',
        `用户点击立即重启应用更新: version=${targetVersion || 'unknown'}, install_before_restart=${shouldInstall}`,
      );
      setUpdateRetryStatus(
        t('update_notification.stoppingApiService', '正在关闭 API 服务...'),
      );
      setUpdateDownloadError('');
      setUpdateErrorDetails('');
      await prepareCodexLocalAccessBeforeRelaunch();
      failureStage = 'install';
      const pendingUpdate = pendingSilentUpdateRef.current;
      if (shouldInstall && pendingUpdate) {
        await pendingUpdate.install();
      }
      if (pendingUpdate) {
        await pendingUpdate.close().catch(() => {});
        pendingSilentUpdateRef.current = null;
      }
      setSilentUpdateVersion(null);
      setUpdateRetryStatus('');
      setUpdateDownloadError('');
      setUpdateErrorDetails('');
      setUpdateAction({
        state: 'ready',
        version: targetVersion || null,
        progress: 100,
        requiresInstall: false,
      });
      failureStage = 'relaunch';
      const { relaunch } = await import('@tauri-apps/plugin-process');
      await relaunch();
    } catch (error) {
      await restoreCodexLocalAccessAfterRelaunchFailure();
      console.error('[App] Failed to apply pending update:', error);
      const compactError = sanitizeUpdaterErrorMessage(error);
      const errorMessage = failureStage === 'prepare'
        ? t('update_notification.stopApiServiceFailed', '无法关闭 API 服务，请先停用后重试。')
        : failureStage === 'install'
          ? t('update_notification.installFailed', '系统安装失败，请稍后重试或手动下载安装。')
          : t('update_notification.restartRequiredAfterInstall', '更新已安装，请手动重启应用完成切换。');
      setUpdateRetryStatus('');
      setUpdateDownloadError(errorMessage);
      setUpdateErrorDetails(compactError);
      writeUpdateLog(
        'error',
        `用户手动应用更新失败: stage=${failureStage}, error=${compactError}`,
      );
      throw error;
    }
  }, [
    prepareCodexLocalAccessBeforeRelaunch,
    restoreCodexLocalAccessAfterRelaunchFailure,
    silentUpdateVersion,
    updateAction,
    t,
    writeUpdateLog,
  ]);

  const runLinuxManagedUpdate = useCallback(async (expectedVersion: string) => {
    setUpdateRetryStatus('');
    setUpdateDownloadError('');
    setUpdateErrorDetails('');
    setSilentUpdateVersion(null);
    setUpdateAction({
      state: 'downloading',
      version: expectedVersion,
      progress: 0,
      requiresInstall: false,
    });

    if (pendingSilentUpdateRef.current) {
      await closeUpdaterHandle(pendingSilentUpdateRef.current);
      pendingSilentUpdateRef.current = null;
    }

    writeUpdateLog('info', `Linux 托管更新开始执行: version=${expectedVersion}`);

    try {
      await invoke('install_linux_update', {
        expectedVersion,
      });

      setUpdateAction({
        state: 'ready',
        version: expectedVersion,
        progress: 100,
        requiresInstall: false,
      });
      setUpdateRetryStatus(t('update_notification.installSuccess', '更新已安装，正在重启...'));
      setUpdateDownloadError('');
      setUpdateErrorDetails('');

      let relaunchStage: 'prepare' | 'relaunch' = 'prepare';
      try {
        await prepareCodexLocalAccessBeforeRelaunch();
        relaunchStage = 'relaunch';
        const { relaunch } = await import('@tauri-apps/plugin-process');
        await relaunch();
      } catch (error) {
        await restoreCodexLocalAccessAfterRelaunchFailure();
        const compactError = sanitizeUpdaterErrorMessage(error);
        console.error('[App] Linux managed update installed but relaunch failed:', error);
        writeUpdateLog(
          'error',
          `Linux 托管更新安装完成但重启失败: version=${expectedVersion}, error=${compactError}`,
        );
        setUpdateRetryStatus('');
        setUpdateDownloadError(
          relaunchStage === 'prepare'
            ? t('update_notification.stopApiServiceFailed', '无法关闭 API 服务，请先停用后重试。')
            : t('update_notification.restartRequiredAfterInstall', '更新已安装，请手动重启应用完成切换。'),
        );
        setUpdateErrorDetails(compactError);
      }
    } catch (error) {
      console.error('[App] Linux managed update failed:', error);
      const compactError = sanitizeUpdaterErrorMessage(error);
      writeUpdateLog('error', `Linux 托管更新失败: version=${expectedVersion}, error=${compactError}`);
      setUpdateRetryStatus('');
      setUpdateDownloadError(
        t('update_notification.installFailed', '系统安装失败，请稍后重试或手动下载安装。'),
      );
      setUpdateErrorDetails(compactError);
      setUpdateAction({
        state: 'available',
        version: expectedVersion,
        progress: 0,
        requiresInstall: true,
      });
      throw error;
    }
  }, [
    closeUpdaterHandle,
    prepareCodexLocalAccessBeforeRelaunch,
    restoreCodexLocalAccessAfterRelaunchFailure,
    t,
    writeUpdateLog,
  ]);

  const runSharedUpdateDownload = useCallback(async (expectedVersion: string) => {
    const taskId = Date.now();
    updateDownloadTaskIdRef.current = taskId;
    updateCancelRequestedRef.current = false;
    updateDownloadOwnerRef.current = 'shared';
    setUpdateRetryStatus('');
    setUpdateDownloadError('');
    setUpdateErrorDetails('');
    setUpdateAction({
      state: 'downloading',
      version: expectedVersion,
      progress: 0,
      requiresInstall: true,
    });
    writeUpdateLog('info', `统一更新任务开始下载: version=${expectedVersion}`);

    let usedAttempts = 0;
    try {
      const downloadedUpdate = await retryWithBackoff(
        async (attempt) => {
          usedAttempts = attempt;
          if (updateCancelRequestedRef.current || updateDownloadTaskIdRef.current !== taskId) {
            throw createUpdaterCanceledError();
          }

          let candidate: UpdaterUpdate | null = null;
          try {
            candidate = await runUpdaterCheck();
            if (!candidate) {
              throw new Error('No update available from updater plugin');
            }
            activeUpdateDownloadRef.current = candidate;

            const candidateVersion = candidate.version;
            const { releaseNotes, releaseNotesZh } = parseUpdaterReleaseNotes(candidate.body);
            await invoke('save_pending_update_notes', {
              version: candidateVersion,
              releaseNotes,
              releaseNotesZh,
            }).catch((error) => {
              console.error('[App] Failed to cache shared update notes:', error);
              writeUpdateLog(
                'warn',
                `缓存统一更新说明失败: version=${candidateVersion}, error=${sanitizeUpdaterErrorMessage(error)}`,
              );
            });

            let downloaded = 0;
            let contentLength = 0;
            await candidate.download((event) => {
              if (updateCancelRequestedRef.current || updateDownloadTaskIdRef.current !== taskId) {
                throw createUpdaterCanceledError();
              }
              setUpdateAction((prev) => {
                if (prev.state !== 'downloading') {
                  return prev;
                }

                if (event.event === 'Started') {
                  contentLength = event.data.contentLength ?? 0;
                  return {
                    ...prev,
                    version: candidateVersion,
                    progress: 0,
                  };
                }

                if (event.event === 'Progress') {
                  downloaded += event.data.chunkLength;
                  const nextProgress = contentLength > 0
                    ? Math.min(100, Math.round((downloaded / contentLength) * 100))
                    : Math.min(95, prev.progress + 1);
                  return {
                    ...prev,
                    version: candidateVersion,
                    progress: nextProgress,
                  };
                }

                return {
                  ...prev,
                  version: candidateVersion,
                  progress: 100,
                };
              });
            });

            if (updateCancelRequestedRef.current || updateDownloadTaskIdRef.current !== taskId) {
              throw createUpdaterCanceledError();
            }

            return candidate;
          } catch (error) {
            if (candidate) {
              await closeUpdaterHandle(candidate);
            }
            if (activeUpdateDownloadRef.current === candidate) {
              activeUpdateDownloadRef.current = null;
            }
            throw error;
          }
        },
        {
          delaysMs: UPDATE_DOWNLOAD_RETRY_DELAYS_MS,
          shouldRetry: isRetryableUpdaterError,
          onRetry: ({ retryIndex, totalRetries, delayMs, error }) => {
            const compactError = sanitizeUpdaterErrorMessage(error);
            setUpdateRetryStatus(
              t('update_notification.downloadRetrying', {
                attempt: retryIndex,
                total: totalRetries,
              }),
            );
            writeUpdateLog(
              'warn',
              `统一更新下载失败，准备重试(${retryIndex}/${totalRetries})，delay=${delayMs}ms，error=${compactError}`,
            );
            setUpdateAction((prev) => {
              if (prev.state !== 'downloading') {
                return prev;
              }
              return {
                ...prev,
                progress: 0,
              };
            });
          },
        },
      );

      if (updateCancelRequestedRef.current || updateDownloadTaskIdRef.current !== taskId) {
        await closeUpdaterHandle(downloadedUpdate);
        return;
      }

      if (pendingSilentUpdateRef.current) {
        await closeUpdaterHandle(pendingSilentUpdateRef.current);
      }
      pendingSilentUpdateRef.current = downloadedUpdate;
      activeUpdateDownloadRef.current = null;
      setSilentUpdateVersion(downloadedUpdate.version);
      setUpdateRetryStatus('');
      setUpdateDownloadError('');
      setUpdateErrorDetails('');
      setUpdateAction({
        state: 'ready',
        version: downloadedUpdate.version,
        progress: 100,
        requiresInstall: true,
      });
      writeUpdateLog('info', `统一更新下载完成，等待重启安装: version=${downloadedUpdate.version}`);
    } catch (error) {
      if (isUpdaterCanceledError(error) || updateCancelRequestedRef.current || updateDownloadTaskIdRef.current !== taskId) {
        writeUpdateLog('info', `统一更新下载已取消: version=${expectedVersion}`);
        setUpdateRetryStatus(t('update_notification.updateCancelled', '已取消更新'));
        setUpdateDownloadError('');
        setUpdateErrorDetails('');
        return;
      }

      console.error('[App] Shared update download failed:', error);
      writeUpdateLog('error', `统一更新下载失败: error=${sanitizeUpdaterErrorMessage(error)}`);
      setUpdateRetryStatus('');
      setUpdateDownloadError(
        t('update_notification.autoUpdateFailedAfterRetries', {
          count: Math.max(usedAttempts, 1),
        }),
      );
      setUpdateErrorDetails(sanitizeUpdaterErrorMessage(error));
      setUpdateAction({
        state: 'available',
        version: expectedVersion,
        progress: 0,
        requiresInstall: true,
      });
      throw error;
    } finally {
      if (updateDownloadTaskIdRef.current === taskId && updateDownloadOwnerRef.current === 'shared') {
        updateDownloadOwnerRef.current = 'none';
      }
    }
  }, [closeUpdaterHandle, runUpdaterCheck, t, writeUpdateLog]);

  const cancelUpdateDownload = useCallback(async () => {
    if (updateAction.state !== 'downloading') {
      return;
    }
    if (updateDownloadOwnerRef.current !== 'shared') {
      writeUpdateLog('info', '当前下载任务不支持取消（非统一更新任务）');
      return;
    }

    const version = updateAction.version;
    updateCancelRequestedRef.current = true;
    updateDownloadTaskIdRef.current += 1;
    setUpdateRetryStatus('');
    setUpdateDownloadError('');
    setUpdateErrorDetails('');

    const active = activeUpdateDownloadRef.current;
    if (active) {
      await closeUpdaterHandle(active);
      activeUpdateDownloadRef.current = null;
    }

    if (version) {
      setUpdateAction({
        state: 'available',
        version,
        progress: 0,
        requiresInstall: true,
      });
    } else {
      setUpdateAction({
        state: 'hidden',
        version: null,
        progress: 0,
        requiresInstall: true,
      });
    }
    updateDownloadOwnerRef.current = 'none';
    writeUpdateLog('info', `用户取消统一更新下载: version=${version || 'unknown'}`);
  }, [closeUpdaterHandle, updateAction.state, updateAction.version, writeUpdateLog]);

  const handleQuickUpdateActionClick = useCallback(async () => {
    if (updateAction.state === 'downloading') {
      setShowUpdateNotification(true);
      return;
    }
    if (updateAction.state === 'installing') {
      return;
    }

    if (updateAction.state === 'ready') {
      try {
        await handleApplyPendingUpdate();
      } catch (error) {
        console.error('[App] Quick update restart failed:', error);
        writeUpdateLog('error', `侧边栏重启更新失败: error=${sanitizeUpdaterErrorMessage(error)}`);
        openUpdateNotification('manual');
      }
      return;
    }

    if (updateAction.state !== 'available' || !updateAction.version) {
      return;
    }

    const expectedVersion = updateAction.version;
    try {
      if (isLinuxManagedUpdate) {
        await runLinuxManagedUpdate(expectedVersion);
      } else {
        await runSharedUpdateDownload(expectedVersion);
      }
    } catch (error) {
      console.error('[App] Quick update download failed:', error);
      writeUpdateLog('error', `侧边栏更新失败: error=${sanitizeUpdaterErrorMessage(error)}`);
      openUpdateNotification('manual');
    }
  }, [
    handleApplyPendingUpdate,
    isLinuxManagedUpdate,
    openUpdateNotification,
    runLinuxManagedUpdate,
    runSharedUpdateDownload,
    updateAction,
    writeUpdateLog,
  ]);

  const handleSkipUpdateVersion = useCallback(async () => {
    const targetVersion = updateNotificationInfo?.latest_version;
    if (!targetVersion) {
      return;
    }
    setUpdateSkipError('');
    try {
      const settings = await invoke<{
        auto_check?: boolean;
        check_interval_hours?: number;
        auto_install?: boolean;
        last_run_version?: string;
        remind_on_update?: boolean;
        skipped_version?: string;
      }>('get_update_settings');
      await invoke('save_update_settings', {
        settings: { ...settings, skipped_version: targetVersion },
      });
      const pendingUpdate = pendingSilentUpdateRef.current;
      if (pendingUpdate && pendingUpdate.version === targetVersion) {
        await closeUpdaterHandle(pendingUpdate);
        pendingSilentUpdateRef.current = null;
      }
      writeUpdateLog('info', `用户跳过更新版本: version=${targetVersion}`);
      setUpdateAction((prev) => {
        if (
          prev.version === targetVersion
          && (prev.state === 'available' || prev.state === 'downloading' || prev.state === 'ready')
        ) {
          return {
            state: 'hidden',
            version: null,
            progress: 0,
            requiresInstall: true,
          };
        }
        return prev;
      });
      setShowUpdateNotification(false);
      setUpdateNotificationInfo(null);
      setUpdateRetryStatus('');
      setUpdateDownloadError('');
      setUpdateErrorDetails('');
      setSilentUpdateVersion(null);
      updateDownloadOwnerRef.current = 'none';
      setUpdateSkipError('');
    } catch (error) {
      console.error('[App] Failed to skip update version:', error);
      setUpdateSkipError(
        t('update_notification.skipFailed', {
          error: sanitizeUpdaterErrorMessage(error),
        }),
      );
      writeUpdateLog('error', `跳过更新版本失败: error=${sanitizeUpdaterErrorMessage(error)}`);
    }
  }, [closeUpdaterHandle, t, updateNotificationInfo, writeUpdateLog]);

  useEffect(() => {
    return () => {
      const pendingUpdate = pendingSilentUpdateRef.current;
      if (pendingUpdate) {
        void pendingUpdate.close();
        pendingSilentUpdateRef.current = null;
      }
      const activeUpdate = activeUpdateDownloadRef.current;
      if (activeUpdate) {
        void activeUpdate.close();
        activeUpdateDownloadRef.current = null;
      }
    };
  }, []);

  const openQuickSettingsForPlatform = useCallback((platform: QuotaAlertPlatform) => {
    const targetPage = getQuotaAlertTargetPage(platform);
    const targetType = getQuotaAlertQuickSettingsType(platform);
    closeModal();
    setPage(targetPage);
    window.requestAnimationFrame(() => {
      window.requestAnimationFrame(() => {
        window.dispatchEvent(new CustomEvent('quick-settings:open', { detail: { type: targetType } }));
      });
    });
  }, [closeModal]);

  useEffect(() => {
    let cleanup: (() => void) | null = null;

    const applyTheme = (newTheme: string) => {
      if (newTheme === 'system') {
        const isDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
        document.documentElement.setAttribute('data-theme', isDark ? 'dark' : 'light');
      } else {
        document.documentElement.setAttribute('data-theme', newTheme);
      }
    };

    const applyUiScale = async (rawScale?: number) => {
      const scale = typeof rawScale === 'number' && Number.isFinite(rawScale) ? rawScale : 1;
      const normalizedScale = Math.min(2, Math.max(0.8, scale));
      try {
        await getCurrentWebview().setZoom(normalizedScale);
      } catch (error) {
        console.error('Failed to apply UI scale:', error);
      }
    };

    const watchSystemTheme = () => {
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
    };

    const initTheme = async () => {
      try {
        const config = await invoke<GeneralConfigTheme>('get_general_config');
        applyTheme(config.theme);
        void applyUiScale(config.ui_scale);
        if (config.theme === 'system') {
          cleanup = watchSystemTheme();
        }
      } catch (error) {
        console.error('Failed to load theme config:', error);
      }
    };

    initTheme();

    return () => {
      if (cleanup) {
        cleanup();
      }
    };
  }, []);

  useEffect(() => {
    const syncWakeupStateOnStartup = async () => {
      let officialLsVersionMode = loadWakeupOfficialLsVersionMode();
      try {
        // 一次性迁移：升级到该版本后先将唤醒总开关置为关闭，用户仍可手动再开启
        if (localStorage.getItem(WAKEUP_FORCE_DISABLE_MIGRATION_KEY) !== '1') {
          localStorage.setItem(WAKEUP_ENABLED_KEY, 'false');
          localStorage.setItem(WAKEUP_FORCE_DISABLE_MIGRATION_KEY, '1');
        }
        const enabled = localStorage.getItem(WAKEUP_ENABLED_KEY) === 'true';
        const tasksRaw = localStorage.getItem(TASKS_STORAGE_KEY);
        const tasks = tasksRaw ? JSON.parse(tasksRaw) : [];
        officialLsVersionMode = loadWakeupOfficialLsVersionMode();
        await invoke('wakeup_sync_state', {
          enabled,
          tasks,
          officialLsVersionMode,
          runStartupTasks: true,
        });
      } catch (error) {
        console.error('唤醒任务状态同步失败:', error);
      }
    };
    void syncWakeupStateOnStartup();
  }, []);

  useEffect(() => {
    const AUTO_BACKUP_STARTUP_DELAY_MS = 5 * 60 * 1000;
    const AUTO_BACKUP_POLL_INTERVAL_MS = 60 * 60 * 1000;
    let startupTimerId: number | undefined;
    let intervalId: number | undefined;
    let inFlight = false;

    const checkAutoBackup = async () => {
      if (inFlight) {
        return;
      }
      inFlight = true;
      try {
        await runAutoBackupCycle();
      } catch (error) {
        console.warn('[AutoBackup] 定期备份执行失败:', error);
      } finally {
        inFlight = false;
      }
    };

    startupTimerId = window.setTimeout(() => {
      void checkAutoBackup();
      intervalId = window.setInterval(() => {
        void checkAutoBackup();
      }, AUTO_BACKUP_POLL_INTERVAL_MS);
    }, AUTO_BACKUP_STARTUP_DELAY_MS);

    return () => {
      if (startupTimerId !== undefined) {
        window.clearTimeout(startupTimerId);
      }
      if (intervalId !== undefined) {
        window.clearInterval(intervalId);
      }
    };
  }, []);

  // Check for updates on startup
  useEffect(() => {
    if (!updateRuntimeInfoLoaded) {
      return;
    }

    const UPDATE_POLL_INTERVAL_MS = 60 * 60 * 1000;
    let updateCheckInFlight = false;
    let intervalId: number | undefined;

    const checkUpdates = async (trigger: 'startup' | 'hourly') => {
      if (updateCheckInFlight) {
        writeUpdateLog('info', `${trigger === 'startup' ? '启动' : '每小时轮询'}更新检查跳过：上一次尚未结束`);
        return;
      }
      updateCheckInFlight = true;
      const triggerLabel = trigger === 'startup' ? '启动' : '每小时轮询';
      const updateFlowStartedAt = performance.now();
      try {
        console.log(`[App] ${triggerLabel} update check triggered.`);
        console.log(`[StartupPerf][UpdateCheck] ${triggerLabel} update check started`);
        writeUpdateLog('info', `${triggerLabel}触发自动更新检查流程`);

        const settingsInvokeStartedAt = performance.now();
        const settings = await invoke<{
          auto_check?: boolean;
          check_interval_hours?: number;
          auto_install?: boolean;
          remind_on_update?: boolean;
          skipped_version?: string;
        }>('get_update_settings');
        const settingsInvokeElapsed = performance.now() - settingsInvokeStartedAt;
        console.log(
          `[StartupPerf][UpdateCheck] get_update_settings completed in ${settingsInvokeElapsed.toFixed(2)}ms`,
        );
        const autoInstall = settings?.auto_install ?? false;
        const remindOnUpdate = settings?.remind_on_update ?? true;
        const skippedVersion = (settings?.skipped_version ?? '').trim();
        setUpdateRemindersEnabled(remindOnUpdate);
        writeUpdateLog(
          'info',
          `读取更新设置: auto_install=${autoInstall}；启动始终执行更新检查`,
        );

        writeUpdateLog('info', '启动检查立即执行');

        if (autoInstall && !isLinuxManagedUpdate) {
          // Silent update: check and download in background, install on restart
          console.log('[App] Auto-install enabled, attempting silent update...');
          writeUpdateLog('info', '后台自动更新已开启，尝试静默检查并下载');
          let preparedUpdateInfo: UpdateInfo | null = null;
          try {
            const silentCheckStartedAt = performance.now();
            const update = await retryWithBackoff(
              async () => runUpdaterCheck(),
              {
                delaysMs: UPDATE_CHECK_RETRY_DELAYS_MS,
                shouldRetry: isRetryableUpdaterError,
                onRetry: ({ retryIndex, totalRetries, delayMs, error }) => {
                  const compactError = sanitizeUpdaterErrorMessage(error);
                  console.warn(
                    `[App] Silent update check failed, retrying (${retryIndex}/${totalRetries}) in ${delayMs}ms:`,
                    error,
                  );
                  writeUpdateLog(
                    'warn',
                    `静默更新检查失败，准备重试(${retryIndex}/${totalRetries})，delay=${delayMs}ms，error=${compactError}`,
                  );
                },
              },
            );
            console.log(
              `[StartupPerf][UpdateCheck] silent runUpdaterCheck completed in ${(performance.now() - silentCheckStartedAt).toFixed(2)}ms; hasUpdate=${Boolean(update)}`,
            );
            if (update) {
              if (skippedVersion && update.version === skippedVersion) {
                console.log('[App] Update skipped by user, ignoring:', update.version);
                writeUpdateLog('info', `检测到新版本但已跳过: version=${update.version}`);
                await closeUpdaterHandle(update);
                setUpdateAction((prev) => {
                  if (prev.state === 'available' && prev.version === update.version) {
                    return {
                      state: 'hidden',
                      version: null,
                      progress: 0,
                      requiresInstall: true,
                    };
                  }
                  return prev;
                });
              } else {
                preparedUpdateInfo = await prepareUpdateNotificationInfo(update);
                if (remindOnUpdate) {
                  setUpdateNotificationInfo(preparedUpdateInfo);
                  handleUpdateCheckResult({
                    source: 'auto',
                    status: 'has_update',
                    currentVersion: preparedUpdateInfo.current_version,
                    latestVersion: preparedUpdateInfo.latest_version,
                  });
                }
                console.log('[App] Update found, downloading silently with retry...');
                writeUpdateLog('info', `检测到新版本，开始静默下载: version=${update.version}`);
                updateDownloadOwnerRef.current = 'silent';
                setUpdateRetryStatus('');
                setUpdateDownloadError('');
                setUpdateErrorDetails('');
                setUpdateAction((prev) => {
                  if (prev.state === 'ready' && prev.version === update.version) {
                    return prev;
                  }
                  return {
                    state: 'downloading',
                    version: update.version,
                    progress: 0,
                    requiresInstall: true,
                  };
                });
                await invoke('save_pending_update_notes', {
                  version: update.version,
                  releaseNotes: preparedUpdateInfo.release_notes,
                  releaseNotesZh: preparedUpdateInfo.release_notes_zh,
                }).catch((error) => {
                  console.error('[App] Failed to cache silent update notes:', error);
                  writeUpdateLog(
                    'warn',
                    `缓存待安装更新说明失败: version=${update.version}, error=${sanitizeUpdaterErrorMessage(error)}`,
                  );
                });
                const silentDownloadStartedAt = performance.now();
                const downloadedUpdate = await retryWithBackoff(
                  async (attempt) => {
                    let candidate: UpdaterUpdate | null = null;
                    try {
                      if (attempt === 1) {
                        candidate = update;
                      } else {
                        candidate = await runUpdaterCheck();
                      }

                      if (!candidate) {
                        throw new Error('No update available from updater plugin');
                      }

                      let downloaded = 0;
                      let contentLength = 0;
                      const candidateVersion = candidate.version;
                      await candidate.download((event) => {
                        setUpdateAction((prev) => {
                          if (prev.state !== 'downloading') {
                            return prev;
                          }

                          if (event.event === 'Started') {
                            contentLength = event.data.contentLength ?? 0;
                            return {
                              ...prev,
                              version: candidateVersion,
                              progress: 0,
                            };
                          }

                          if (event.event === 'Progress') {
                            downloaded += event.data.chunkLength;
                            const nextProgress = contentLength > 0
                              ? Math.min(100, Math.round((downloaded / contentLength) * 100))
                              : Math.min(95, prev.progress + 1);
                            return {
                              ...prev,
                              version: candidateVersion,
                              progress: nextProgress,
                            };
                          }

                          return {
                            ...prev,
                            version: candidateVersion,
                            progress: 100,
                          };
                        });
                      });
                      return candidate;
                    } catch (error) {
                      if (candidate) {
                        await candidate.close().catch(() => {});
                      }
                      throw error;
                    }
                  },
                  {
                    delaysMs: UPDATE_DOWNLOAD_RETRY_DELAYS_MS,
                    shouldRetry: isRetryableUpdaterError,
                    onRetry: ({ retryIndex, totalRetries, delayMs, error }) => {
                      const compactError = sanitizeUpdaterErrorMessage(error);
                      setUpdateRetryStatus(
                        t('update_notification.downloadRetrying', {
                          attempt: retryIndex,
                          total: totalRetries,
                        }),
                      );
                      console.warn(
                        `[App] Silent update download failed, retrying (${retryIndex}/${totalRetries}) in ${delayMs}ms:`,
                        error,
                      );
                      writeUpdateLog(
                        'warn',
                        `静默更新下载失败，准备重试(${retryIndex}/${totalRetries})，delay=${delayMs}ms，error=${compactError}`,
                      );
                      setUpdateAction((prev) => {
                        if (prev.state !== 'downloading') {
                          return prev;
                        }
                        return {
                          ...prev,
                          progress: 0,
                        };
                      });
                    },
                  },
                );
                console.log(
                  `[StartupPerf][UpdateCheck] silent update download completed in ${(performance.now() - silentDownloadStartedAt).toFixed(2)}ms; version=${downloadedUpdate.version}`,
                );

                if (pendingSilentUpdateRef.current) {
                  await pendingSilentUpdateRef.current.close();
                }
                pendingSilentUpdateRef.current = downloadedUpdate;
                console.log('[App] Silent download complete, waiting for restart to install.');
                writeUpdateLog(
                  'info',
                  `静默更新下载完成，等待用户重启应用生效: version=${downloadedUpdate.version}`,
                );
                updateDownloadOwnerRef.current = 'none';
                setUpdateRetryStatus('');
                setUpdateDownloadError('');
                setUpdateErrorDetails('');
                setSilentUpdateVersion(downloadedUpdate.version);
                setUpdateAction({
                  state: 'ready',
                  version: downloadedUpdate.version,
                  progress: 100,
                  requiresInstall: true,
                });
                if (remindOnUpdate) {
                  openUpdateNotification('auto');
                }
              }
            } else {
              console.log('[App] No update available.');
              writeUpdateLog('info', '更新检查完成：当前已是最新版本');
              updateDownloadOwnerRef.current = 'none';
              setUpdateRetryStatus('');
              setUpdateDownloadError('');
              setUpdateErrorDetails('');
              setUpdateAction((prev) => {
                if (prev.state === 'ready') {
                  return prev;
                }
                return {
                  state: 'hidden',
                  version: null,
                  progress: 0,
                  requiresInstall: true,
                };
              });
            }
          } catch (err) {
            console.error('[App] Silent update failed:', err);
            updateDownloadOwnerRef.current = 'none';
            writeUpdateLog(
              'error',
              `静默更新失败，展示更新弹窗: error=${sanitizeUpdaterErrorMessage(err)}`,
            );
            if (!remindOnUpdate) {
              setUpdateRetryStatus('');
              setUpdateDownloadError('');
              setUpdateErrorDetails('');
              setUpdateAction((prev) => {
                if (prev.state === 'downloading' || prev.state === 'available') {
                  return {
                    state: 'hidden',
                    version: null,
                    progress: 0,
                    requiresInstall: true,
                  };
                }
                return prev;
              });
            }
            if (preparedUpdateInfo && remindOnUpdate) {
              setUpdateNotificationInfo(preparedUpdateInfo);
              setUpdateAction({
                state: 'available',
                version: preparedUpdateInfo.latest_version,
                progress: 0,
                requiresInstall: true,
              });
              openUpdateNotification('auto');
            }
          }
        } else {
          // Auto-check only opens the dialog after a real update is found.
          if (autoInstall && isLinuxManagedUpdate) {
            writeUpdateLog(
              'info',
              `Linux 包管理安装(${updateRuntimeInfo?.linux_install_kind || 'unknown'})跳过静默下载，改为一键安装弹窗`,
            );
          }
          writeUpdateLog('info', '后台自动更新关闭，先执行无弹窗检查，仅在发现新版本时展示弹窗');
          try {
            const manualCheckStartedAt = performance.now();
            const update = await retryWithBackoff(
              async () => runUpdaterCheck(),
              {
                delaysMs: UPDATE_CHECK_RETRY_DELAYS_MS,
                shouldRetry: isRetryableUpdaterError,
                onRetry: ({ retryIndex, totalRetries, delayMs, error }) => {
                  const compactError = sanitizeUpdaterErrorMessage(error);
                  console.warn(
                    `[App] Background manual update check failed, retrying (${retryIndex}/${totalRetries}) in ${delayMs}ms:`,
                    error,
                  );
                  writeUpdateLog(
                    'warn',
                    `后台手动更新检查失败，准备重试(${retryIndex}/${totalRetries})，delay=${delayMs}ms，error=${compactError}`,
                  );
                },
              },
            );
            console.log(
              `[StartupPerf][UpdateCheck] manual runUpdaterCheck completed in ${(performance.now() - manualCheckStartedAt).toFixed(2)}ms; hasUpdate=${Boolean(update)}`,
            );

            if (update) {
              if (skippedVersion && update.version === skippedVersion) {
                console.log('[App] Update skipped by user, ignoring:', update.version);
                writeUpdateLog('info', `检测到新版本但已跳过: version=${update.version}`);
                await closeUpdaterHandle(update);
                setUpdateAction((prev) => {
                  if (prev.state === 'available' && prev.version === update.version) {
                    return {
                      state: 'hidden',
                      version: null,
                      progress: 0,
                      requiresInstall: true,
                    };
                  }
                  return prev;
                });
              } else {
                const info = await prepareUpdateNotificationInfo(update);
                if (remindOnUpdate) {
                  setUpdateNotificationInfo(info);
                }
                handleUpdateCheckResult({
                  source: 'auto',
                  status: 'has_update',
                  currentVersion: info.current_version,
                  latestVersion: info.latest_version,
                });
                writeUpdateLog('info', `检测到新版本，展示手动更新弹窗: version=${update.version}`);
                await closeUpdaterHandle(update);
                if (remindOnUpdate) {
                  openUpdateNotification('auto');
                }
              }
            } else {
              writeUpdateLog('info', '更新检查完成：当前已是最新版本');
              setUpdateRetryStatus('');
              setUpdateDownloadError('');
              setUpdateErrorDetails('');
              setUpdateAction((prev) => {
                if (prev.state === 'ready') {
                  return prev;
                }
                return {
                  state: 'hidden',
                  version: null,
                  progress: 0,
                  requiresInstall: true,
                };
              });
            }
          } catch (err) {
            console.error('[App] Background update check failed:', err);
            writeUpdateLog(
              'warn',
              `后台手动更新检查失败，跳过弹窗: error=${sanitizeUpdaterErrorMessage(err)}`,
            );
          }
        }

        const updateLastCheckStartedAt = performance.now();
        await invoke('update_last_check_time');
        console.log(
          `[StartupPerf][UpdateCheck] update_last_check_time completed in ${(performance.now() - updateLastCheckStartedAt).toFixed(2)}ms`,
        );
        writeUpdateLog('info', '已更新 last_check_time，结束本次更新检查流程');
        console.log('[App] Update check cycle completed.');
        console.log(
          `[StartupPerf][UpdateCheck] ${triggerLabel} update check completed in ${(performance.now() - updateFlowStartedAt).toFixed(2)}ms`,
        );
      } catch (error) {
        console.error('Failed to check update settings:', error);
        console.error(
          `[StartupPerf][UpdateCheck] ${triggerLabel} update check failed after ${(performance.now() - updateFlowStartedAt).toFixed(2)}ms:`,
          error,
        );
        writeUpdateLog('error', `更新检查流程异常中断: error=${sanitizeUpdaterErrorMessage(error)}`);
      } finally {
        updateCheckInFlight = false;
      }
    };

    void checkUpdates('startup');
    intervalId = window.setInterval(() => {
      void checkUpdates('hourly');
    }, UPDATE_POLL_INTERVAL_MS);
    return () => {
      if (intervalId !== undefined) {
        window.clearInterval(intervalId);
      }
    };
  }, [
    closeUpdaterHandle,
    handleUpdateCheckResult,
    isLinuxManagedUpdate,
    openUpdateNotification,
    prepareUpdateNotificationInfo,
    runUpdaterCheck,
    updateRuntimeInfo?.linux_install_kind,
    updateRuntimeInfoLoaded,
    writeUpdateLog,
  ]);

  // Version jump detection (post-update changelog)
  useEffect(() => {
    const detectVersionJump = async () => {
      const versionJumpStartedAt = performance.now();
      try {
        console.log('[StartupPerf][VersionJump] detection started');
        const versionJumpInvokeStartedAt = performance.now();
        const jumpInfo = await invoke<{
          previous_version: string;
          current_version: string;
          release_notes: string;
          release_notes_zh: string;
        } | null>('check_version_jump');
        console.log(
          `[StartupPerf][VersionJump] check_version_jump completed in ${(performance.now() - versionJumpInvokeStartedAt).toFixed(2)}ms; hasJump=${Boolean(jumpInfo)}`,
        );
        if (jumpInfo) {
          console.log('[App] Version jump detected:', jumpInfo.previous_version, '->', jumpInfo.current_version);
          (
            window as Window & {
              __agtoolsVersionJumpModalRequestedAt?: number;
            }
          ).__agtoolsVersionJumpModalRequestedAt = performance.now();
          setVersionJumpInfo(jumpInfo);
          requestAnimationFrame(() => {
            console.log(
              `[StartupPerf][VersionJump] first frame after setVersionJumpInfo in ${(performance.now() - versionJumpStartedAt).toFixed(2)}ms`,
            );
          });
        }
        console.log(
          `[StartupPerf][VersionJump] detection finished in ${(performance.now() - versionJumpStartedAt).toFixed(2)}ms`,
        );
      } catch (error) {
        console.error('Failed to check version jump:', error);
        console.error(
          `[StartupPerf][VersionJump] detection failed after ${(performance.now() - versionJumpStartedAt).toFixed(2)}ms:`,
          error,
        );
      }
    };

    const timer = setTimeout(detectVersionJump, 1000);
    return () => clearTimeout(timer);
  }, []);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;

    listen<string>('settings:language_changed', (event) => {
      const nextLanguage = normalizeLanguage(String(event.payload || ''));
      if (!nextLanguage || nextLanguage === getCurrentLanguage()) {
        return;
      }
      void changeLanguage(nextLanguage);
      window.dispatchEvent(new CustomEvent('general-language-updated', { detail: { language: nextLanguage } }));
    }).then((fn) => { unlisten = fn; });

    return () => {
      if (unlisten) {
        unlisten();
      }
    };
  }, []);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let disposed = false;

    listen<QuotaAlertPayload>('quota:alert', (event) => {
      const payload = event.payload;
      if (!payload || !payload.current_account_id) {
        return;
      }

      const platform = normalizeQuotaAlertPlatform(payload.platform);
      const platformLabel = getQuotaAlertPlatformLabel(platform, t);
      const hasRecommendation = Boolean(payload.recommended_account_id && payload.recommended_email);
      const modelsText = payload.low_models.length > 0
        ? payload.low_models.join(', ')
        : t('quotaAlert.modal.unknownModel', '未知模型');

      showModal({
        title: t('quotaAlert.modal.title', '配额预警'),
        description: t(
          'quotaAlert.modal.desc',
          '当前账号配额已达到预警阈值，请尽快处理。'
        ),
        width: 'md',
        closeOnOverlay: false,
        content: (
          <div className="quota-alert-modal-content">
            <div className="quota-alert-modal-row">
              <span>{t('quotaAlert.modal.platform', '平台')}</span>
              <strong>{platformLabel}</strong>
            </div>
            <div className="quota-alert-modal-row">
              <span>{t('quotaAlert.modal.account', '当前账号')}</span>
              <strong>{payload.current_email}</strong>
            </div>
            <div className="quota-alert-modal-row">
              <span>{t('quotaAlert.modal.threshold', '预警阈值')}</span>
              <strong>{payload.threshold_display || `${payload.threshold}%`}</strong>
            </div>
            <div className="quota-alert-modal-row">
              <span>{t('quotaAlert.modal.lowest', '当前最低')}</span>
              <strong>{payload.lowest_percentage}%</strong>
            </div>
            <div className="quota-alert-modal-row quota-alert-modal-row--stack">
              <span>{t('quotaAlert.modal.models', '触发模型')}</span>
              <strong>{modelsText}</strong>
            </div>
            <div className="quota-alert-modal-row">
              <span>{t('quotaAlert.modal.recommended', '建议切换')}</span>
              <strong>
                {payload.recommended_email || t('quotaAlert.modal.noRecommendation', '暂无可切换账号')}
              </strong>
            </div>
          </div>
        ),
        actions: [
          {
            id: 'quota-alert-later',
            label: t('quotaAlert.modal.later', '稍后处理'),
            variant: 'secondary',
          },
          {
            id: 'quota-alert-open-settings',
            label: t('quotaAlert.modal.openSettings', '调整预警设置'),
            variant: 'secondary',
            autoClose: false,
            onClick: () => {
              openQuickSettingsForPlatform(platform);
            },
          },
          ...(hasRecommendation
            ? [{
                id: 'quota-alert-switch',
                label: t('quotaAlert.modal.switchNow', '快捷切号到 {{email}}', {
                  email: payload.recommended_email as string,
                }),
                variant: 'primary' as const,
                autoClose: false,
                onClick: async () => {
                  try {
                    const targetAccountId = payload.recommended_account_id as string;
                    if (platform === 'codex') {
                      await useCodexAccountStore.getState().switchAccount(targetAccountId);
                      setPage('codex');
                    } else if (platform === 'github_copilot') {
                      await useGitHubCopilotAccountStore.getState().switchAccount(targetAccountId);
                      setPage('github-copilot');
                    } else if (platform === 'windsurf') {
                      await useWindsurfAccountStore.getState().switchAccount(targetAccountId);
                      setPage('windsurf');
                    } else if (platform === 'kiro') {
                      await useKiroAccountStore.getState().switchAccount(targetAccountId);
                      setPage('kiro');
                    } else if (platform === 'cursor') {
                      await useCursorAccountStore.getState().switchAccount(targetAccountId);
                      setPage('cursor');
                    } else if (platform === 'gemini') {
                      await useGeminiAccountStore.getState().switchAccount(targetAccountId);
                      setPage('gemini');
                    } else if (platform === 'codebuddy') {
                      await useCodebuddyAccountStore.getState().switchAccount(targetAccountId);
                      setPage('codebuddy');
                    } else if (platform === 'codebuddy_cn') {
                      await useCodebuddyCnAccountStore.getState().switchAccount(targetAccountId);
                      setPage('codebuddy-cn');
                    } else if (platform === 'qoder') {
                      await useQoderAccountStore.getState().switchAccount(targetAccountId);
                      setPage('qoder');
                    } else if (platform === 'trae') {
                      await useTraeAccountStore.getState().switchAccount(targetAccountId);
                      setPage('trae');
                    } else if (platform === 'workbuddy') {
                      await useWorkbuddyAccountStore.getState().switchAccount(targetAccountId);
                      setPage('workbuddy');
                    } else if (platform === 'zed') {
                      await useZedAccountStore.getState().switchAccount(targetAccountId);
                      setPage('zed');
                    } else {
                      await useAccountStore.getState().switchAccount(targetAccountId);
                      setPage('overview');
                    }
                    closeModal();
                  } catch (error) {
                    showModal({
                      title: t('quotaAlert.modal.switchFailedTitle', '切号失败'),
                      description: t('quotaAlert.modal.switchFailedBody', '快捷切号失败：{{error}}', {
                        error: String(error),
                      }),
                      width: 'sm',
                      actions: [
                        {
                          id: 'quota-alert-switch-failed-ok',
                          label: t('common.confirm', '确定'),
                          variant: 'primary',
                        },
                      ],
                    });
                  }
                },
              }]
            : []),
        ],
      });
    }).then((fn) => {
      if (disposed) {
        fn();
        return;
      }
      unlisten = fn;
    });

    return () => {
      disposed = true;
      if (unlisten) {
        unlisten();
      }
    };
  }, [closeModal, openQuickSettingsForPlatform, showModal, t]);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;

    const handleWakeupResult = (payload: WakeupTaskResultPayload) => {
      if (!payload || typeof payload.taskId !== 'string') return;

      // 更新任务的最后运行时间
      const tasksRaw = localStorage.getItem(TASKS_STORAGE_KEY);
      if (tasksRaw) {
        try {
          const tasks = JSON.parse(tasksRaw) as Array<{ id: string; lastRunAt?: number }>;
          const nextTasks = tasks.map((task) =>
            task.id === payload.taskId ? { ...task, lastRunAt: payload.lastRunAt } : task
          );
          localStorage.setItem(TASKS_STORAGE_KEY, JSON.stringify(nextTasks));
        } catch (error) {
          console.error('更新唤醒任务时间失败:', error);
        }
      }

      // 历史记录已由后端写入文件，这里只需通知前端刷新
      window.dispatchEvent(new CustomEvent('wakeup-task-result', { detail: payload }));
      window.dispatchEvent(new Event('wakeup-tasks-updated'));
    };

    listen<WakeupTaskResultPayload>('wakeup://task-result', (event) => {
      handleWakeupResult(event.payload);
    }).then((fn) => {
      unlisten = fn;
    });

    return () => {
      if (unlisten) {
        unlisten();
      }
    };
  }, []);

  useEffect(() => {
    const handleUpdateRequest = (event: Event) => {
      const detail = (event as CustomEvent<{ source?: UpdateCheckSource }>).detail;
      const source: UpdateCheckSource = detail?.source === 'manual' ? 'manual' : 'auto';
      void runModalUpdateCheck(source);
    };
    window.addEventListener('update-check-requested', handleUpdateRequest as EventListener);
    return () => {
      window.removeEventListener('update-check-requested', handleUpdateRequest as EventListener);
    };
  }, [runModalUpdateCheck]);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;

    listen<LinuxUpdateProgressPayload>('update://linux-progress', (event) => {
      const { phase, progress, version } = event.payload;
      setUpdateDownloadError('');
      setUpdateErrorDetails('');

      setUpdateAction((prev) => {
        if (
          prev.version
          && prev.version !== version
          && (prev.state === 'downloading' || prev.state === 'installing' || prev.state === 'ready')
        ) {
          return prev;
        }

        if (phase === 'completed') {
          return {
            state: 'ready',
            version,
            progress: 100,
            requiresInstall: false,
          };
        }

        if (phase === 'auth_required' || phase === 'installing' || phase === 'downloaded') {
          return {
            state: 'installing',
            version,
            progress: 100,
            requiresInstall: false,
          };
        }

        return {
          state: 'downloading',
          version,
          progress: Math.max(0, Math.min(100, Math.round(progress ?? 0))),
          requiresInstall: false,
        };
      });

      if (phase === 'auth_required' || phase === 'downloaded') {
        setUpdateRetryStatus(
          t('update_notification.authorizing', '等待系统授权安装...'),
        );
        return;
      }

      if (phase === 'installing') {
        setUpdateRetryStatus(
          t('update_notification.installing', '安装中...'),
        );
        return;
      }

      if (phase === 'completed') {
        setUpdateRetryStatus(
          t('update_notification.installSuccess', '更新已安装，正在重启...'),
        );
        return;
      }

      setUpdateRetryStatus('');
    }).then((fn) => {
      unlisten = fn;
    });

    return () => {
      if (unlisten) {
        unlisten();
      }
    };
  }, [t]);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;

    const refreshTasks = [
      {
        command: 'refresh_current_quota',
        errorMessage: 'Failed to refresh Antigravity quotas:',
      },
      {
        command: 'refresh_current_codex_quota',
        errorMessage: 'Failed to refresh Codex quotas:',
      },
      {
        command: 'refresh_all_github_copilot_tokens',
        errorMessage: 'Failed to refresh GitHub Copilot quotas:',
      },
      {
        command: 'refresh_all_windsurf_tokens',
        errorMessage: 'Failed to refresh Windsurf quotas:',
      },
      {
        command: 'refresh_all_kiro_tokens',
        errorMessage: 'Failed to refresh Kiro quotas:',
      },
      {
        command: 'refresh_all_cursor_tokens',
        errorMessage: 'Failed to refresh Cursor:',
      },
      {
        command: 'refresh_all_gemini_tokens',
        errorMessage: 'Failed to refresh Gemini:',
      },
      {
        command: 'refresh_all_codebuddy_tokens',
        errorMessage: 'Failed to refresh CodeBuddy:',
      },
      {
        command: 'refresh_all_codebuddy_cn_tokens',
        errorMessage: 'Failed to refresh CodeBuddy CN:',
      },
      {
        command: 'refresh_all_qoder_tokens',
        errorMessage: 'Failed to refresh Qoder:',
      },
      {
        command: 'refresh_all_trae_tokens',
        errorMessage: 'Failed to refresh Trae:',
      },
      {
        command: 'refresh_all_zed_tokens',
        errorMessage: 'Failed to refresh Zed:',
      },
    ] as const;

    listen('tray:refresh_quota', async () => {
      if (trayRefreshInFlightRef.current) {
        return;
      }
      trayRefreshInFlightRef.current = true;

      try {
        await Promise.all(
          refreshTasks.map(({ command, errorMessage }) =>
            invoke(command).catch((error) => {
              console.error(errorMessage, error);
            }),
          ),
        );
      } finally {
        trayRefreshInFlightRef.current = false;
      }
    }).then((fn) => { unlisten = fn; });

    return () => {
      if (unlisten) {
        unlisten();
      }
    };
  }, []);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    const handlePayload = (payload: unknown) => {
      if (!payload || typeof payload !== 'object') return;
      const detail = payload as AppPathMissingDetail;
      if (
        detail.app !== 'antigravity' &&
        detail.app !== 'codex' &&
        detail.app !== 'vscode' &&
        detail.app !== 'windsurf' &&
        detail.app !== 'kiro' &&
        detail.app !== 'cursor' &&
        detail.app !== 'codebuddy' &&
        detail.app !== 'codebuddy_cn' &&
        detail.app !== 'qoder' &&
        detail.app !== 'trae' &&
        detail.app !== 'zed'
      ) {
        return;
      }
      setAppPathMissing(detail);
    };

    listen('app:path_missing', (event) => {
      handlePayload(event.payload);
    }).then((fn) => { unlisten = fn; });

    const handleWindowEvent = (event: Event) => {
      const custom = event as CustomEvent<AppPathMissingDetail>;
      handlePayload(custom.detail);
    };
    window.addEventListener('app-path-missing', handleWindowEvent as EventListener);

    return () => {
      if (unlisten) {
        unlisten();
      }
      window.removeEventListener('app-path-missing', handleWindowEvent as EventListener);
    };
  }, []);

  useEffect(() => {
    let active = true;
    if (!appPathMissing) {
      setAppPathDraft('');
      setAppPathDetecting(false);
      setAppPathActionError('');
      setAppPathCodexLaunchOnSwitch(true);
      setAppPathCodexLaunchSetting(false);
      return () => {
        active = false;
      };
    }
    setAppPathActionError('');
    (async () => {
      try {
        const config = await invoke<GeneralConfig>('get_general_config');
        const currentPath =
          appPathMissing.app === 'codex'
            ? config.codex_app_path
            : appPathMissing.app === 'vscode'
              ? config.vscode_app_path
              : appPathMissing.app === 'windsurf'
                ? config.windsurf_app_path
              : appPathMissing.app === 'kiro'
                ? config.kiro_app_path
              : appPathMissing.app === 'cursor'
                ? config.cursor_app_path
              : appPathMissing.app === 'codebuddy'
                ? config.codebuddy_app_path
              : appPathMissing.app === 'codebuddy_cn'
                ? config.codebuddy_cn_app_path
              : appPathMissing.app === 'qoder'
                ? config.qoder_app_path
              : appPathMissing.app === 'trae'
                ? config.trae_app_path
              : appPathMissing.app === 'zed'
                ? config.zed_app_path
              : config.antigravity_app_path;
        if (active) {
          setAppPathDraft(currentPath || '');
          setAppPathCodexLaunchOnSwitch(config.codex_launch_on_switch ?? true);
        }
      } catch (error) {
        console.error('Failed to load app path config:', error);
      }
    })();
    return () => {
      active = false;
    };
  }, [appPathMissing]);

  const handlePickMissingAppPath = async () => {
    if (appPathSetting) return;
    try {
      const selected = await open({
        multiple: false,
        directory: false,
      });
      const path = Array.isArray(selected) ? selected[0] : selected;
      if (path) {
        setAppPathActionError('');
        setAppPathDraft(path);
      }
    } catch (error) {
      console.error('选择应用路径失败:', error);
    }
  };

  const handleSaveMissingAppPath = async () => {
    if (!appPathMissing || appPathSetting || appPathDetecting) return;
    const path = appPathDraft.trim();
    if (!path) return;
    setAppPathSetting(true);
    setAppPathActionError('');
    try {
      const app = appPathMissing.app;
      const retry = appPathMissing.retry;
      await invoke('set_app_path', { app, path });
      if (retry?.kind === 'switchAccount' && retry.accountId && app === 'zed') {
        await useZedAccountStore.getState().switchAccount(retry.accountId);
        setPage('zed');
      } else if (retry?.kind === 'switchAccount' && retry.accountId) {
        await invoke('switch_account', { accountId: retry.accountId });
        await Promise.allSettled([
          useAccountStore.getState().fetchAccounts(),
          useAccountStore.getState().fetchCurrentAccount(),
        ]);
      } else if (retry?.kind === 'instance' && retry.instanceId) {
        if (app === 'codex') {
          await invoke('codex_start_instance', { instanceId: retry.instanceId });
        } else if (app === 'vscode') {
          await invoke('github_copilot_start_instance', { instanceId: retry.instanceId });
        } else if (app === 'windsurf') {
          await invoke('windsurf_start_instance', { instanceId: retry.instanceId });
        } else if (app === 'kiro') {
          await invoke('kiro_start_instance', { instanceId: retry.instanceId });
        } else if (app === 'cursor') {
          await invoke('cursor_start_instance', { instanceId: retry.instanceId });
        } else if (app === 'codebuddy') {
          await invoke('codebuddy_start_instance', { instanceId: retry.instanceId });
        } else if (app === 'codebuddy_cn') {
          await invoke('codebuddy_cn_start_instance', { instanceId: retry.instanceId });
        } else if (app === 'qoder') {
          await invoke('qoder_start_instance', { instanceId: retry.instanceId });
        } else if (app === 'trae') {
          await invoke('trae_start_instance', { instanceId: retry.instanceId });
        } else if (app === 'zed') {
          await invoke('zed_start_default_session');
        } else {
          await invoke('start_instance', { instanceId: retry.instanceId });
        }
      } else {
        if (app === 'codex') {
          await invoke('codex_start_instance', { instanceId: '__default__' });
        } else if (app === 'vscode') {
          await invoke('github_copilot_start_instance', { instanceId: '__default__' });
        } else if (app === 'windsurf') {
          await invoke('windsurf_start_instance', { instanceId: '__default__' });
        } else if (app === 'kiro') {
          await invoke('kiro_start_instance', { instanceId: '__default__' });
        } else if (app === 'cursor') {
          await invoke('cursor_start_instance', { instanceId: '__default__' });
        } else if (app === 'codebuddy') {
          await invoke('codebuddy_start_instance', { instanceId: '__default__' });
        } else if (app === 'codebuddy_cn') {
          await invoke('codebuddy_cn_start_instance', { instanceId: '__default__' });
        } else if (app === 'qoder') {
          await invoke('qoder_start_instance', { instanceId: '__default__' });
        } else if (app === 'trae') {
          await invoke('trae_start_instance', { instanceId: '__default__' });
        } else if (app === 'zed') {
          await invoke('zed_start_default_session');
        } else {
          await invoke('start_instance', { instanceId: '__default__' });
        }
      }
      setAppPathMissing(null);
      setAppPathSetting(false);
    } catch (error) {
      console.error('设置应用路径失败:', error);
      setAppPathActionError(String(error));
      setAppPathSetting(false);
    }
  };

  const handleResetMissingAppPath = async () => {
    if (!appPathMissing || appPathSetting || appPathDetecting) return;
    setAppPathDetecting(true);
    try {
      const detected = await invoke<string | null>('detect_app_path', {
        app: appPathMissing.app,
        force: true,
      });
      setAppPathActionError('');
      setAppPathDraft((detected || '').trim());
    } catch (error) {
      console.error('自动探测应用路径失败:', error);
    } finally {
      setAppPathDetecting(false);
    }
  };

  const handleToggleCodexLaunchInMissingPath = async (enabled: boolean) => {
    if (!appPathMissing || appPathMissing.app !== 'codex') return;
    if (appPathSetting || appPathDetecting || appPathCodexLaunchSetting) return;
    setAppPathCodexLaunchSetting(true);
    setAppPathActionError('');
    try {
      await invoke('set_codex_launch_on_switch', { enabled });
      setAppPathCodexLaunchOnSwitch(enabled);
      if (!enabled) {
        setAppPathMissing(null);
      }
    } catch (error) {
      console.error('更新 Codex 自动启动配置失败:', error);
      setAppPathActionError(String(error));
    } finally {
      setAppPathCodexLaunchSetting(false);
    }
  };

  // 监听窗口关闭请求事件
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;

    listen('window:close_requested', () => {
      setShowCloseDialog(true);
    }).then((fn) => { unlisten = fn; });

    return () => {
      if (unlisten) {
        unlisten();
      }
    };
  }, []);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;

        listen<string>('tray:navigate', (event) => {
          const target = String(event.payload || '');
          switch (target) {
            case 'overview':
            case 'codex':
            case 'github-copilot':
            case 'windsurf':
            case 'kiro':
            case 'cursor':
            case 'gemini':
            case 'codebuddy':
            case 'codebuddy-cn':
            case 'qoder':
            case 'trae':
            case 'workbuddy':
            case 'zed':
            case 'manual':
            case 'settings':
              setPage(target as Page);
              break;
            default:
              break;
          }
        }).then((fn) => { unlisten = fn; });

    return () => {
      if (unlisten) {
        unlisten();
      }
    };
  }, []);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    listen('external:provider-import', (event) => {
      console.info('[ExternalImport][App] 收到 Tauri 事件 external:provider-import');
      void handleExternalProviderImportRawPayload(event.payload);
    }).then((fn) => {
      unlisten = fn;
    });

    return () => {
      if (unlisten) {
        unlisten();
      }
    };
  }, [handleExternalProviderImportRawPayload]);

  useEffect(() => {
    let canceled = false;
    void invoke<unknown>('external_import_take_pending')
      .then((payload) => {
        if (canceled) return;
        if (!payload) {
          console.info('[ExternalImport][App] 启动时无待处理导入 payload');
          return;
        }
        console.info('[ExternalImport][App] 启动时读取到待处理导入 payload');
        void handleExternalProviderImportRawPayload(payload);
      })
      .catch((error) => {
        console.warn('[ExternalImport] 读取待处理导入请求失败:', error);
      });
    return () => {
      canceled = true;
    };
  }, [handleExternalProviderImportRawPayload]);

  // 窗口拖拽处理
  const handleDragStart = (event: ReactMouseEvent<HTMLDivElement>) => {
    if (event.button !== 0) {
      return;
    }
    void getCurrentWindow().startDragging().catch((error) => {
      console.warn('[Window] startDragging failed:', error);
    });
  };

  useEffect(() => {
    const handleRequestNavigate = (e: Event) => {
      const custom = e as CustomEvent<Page>;
      if (custom.detail) {
        setPage(custom.detail);
      }
    };
    window.addEventListener('app-request-navigate', handleRequestNavigate as EventListener);
    return () => {
      window.removeEventListener('app-request-navigate', handleRequestNavigate as EventListener);
    };
  }, []);

  useEffect(() => {
    const handleOpenPlatformLayout = (e: Event) => {
      const custom = e as CustomEvent<{ groupId?: string | null }>;
      const groupId =
        custom.detail && typeof custom.detail.groupId === 'string' && custom.detail.groupId.trim()
          ? custom.detail.groupId.trim()
          : null;
      setPlatformLayoutRequestedGroupId(groupId);
      setShowPlatformLayoutModal(true);
    };

    window.addEventListener('app-open-platform-layout', handleOpenPlatformLayout as EventListener);
    return () => {
      window.removeEventListener('app-open-platform-layout', handleOpenPlatformLayout as EventListener);
    };
  }, []);
  const suspenseFallback = (
    <div className="loading-state">
      {t('common.loading', '加载中...')}
    </div>
  );

  const appPathMissingAppName = appPathMissing
    ? appPathMissing.app === 'codex'
      ? 'Codex'
      : appPathMissing.app === 'vscode'
        ? 'VS Code'
        : appPathMissing.app === 'windsurf'
          ? 'Windsurf'
          : appPathMissing.app === 'kiro'
            ? 'Kiro'
            : appPathMissing.app === 'cursor'
            ? 'Cursor'
            : appPathMissing.app === 'codebuddy'
              ? 'CodeBuddy'
              : appPathMissing.app === 'codebuddy_cn'
                ? 'CodeBuddy CN'
              : appPathMissing.app === 'qoder'
                ? 'Qoder'
              : appPathMissing.app === 'trae'
                ? 'Trae'
              : 'Antigravity'
    : '';

  const appPathMissingPathLabel = appPathMissing
    ? appPathMissing.app === 'codex'
      ? t('quickSettings.codex.appPath', '启动路径')
      : appPathMissing.app === 'vscode'
        ? t('quickSettings.githubCopilot.appPath', 'VS Code 路径')
        : appPathMissing.app === 'windsurf'
          ? t('quickSettings.windsurf.appPath', 'Windsurf 路径')
          : appPathMissing.app === 'kiro'
            ? t('quickSettings.kiro.appPath', 'Kiro 路径')
            : appPathMissing.app === 'cursor'
            ? t('quickSettings.cursor.appPath', 'Cursor 路径')
            : appPathMissing.app === 'codebuddy'
              ? t('quickSettings.codebuddy.appPath', 'CodeBuddy 路径')
              : appPathMissing.app === 'codebuddy_cn'
                ? t('quickSettings.codebuddyCn.appPath', 'CodeBuddy CN 路径')
              : appPathMissing.app === 'qoder'
                ? t('quickSettings.qoder.appPath', 'Qoder 路径')
              : appPathMissing.app === 'trae'
                ? t('quickSettings.trae.appPath', 'Trae 路径')
              : t('quickSettings.antigravity.appPath', '启动路径')
    : t('quickSettings.antigravity.appPath', '启动路径');
  const appPathMissingBusy = appPathSetting || appPathDetecting || appPathCodexLaunchSetting;
  const shouldRenderUpdateNotification = showUpdateNotification
    || (updateRemindersEnabled && updateAction.state !== 'hidden');

  return (
    <div
      className={`app-container${isWindowsPlatform() ? ' app-container-windows' : ''}${sideNavLayoutMode === 'classic' ? ' app-container-side-nav-classic' : ''}${sideNavLayoutMode === 'classic' && sideNavClassicCollapsed ? ' app-container-side-nav-classic-collapsed' : ''}`}
    >
      {/* 更新通知：活跃状态时保持挂载，关闭后继续保留当前更新状态 */}
      {shouldRenderUpdateNotification && (
        <div style={showUpdateNotification ? undefined : { display: 'none' }}>
        <Suspense fallback={null}>
          <UpdateNotification
            key={updateNotificationKey}
            updateInfo={updateNotificationInfo}
            checking={updateNotificationChecking}
            onRestartUpdate={handleApplyPendingUpdate}
            actionState={updateAction.state}
            actionVersion={updateAction.version}
            actionProgress={updateAction.progress}
            actionRetryStatus={updateRetryStatus}
            actionError={updateDownloadError}
            actionErrorDetails={updateErrorDetails}
            skipError={updateSkipError}
            onPrimaryAction={handleQuickUpdateActionClick}
            onCancelUpdate={cancelUpdateDownload}
            onSkipUpdate={handleSkipUpdateVersion}
            onClose={closeUpdateNotification}
          />
        </Suspense>
        </div>
      )}
      {/* 版本跳跃通知（更新后首次启动） */}
      {versionJumpInfo && (
        <Suspense fallback={null}>
          <VersionJumpNotification
            info={versionJumpInfo}
            onClose={() => setVersionJumpInfo(null)}
          />
        </Suspense>
      )}
      <GlobalModal />

      {/* 关闭确认对话框 */}
      {showCloseDialog && (
        <Suspense fallback={null}>
          <CloseConfirmDialog onClose={() => setShowCloseDialog(false)} />
        </Suspense>
      )}

      {hasBreakoutSession && (
        <Suspense fallback={null}>
          <BreakoutModal
            open={showBreakout}
            onMinimize={handleBreakoutMinimize}
            onTerminate={handleBreakoutTerminate}
          />
        </Suspense>
      )}

      {appPathMissing && (
        <div className="qs-overlay" style={{ zIndex: 10100 }}>
          <div className="qs-modal app-path-missing-modal" onClick={(e) => e.stopPropagation()}>
            <div className="qs-header">
              <span className="qs-title">{t('appPath.missing.title', '未找到应用程序路径')}</span>
              <button
                className="qs-close"
                onClick={() => setAppPathMissing(null)}
                aria-label={t('common.close', '关闭')}
                disabled={appPathMissingBusy}
              >
                <X size={16} />
              </button>
            </div>

            <div className="qs-body">
              <div className="qs-section">
                <p className="app-path-missing-desc">
                  {t('appPath.missing.desc', '未找到 {{app}} 应用程序路径，请立即设置后继续启动。', {
                    app: appPathMissingAppName,
                  })}
                </p>
              </div>

              {appPathMissing.app === 'codex' ? (
                <div className="qs-section">
                  <div className="qs-row">
                    <div className="qs-row-label">
                      {t('settings.general.codexLaunchOnSwitch', '切换 Codex 时自动启动 Codex App')}
                    </div>
                    <label className="qs-switch">
                      <input
                        type="checkbox"
                        checked={appPathCodexLaunchOnSwitch}
                        disabled={appPathMissingBusy}
                        onChange={(e) => handleToggleCodexLaunchInMissingPath(e.target.checked)}
                      />
                      <span className="qs-switch-slider" />
                    </label>
                  </div>
                  <p className="app-path-missing-hint">
                    {t(
                      'appPath.missing.codexLaunchHint',
                      '关闭后仅执行切号与登录覆盖，不再尝试启动 Codex App，也不会再次要求设置启动路径。'
                    )}
                  </p>
                </div>
              ) : null}

              <div className="qs-section">
                <div className="qs-section-header">
                  <FolderOpen size={15} />
                  <span>{appPathMissingPathLabel}</span>
                </div>
                <div className="qs-path-control">
                  <input
                    type="text"
                    className="qs-path-input"
                    value={appPathDraft}
                    placeholder={t('settings.general.codexAppPathPlaceholder', '默认路径')}
                    onChange={(e) => setAppPathDraft(e.target.value)}
                    disabled={appPathMissingBusy}
                  />
                  <div className="qs-path-actions">
                    <button
                      className="qs-btn"
                      onClick={handlePickMissingAppPath}
                      disabled={appPathMissingBusy}
                    >
                      {t('settings.general.codexPathSelect', '选择')}
                    </button>
                    <button
                      className="qs-btn"
                      onClick={handleResetMissingAppPath}
                      disabled={appPathMissingBusy}
                      title={
                        appPathDetecting
                          ? t('common.loading', '加载中...')
                          : (
                            appPathMissing.app === 'vscode'
                              ? t('settings.general.vscodePathReset', '重置默认')
                              : appPathMissing.app === 'windsurf'
                                ? t('settings.general.windsurfPathReset', '重置默认')
                                : appPathMissing.app === 'kiro'
                                  ? t('settings.general.kiroPathReset', '重置默认')
                                  : appPathMissing.app === 'cursor'
                                  ? t('settings.general.cursorPathReset', '重置默认')
                                    : appPathMissing.app === 'codebuddy'
                                      ? t('settings.general.codebuddyPathReset', '重置默认')
                                    : appPathMissing.app === 'codebuddy_cn'
                                      ? t('settings.general.codebuddyPathReset', '重置默认')
                                    : appPathMissing.app === 'qoder'
                                      ? t('settings.general.qoderPathReset', '重置默认')
                                    : appPathMissing.app === 'trae'
                                      ? t('settings.general.traePathReset', '重置默认')
                                    : t('settings.general.codexPathReset', '重置默认')
                          )
                      }
                    >
                      <RefreshCw size={12} className={appPathDetecting ? 'spin' : undefined} />
                    </button>
                  </div>
                </div>
                {appPathActionError ? (
                  <p className="app-path-missing-error">
                    {t('messages.switchFailed', { error: appPathActionError })}
                  </p>
                ) : null}
              </div>
            </div>

            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={() => setAppPathMissing(null)}
                disabled={appPathMissingBusy}
              >
                {t('common.cancel', '取消')}
              </button>
              <button
                className="btn btn-primary"
                onClick={handleSaveMissingAppPath}
                disabled={appPathMissingBusy || !appPathDraft.trim()}
              >
                {t('common.save', '保存')}
              </button>
            </div>
          </div>
        </div>
      )}
      
      {/* 顶部固定拖拽区域 */}
      <div
        className="drag-region"
        data-tauri-drag-region
        onMouseDown={handleDragStart}
      />

      {/* 左侧悬浮导航 */}
      <SideNav
        page={page}
        setPage={setPage}
        onOpenPlatformLayout={openPlatformLayoutModal}
        easterEggClickCount={easterEggClickCount}
        onEasterEggTriggerClick={handleBreakoutEntryTriggerClick}
        hasBreakoutSession={hasBreakoutSession}
        updateActionState={updateAction.state}
        updateProgress={updateAction.progress}
        onUpdateActionClick={handleQuickUpdateActionClick}
        updateRemindersEnabled={updateRemindersEnabled}
        onOpenLogViewer={() => setShowLogViewer(true)}
      />

      {sideNavLayoutMode !== 'classic' && (
        <button
          className="log-entry-fab"
          onClick={() => setShowLogViewer(true)}
          title={t('manual.dataPrivacy.keywords.5', '日志')}
          aria-label={t('manual.dataPrivacy.keywords.5', '日志')}
        >
          <FileText size={18} />
        </button>
      )}

      <Suspense fallback={null}>
        <PlatformLayoutModal
          open={showPlatformLayoutModal}
          requestedEditGroupId={platformLayoutRequestedGroupId}
          onClose={() => {
            setShowPlatformLayoutModal(false);
            setPlatformLayoutRequestedGroupId(null);
          }}
        />
        <LogViewerModal
          open={showLogViewer}
          onClose={() => setShowLogViewer(false)}
        />
      </Suspense>

      <div className="main-wrapper">
        {/* overview 现在是合并后的账号总览页面 */}
        <Suspense fallback={suspenseFallback}>
          {page === 'dashboard' && (
            <DashboardPage
              onNavigate={setPage}
              onOpenPlatformLayout={openPlatformLayoutModal}
              onEasterEggTriggerClick={handleBreakoutEntryTriggerClick}
              topCenterBanner={
                topRightAdState.ad ? (
                  <div
                    className="global-promo-center"
                    role="complementary"
                    aria-label={t('common.topRightAd.ariaLabel', '全局右上角广告位')}
                  >
                    <div className="global-promo-slot">
                      <span className="global-ad-slot-badge">
                        {topRightAdState.ad.badge || t('common.topRightAd.badge', '广告')}
                      </span>
                      <div className="global-promo-main">
                        <p className="global-promo-text">{topRightAdState.ad.text}</p>
                      </div>
                      {topRightAdState.ad.ctaUrl ? (
                        <button className="global-ad-slot-action" onClick={handleTopRightAdClick}>
                          {topRightAdState.ad.ctaLabel || t('common.topRightAd.action', '查看详情')}
                        </button>
                      ) : null}
                    </div>
                  </div>
                ) : null
              }
            />
          )}
          {page === 'overview' && <AccountsPage onNavigate={setPage} />}
          {page === 'codex' && <CodexAccountsPage />}
          {page === 'github-copilot' && <GitHubCopilotAccountsPage />}
          {page === 'windsurf' && <WindsurfAccountsPage />}
          {page === 'kiro' && <KiroAccountsPage />}
          {page === 'cursor' && <CursorAccountsPage />}
          {page === 'gemini' && <GeminiAccountsPage />}
          {page === 'codebuddy' && <CodebuddyAccountsPage />}
          {page === 'codebuddy-cn' && <CodebuddyCnAccountsPage />}
          {page === 'qoder' && <QoderAccountsPage />}
          {page === 'trae' && <TraeAccountsPage />}
          {page === 'workbuddy' && <WorkbuddyAccountsPage />}
          {page === 'zed' && <ZedAccountsPage />}
          {page === 'instances' && <InstancesPage onNavigate={setPage} />}
          {page === 'fingerprints' && <FingerprintsPage onNavigate={setPage} />}
          {page === 'wakeup' && <WakeupTasksPage onNavigate={setPage} />}
          {page === 'verification' && <WakeupVerificationPage onNavigate={setPage} />}
          {page === '2fa' && <TwoFactorAuthPage />}
          {page === 'manual' && (
            <ManualPage
              onNavigate={setPage}
              onOpenPlatformLayout={openPlatformLayoutModal}
            />
          )}
          {page === 'settings' && <SettingsPage />}
        </Suspense>
      </div>
    </div>
  );
}

function App() {
  const windowLabel = getCurrentWindow().label;
  if (windowLabel === 'floating-card' || windowLabel.startsWith('instance-floating-card-')) {
    return <FloatingCardWindow />;
  }

  return <MainApp />;
}

export default App;
