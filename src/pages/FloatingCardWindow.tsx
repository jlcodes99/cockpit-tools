import { type MouseEvent as ReactMouseEvent, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ChevronLeft, ChevronRight, ExternalLink, Pin, PinOff, RefreshCw, Star, Undo2, X } from 'lucide-react';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { LogicalSize } from '@tauri-apps/api/dpi';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { invoke } from '@tauri-apps/api/core';
import { TauriEvent, listen } from '@tauri-apps/api/event';
import { useTranslation } from 'react-i18next';
import {
  buildAntigravityAccountPresentation,
  buildCodebuddyAccountPresentation,
  buildCodexAccountPresentation,
  buildCursorAccountPresentation,
  buildGeminiAccountPresentation,
  buildGitHubCopilotAccountPresentation,
  buildKiroAccountPresentation,
  buildQoderAccountPresentation,
  buildTraeAccountPresentation,
  buildWindsurfAccountPresentation,
  buildWorkbuddyAccountPresentation,
  buildZedAccountPresentation,
  UnifiedAccountPresentation,
} from '../presentation/platformAccountPresentation';
import { DisplayGroup, getDisplayGroups } from '../services/groupService';
import {
  getFloatingCardContext,
  hideCurrentFloatingCardWindow,
  hideFloatingCardWindow,
  saveFloatingCardPosition,
  setCurrentFloatingCardWindowAlwaysOnTop,
  setFloatingCardAlwaysOnTop,
  setFloatingCardConfirmOnClose,
  showInstanceFloatingCardWindow,
  showMainWindowAndNavigate,
  type FloatingCardInstanceContext,
} from '../services/floatingCardService';
import { useAccountStore } from '../stores/useAccountStore';
import { useCodebuddyAccountStore } from '../stores/useCodebuddyAccountStore';
import { useCodebuddyCnAccountStore } from '../stores/useCodebuddyCnAccountStore';
import { useCodexAccountStore } from '../stores/useCodexAccountStore';
import { useCursorAccountStore } from '../stores/useCursorAccountStore';
import { useGeminiAccountStore } from '../stores/useGeminiAccountStore';
import { useGitHubCopilotAccountStore } from '../stores/useGitHubCopilotAccountStore';
import { useKiroAccountStore } from '../stores/useKiroAccountStore';
import { usePlatformLayoutStore } from '../stores/usePlatformLayoutStore';
import { useQoderAccountStore } from '../stores/useQoderAccountStore';
import { useTraeAccountStore } from '../stores/useTraeAccountStore';
import { useWindsurfAccountStore } from '../stores/useWindsurfAccountStore';
import { useWorkbuddyAccountStore } from '../stores/useWorkbuddyAccountStore';
import { useZedAccountStore } from '../stores/useZedAccountStore';
import { useCodebuddyCnInstanceStore } from '../stores/useCodebuddyCnInstanceStore';
import { useCodebuddyInstanceStore } from '../stores/useCodebuddyInstanceStore';
import { useCodexInstanceStore } from '../stores/useCodexInstanceStore';
import { useCursorInstanceStore } from '../stores/useCursorInstanceStore';
import { useGeminiInstanceStore } from '../stores/useGeminiInstanceStore';
import { useGitHubCopilotInstanceStore } from '../stores/useGitHubCopilotInstanceStore';
import type { InstanceStoreState } from '../stores/createInstanceStore';
import { useInstanceStore } from '../stores/useInstanceStore';
import { useKiroInstanceStore } from '../stores/useKiroInstanceStore';
import { useQoderInstanceStore } from '../stores/useQoderInstanceStore';
import { useTraeInstanceStore } from '../stores/useTraeInstanceStore';
import { useWindsurfInstanceStore } from '../stores/useWindsurfInstanceStore';
import { useWorkbuddyInstanceStore } from '../stores/useWorkbuddyInstanceStore';
import { ALL_PLATFORM_IDS, PLATFORM_PAGE_MAP, PlatformId } from '../types/platform';
import type { InstanceProfile } from '../types/instance';
import { isPrivacyModeEnabledByDefault, maskSensitiveValue } from '../utils/privacy';
import { getPlatformLabel, renderPlatformIcon } from '../utils/platformMeta';
import {
  getRecommendedAntigravityAccount,
  getRecommendedCodebuddyAccount,
  getRecommendedCodebuddyCnAccount,
  getRecommendedCodexAccount,
  getRecommendedCursorAccount,
  getRecommendedGeminiAccount,
  getRecommendedGitHubCopilotAccount,
  getRecommendedKiroAccount,
  getRecommendedQoderAccount,
  getRecommendedTraeAccount,
  getRecommendedWindsurfAccount,
  getRecommendedWorkbuddyAccount,
  getRecommendedZedAccount,
  resolveCurrentOrMostRecentAccount,
} from '../utils/floatingCardSelectors';
import { changeLanguage, normalizeLanguage } from '../i18n';
import {
  ACCOUNTS_CHANGED_EVENT,
  CURRENT_ACCOUNT_CHANGED_EVENT,
  type AccountSyncEventPayload,
} from '../utils/accountSyncEvents';
import './FloatingCardWindow.css';

const windowInstance = getCurrentWindow();
const FLOATING_CARD_WINDOW_LABEL = 'floating-card';
const INSTANCE_FLOATING_CARD_WINDOW_LABEL_PREFIX = 'instance-floating-card-';
const FLOATING_CARD_PLATFORM_STORAGE_KEY = 'agtools.floating_card.platform';
const DEFAULT_INSTANCE_ID = '__default__';
const FLOATING_CARD_BASE_HEIGHT = 290;
const FLOATING_CARD_MAX_HEIGHT = 520;
const FLOATING_CARD_NO_DRAG_SELECTOR =
  'button, select, input, textarea, a, option, [role="button"], [data-floating-card-no-drag="true"]';

type FloatingCardGeneralConfig = {
  language: string;
  theme: string;
  ui_scale?: number;
  floating_card_always_on_top?: boolean;
  floating_card_confirm_on_close?: boolean;
};

type FloatingCardAccount =
  | ReturnType<typeof useAccountStore.getState>['accounts'][number]
  | ReturnType<typeof useCodexAccountStore.getState>['accounts'][number]
  | ReturnType<typeof useGitHubCopilotAccountStore.getState>['accounts'][number]
  | ReturnType<typeof useWindsurfAccountStore.getState>['accounts'][number]
  | ReturnType<typeof useKiroAccountStore.getState>['accounts'][number]
  | ReturnType<typeof useCursorAccountStore.getState>['accounts'][number]
  | ReturnType<typeof useGeminiAccountStore.getState>['accounts'][number]
  | ReturnType<typeof useCodebuddyAccountStore.getState>['accounts'][number]
  | ReturnType<typeof useCodebuddyCnAccountStore.getState>['accounts'][number]
  | ReturnType<typeof useQoderAccountStore.getState>['accounts'][number]
  | ReturnType<typeof useTraeAccountStore.getState>['accounts'][number]
  | ReturnType<typeof useWorkbuddyAccountStore.getState>['accounts'][number]
  | ReturnType<typeof useZedAccountStore.getState>['accounts'][number];

type FloatingCardInstanceStoreApi = Pick<
  InstanceStoreState,
  'refreshInstances' | 'updateInstance' | 'startInstance'
>;

function loadInitialPlatform(): PlatformId {
  try {
    const saved = localStorage.getItem(FLOATING_CARD_PLATFORM_STORAGE_KEY);
    if (saved && ALL_PLATFORM_IDS.includes(saved as PlatformId)) {
      return saved as PlatformId;
    }
  } catch {
    // ignore storage read failures
  }
  return 'antigravity';
}

function resolveCurrentAccountById<T extends { id: string }>(
  accounts: T[],
  currentId: string | null | undefined,
): T | null {
  if (!currentId) return null;
  return accounts.find((account) => account.id === currentId) ?? null;
}

function resolveAppliedTheme(theme: string): 'light' | 'dark' {
  if (theme === 'system') {
    return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  }
  return theme === 'dark' ? 'dark' : 'light';
}

function resolveInstanceStoreApi(platformId: PlatformId): FloatingCardInstanceStoreApi | null {
  switch (platformId) {
    case 'antigravity':
      return useInstanceStore.getState();
    case 'codex':
      return useCodexInstanceStore.getState();
    case 'github-copilot':
      return useGitHubCopilotInstanceStore.getState();
    case 'windsurf':
      return useWindsurfInstanceStore.getState();
    case 'kiro':
      return useKiroInstanceStore.getState();
    case 'cursor':
      return useCursorInstanceStore.getState();
    case 'gemini':
      return useGeminiInstanceStore.getState();
    case 'codebuddy':
      return useCodebuddyInstanceStore.getState();
    case 'codebuddy_cn':
      return useCodebuddyCnInstanceStore.getState();
    case 'qoder':
      return useQoderInstanceStore.getState();
    case 'trae':
      return useTraeInstanceStore.getState();
    case 'workbuddy':
      return useWorkbuddyInstanceStore.getState();
    case 'zed':
      return null;
  }
}

function findInstanceById(instances: InstanceProfile[], instanceId: string): InstanceProfile | null {
  return instances.find((instance) => instance.id === instanceId) ?? null;
}

export function FloatingCardWindow() {
  const { t } = useTranslation();
  const currentWindowLabel = windowInstance.label;
  const isPrimaryFloatingCardWindow = currentWindowLabel === FLOATING_CARD_WINDOW_LABEL;
  const isInstanceFloatingCardWindow = currentWindowLabel.startsWith(
    INSTANCE_FLOATING_CARD_WINDOW_LABEL_PREFIX,
  );
  const orderedPlatformIds = usePlatformLayoutStore((state) => state.orderedPlatformIds);
  const { accounts: agAccounts, currentAccount: agCurrent } = useAccountStore();
  const { accounts: codexAccounts, currentAccount: codexCurrent } = useCodexAccountStore();
  const {
    accounts: githubCopilotAccounts,
    currentAccountId: githubCopilotCurrentId,
  } = useGitHubCopilotAccountStore();
  const {
    accounts: windsurfAccounts,
    currentAccountId: windsurfCurrentId,
  } = useWindsurfAccountStore();
  const {
    accounts: kiroAccounts,
    currentAccountId: kiroCurrentId,
  } = useKiroAccountStore();
  const {
    accounts: cursorAccounts,
    currentAccountId: cursorCurrentId,
  } = useCursorAccountStore();
  const {
    accounts: geminiAccounts,
    currentAccountId: geminiCurrentId,
  } = useGeminiAccountStore();
  const {
    accounts: codebuddyAccounts,
    currentAccountId: codebuddyCurrentId,
  } = useCodebuddyAccountStore();
  const {
    accounts: codebuddyCnAccounts,
    currentAccountId: codebuddyCnCurrentId,
  } = useCodebuddyCnAccountStore();
  const {
    accounts: qoderAccounts,
    currentAccountId: qoderCurrentId,
  } = useQoderAccountStore();
  const {
    accounts: traeAccounts,
    currentAccountId: traeCurrentId,
  } = useTraeAccountStore();
  const {
    accounts: workbuddyAccounts,
    currentAccountId: workbuddyCurrentId,
  } = useWorkbuddyAccountStore();
  const {
    accounts: zedAccounts,
    currentAccountId: zedCurrentId,
  } = useZedAccountStore();
  const shellRef = useRef<HTMLDivElement | null>(null);
  const previousInstanceContextRef = useRef<FloatingCardInstanceContext | null>(null);
  const [displayGroups, setDisplayGroups] = useState<DisplayGroup[]>([]);
  const [selectedPlatform, setSelectedPlatform] = useState<PlatformId>(loadInitialPlatform);
  const [instanceContext, setInstanceContext] = useState<FloatingCardInstanceContext | null>(null);
  const [viewedAccountIds, setViewedAccountIds] = useState<Partial<Record<PlatformId, string | null>>>({});
  const [privacyModeEnabled, setPrivacyModeEnabled] = useState<boolean>(() =>
    isPrivacyModeEnabledByDefault(),
  );
  const [alwaysOnTop, setAlwaysOnTop] = useState(false);
  const [confirmOnClose, setConfirmOnClose] = useState(true);
  const [closeConfirmOpen, setCloseConfirmOpen] = useState(false);
  const [closeConfirmSkipPrompt, setCloseConfirmSkipPrompt] = useState(false);
  const [switchingAccountId, setSwitchingAccountId] = useState<string | null>(null);
  const [refreshingAccountId, setRefreshingAccountId] = useState<string | null>(null);
  const [errorText, setErrorText] = useState<string | null>(null);
  const [platformLoading, setPlatformLoading] = useState(false);

  const platformOrder = useMemo(() => {
    const seen = new Set<PlatformId>();
    const ordered: PlatformId[] = [];

    for (const platformId of orderedPlatformIds) {
      if (!ALL_PLATFORM_IDS.includes(platformId) || seen.has(platformId)) continue;
      ordered.push(platformId);
      seen.add(platformId);
    }

    for (const platformId of ALL_PLATFORM_IDS) {
      if (seen.has(platformId)) continue;
      ordered.push(platformId);
      seen.add(platformId);
    }

    return ordered;
  }, [orderedPlatformIds]);

  useEffect(() => {
    if (platformOrder.includes(selectedPlatform)) {
      return;
    }
    setSelectedPlatform(platformOrder[0] ?? 'antigravity');
  }, [platformOrder, selectedPlatform]);

  useEffect(() => {
    let disposed = false;
    let unlistenFocus: (() => void) | null = null;

    const syncFloatingCardContext = async () => {
      try {
        const nextContext = await getFloatingCardContext(currentWindowLabel);
        if (!disposed) {
          setInstanceContext(nextContext);
        }
      } catch (error) {
        console.error('Failed to load floating card context:', error);
      }
    };

    const bindFloatingCardContext = async () => {
      await syncFloatingCardContext();
      unlistenFocus = await listen(TauriEvent.WINDOW_FOCUS, async () => {
        await syncFloatingCardContext();
      });
    };

    void bindFloatingCardContext();

    return () => {
      disposed = true;
      unlistenFocus?.();
    };
  }, [currentWindowLabel]);

  useEffect(() => {
    const previousContext = previousInstanceContextRef.current;

    if (instanceContext) {
      if (selectedPlatform !== instanceContext.platformId) {
        setSelectedPlatform(instanceContext.platformId);
      }
      setViewedAccountIds((prev) => {
        if (prev[instanceContext.platformId] === instanceContext.boundAccountId) {
          return prev;
        }
        return {
          ...prev,
          [instanceContext.platformId]: instanceContext.boundAccountId,
        };
      });
    } else if (previousContext) {
      const restoredPlatform = loadInitialPlatform();
      if (selectedPlatform !== restoredPlatform) {
        setSelectedPlatform(restoredPlatform);
      }
    }

    previousInstanceContextRef.current = instanceContext;
  }, [instanceContext, selectedPlatform]);

  const syncPrivacyMode = useCallback(() => {
    setPrivacyModeEnabled(isPrivacyModeEnabledByDefault());
  }, []);

  const maskAccountText = useCallback(
    (value?: string | null) => maskSensitiveValue(value, privacyModeEnabled),
    [privacyModeEnabled],
  );
  const closeConfirmPath = useMemo(
    () =>
      isPrimaryFloatingCardWindow || !isInstanceFloatingCardWindow
        ? [
            t('nav.settings', '设置'),
            t('settings.general.commonTitle', '通用'),
            t('settings.general.floatingCardShowNowAction', '显示悬浮卡片'),
          ].join(' > ')
        : [
            t('instances.title', '多开实例'),
            instanceContext?.instanceName || t('instances.defaultName', '默认实例'),
          ].join(' > '),
    [instanceContext?.instanceName, isInstanceFloatingCardWindow, isPrimaryFloatingCardWindow, t],
  );

  const fetchPlatformData = useCallback(async (platformId: PlatformId) => {
    setPlatformLoading(true);
    try {
      switch (platformId) {
        case 'antigravity': {
          await Promise.allSettled([
            useAccountStore.getState().fetchAccounts(),
            useAccountStore.getState().fetchCurrentAccount(),
          ]);
          const groups = await getDisplayGroups();
          setDisplayGroups(groups);
          break;
        }
        case 'codex':
          await Promise.allSettled([
            useCodexAccountStore.getState().fetchAccounts(),
            useCodexAccountStore.getState().fetchCurrentAccount(),
          ]);
          break;
        case 'github-copilot':
          await useGitHubCopilotAccountStore.getState().fetchAccounts();
          break;
        case 'windsurf':
          await useWindsurfAccountStore.getState().fetchAccounts();
          break;
        case 'kiro':
          await useKiroAccountStore.getState().fetchAccounts();
          break;
        case 'cursor':
          await useCursorAccountStore.getState().fetchAccounts();
          break;
        case 'gemini':
          await useGeminiAccountStore.getState().fetchAccounts();
          break;
        case 'codebuddy':
          await useCodebuddyAccountStore.getState().fetchAccounts();
          break;
        case 'codebuddy_cn':
          await useCodebuddyCnAccountStore.getState().fetchAccounts();
          break;
        case 'qoder':
          await useQoderAccountStore.getState().fetchAccounts();
          break;
        case 'trae':
          await useTraeAccountStore.getState().fetchAccounts();
          break;
        case 'workbuddy':
          await useWorkbuddyAccountStore.getState().fetchAccounts();
          break;
        case 'zed':
          await useZedAccountStore.getState().fetchAccounts();
          break;
      }
    } finally {
      setPlatformLoading(false);
    }
  }, []);

  useEffect(() => {
    void fetchPlatformData(selectedPlatform);
  }, [fetchPlatformData, selectedPlatform]);

  useEffect(() => {
    let disposed = false;
    let unlistenAccountsChanged: (() => void) | null = null;
    let unlistenCurrentAccountChanged: (() => void) | null = null;

    const bindAccountSyncListeners = async () => {
      unlistenAccountsChanged = await listen<AccountSyncEventPayload>(
        ACCOUNTS_CHANGED_EVENT,
        async (event) => {
          const payload = event.payload;
          if (
            !payload ||
            payload.platformId !== selectedPlatform ||
            payload.sourceWindowLabel === currentWindowLabel
          ) {
            return;
          }
          await fetchPlatformData(payload.platformId);
        },
      );

      unlistenCurrentAccountChanged = await listen<AccountSyncEventPayload>(
        CURRENT_ACCOUNT_CHANGED_EVENT,
        async (event) => {
          const payload = event.payload;
          if (
            !payload ||
            payload.platformId !== selectedPlatform ||
            payload.sourceWindowLabel === currentWindowLabel ||
            instanceContext
          ) {
            return;
          }
          await fetchPlatformData(payload.platformId);
          if (disposed) return;
          setViewedAccountIds((prev) => ({
            ...prev,
            [payload.platformId]: payload.accountId ?? null,
          }));
        },
      );
    };

    void bindAccountSyncListeners();

    return () => {
      disposed = true;
      unlistenAccountsChanged?.();
      unlistenCurrentAccountChanged?.();
    };
  }, [
    currentWindowLabel,
    fetchPlatformData,
    instanceContext,
    selectedPlatform,
  ]);

  useEffect(() => {
    const rootElement = document.getElementById('root');
    document.documentElement.classList.add('floating-card-overlay-root');
    document.body.classList.add('floating-card-overlay-root');
    rootElement?.classList.add('floating-card-overlay-root');

    return () => {
      document.documentElement.classList.remove('floating-card-overlay-root');
      document.body.classList.remove('floating-card-overlay-root');
      rootElement?.classList.remove('floating-card-overlay-root');
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    let cleanupThemeWatcher: (() => void) | null = null;

    const applyTheme = (theme: string) => {
      const appliedTheme = resolveAppliedTheme(theme);
      document.documentElement.setAttribute('data-theme', appliedTheme);
      document.body.setAttribute('data-theme', appliedTheme);
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

    const loadGeneralConfig = async () => {
      try {
        const config = await invoke<FloatingCardGeneralConfig>('get_general_config');
        if (disposed) return;

        await changeLanguage(normalizeLanguage(config.language));
        applyTheme(config.theme);
        if (config.theme === 'system') {
          cleanupThemeWatcher = watchSystemTheme();
        }
        await getCurrentWebview().setZoom(
          typeof config.ui_scale === 'number' && Number.isFinite(config.ui_scale)
            ? Math.min(2, Math.max(0.8, config.ui_scale))
            : 1,
        );
        setAlwaysOnTop(Boolean(config.floating_card_always_on_top));
        setConfirmOnClose(config.floating_card_confirm_on_close !== false);
      } catch (error) {
        console.error('Failed to apply floating card config:', error);
      }
    };

    void loadGeneralConfig();

    return () => {
      disposed = true;
      cleanupThemeWatcher?.();
    };
  }, []);

  useEffect(() => {
    syncPrivacyMode();

    const handleVisibilityChange = () => {
      if (document.visibilityState === 'visible') {
        syncPrivacyMode();
      }
    };

    window.addEventListener('focus', syncPrivacyMode);
    window.addEventListener('storage', syncPrivacyMode);
    document.addEventListener('visibilitychange', handleVisibilityChange);

    return () => {
      window.removeEventListener('focus', syncPrivacyMode);
      window.removeEventListener('storage', syncPrivacyMode);
      document.removeEventListener('visibilitychange', handleVisibilityChange);
    };
  }, [syncPrivacyMode]);

  useEffect(() => {
    let unlistenCloseRequested: (() => void) | null = null;
    let unlistenLanguageChanged: (() => void) | null = null;
    let unlistenMoved: (() => void) | null = null;

    const bindWindowEvents = async () => {
      unlistenCloseRequested = await windowInstance.onCloseRequested((event) => {
        event.preventDefault();
        setErrorText(null);
        if (confirmOnClose) {
          setCloseConfirmSkipPrompt(false);
          setCloseConfirmOpen(true);
          return;
        }
        void (isPrimaryFloatingCardWindow
          ? hideFloatingCardWindow()
          : hideCurrentFloatingCardWindow()).catch((error) => {
          setErrorText(
            t('floatingCard.errors.closeFailed', {
              error: String(error),
              defaultValue: '关闭失败：{{error}}',
            }),
          );
        });
      });

      unlistenLanguageChanged = await listen<string>('settings:language_changed', (event) => {
        void changeLanguage(normalizeLanguage(event.payload));
      });

      unlistenMoved = await windowInstance.onMoved(({ payload }) => {
        if (!isPrimaryFloatingCardWindow) {
          return;
        }
        void saveFloatingCardPosition(payload.x, payload.y).catch((error) => {
          console.error('Failed to persist floating card position:', error);
        });
      });
    };

    void bindWindowEvents();

    return () => {
      unlistenCloseRequested?.();
      unlistenLanguageChanged?.();
      unlistenMoved?.();
    };
  }, [confirmOnClose, isPrimaryFloatingCardWindow, t]);

  useEffect(() => {
    if (instanceContext) {
      return;
    }
    try {
      localStorage.setItem(FLOATING_CARD_PLATFORM_STORAGE_KEY, selectedPlatform);
    } catch {
      // ignore storage write failures
    }
  }, [instanceContext, selectedPlatform]);

  const githubCopilotCurrent = useMemo(
    () => resolveCurrentOrMostRecentAccount(githubCopilotAccounts, githubCopilotCurrentId),
    [githubCopilotAccounts, githubCopilotCurrentId],
  );
  const windsurfCurrent = useMemo(
    () => resolveCurrentAccountById(windsurfAccounts, windsurfCurrentId),
    [windsurfAccounts, windsurfCurrentId],
  );
  const kiroCurrent = useMemo(
    () => resolveCurrentAccountById(kiroAccounts, kiroCurrentId),
    [kiroAccounts, kiroCurrentId],
  );
  const cursorCurrent = useMemo(
    () => resolveCurrentAccountById(cursorAccounts, cursorCurrentId),
    [cursorAccounts, cursorCurrentId],
  );
  const geminiCurrent = useMemo(
    () => resolveCurrentAccountById(geminiAccounts, geminiCurrentId),
    [geminiAccounts, geminiCurrentId],
  );
  const codebuddyCurrent = useMemo(
    () => resolveCurrentAccountById(codebuddyAccounts, codebuddyCurrentId),
    [codebuddyAccounts, codebuddyCurrentId],
  );
  const codebuddyCnCurrent = useMemo(
    () => resolveCurrentAccountById(codebuddyCnAccounts, codebuddyCnCurrentId),
    [codebuddyCnAccounts, codebuddyCnCurrentId],
  );
  const qoderCurrent = useMemo(
    () => resolveCurrentAccountById(qoderAccounts, qoderCurrentId),
    [qoderAccounts, qoderCurrentId],
  );
  const traeCurrent = useMemo(
    () => resolveCurrentAccountById(traeAccounts, traeCurrentId),
    [traeAccounts, traeCurrentId],
  );
  const workbuddyCurrent = useMemo(
    () => resolveCurrentAccountById(workbuddyAccounts, workbuddyCurrentId),
    [workbuddyAccounts, workbuddyCurrentId],
  );
  const zedCurrent = useMemo(
    () => resolveCurrentAccountById(zedAccounts, zedCurrentId),
    [zedAccounts, zedCurrentId],
  );

  const selectedState = useMemo(() => {
    switch (selectedPlatform) {
      case 'antigravity':
        return {
          accounts: agAccounts,
          actualCurrentAccount: agCurrent,
        };
      case 'codex':
        return {
          accounts: codexAccounts,
          actualCurrentAccount: codexCurrent,
        };
      case 'github-copilot':
        return {
          accounts: githubCopilotAccounts,
          actualCurrentAccount: githubCopilotCurrent,
        };
      case 'windsurf':
        return {
          accounts: windsurfAccounts,
          actualCurrentAccount: windsurfCurrent,
        };
      case 'kiro':
        return {
          accounts: kiroAccounts,
          actualCurrentAccount: kiroCurrent,
        };
      case 'cursor':
        return {
          accounts: cursorAccounts,
          actualCurrentAccount: cursorCurrent,
        };
      case 'gemini':
        return {
          accounts: geminiAccounts,
          actualCurrentAccount: geminiCurrent,
        };
      case 'codebuddy':
        return {
          accounts: codebuddyAccounts,
          actualCurrentAccount: codebuddyCurrent,
        };
      case 'codebuddy_cn':
        return {
          accounts: codebuddyCnAccounts,
          actualCurrentAccount: codebuddyCnCurrent,
        };
      case 'qoder':
        return {
          accounts: qoderAccounts,
          actualCurrentAccount: qoderCurrent,
        };
      case 'trae':
        return {
          accounts: traeAccounts,
          actualCurrentAccount: traeCurrent,
        };
      case 'workbuddy':
        return {
          accounts: workbuddyAccounts,
          actualCurrentAccount: workbuddyCurrent,
        };
      case 'zed':
        return {
          accounts: zedAccounts,
          actualCurrentAccount: zedCurrent,
        };
    }
  }, [
    agAccounts,
    agCurrent,
    codebuddyAccounts,
    codebuddyCnAccounts,
    codebuddyCnCurrent,
    codebuddyCurrent,
    codexAccounts,
    codexCurrent,
    cursorAccounts,
    cursorCurrent,
    geminiAccounts,
    geminiCurrent,
    githubCopilotAccounts,
    githubCopilotCurrent,
    kiroAccounts,
    kiroCurrent,
    qoderAccounts,
    qoderCurrent,
    selectedPlatform,
    traeAccounts,
    traeCurrent,
    windsurfAccounts,
    windsurfCurrent,
    workbuddyAccounts,
    workbuddyCurrent,
    zedAccounts,
    zedCurrent,
  ]);

  const accounts = selectedState.accounts as FloatingCardAccount[];
  const actualCurrentAccount = selectedState.actualCurrentAccount as FloatingCardAccount | null;
  const currentAccount = useMemo(() => {
    if (!instanceContext || instanceContext.platformId !== selectedPlatform) {
      return actualCurrentAccount;
    }
    return (
      accounts.find((account) => account.id === instanceContext.boundAccountId) ?? null
    );
  }, [accounts, actualCurrentAccount, instanceContext, selectedPlatform]);
  const recommendedAccount = useMemo(() => {
    const effectiveCurrentId = currentAccount?.id;
    switch (selectedPlatform) {
      case 'antigravity':
        return getRecommendedAntigravityAccount(agAccounts, effectiveCurrentId);
      case 'codex':
        return getRecommendedCodexAccount(codexAccounts, effectiveCurrentId);
      case 'github-copilot':
        return getRecommendedGitHubCopilotAccount(githubCopilotAccounts, effectiveCurrentId);
      case 'windsurf':
        return getRecommendedWindsurfAccount(windsurfAccounts, effectiveCurrentId);
      case 'kiro':
        return getRecommendedKiroAccount(kiroAccounts, effectiveCurrentId);
      case 'cursor':
        return getRecommendedCursorAccount(cursorAccounts, effectiveCurrentId);
      case 'gemini':
        return getRecommendedGeminiAccount(geminiAccounts, effectiveCurrentId);
      case 'codebuddy':
        return getRecommendedCodebuddyAccount(codebuddyAccounts, effectiveCurrentId);
      case 'codebuddy_cn':
        return getRecommendedCodebuddyCnAccount(codebuddyCnAccounts, effectiveCurrentId);
      case 'qoder':
        return getRecommendedQoderAccount(qoderAccounts, effectiveCurrentId);
      case 'trae':
        return getRecommendedTraeAccount(traeAccounts, effectiveCurrentId);
      case 'workbuddy':
        return getRecommendedWorkbuddyAccount(workbuddyAccounts, effectiveCurrentId);
      case 'zed':
        return getRecommendedZedAccount(zedAccounts, effectiveCurrentId);
    }
  }, [
    agAccounts,
    codebuddyAccounts,
    codebuddyCnAccounts,
    codexAccounts,
    currentAccount?.id,
    cursorAccounts,
    geminiAccounts,
    githubCopilotAccounts,
    kiroAccounts,
    qoderAccounts,
    selectedPlatform,
    traeAccounts,
    windsurfAccounts,
    workbuddyAccounts,
    zedAccounts,
  ]) as FloatingCardAccount | null;
  const viewedAccountId = viewedAccountIds[selectedPlatform] ?? null;
  const viewedAccount = useMemo(() => {
    if (accounts.length === 0) return null;
    if (viewedAccountId) {
      const target = accounts.find((account) => account.id === viewedAccountId);
      if (target) return target;
    }
    return currentAccount ?? accounts[0] ?? null;
  }, [accounts, currentAccount, instanceContext, viewedAccountId]);

  useEffect(() => {
    if (accounts.length === 0) {
      setViewedAccountIds((prev) => ({ ...prev, [selectedPlatform]: null }));
      return;
    }

    setViewedAccountIds((prev) => {
      const existingId = prev[selectedPlatform];
      if (existingId && accounts.some((account) => account.id === existingId)) {
        return prev;
      }
      const fallbackId = currentAccount?.id ?? accounts[0]?.id ?? null;
      if (existingId === fallbackId) {
        return prev;
      }
      return {
        ...prev,
        [selectedPlatform]: fallbackId,
      };
    });
  }, [accounts, currentAccount?.id, selectedPlatform]);

  const accountIndex = useMemo(() => {
    if (!viewedAccount) return -1;
    return accounts.findIndex((account) => account.id === viewedAccount.id);
  }, [accounts, viewedAccount]);

  const presentation = useMemo<UnifiedAccountPresentation | null>(() => {
    if (!viewedAccount) return null;
    switch (selectedPlatform) {
      case 'antigravity':
        return buildAntigravityAccountPresentation(viewedAccount as typeof agAccounts[number], displayGroups, t);
      case 'codex':
        return buildCodexAccountPresentation(viewedAccount as typeof codexAccounts[number], t);
      case 'github-copilot':
        return buildGitHubCopilotAccountPresentation(viewedAccount as typeof githubCopilotAccounts[number], t);
      case 'windsurf':
        return buildWindsurfAccountPresentation(viewedAccount as typeof windsurfAccounts[number], t);
      case 'kiro':
        return buildKiroAccountPresentation(viewedAccount as typeof kiroAccounts[number], t);
      case 'cursor':
        return buildCursorAccountPresentation(viewedAccount as typeof cursorAccounts[number], t);
      case 'gemini':
        return buildGeminiAccountPresentation(viewedAccount as typeof geminiAccounts[number], t);
      case 'codebuddy':
        return buildCodebuddyAccountPresentation(viewedAccount as typeof codebuddyAccounts[number], t);
      case 'codebuddy_cn':
        return buildCodebuddyAccountPresentation(viewedAccount as typeof codebuddyCnAccounts[number], t);
      case 'qoder':
        return buildQoderAccountPresentation(viewedAccount as typeof qoderAccounts[number], t);
      case 'trae':
        return buildTraeAccountPresentation(viewedAccount as typeof traeAccounts[number], t);
      case 'workbuddy':
        return buildWorkbuddyAccountPresentation(viewedAccount as typeof workbuddyAccounts[number], t);
      case 'zed':
        return buildZedAccountPresentation(viewedAccount as typeof zedAccounts[number], t);
    }
  }, [
    agAccounts,
    codebuddyAccounts,
    codebuddyCnAccounts,
    codexAccounts,
    cursorAccounts,
    displayGroups,
    geminiAccounts,
    githubCopilotAccounts,
    kiroAccounts,
    qoderAccounts,
    selectedPlatform,
    t,
    traeAccounts,
    viewedAccount,
    windsurfAccounts,
    workbuddyAccounts,
    zedAccounts,
  ]);

  const isCurrentViewed = Boolean(viewedAccount?.id && viewedAccount.id === currentAccount?.id);
  const visibleQuotaItemLimit = selectedPlatform === 'antigravity' ? 3 : 2;
  const visibleQuotaItems = presentation?.quotaItems.slice(0, visibleQuotaItemLimit) ?? [];
  const accountStateLabel = viewedAccount
    ? isCurrentViewed
      ? t('floatingCard.currentAccount', '当前账号')
      : t('floatingCard.accountPreview', '账号预览')
    : null;
  const floatingCardTitle = instanceContext?.instanceName || t('instances.defaultName', '默认实例');
  const platformLabel = getPlatformLabel(selectedPlatform, t);
  const platformLocked = Boolean(instanceContext);

  const selectAccount = useCallback((platformId: PlatformId, accountId: string | null) => {
    setViewedAccountIds((prev) => ({ ...prev, [platformId]: accountId }));
    setErrorText(null);
  }, []);

  const handleMoveAccount = useCallback((direction: -1 | 1) => {
    if (accountIndex < 0 || accounts.length <= 1) return;
    const nextIndex = (accountIndex + direction + accounts.length) % accounts.length;
    selectAccount(selectedPlatform, accounts[nextIndex].id);
  }, [accountIndex, accounts, selectAccount, selectedPlatform]);

  const refreshDisplayedAccount = useCallback(
    async (options?: { silent?: boolean }) => {
      if (!viewedAccount || refreshingAccountId || switchingAccountId) return;

      const silent = options?.silent === true;
      if (!silent) {
        setRefreshingAccountId(viewedAccount.id);
        setErrorText(null);
      }

      try {
        switch (selectedPlatform) {
          case 'antigravity':
            await useAccountStore.getState().refreshQuota(viewedAccount.id);
            break;
          case 'codex':
            await useCodexAccountStore.getState().refreshQuota(viewedAccount.id);
            break;
          case 'github-copilot':
            await useGitHubCopilotAccountStore.getState().refreshToken(viewedAccount.id);
            break;
          case 'windsurf':
            await useWindsurfAccountStore.getState().refreshToken(viewedAccount.id);
            break;
          case 'kiro':
            await useKiroAccountStore.getState().refreshToken(viewedAccount.id);
            break;
          case 'cursor':
            await useCursorAccountStore.getState().refreshToken(viewedAccount.id);
            break;
          case 'gemini':
            await useGeminiAccountStore.getState().refreshToken(viewedAccount.id);
            break;
          case 'codebuddy':
            await useCodebuddyAccountStore.getState().refreshToken(viewedAccount.id);
            break;
          case 'codebuddy_cn':
            await useCodebuddyCnAccountStore.getState().refreshToken(viewedAccount.id);
            break;
          case 'qoder':
            await useQoderAccountStore.getState().refreshToken(viewedAccount.id);
            break;
          case 'trae':
            await useTraeAccountStore.getState().refreshToken(viewedAccount.id);
            break;
          case 'workbuddy':
            await useWorkbuddyAccountStore.getState().refreshToken(viewedAccount.id);
            break;
          case 'zed':
            await useZedAccountStore.getState().refreshToken(viewedAccount.id);
            break;
        }
      } catch (error) {
        if (!silent) {
          setErrorText(
            t('floatingCard.errors.refreshFailed', {
              error: String(error),
              defaultValue: '刷新失败：{{error}}',
            }),
          );
        }
      } finally {
        if (!silent) {
          setRefreshingAccountId(null);
        }
      }
    },
    [refreshingAccountId, selectedPlatform, switchingAccountId, t, viewedAccount],
  );

  const handleRefresh = useCallback(async () => {
    await refreshDisplayedAccount();
  }, [refreshDisplayedAccount]);

  useEffect(() => {
    if (!viewedAccount) {
      return;
    }

    const timerId = window.setInterval(() => {
      void refreshDisplayedAccount({ silent: true });
    }, 60_000);

    return () => {
      window.clearInterval(timerId);
    };
  }, [refreshDisplayedAccount, viewedAccount]);

  const handleSwitch = useCallback(async () => {
    if (!viewedAccount || switchingAccountId || isCurrentViewed) return;
    setSwitchingAccountId(viewedAccount.id);
    setErrorText(null);
    try {
      if (instanceContext) {
        const instanceStore = resolveInstanceStoreApi(selectedPlatform);
        if (!instanceStore) {
          throw new Error(t('common.shared.instances.unsupported.title', '暂不支持当前系统'));
        }
        const instances = await instanceStore.refreshInstances();
        const targetInstance = findInstanceById(instances, instanceContext.instanceId);
        const wasRunning = targetInstance?.running === true;
        await instanceStore.updateInstance({
          instanceId: instanceContext.instanceId,
          bindAccountId: viewedAccount.id,
          followLocalAccount: instanceContext.instanceId === DEFAULT_INSTANCE_ID ? false : undefined,
        });
        if (wasRunning) {
          await instanceStore.startInstance(instanceContext.instanceId);
        }
        const nextContext = {
          ...instanceContext,
          boundAccountId: viewedAccount.id,
        };
        setInstanceContext(nextContext);
        await showInstanceFloatingCardWindow(nextContext);
      } else {
        switch (selectedPlatform) {
          case 'antigravity':
            await useAccountStore.getState().switchAccount(viewedAccount.id);
            await useAccountStore.getState().fetchCurrentAccount();
            break;
          case 'codex':
            await useCodexAccountStore.getState().switchAccount(viewedAccount.id);
            await useCodexAccountStore.getState().fetchCurrentAccount();
            break;
          case 'github-copilot':
            await useGitHubCopilotAccountStore.getState().switchAccount(viewedAccount.id);
            break;
          case 'windsurf':
            await useWindsurfAccountStore.getState().switchAccount(viewedAccount.id);
            break;
          case 'kiro':
            await useKiroAccountStore.getState().switchAccount(viewedAccount.id);
            break;
          case 'cursor':
            await useCursorAccountStore.getState().switchAccount(viewedAccount.id);
            break;
          case 'gemini':
            await useGeminiAccountStore.getState().switchAccount(viewedAccount.id);
            break;
          case 'codebuddy':
            await useCodebuddyAccountStore.getState().switchAccount(viewedAccount.id);
            break;
          case 'codebuddy_cn':
            await useCodebuddyCnAccountStore.getState().switchAccount(viewedAccount.id);
            break;
          case 'qoder':
            await useQoderAccountStore.getState().switchAccount(viewedAccount.id);
            break;
          case 'trae':
            await useTraeAccountStore.getState().switchAccount(viewedAccount.id);
            break;
          case 'workbuddy':
            await useWorkbuddyAccountStore.getState().switchAccount(viewedAccount.id);
            break;
          case 'zed':
            await useZedAccountStore.getState().switchAccount(viewedAccount.id);
            break;
        }
      }
      selectAccount(selectedPlatform, viewedAccount.id);
    } catch (error) {
      setErrorText(
        t('floatingCard.errors.switchFailed', {
          error: String(error),
          defaultValue: '切换失败：{{error}}',
        }),
      );
    } finally {
      setSwitchingAccountId(null);
    }
  }, [
    isCurrentViewed,
    instanceContext,
    selectAccount,
    selectedPlatform,
    switchingAccountId,
    t,
    viewedAccount,
  ]);

  const handleOpenDetails = useCallback(async () => {
    await showMainWindowAndNavigate(PLATFORM_PAGE_MAP[selectedPlatform]);
  }, [selectedPlatform]);

  const handleTogglePin = useCallback(async () => {
    const nextValue = !alwaysOnTop;
    setAlwaysOnTop(nextValue);
    try {
      if (isPrimaryFloatingCardWindow) {
        await setFloatingCardAlwaysOnTop(nextValue);
      } else {
        await setCurrentFloatingCardWindowAlwaysOnTop(nextValue);
      }
    } catch (error) {
      setAlwaysOnTop(!nextValue);
      setErrorText(
        t('floatingCard.errors.pinFailed', {
          error: String(error),
          defaultValue: '置顶设置失败：{{error}}',
        }),
      );
    }
  }, [alwaysOnTop, isPrimaryFloatingCardWindow, t]);

  const hideCurrentFloatingCard = useCallback(async (): Promise<boolean> => {
    try {
      if (isPrimaryFloatingCardWindow) {
        await hideFloatingCardWindow();
      } else {
        await hideCurrentFloatingCardWindow();
      }
      return true;
    } catch (error) {
      setErrorText(
        t('floatingCard.errors.closeFailed', {
          error: String(error),
          defaultValue: '关闭失败：{{error}}',
        }),
      );
      return false;
    }
  }, [isPrimaryFloatingCardWindow, t]);

  const requestCloseWindow = useCallback(async () => {
    setErrorText(null);
    if (confirmOnClose) {
      setCloseConfirmSkipPrompt(false);
      setCloseConfirmOpen(true);
      return;
    }
    await hideCurrentFloatingCard();
  }, [confirmOnClose, hideCurrentFloatingCard]);

  const handleCloseConfirmDismiss = useCallback(() => {
    setCloseConfirmOpen(false);
    setCloseConfirmSkipPrompt(false);
  }, []);

  const handleCloseConfirmAccept = useCallback(async () => {
    if (isPrimaryFloatingCardWindow && closeConfirmSkipPrompt && confirmOnClose) {
      try {
        await setFloatingCardConfirmOnClose(false);
        setConfirmOnClose(false);
      } catch (error) {
        console.error('Failed to persist floating card close confirmation preference:', error);
      }
    }

    const hidden = await hideCurrentFloatingCard();
    if (!hidden) {
      return;
    }
    setCloseConfirmOpen(false);
    setCloseConfirmSkipPrompt(false);
  }, [closeConfirmSkipPrompt, confirmOnClose, hideCurrentFloatingCard, isPrimaryFloatingCardWindow]);

  const handleWindowDragStart = useCallback((event: ReactMouseEvent<HTMLElement>) => {
    if (event.button !== 0) return;
    const target = event.target instanceof Element ? event.target : null;
    if (target?.closest(FLOATING_CARD_NO_DRAG_SELECTOR)) {
      return;
    }
    event.preventDefault();
    void windowInstance.startDragging().catch((error) => {
      console.error('Failed to start floating card dragging:', error);
    });
  }, []);

  useEffect(() => {
    const shell = shellRef.current;
    if (!shell) return;

    let cancelled = false;
    const frameId = window.requestAnimationFrame(() => {
      if (cancelled) return;
      const contentHeight = Math.max(
        shell.scrollHeight,
        document.documentElement.scrollHeight,
        document.body.scrollHeight,
      );
      const targetHeight = Math.max(
        FLOATING_CARD_BASE_HEIGHT,
        Math.min(FLOATING_CARD_MAX_HEIGHT, Math.ceil(contentHeight)),
      );
      if (Math.abs(targetHeight - window.innerHeight) <= 2) {
        return;
      }

      void windowInstance
        .setSize(new LogicalSize(window.innerWidth, targetHeight))
        .catch((error) => console.error('Failed to resize floating card window:', error));
    });

    return () => {
      cancelled = true;
      window.cancelAnimationFrame(frameId);
    };
  }, [
    accountIndex,
    accounts.length,
    currentAccount?.id,
    errorText,
    isCurrentViewed,
    platformLoading,
    presentation,
    recommendedAccount?.id,
    refreshingAccountId,
    selectedPlatform,
    switchingAccountId,
    viewedAccount?.id,
  ]);

  return (
    <div className="floating-card-window">
      <div className="floating-card-shell" ref={shellRef} onMouseDown={handleWindowDragStart}>
        <div className="floating-card-header">
          <div className="floating-card-header-main">
            <span className="floating-card-platform-icon">{renderPlatformIcon(selectedPlatform, 18)}</span>
            <div className="floating-card-header-title">{floatingCardTitle}</div>
          </div>

          <div className="floating-card-header-actions">
            <button
              className="floating-card-icon-button"
              type="button"
              onClick={() => void handleTogglePin()}
              title={alwaysOnTop ? t('floatingCard.actions.unpin', '取消置顶') : t('floatingCard.actions.pin', '置顶')}
              aria-label={alwaysOnTop ? t('floatingCard.actions.unpin', '取消置顶') : t('floatingCard.actions.pin', '置顶')}
            >
              {alwaysOnTop ? <PinOff size={15} /> : <Pin size={15} />}
            </button>
            <button
              className="floating-card-icon-button"
              type="button"
              onClick={() => void requestCloseWindow()}
              title={t('floatingCard.actions.close', '关闭')}
              aria-label={t('floatingCard.actions.close', '关闭')}
            >
              <X size={15} />
            </button>
          </div>
        </div>

        <div className="floating-card-body">
          <div className="floating-card-hud-strip">
            <div className="floating-card-platform-slot">
              {platformLocked ? (
                <div
                  className="floating-card-platform-lock"
                  title={platformLabel}
                  aria-label={t('floatingCard.lockedPlatform', '实例已锁定平台')}
                >
                  {platformLabel}
                </div>
              ) : (
                <select
                  className="floating-card-platform-select"
                  value={selectedPlatform}
                  onChange={(event) => setSelectedPlatform(event.target.value as PlatformId)}
                  aria-label={t('floatingCard.selectPlatform', '切换平台')}
                >
                  {platformOrder.map((platformId) => (
                    <option key={platformId} value={platformId}>
                      {getPlatformLabel(platformId, t)}
                    </option>
                  ))}
                </select>
              )}
            </div>

            <div className="floating-card-pager">
              <button
                className="floating-card-nav-button"
                type="button"
                onClick={() => handleMoveAccount(-1)}
                disabled={accountIndex < 0 || accounts.length <= 1}
                aria-label={t('floatingCard.actions.previousAccount', '上一个账号')}
              >
                <ChevronLeft size={15} />
              </button>
              <span className="floating-card-pager-text">
                {accounts.length > 0
                  ? t('floatingCard.pager', {
                      current: Math.max(1, accountIndex + 1),
                      total: accounts.length,
                      defaultValue: '{{current}} / {{total}}',
                    })
                  : '-- / --'}
              </span>
              <button
                className="floating-card-nav-button"
                type="button"
                onClick={() => handleMoveAccount(1)}
                disabled={accountIndex < 0 || accounts.length <= 1}
                aria-label={t('floatingCard.actions.nextAccount', '下一个账号')}
              >
                <ChevronRight size={15} />
              </button>
            </div>

            {accountStateLabel ? (
              <div className="floating-card-state-strip">
                <span className="floating-card-state-pill">{accountStateLabel}</span>
              </div>
            ) : null}
          </div>

          {viewedAccount && presentation ? (
            <div className="floating-card-account">
              <div className="floating-card-account-head">
                <div className="floating-card-account-title">
                  <div className="floating-card-account-name">
                    {maskAccountText(presentation.displayName)}
                  </div>
                  {presentation.cycleText ? (
                    <div className="floating-card-account-subline">
                      <span className="floating-card-section-label">
                        {t('floatingCard.cycle', '周期')}
                      </span>
                      <span className="floating-card-inline-value">{presentation.cycleText}</span>
                    </div>
                  ) : null}
                </div>
                <span className={`floating-card-plan floating-card-plan--${presentation.planClass || 'unknown'}`}>
                  {presentation.planLabel || '--'}
                </span>
              </div>

              <div className="floating-card-inline-meta">
                {presentation.sublineText ? (
                  <div className="floating-card-meta-pill">
                    <span className="floating-card-section-label">
                      {t('floatingCard.status', '状态')}
                    </span>
                    <span
                      className={`floating-card-inline-value floating-card-inline-value--${presentation.sublineClass || 'neutral'}`}
                    >
                      {presentation.sublineText}
                    </span>
                  </div>
                ) : null}
              </div>

              <div className="floating-card-quota-panel">
                {visibleQuotaItems.length > 0 ? (
                  visibleQuotaItems.map((item) => {
                    const progressPercent = Math.max(
                      0,
                      Math.min(100, item.progressPercent ?? item.percentage ?? 0),
                    );
                    return (
                      <div key={item.key} className="floating-card-quota-row">
                        <div className="floating-card-quota-top">
                          <span className="floating-card-quota-label">{item.label}</span>
                          <span className={`floating-card-quota-value floating-card-quota-value--${item.quotaClass || 'high'}`}>
                            {item.valueText || '--'}
                          </span>
                        </div>
                        {item.showProgress !== false ? (
                          <div className="floating-card-progress-track">
                            <div
                              className={`floating-card-progress-bar floating-card-progress-bar--${item.quotaClass || 'high'}`}
                              style={{ width: `${progressPercent}%` }}
                            />
                          </div>
                        ) : null}
                        {item.resetText ? (
                          <div className="floating-card-quota-reset">{item.resetText}</div>
                        ) : null}
                      </div>
                    );
                  })
                ) : (
                  <div className="floating-card-empty-text">
                    {t('common.shared.quota.noData', '暂无配额数据')}
                  </div>
                )}
              </div>
            </div>
          ) : (
            <div className="floating-card-empty-state">
              <div className="floating-card-section-label">
                {platformLoading ? t('common.loading', '加载中...') : t('floatingCard.empty.title', '暂无账号')}
              </div>
              <div className="floating-card-empty-text">
                {platformLoading
                  ? t('floatingCard.empty.loading', '正在读取当前平台账号信息')
                  : t('floatingCard.empty.desc', '当前平台还没有可展示的账号')}
              </div>
            </div>
          )}

          {errorText ? <div className="floating-card-error">{errorText}</div> : null}
        </div>

        <div className="floating-card-footer">
          <div className="floating-card-primary-actions">
            {isCurrentViewed &&
            recommendedAccount &&
            recommendedAccount.id !== currentAccount?.id ? (
              <button
                className="floating-card-button floating-card-button--secondary"
                type="button"
                onClick={() => selectAccount(selectedPlatform, recommendedAccount.id)}
              >
                <Star size={14} />
                {t('floatingCard.actions.viewRecommended', '查看推荐账号')}
              </button>
            ) : null}
            {!isCurrentViewed && currentAccount ? (
              <button
                className="floating-card-button floating-card-button--secondary"
                type="button"
                onClick={() => selectAccount(selectedPlatform, currentAccount.id)}
              >
                <Undo2 size={14} />
                {t('floatingCard.actions.backToCurrent', '回到当前账号')}
              </button>
            ) : null}
            {!isCurrentViewed && viewedAccount ? (
              <button
                className="floating-card-button floating-card-button--primary"
                type="button"
                onClick={() => void handleSwitch()}
                disabled={switchingAccountId === viewedAccount.id}
              >
                {switchingAccountId === viewedAccount.id ? (
                  <RefreshCw size={14} className="floating-card-spin" />
                ) : null}
                {t('floatingCard.actions.switchToThisAccount', '切换到此账号')}
              </button>
            ) : null}
          </div>

          <div className="floating-card-secondary-actions">
            <button
              className="floating-card-button floating-card-button--ghost floating-card-button--icon"
              type="button"
              onClick={() => void handleRefresh()}
              disabled={!viewedAccount || Boolean(refreshingAccountId)}
              title={t('common.refresh', '刷新')}
              aria-label={t('common.refresh', '刷新')}
            >
              <RefreshCw size={14} className={refreshingAccountId ? 'floating-card-spin' : undefined} />
            </button>
            <button
              className="floating-card-button floating-card-button--ghost floating-card-button--icon"
              type="button"
              onClick={() => void handleOpenDetails()}
              title={t('floatingCard.actions.openDetails', '打开详情页')}
              aria-label={t('floatingCard.actions.openDetails', '打开详情页')}
            >
              <ExternalLink size={14} />
            </button>
          </div>
        </div>

        {closeConfirmOpen ? (
          <div className="floating-card-close-confirm-backdrop" data-floating-card-no-drag="true">
            <div className="floating-card-close-confirm" data-floating-card-no-drag="true" role="dialog" aria-modal="true">
              <div className="floating-card-close-confirm-title">
                {t('floatingCard.closeConfirm.title', '关闭悬浮卡片？')}
              </div>
              <div className="floating-card-close-confirm-message">
                {t('floatingCard.closeConfirm.message', {
                  path: closeConfirmPath,
                  defaultValue: '关闭后可在“{{path}}”中重新打开。',
                })}
              </div>
              <label className="floating-card-close-confirm-checkbox" data-floating-card-no-drag="true">
                <input
                  type="checkbox"
                  checked={closeConfirmSkipPrompt}
                  onChange={(event) => setCloseConfirmSkipPrompt(event.target.checked)}
                />
                <span>{t('floatingCard.closeConfirm.dontAskAgain', '不再提示')}</span>
              </label>
              <div className="floating-card-close-confirm-actions">
                <button
                  className="floating-card-button floating-card-button--secondary"
                  type="button"
                  onClick={handleCloseConfirmDismiss}
                >
                  {t('common.cancel', '取消')}
                </button>
                <button
                  className="floating-card-button floating-card-button--primary"
                  type="button"
                  onClick={() => void handleCloseConfirmAccept()}
                >
                  {t('floatingCard.actions.close', '关闭')}
                </button>
              </div>
            </div>
          </div>
        ) : null}
      </div>
    </div>
  );
}
