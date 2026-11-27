import { createElement, useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { listen } from '@tauri-apps/api/event';
import { useTranslation } from 'react-i18next';
import { useDurableUiBooleanSetting } from '../../hooks/useDurableUiSetting';
import * as service from '../../services/t3codeService';
import { getAccountsForDriver, getDefaultProvider, getT3codePlatformSpec } from './t3codePlatformRegistry';
import { useAccountStore } from '../../stores/useAccountStore';
import { useClaudeAccountStore } from '../../stores/useClaudeAccountStore';
import { useCodexAccountStore } from '../../stores/useCodexAccountStore';
import { useT3codeInstanceStore } from '../../stores/useT3codeInstanceStore';
import type {
  T3CodeDriverKind,
  T3CodeProviderInstanceView,
  T3CodeProviderRuntimeView,
  T3CodeProviderWorkspaceView,
  T3CodeQuotaWindowView,
} from '../../types/t3code';
import {
  isPrivacyModeEnabledByDefault,
  maskSensitiveValue,
  persistPrivacyModeEnabled,
} from '../../utils/privacy';
import {
  T3codeAccountPlanBadge,
  makeProviderSlug,
  resolveEffectiveRuntime,
  resolveProviderRowStatus,
  suggestInstanceName,
  type T3codeResolvedRowStatus,
} from './t3codePresentation';

export const DEFAULT_T3_INSTANCE = '__default__';
const T3CODE_FLOW_NOTICE_COLLAPSED_KEY = 'agtools.t3code.flow_notice_collapsed';

export interface T3codeProviderRowView {
  instance: T3CodeProviderInstanceView;
  runtime: T3CodeProviderRuntimeView | undefined;
  status: T3codeResolvedRowStatus;
}

export interface T3codeBannerMessage {
  tone: 'success' | 'error';
  text: string;
}

export function useT3codeWorkspaceController() {
  const { t } = useTranslation();
  const startInstance = useT3codeInstanceStore((state) => state.startInstance);
  const instances = useT3codeInstanceStore((state) => state.instances);
  const fetchInstances = useT3codeInstanceStore((state) => state.fetchInstances);
  const [targetId, setTargetId] = useState(DEFAULT_T3_INSTANCE);

  const [workspace, setWorkspace] = useState<T3CodeProviderWorkspaceView | null>(null);
  const [apiServiceQuota, setApiServiceQuota] = useState<T3CodeQuotaWindowView[]>([]);
  const apiServiceQuotaPending = useRef(false);
  const [liveRuntime, setLiveRuntime] = useState<{
    target: string;
    rows: Map<string, T3CodeProviderRuntimeView>;
  } | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState('');
  const [message, setMessage] = useState<T3codeBannerMessage | null>(null);
  const [busyInstanceId, setBusyInstanceId] = useState<string | null>(null);
  const [refreshingAll, setRefreshingAll] = useState(false);
  const [removalPendingAt, setRemovalPendingAt] = useState<number>();
  const [restarting, setRestarting] = useState(false);

  useEffect(() => {
    void fetchInstances().catch(() => {});
  }, [fetchInstances]);

  const selectTarget = (id: string) => {
    setMessage(null);
    setLoadError('');
    setRemovalPendingAt(undefined);
    setTargetId(id);
  };

  const [isFlowNoticeCollapsed, setIsFlowNoticeCollapsed] = useDurableUiBooleanSetting(
    T3CODE_FLOW_NOTICE_COLLAPSED_KEY,
  );
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

  const sequence = useRef(0);
  const antigravityAccounts = useAccountStore((state) => state.accounts);
  const codexAccounts = useCodexAccountStore((state) => state.accounts);
  const claudeAccounts = useClaudeAccountStore((state) => state.accounts);

  const antigravityById = useMemo(
    () => new Map(antigravityAccounts.map((account) => [account.id, account])),
    [antigravityAccounts],
  );
  const codexById = useMemo(
    () => new Map(codexAccounts.map((account) => [account.id, account])),
    [codexAccounts],
  );
  const claudeById = useMemo(
    () => new Map(claudeAccounts.map((account) => [account.id, account])),
    [claudeAccounts],
  );

  const loadApiServiceQuota = useCallback(async () => {
    if (apiServiceQuotaPending.current) return;
    apiServiceQuotaPending.current = true;
    try {
      const windows = await service.getT3codeApiServiceQuota();
      if (windows !== null) setApiServiceQuota(windows);
    } catch (error) {
      console.warn('[T3 Code] Unable to read Codex API Service quota', error);
    } finally {
      apiServiceQuotaPending.current = false;
    }
  }, []);

  const loadWorkspace = useCallback(async (id: string, silent = false) => {
    const request = ++sequence.current;
    void loadApiServiceQuota();
    if (!silent) {
      setLoading(true);
      setLoadError('');
    }
    try {
      const next = await service.getT3codeProviderWorkspace(
        id === DEFAULT_T3_INSTANCE ? null : id,
      );
      if (request === sequence.current) {
        setWorkspace(next);
        setLoadError('');
      }
    } catch (error) {
      if (request === sequence.current && !silent) {
        setLoadError(String(error));
      }
    } finally {
      if (request === sequence.current) {
        setLoading(false);
      }
    }
  }, [loadApiServiceQuota]);

  useEffect(() => {
    const ag = useAccountStore.getState();
    if (ag.accounts.length === 0) void ag.fetchAccounts().catch(() => {});
    const codex = useCodexAccountStore.getState();
    if (codex.accounts.length === 0) void codex.fetchAccounts().catch(() => {});
    const claude = useClaudeAccountStore.getState();
    if (claude.accounts.length === 0) void claude.fetchAccounts().catch(() => {});
    return () => {
      sequence.current += 1;
    };
  }, []);

  useEffect(() => {
    void loadWorkspace(targetId);
    void service.syncT3codeProviderLaunchPaths(targetId).catch(() => {});
    const reload = () => { void loadWorkspace(targetId, true); };
    window.addEventListener('focus', reload);
    return () => window.removeEventListener('focus', reload);
  }, [targetId, loadWorkspace]);

  useEffect(() => {
    let active = true;
    const stop = listen<string>('t3code-provider-binding-changed', ({ payload }) => {
      if (!active || payload !== targetId) return;
      void loadWorkspace(targetId, true);
    }).catch(() => undefined);
    return () => {
      active = false;
      void stop.then((unlisten) => unlisten?.());
    };
  }, [targetId, loadWorkspace]);

  useEffect(() => {
    let active = true;
    let timer: number | undefined;
    const pollRuntime = async () => {
      try {
        const rows = await service.getT3codeProviderRuntimeStatus(targetId);
        if (active) {
          setLiveRuntime({
            target: targetId,
            rows: new Map(rows.map((row) => [row.instanceId, row])),
          });
        }
      } catch {
        if (active) setLiveRuntime({ target: targetId, rows: new Map() });
      } finally {
        if (active) timer = window.setTimeout(pollRuntime, 4000);
      }
    };
    void pollRuntime();
    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [targetId]);

  const gatewayName = t('codex.apiService.title');
  const activeWorkspace = useMemo(() => {
    if (workspace?.t3InstanceId !== targetId) return null;
    return {
      ...workspace,
      codexAccounts: workspace.codexAccounts.map((account) => account.isApiService
        ? { ...account, windows: apiServiceQuota } : account),
      providerInstances: workspace.providerInstances.map((instance) => instance.boundIsApiService
        ? { ...instance, windows: apiServiceQuota } : instance),
    };
  }, [workspace, targetId, apiServiceQuota]);
  const providerRows = activeWorkspace?.providerInstances ?? [];
  const t3Running = Boolean(activeWorkspace?.t3Running);
  const t3StartedAt = activeWorkspace?.t3StartedAt;
  const injectionConnected = Boolean(activeWorkspace?.injectionConnected);

  const renderPlanBadge = useCallback(
    (
      driver: T3CodeDriverKind,
      id?: string,
      plan?: string,
      apiService?: boolean,
    ): ReactNode =>
      createElement(T3codeAccountPlanBadge, {
        driver,
        plan,
        apiService,
        antigravity: driver === 'antigravity' && id ? antigravityById.get(id) : undefined,
        codex: driver === 'codex' && id ? codexById.get(id) : undefined,
        claude: driver === 'claudeAgent' && id ? claudeById.get(id) : undefined,
      }),
    [antigravityById, codexById, claudeById],
  );

  const providers: T3codeProviderRowView[] = useMemo(
    () =>
      providerRows.map((instance) => {
        const live =
          liveRuntime?.target === targetId ? liveRuntime.rows.get(instance.instanceId) : undefined;
        const runtime = resolveEffectiveRuntime(instance, live);
        const status = resolveProviderRowStatus(instance, runtime, t3Running, t, t3StartedAt);
        return { instance, runtime, status };
      }),
    [providerRows, liveRuntime, targetId, t3Running, t, t3StartedAt],
  );

  const restartAfterRemoval = removalPendingAt !== undefined &&
    (!t3StartedAt || Date.parse(t3StartedAt) <= removalPendingAt);
  const showRestartBanner = t3Running && (restartAfterRemoval || providers.some(
    ({ instance, status }) => Boolean(instance.boundAccountId || instance.boundAccountEmail) && status.needsRestart,
  ));

  const handleRefreshAll = useCallback(async () => {
    if (refreshingAll || loading || Boolean(busyInstanceId) || restarting) return;
    setRefreshingAll(true);
    setMessage(null);
    const t3InstanceId = targetId === DEFAULT_T3_INSTANCE ? null : targetId;
    try {
      const freshWorkspace = await service.getT3codeProviderWorkspace(t3InstanceId);
      setWorkspace(freshWorkspace);
      void service.syncT3codeProviderLaunchPaths(targetId).catch(() => {});
      const boundTargets = freshWorkspace.providerInstances.filter(
        (item) => item.enabled && item.boundAccountId && !item.boundIsApiService,
      );
      let latest = freshWorkspace;
      for (const item of boundTargets) {
        try {
          latest = await service.refreshT3codeProviderAccountQuota({
            t3InstanceId,
            providerInstanceId: item.instanceId,
          });
          setWorkspace(latest);
        } catch {
          // Keep refreshing remaining bound providers without blocking UI
        }
      }
    } catch (error) {
      setMessage({ tone: 'error', text: String(error) });
    } finally {
      void loadApiServiceQuota();
      setRefreshingAll(false);
    }
  }, [refreshingAll, loading, busyInstanceId, restarting, targetId, loadApiServiceQuota]);

  const handleStartOrRestart = useCallback(async () => {
    if (restarting || busyInstanceId) return;
    setRestarting(true);
    setMessage(null);
    try {
      await startInstance(targetId);
      setRemovalPendingAt(undefined);
      await loadWorkspace(targetId, true);
    } catch (error) {
      setMessage({ tone: 'error', text: String(error) });
    } finally {
      setRestarting(false);
    }
  }, [restarting, busyInstanceId, startInstance, targetId, loadWorkspace]);

  const addProviderAccount = useCallback(
    async (driver: T3CodeDriverKind, accountId: string) => {
      const target = getDefaultProvider(activeWorkspace, driver);
      const hasAccounts = activeWorkspace?.providerInstances.some(
        (row) => row.driver === driver && (row.boundAccountId || row.boundAccountEmail || !row.isBuiltinDefault),
      );
      const useDefault = !hasAccounts && !target?.boundAccountId && !target?.boundAccountEmail;
      const instanceId = useDefault ? driver : makeProviderSlug(driver);
      const account = getAccountsForDriver(activeWorkspace, driver).find((row) => row.id === accountId);
      setBusyInstanceId(instanceId);
      setMessage(null);
      try {
        const updated = await service.createT3codeProviderInstance({
          t3InstanceId: targetId === DEFAULT_T3_INSTANCE ? null : targetId,
          instanceId,
          driver,
          displayName: useDefault
            ? target?.displayName ?? getT3codePlatformSpec(driver).label
            : suggestInstanceName(driver, account, gatewayName),
          bindAccountId: accountId,
        });
        setWorkspace(updated);
        if (account?.isApiService) void loadApiServiceQuota();
      } finally {
        setBusyInstanceId(null);
      }
    },
    [activeWorkspace, gatewayName, loadApiServiceQuota, targetId],
  );

  const switchProviderAccount = useCallback(async (instance: T3CodeProviderInstanceView, accountId: string) => {
    setBusyInstanceId(instance.instanceId);
    setMessage(null);
    const request = ++sequence.current;
    try {
      const result = await service.switchT3codeProviderInstanceAccount({
        t3InstanceId: targetId === DEFAULT_T3_INSTANCE ? null : targetId,
        providerInstanceId: instance.instanceId,
        driver: instance.driver,
        cockpitAccountId: accountId,
      });
      if (request === sequence.current) setWorkspace(result.workspace);
      else void loadWorkspace(targetId, true);
      setMessage({ tone: 'success', text: result.message });
    } finally {
      setBusyInstanceId(null);
    }
  }, [targetId, loadWorkspace]);

  const promoteProviderAccount = useCallback(async (instance: T3CodeProviderInstanceView) => {
    if (busyInstanceId || restarting || !instance.boundAccountId) return;
    setBusyInstanceId(instance.instanceId);
    setMessage(null);
    const request = ++sequence.current;
    try {
      const result = await service.promoteT3codeProviderAccount({
        t3InstanceId: targetId === DEFAULT_T3_INSTANCE ? null : targetId,
        providerInstanceId: instance.instanceId,
        cockpitAccountId: instance.boundAccountId,
      });
      if (request === sequence.current) setWorkspace(result.workspace);
      else void loadWorkspace(targetId, true);
      setMessage({ tone: 'success', text: result.message });
    } catch (error) {
      setMessage({ tone: 'error', text: String(error) });
    } finally {
      setBusyInstanceId(null);
    }
  }, [busyInstanceId, restarting, targetId, loadWorkspace]);

  const removeProviderInstance = useCallback(async (instance: T3CodeProviderInstanceView) => {
    setBusyInstanceId(instance.instanceId);
    setMessage(null);
    try {
      const result = await service.removeT3codeProviderInstance({
        t3InstanceId: targetId === DEFAULT_T3_INSTANCE ? null : targetId,
        instanceId: instance.instanceId,
        cockpitAccountId: instance.boundAccountId ?? null,
      });
      setWorkspace(result.workspace);
      if (result.restartRequired) setRemovalPendingAt(Date.now());
      setMessage({ tone: 'success', text: result.message });
    } finally {
      setBusyInstanceId(null);
    }
  }, [targetId]);

  return {
    targetId,
    instances,
    selectTarget,
    activeWorkspace,
    providers,
    t3Running,
    injectionConnected,
    loading,
    loadError,
    message,
    setMessage,
    busyInstanceId,
    refreshingAll,
    showRestartBanner,
    restarting,
    gatewayName,
    isFlowNoticeCollapsed,
    setIsFlowNoticeCollapsed,
    privacyModeEnabled,
    togglePrivacyMode,
    maskAccountText,
    renderPlanBadge,
    loadWorkspace,
    loadApiServiceQuota,
    handleRefreshAll,
    handleStartOrRestart,
    addProviderAccount,
    switchProviderAccount,
    promoteProviderAccount,
    removeProviderInstance,
  };
}
