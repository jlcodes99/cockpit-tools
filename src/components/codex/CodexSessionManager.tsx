import { type MouseEvent, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { confirm as confirmDialog, open as openFileDialog, save as saveFileDialog } from '@tauri-apps/plugin-dialog';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { Check, ChevronDown, ChevronRight, Copy, Download, Eye, Folder, FolderOpen, Minimize2, RefreshCw, RotateCcw, Search, Trash2, Upload, X } from 'lucide-react';
import { ModalErrorMessage, useModalErrorState } from '../ModalErrorMessage';
import { SingleSelectDropdown, type SingleSelectOption } from '../SingleSelectDropdown';
import { useEscClose } from '../../hooks/useEscClose';
import type {
  CodexSessionImportPreview,
  CodexSessionImportPreviewItem,
  CodexSessionExportPreview,
  CodexSessionExportPreviewItem,
  CodexSessionRecord,
  CodexSessionTokenStats,
  CodexSessionTransferOperation,
  CodexSessionTransferProgress,
  CodexTrashedSessionRecord,
} from '../../types/codex';
import type { InstanceProfile } from '../../types/instance';
import { useCodexInstanceStore } from '../../stores/useCodexInstanceStore';
import { CodexSessionVisibilityRepairModal } from './CodexSessionVisibilityRepairModal';

type MessageState = { text: string; tone?: 'error' };
type SessionTokenStatsMap = Record<string, CodexSessionTokenStats>;

type SessionGroup = {
  cwd: string;
  sessions: CodexSessionRecord[];
  latestUpdatedAt: number;
};

type InstanceSortField = 'createdAt' | 'lastLaunchedAt';
type InstanceSortDirection = 'asc' | 'desc';
type SessionTransferStatus = 'running' | 'success' | 'error';
type ExportSelectionFilter = 'all' | 'selected' | 'unselected';

type SessionTransferTask = {
  id: string;
  operation: CodexSessionTransferOperation;
  status: SessionTransferStatus;
  progress: CodexSessionTransferProgress;
  message?: string;
  error?: string;
};

function readCodexInstanceSortPreference(): {
  field: InstanceSortField;
  direction: InstanceSortDirection;
} {
  const sortField = localStorage.getItem('agtools.codex.instances.sort_field');
  const sortDirection = localStorage.getItem('agtools.codex.instances.sort_direction');
  return {
    field: sortField === 'lastLaunchedAt' ? 'lastLaunchedAt' : 'createdAt',
    direction: sortDirection === 'desc' ? 'desc' : 'asc',
  };
}

function sortInstancesForDisplay(instances: InstanceProfile[]): InstanceProfile[] {
  const sortPreference = readCodexInstanceSortPreference();
  return [...instances].sort((left, right) => {
    if (left.isDefault && !right.isDefault) return -1;
    if (!left.isDefault && right.isDefault) return 1;
    const leftValue =
      sortPreference.field === 'createdAt'
        ? left.createdAt || 0
        : left.lastLaunchedAt || 0;
    const rightValue =
      sortPreference.field === 'createdAt'
        ? right.createdAt || 0
        : right.lastLaunchedAt || 0;
    return sortPreference.direction === 'asc'
      ? leftValue - rightValue
      : rightValue - leftValue;
  });
}

function buildGroups(sessions: CodexSessionRecord[]): SessionGroup[] {
  const groups = new Map<string, CodexSessionRecord[]>();
  sessions.forEach((session) => {
    const bucket = groups.get(session.cwd) ?? [];
    bucket.push(session);
    groups.set(session.cwd, bucket);
  });

  return Array.from(groups.entries())
    .map(([cwd, groupSessions]) => ({
      cwd,
      sessions: [...groupSessions].sort(
        (left, right) => (right.updatedAt ?? 0) - (left.updatedAt ?? 0) || left.title.localeCompare(right.title),
      ),
      latestUpdatedAt: Math.max(...groupSessions.map((item) => item.updatedAt ?? 0), 0),
    }))
    .sort(
      (left, right) =>
        right.latestUpdatedAt - left.latestUpdatedAt || left.cwd.localeCompare(right.cwd, 'zh-CN'),
    );
}

function buildDefaultExpandedGroups(_groups: SessionGroup[]): string[] {
  return [];
}

function formatRelativeTime(value: number | null | undefined, isZh: boolean): string {
  if (!value) return isZh ? '时间未知' : 'Unknown';
  const diffSeconds = Math.max(0, Math.floor(Date.now() / 1000) - value);
  const minute = 60;
  const hour = 60 * minute;
  const day = 24 * hour;
  const week = 7 * day;

  if (diffSeconds < hour) {
    const minutes = Math.max(1, Math.floor(diffSeconds / minute));
    return isZh ? `${minutes} 分钟` : `${minutes}m`;
  }
  if (diffSeconds < day) {
    const hours = Math.floor(diffSeconds / hour);
    return isZh ? `${hours} 小时` : `${hours}h`;
  }
  if (diffSeconds < week) {
    const days = Math.floor(diffSeconds / day);
    return isZh ? `${days} 天` : `${days}d`;
  }
  const weeks = Math.floor(diffSeconds / week);
  return isZh ? `${weeks} 周` : `${weeks}w`;
}

function resolveGroupLabel(cwd: string): string {
  const normalized = cwd.replace(/\\/g, '/').replace(/\/$/, '');
  const parts = normalized.split('/').filter(Boolean);
  return parts[parts.length - 1] || cwd;
}

function formatSessionId(sessionId: string): string {
  if (sessionId.length <= 18) return sessionId;
  return `${sessionId.slice(0, 8)}...${sessionId.slice(-6)}`;
}

function formatLargeNumber(value: number): string {
  if (value >= 1_000_000) {
    return `${(value / 1_000_000).toFixed(1)}M`;
  }
  if (value >= 1_000) {
    return `${(value / 1_000).toFixed(1)}K`;
  }
  return value.toLocaleString();
}

function formatTokenStats(stats?: CodexSessionTokenStats): string {
  if (stats) {
    return `${formatLargeNumber(stats.inputTokens)} / ${formatLargeNumber(stats.outputTokens)} tokens`;
  }

  return '';
}

function formatBytes(value: number): string {
  if (value >= 1024 * 1024 * 1024) {
    return `${(value / (1024 * 1024 * 1024)).toFixed(1)} GB`;
  }
  if (value >= 1024 * 1024) {
    return `${(value / (1024 * 1024)).toFixed(1)} MB`;
  }
  if (value >= 1024) {
    return `${(value / 1024).toFixed(1)} KB`;
  }
  return `${value} B`;
}

function buildDefaultSessionExportName(): string {
  const now = new Date();
  const pad = (value: number) => String(value).padStart(2, '0');
  const timestamp = [
    now.getFullYear(),
    pad(now.getMonth() + 1),
    pad(now.getDate()),
    '-',
    pad(now.getHours()),
    pad(now.getMinutes()),
    pad(now.getSeconds()),
  ].join('');
  return `codex-sessions-${timestamp}.zip`;
}

function createSessionTransferId(): string {
  return `session-transfer-${Date.now().toString(36)}-${Math.random()
    .toString(36)
    .slice(2, 8)}`;
}

function clampPercent(value: number | null | undefined): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(100, Math.round(value)));
}

function buildInitialTransferProgress(
  transferId: string,
  operation: CodexSessionTransferOperation,
  total: number,
): CodexSessionTransferProgress {
  return {
    transferId,
    operation,
    phase: 'prepare',
    current: 0,
    total,
    percent: 0,
    currentLabel: null,
    running: true,
  };
}

export function CodexSessionManager() {
  const { t, i18n } = useTranslation();
  const instances = useCodexInstanceStore((state) => state.instances);
  const refreshInstances = useCodexInstanceStore((state) => state.refreshInstances);
  const syncThreadsAcrossInstances = useCodexInstanceStore((state) => state.syncThreadsAcrossInstances);
  const syncSessionsToInstance = useCodexInstanceStore((state) => state.syncSessionsToInstance);
  const listSessionsAcrossInstances = useCodexInstanceStore((state) => state.listSessionsAcrossInstances);
  const getSessionTokenStatsAcrossInstances = useCodexInstanceStore(
    (state) => state.getSessionTokenStatsAcrossInstances,
  );
  const moveSessionsToTrashAcrossInstances = useCodexInstanceStore(
    (state) => state.moveSessionsToTrashAcrossInstances,
  );
  const listTrashedSessionsAcrossInstances = useCodexInstanceStore(
    (state) => state.listTrashedSessionsAcrossInstances,
  );
  const restoreSessionsFromTrashAcrossInstances = useCodexInstanceStore(
    (state) => state.restoreSessionsFromTrashAcrossInstances,
  );
  const deleteTrashedSessionsAcrossInstances = useCodexInstanceStore(
    (state) => state.deleteTrashedSessionsAcrossInstances,
  );
  const emptySessionTrashAcrossInstances = useCodexInstanceStore(
    (state) => state.emptySessionTrashAcrossInstances,
  );
  const previewSessionExport = useCodexInstanceStore((state) => state.previewSessionExport);
  const exportSessions = useCodexInstanceStore((state) => state.exportSessions);
  const previewSessionImport = useCodexInstanceStore((state) => state.previewSessionImport);
  const importSessions = useCodexInstanceStore((state) => state.importSessions);
  const openSessionLocation = useCodexInstanceStore((state) => state.openSessionLocation);
  const [sessions, setSessions] = useState<CodexSessionRecord[]>([]);
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [expandedGroups, setExpandedGroups] = useState<string[]>([]);
  const [showSyncTargetModal, setShowSyncTargetModal] = useState(false);
  const [syncTargetInstanceId, setSyncTargetInstanceId] = useState('');
  const [showRestoreModal, setShowRestoreModal] = useState(false);
  const [showExportModal, setShowExportModal] = useState(false);
  const [showImportModal, setShowImportModal] = useState(false);
  const [showRepairVisibilityModal, setShowRepairVisibilityModal] = useState(false);
  const [trashedSessions, setTrashedSessions] = useState<CodexTrashedSessionRecord[]>([]);
  const [selectedTrashIds, setSelectedTrashIds] = useState<string[]>([]);
  const [exportPreview, setExportPreview] = useState<CodexSessionExportPreview | null>(null);
  const [exportPath, setExportPath] = useState('');
  const [selectedExportIds, setSelectedExportIds] = useState<string[]>([]);
  const [removedExportIds, setRemovedExportIds] = useState<string[]>([]);
  const [exportSourceFilter, setExportSourceFilter] = useState('all');
  const [exportSelectionFilter, setExportSelectionFilter] = useState<ExportSelectionFilter>('all');
  const [importPreview, setImportPreview] = useState<CodexSessionImportPreview | null>(null);
  const [importTargetInstanceId, setImportTargetInstanceId] = useState('');
  const [selectedImportIds, setSelectedImportIds] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [syncingToInstance, setSyncingToInstance] = useState(false);
  const [repairingVisibility, setRepairingVisibility] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [loadingTrash, setLoadingTrash] = useState(false);
  const [restoring, setRestoring] = useState(false);
  const [purgingTrash, setPurgingTrash] = useState(false);
  const [loadingExportPreview, setLoadingExportPreview] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [loadingImportPreview, setLoadingImportPreview] = useState(false);
  const [importing, setImporting] = useState(false);
  const [transferTask, setTransferTask] = useState<SessionTransferTask | null>(null);
  const [showTransferModal, setShowTransferModal] = useState(false);
  const [message, setMessage] = useState<MessageState | null>(null);
  const [copiedSessionId, setCopiedSessionId] = useState<string | null>(null);
  const [tokenStatsBySessionId, setTokenStatsBySessionId] = useState<SessionTokenStatsMap>({});
  const [loadingTokenGroupCwds, setLoadingTokenGroupCwds] = useState<string[]>([]);
  const [loadedTokenGroupCwds, setLoadedTokenGroupCwds] = useState<string[]>([]);
  const [titleSearchInput, setTitleSearchInput] = useState('');
  const [appliedTitleSearch, setAppliedTitleSearch] = useState('');
  const {
    message: restoreModalError,
    scrollKey: restoreModalErrorScrollKey,
    set: setRestoreModalError,
  } = useModalErrorState();
  const {
    message: syncTargetModalError,
    scrollKey: syncTargetModalErrorScrollKey,
    set: setSyncTargetModalError,
  } = useModalErrorState();
  const {
    message: importModalError,
    scrollKey: importModalErrorScrollKey,
    set: setImportModalError,
  } = useModalErrorState();
  const {
    message: exportModalError,
    scrollKey: exportModalErrorScrollKey,
    set: setExportModalError,
  } = useModalErrorState();
  const hasInitializedExpandedGroupsRef = useRef(false);
  const loadSessionsPromiseRef = useRef<Promise<void> | null>(null);
  const copyResetTimerRef = useRef<number | null>(null);
  const tokenStatsVersionRef = useRef(0);
  const transferTaskIdRef = useRef<string | null>(null);
  const isZh = i18n.resolvedLanguage?.toLowerCase().startsWith('zh') ?? true;

  const groupedSessions = useMemo(() => buildGroups(sessions), [sessions]);
  const allSessionIds = useMemo(
    () => Array.from(new Set(sessions.map((session) => session.sessionId))),
    [sessions],
  );
  const selectedIdSet = useMemo(() => new Set(selectedIds), [selectedIds]);
  const selectedTrashIdSet = useMemo(() => new Set(selectedTrashIds), [selectedTrashIds]);
  const selectedExportIdSet = useMemo(() => new Set(selectedExportIds), [selectedExportIds]);
  const removedExportIdSet = useMemo(() => new Set(removedExportIds), [removedExportIds]);
  const selectedImportIdSet = useMemo(() => new Set(selectedImportIds), [selectedImportIds]);
  const loadingTokenGroupSet = useMemo(() => new Set(loadingTokenGroupCwds), [loadingTokenGroupCwds]);
  const loadedTokenGroupSet = useMemo(() => new Set(loadedTokenGroupCwds), [loadedTokenGroupCwds]);
  const trashTotalSizeBytes = useMemo(
    () => trashedSessions.reduce((sum, session) => sum + (session.sizeBytes ?? 0), 0),
    [trashedSessions],
  );
  const selectedTrashSizeBytes = useMemo(
    () =>
      trashedSessions
        .filter((session) => selectedTrashIdSet.has(session.sessionId))
        .reduce((sum, session) => sum + (session.sizeBytes ?? 0), 0),
    [selectedTrashIdSet, trashedSessions],
  );
  const selectedSessions = useMemo(
    () => sessions.filter((session) => selectedIdSet.has(session.sessionId)),
    [selectedIdSet, sessions],
  );
  const orderedInstances = useMemo(() => sortInstancesForDisplay(instances), [instances]);
  const targetInstanceOptions = useMemo<SingleSelectOption[]>(
    () => [
      {
        value: '',
        label: t('codex.sessionManager.targetModal.pickTarget', '请选择目标实例'),
      },
      ...orderedInstances.map((instance) => ({
        value: instance.id,
        label: instance.isDefault
          ? t('instances.defaultName', '默认实例')
          : instance.name || t('instances.defaultName', '默认实例'),
      })),
    ],
    [orderedInstances, t],
  );
  const importTargetOptions = useMemo<SingleSelectOption[]>(
    () =>
      orderedInstances.map((instance) => ({
        value: instance.id,
        label: instance.isDefault
          ? t('instances.defaultName', '默认实例')
          : instance.name || t('instances.defaultName', '默认实例'),
      })),
    [orderedInstances, t],
  );
  const importReadyItems = useMemo(
    () => importPreview?.items.filter((item) => item.status === 'ready') ?? [],
    [importPreview],
  );
  const exportPreviewItems = useMemo(
    () => exportPreview?.items ?? [],
    [exportPreview],
  );
  const exportAvailableItems = useMemo(
    () => exportPreviewItems.filter((item) => !removedExportIdSet.has(item.sessionId)),
    [exportPreviewItems, removedExportIdSet],
  );
  const exportSelectedItems = useMemo(
    () => exportAvailableItems.filter((item) => selectedExportIdSet.has(item.sessionId)),
    [exportAvailableItems, selectedExportIdSet],
  );
  const exportableSessionIds = useMemo(
    () => exportSelectedItems.map((item) => item.sessionId),
    [exportSelectedItems],
  );
  const exportSelectedSizeBytes = useMemo(
    () => exportSelectedItems.reduce((sum, item) => sum + item.sizeBytes, 0),
    [exportSelectedItems],
  );
  const exportAvailableSizeBytes = useMemo(
    () => exportAvailableItems.reduce((sum, item) => sum + item.sizeBytes, 0),
    [exportAvailableItems],
  );
  const exportSourceOptions = useMemo<SingleSelectOption[]>(() => {
    const sourceMap = new Map<string, { label: string; count: number }>();
    exportAvailableItems.forEach((item) => {
      const current = sourceMap.get(item.sourceInstanceId);
      sourceMap.set(item.sourceInstanceId, {
        label: item.sourceInstanceName,
        count: (current?.count ?? 0) + 1,
      });
    });
    return [
      {
        value: 'all',
        label: t('codex.sessionManager.exportModal.allSources', '全部来源'),
      },
      ...Array.from(sourceMap.entries()).map(([value, option]) => ({
        value,
        label: `${option.label} (${option.count})`,
      })),
    ];
  }, [exportAvailableItems, t]);
  const exportSelectionFilterOptions = useMemo<SingleSelectOption[]>(
    () => [
      { value: 'all', label: t('codex.sessionManager.exportModal.filterAll', '全部') },
      { value: 'selected', label: t('codex.sessionManager.exportModal.filterSelected', '已选') },
      { value: 'unselected', label: t('codex.sessionManager.exportModal.filterUnselected', '未选') },
    ],
    [t],
  );
  const exportFilteredItems = useMemo(() => {
    return exportAvailableItems.filter((item) => {
      if (exportSourceFilter !== 'all' && item.sourceInstanceId !== exportSourceFilter) {
        return false;
      }
      const selected = selectedExportIdSet.has(item.sessionId);
      if (exportSelectionFilter === 'selected' && !selected) return false;
      if (exportSelectionFilter === 'unselected' && selected) return false;
      return true;
    });
  }, [
    exportAvailableItems,
    exportSelectionFilter,
    exportSourceFilter,
    selectedExportIdSet,
  ]);
  const allImportReadySelected = importReadyItems.length > 0
    && importReadyItems.every((item) => selectedImportIdSet.has(item.sessionId));
  const syncTargetInstance = useMemo(
    () => orderedInstances.find((instance) => instance.id === syncTargetInstanceId) ?? null,
    [orderedInstances, syncTargetInstanceId],
  );
  const syncTargetExistingCount = useMemo(() => {
    if (!syncTargetInstance) return 0;
    return selectedSessions.filter((session) =>
      session.locations.some((location) => location.instanceId === syncTargetInstance.id),
    ).length;
  }, [selectedSessions, syncTargetInstance]);
  const allSessionsSelected = allSessionIds.length > 0 && allSessionIds.every((id) => selectedIdSet.has(id));
  const allTrashSelected =
    trashedSessions.length > 0 &&
    trashedSessions.every((session) => selectedTrashIdSet.has(session.sessionId));
  const trashBusy = restoring || purgingTrash || loadingTrash;
  const instanceCount = instances.length;
  const hasAppliedSearch = Boolean(appliedTitleSearch);
  const hasSearchInput = Boolean(titleSearchInput.trim());
  const transferProgress = transferTask?.progress ?? null;
  const transferPercent = clampPercent(transferProgress?.percent);
  const transferRunning = transferTask?.status === 'running';

  const getTransferTitle = useCallback(
    (operation: CodexSessionTransferOperation) =>
      operation === 'export'
        ? t('codex.sessionManager.transferModal.exportTitle', '导出会话')
        : t('codex.sessionManager.transferModal.importTitle', '导入会话'),
    [t],
  );

  const getTransferPhaseText = useCallback(
    (progress: CodexSessionTransferProgress | null) => {
      const operation = progress?.operation ?? transferTask?.operation ?? 'export';
      const phase = progress?.phase ?? 'prepare';
      if (operation === 'export') {
        if (phase === 'collect') return t('codex.sessionManager.transferModal.phase.exportCollect', '正在收集会话...');
        if (phase === 'hash') return t('codex.sessionManager.transferModal.phase.exportHash', '正在校验会话文件...');
        if (phase === 'write') return t('codex.sessionManager.transferModal.phase.exportWrite', '正在写入会话包...');
        if (phase === 'done') return t('codex.sessionManager.transferModal.phase.done', '任务已完成');
        return t('codex.sessionManager.transferModal.phase.exportPrepare', '正在准备导出...');
      }
      if (phase === 'read') return t('codex.sessionManager.transferModal.phase.importRead', '正在读取会话包...');
      if (phase === 'write') return t('codex.sessionManager.transferModal.phase.importWrite', '正在写入目标实例...');
      if (phase === 'rebuild') return t('codex.sessionManager.transferModal.phase.importRebuild', '正在刷新官方会话索引...');
      if (phase === 'done') return t('codex.sessionManager.transferModal.phase.done', '任务已完成');
      return t('codex.sessionManager.transferModal.phase.importPrepare', '正在准备导入...');
    },
    [t, transferTask?.operation],
  );

  const loadSessions = useCallback(async () => {
    if (loadSessionsPromiseRef.current) {
      return await loadSessionsPromiseRef.current;
    }

    const task = (async () => {
      setLoading(true);
      try {
        const nextSessions = await listSessionsAcrossInstances({
          titleQuery: appliedTitleSearch || null,
        });
        const nextGroups = buildGroups(nextSessions);
        const hasInitializedExpandedGroups = hasInitializedExpandedGroupsRef.current;
        tokenStatsVersionRef.current += 1;
        setSessions(nextSessions);
        setTokenStatsBySessionId({});
        setLoadingTokenGroupCwds([]);
        setLoadedTokenGroupCwds([]);
        setSelectedIds((prev) => prev.filter((id) => nextSessions.some((item) => item.sessionId === id)));
        setExpandedGroups((prev) => {
          const valid = prev.filter((cwd) => nextGroups.some((group) => group.cwd === cwd));

          if (prev.length === 0) {
            return hasInitializedExpandedGroups ? [] : buildDefaultExpandedGroups(nextGroups);
          }

          return valid.length > 0 ? valid : buildDefaultExpandedGroups(nextGroups);
        });
        hasInitializedExpandedGroupsRef.current = true;
      } catch (error) {
        setMessage({ text: String(error), tone: 'error' });
      } finally {
        setLoading(false);
      }
    })();

    loadSessionsPromiseRef.current = task;
    try {
      await task;
    } finally {
      if (loadSessionsPromiseRef.current === task) {
        loadSessionsPromiseRef.current = null;
      }
    }
  }, [appliedTitleSearch, listSessionsAcrossInstances]);

  const loadTokenStatsForGroups = useCallback(
    async (groups: SessionGroup[]) => {
      if (groups.length === 0) {
        return;
      }

      const groupCwds = groups.map((group) => group.cwd);
      const sessionIds = Array.from(new Set(groups.flatMap((group) => group.sessions.map((session) => session.sessionId))));
      if (sessionIds.length === 0) {
        setLoadedTokenGroupCwds((prev) => Array.from(new Set([...prev, ...groupCwds])));
        return;
      }

      const requestVersion = tokenStatsVersionRef.current;
      setLoadingTokenGroupCwds((prev) => Array.from(new Set([...prev, ...groupCwds])));

      try {
        const stats = await getSessionTokenStatsAcrossInstances(sessionIds);
        if (tokenStatsVersionRef.current !== requestVersion) {
          return;
        }

        setTokenStatsBySessionId((prev) => {
          const next = { ...prev };
          stats.forEach((item) => {
            next[item.sessionId] = item;
          });
          return next;
        });
      } catch (error) {
        if (tokenStatsVersionRef.current === requestVersion) {
          console.error('Failed to load session token stats:', error);
        }
      } finally {
        if (tokenStatsVersionRef.current !== requestVersion) {
          return;
        }
        setLoadingTokenGroupCwds((prev) => prev.filter((cwd) => !groupCwds.includes(cwd)));
        setLoadedTokenGroupCwds((prev) => Array.from(new Set([...prev, ...groupCwds])));
      }
    },
    [getSessionTokenStatsAcrossInstances],
  );

  const loadTrashedSessions = useCallback(async () => {
    setLoadingTrash(true);
    setRestoreModalError(null);
    setTrashedSessions([]);
    try {
      const nextSessions = await listTrashedSessionsAcrossInstances();
      setTrashedSessions(nextSessions);
      setSelectedTrashIds((prev) => prev.filter((id) => nextSessions.some((item) => item.sessionId === id)));
      return nextSessions;
    } catch (error) {
      setRestoreModalError(String(error));
      return [];
    } finally {
      setLoadingTrash(false);
    }
  }, [listTrashedSessionsAcrossInstances, setRestoreModalError]);

  useEffect(() => {
    void loadSessions();
  }, [loadSessions]);

  useEffect(() => {
    if (exportSourceFilter === 'all') return;
    if (exportSourceOptions.some((option) => option.value === exportSourceFilter)) return;
    setExportSourceFilter('all');
  }, [exportSourceFilter, exportSourceOptions]);

  useEffect(() => {
    const nextTitleQuery = titleSearchInput.trim();
    const timer = window.setTimeout(() => {
      setMessage(null);
      setAppliedTitleSearch((current) => (current === nextTitleQuery ? current : nextTitleQuery));
    }, 300);

    return () => {
      window.clearTimeout(timer);
    };
  }, [titleSearchInput]);

  useEffect(() => {
    const groupsToLoad = groupedSessions.filter(
      (group) =>
        expandedGroups.includes(group.cwd) &&
        !loadingTokenGroupSet.has(group.cwd) &&
        !loadedTokenGroupSet.has(group.cwd),
    );
    if (groupsToLoad.length === 0) {
      return;
    }

    void loadTokenStatsForGroups(groupsToLoad);
  }, [expandedGroups, groupedSessions, loadedTokenGroupSet, loadTokenStatsForGroups, loadingTokenGroupSet]);

  useEffect(() => {
    return () => {
      if (copyResetTimerRef.current !== null) {
        window.clearTimeout(copyResetTimerRef.current);
      }
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: UnlistenFn | null = null;
    void listen<CodexSessionTransferProgress>(
      'codex:session-transfer-progress',
      (event) => {
        const payload = event.payload;
        if (!payload || payload.transferId !== transferTaskIdRef.current) return;
        setTransferTask((current) => {
          if (!current || current.id !== payload.transferId) return current;
          return {
            ...current,
            progress: payload,
            status: payload.running ? 'running' : current.status,
          };
        });
      },
    ).then((nextUnlisten) => {
      if (disposed) {
        nextUnlisten();
      } else {
        unlisten = nextUnlisten;
      }
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const toggleSession = (sessionId: string) => {
    setSelectedIds((prev) =>
      prev.includes(sessionId) ? prev.filter((id) => id !== sessionId) : [...prev, sessionId],
    );
  };

  const toggleGroupSelection = (sessionIds: string[]) => {
    const allSelected = sessionIds.every((id) => selectedIdSet.has(id));
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (allSelected) {
        sessionIds.forEach((id) => next.delete(id));
      } else {
        sessionIds.forEach((id) => next.add(id));
      }
      return Array.from(next);
    });
  };

  const toggleAllSessions = () => {
    if (allSessionIds.length === 0) return;

    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (allSessionIds.every((id) => next.has(id))) {
        allSessionIds.forEach((id) => next.delete(id));
      } else {
        allSessionIds.forEach((id) => next.add(id));
      }
      return Array.from(next);
    });
  };

  const toggleGroupExpanded = (cwd: string) => {
    setExpandedGroups((prev) => (prev.includes(cwd) ? prev.filter((item) => item !== cwd) : [...prev, cwd]));
  };

  const toggleTrashedSession = (sessionId: string) => {
    setSelectedTrashIds((prev) =>
      prev.includes(sessionId) ? prev.filter((id) => id !== sessionId) : [...prev, sessionId],
    );
  };

  const toggleAllTrashedSessions = () => {
    if (trashedSessions.length === 0) return;
    setSelectedTrashIds(allTrashSelected ? [] : trashedSessions.map((session) => session.sessionId));
  };

  const handleOpenRestoreModal = async () => {
    setShowRestoreModal(true);
    setSelectedTrashIds([]);
    await loadTrashedSessions();
  };

  const handleOpenSyncTargetModal = async () => {
    if (selectedIds.length === 0) {
      setMessage({ text: t('codex.sessionManager.messages.pickOne', '请至少选择一条会话'), tone: 'error' });
      return;
    }

    setMessage(null);
    setSyncTargetModalError(null);
    try {
      const latestInstances = await refreshInstances();
      const targetCandidates = sortInstancesForDisplay(
        latestInstances.length > 0 ? latestInstances : instances,
      );
      const firstMissingTarget = targetCandidates.find((instance) =>
        selectedSessions.some((session) =>
          !session.locations.some((location) => location.instanceId === instance.id),
        ),
      );
      setSyncTargetInstanceId((firstMissingTarget ?? targetCandidates[0])?.id ?? '');
      setShowSyncTargetModal(true);
    } catch (error) {
      setMessage({ text: String(error), tone: 'error' });
    }
  };

  const handleCloseSyncTargetModal = () => {
    setShowSyncTargetModal(false);
  };

  useEscClose(showSyncTargetModal, handleCloseSyncTargetModal);

  const handleCloseRestoreModal = () => {
    if (restoring || purgingTrash) return;
    setShowRestoreModal(false);
    setSelectedTrashIds([]);
    setRestoreModalError(null);
  };

  useEscClose(showRestoreModal, handleCloseRestoreModal);

  const handleMinimizeTransferModal = () => {
    setShowTransferModal(false);
  };

  const handleClearTransferTask = () => {
    if (transferTask?.status === 'running') {
      setShowTransferModal(false);
      return;
    }
    transferTaskIdRef.current = null;
    setTransferTask(null);
    setShowTransferModal(false);
  };

  useEscClose(showTransferModal && !transferRunning, handleClearTransferTask);

  const handleSyncSessions = async () => {
    setMessage(null);
    try {
      const latestInstances = await refreshInstances();
      if (latestInstances.length < 2) {
        setMessage({
          text: t('codex.sessionManager.messages.syncNeedTwo', '至少需要两个实例才能同步会话'),
          tone: 'error',
        });
        return;
      }

      const confirmed = await confirmDialog(
        t(
          'codex.sessionManager.confirm.syncMessage',
          '会将缺失会话的 rollout、session_index 条目和会话文件时间同步到所有实例，并对同 ID 会话做事件级合并，随后触发官方 Codex 重建会话索引；写入前会备份目标文件。确认继续？',
        ),
        {
          title: t('codex.sessionManager.actions.syncSessions', '同步会话'),
          okLabel: t('common.confirm', '确认'),
          cancelLabel: t('common.cancel', '取消'),
        },
      );
      if (!confirmed) return;

      setSyncing(true);
      const summary = await syncThreadsAcrossInstances();
      setMessage({ text: summary.message });
      await loadSessions();
    } catch (error) {
      setMessage({ text: String(error), tone: 'error' });
    } finally {
      setSyncing(false);
    }
  };

  const handleSyncSelectedToInstance = async () => {
    if (selectedIds.length === 0) {
      setSyncTargetModalError(t('codex.sessionManager.messages.pickOne', '请至少选择一条会话'));
      return;
    }
    if (!syncTargetInstanceId) {
      setSyncTargetModalError(t('codex.sessionManager.targetModal.pickTarget', '请选择目标实例'));
      return;
    }

    setSyncingToInstance(true);
    setSyncTargetModalError(null);
    try {
      const summary = await syncSessionsToInstance(selectedIds, syncTargetInstanceId);
      setMessage({ text: summary.message });
      setShowSyncTargetModal(false);
      setSyncTargetInstanceId('');
      setSelectedIds([]);
      await loadSessions();
    } catch (error) {
      setSyncTargetModalError(String(error));
    } finally {
      setSyncingToInstance(false);
    }
  };

  const handleRefresh = async () => {
    setMessage(null);
    try {
      await refreshInstances();
      await loadSessions();
      if (showRestoreModal) {
        await loadTrashedSessions();
      }
    } catch (error) {
      setMessage({ text: String(error), tone: 'error' });
    }
  };

  const handleClearSearch = () => {
    setTitleSearchInput('');
    setMessage(null);

    if (!appliedTitleSearch) {
      return;
    }

    setAppliedTitleSearch('');
  };

  const handleRepairVisibility = async () => {
    setMessage(null);
    setShowRepairVisibilityModal(true);
  };

  const handleMoveToTrash = async () => {
    if (selectedIds.length === 0) {
      setMessage({ text: t('codex.sessionManager.messages.pickOne', '请至少选择一条会话'), tone: 'error' });
      return;
    }

    const confirmed = await confirmDialog(
      t(
        'codex.sessionManager.confirm.message',
        '会将所选会话从对应实例中移到废纸篓，便于后续恢复；运行中的实例可能需要重启后才会反映。确认继续？',
      ),
      {
        title: t('codex.sessionManager.confirm.title', '移到废纸篓'),
        okLabel: t('common.confirm', '确认'),
        cancelLabel: t('common.cancel', '取消'),
        kind: 'warning',
      },
    );
    if (!confirmed) return;

    setDeleting(true);
    setMessage(null);
    try {
      const summary = await moveSessionsToTrashAcrossInstances(selectedIds);
      setMessage({ text: summary.message });
      setSelectedIds([]);
      await loadSessions();
      if (showRestoreModal) {
        await loadTrashedSessions();
      }
    } catch (error) {
      setMessage({ text: String(error), tone: 'error' });
    } finally {
      setDeleting(false);
    }
  };

  const handleRestoreFromTrash = async () => {
    if (selectedTrashIds.length === 0) {
      setRestoreModalError(t('codex.sessionManager.messages.pickRestoreOne', '请至少选择一条待恢复会话'));
      return;
    }

    setRestoring(true);
    setRestoreModalError(null);
    try {
      const summary = await restoreSessionsFromTrashAcrossInstances(selectedTrashIds);
      setMessage({ text: summary.message });
      setSelectedTrashIds([]);
      const [nextTrashedSessions] = await Promise.all([loadTrashedSessions(), loadSessions()]);
      if (nextTrashedSessions.length === 0) {
        setShowRestoreModal(false);
      }
    } catch (error) {
      setRestoreModalError(String(error));
    } finally {
      setRestoring(false);
    }
  };

  const handleDeleteTrashedSessions = async (sessionIds: string[]) => {
    const normalizedSessionIds = Array.from(new Set(sessionIds.filter(Boolean)));
    if (normalizedSessionIds.length === 0) {
      setRestoreModalError(t('codex.sessionManager.restoreModal.pickDeleteOne', '请至少选择一条要永久删除的会话'));
      return;
    }

    const deletingSessions = trashedSessions.filter((session) =>
      normalizedSessionIds.includes(session.sessionId),
    );
    const deletingSizeBytes = deletingSessions.reduce(
      (sum, session) => sum + (session.sizeBytes ?? 0),
      0,
    );
    const isSingle = normalizedSessionIds.length === 1;
    const confirmed = await confirmDialog(
      isSingle
        ? t(
            'codex.sessionManager.restoreModal.deleteOneConfirm',
            '确定要永久删除这个会话吗？删除后无法恢复，预计释放 {{size}}。',
            { size: formatBytes(deletingSizeBytes) },
          )
        : t(
            'codex.sessionManager.restoreModal.deleteSelectedConfirm',
            '确定要永久删除选中的 {{count}} 条会话吗？删除后无法恢复，预计释放 {{size}}。',
            { count: normalizedSessionIds.length, size: formatBytes(deletingSizeBytes) },
          ),
      {
        title: t('codex.sessionManager.restoreModal.permanentDeleteTitle', '永久删除'),
        okLabel: t('codex.sessionManager.restoreModal.permanentDeleteAction', '永久删除'),
        cancelLabel: t('common.cancel', '取消'),
        kind: 'warning',
      },
    );
    if (!confirmed) return;

    setPurgingTrash(true);
    setRestoreModalError(null);
    try {
      const summary = await deleteTrashedSessionsAcrossInstances(normalizedSessionIds);
      setMessage({ text: summary.message });
      setSelectedTrashIds((prev) => prev.filter((id) => !normalizedSessionIds.includes(id)));
      const nextTrashedSessions = await loadTrashedSessions();
      if (nextTrashedSessions.length === 0) {
        setShowRestoreModal(false);
      }
    } catch (error) {
      setRestoreModalError(String(error));
    } finally {
      setPurgingTrash(false);
    }
  };

  const handleEmptySessionTrash = async () => {
    if (trashedSessions.length === 0) {
      return;
    }

    const confirmed = await confirmDialog(
      t(
        'codex.sessionManager.restoreModal.emptyTrashConfirm',
        '确定要清空废纸篓吗？其中 {{count}} 条会话将被永久删除且无法恢复，预计释放 {{size}}。',
        { count: trashedSessions.length, size: formatBytes(trashTotalSizeBytes) },
      ),
      {
        title: t('codex.sessionManager.restoreModal.emptyTrash', '清空废纸篓'),
        okLabel: t('codex.sessionManager.restoreModal.emptyTrash', '清空废纸篓'),
        cancelLabel: t('common.cancel', '取消'),
        kind: 'warning',
      },
    );
    if (!confirmed) return;

    setPurgingTrash(true);
    setRestoreModalError(null);
    try {
      const summary = await emptySessionTrashAcrossInstances();
      setMessage({ text: summary.message });
      setSelectedTrashIds([]);
      await loadTrashedSessions();
      setShowRestoreModal(false);
    } catch (error) {
      setRestoreModalError(String(error));
    } finally {
      setPurgingTrash(false);
    }
  };

  const previewImportPackage = useCallback(
    async (filePath: string, targetInstanceId: string) => {
      setLoadingImportPreview(true);
      setImportModalError(null);
      try {
        const preview = await previewSessionImport(filePath, targetInstanceId);
        setImportPreview(preview);
        setImportTargetInstanceId(preview.targetInstanceId);
        setSelectedImportIds(
          preview.items
            .filter((item) => item.status === 'ready')
            .map((item) => item.sessionId),
        );
        return preview;
      } finally {
        setLoadingImportPreview(false);
      }
    },
    [previewSessionImport, setImportModalError],
  );

  const resetExportModalSelection = useCallback(() => {
    setSelectedExportIds([]);
    setRemovedExportIds([]);
    setExportSourceFilter('all');
    setExportSelectionFilter('all');
  }, []);

  const addExportSelection = useCallback((items: CodexSessionExportPreviewItem[]) => {
    const ids = items.map((item) => item.sessionId);
    if (ids.length === 0) return;
    setSelectedExportIds((prev) => Array.from(new Set([...prev, ...ids])));
  }, []);

  const removeExportSelection = useCallback((items: CodexSessionExportPreviewItem[]) => {
    const ids = new Set(items.map((item) => item.sessionId));
    if (ids.size === 0) return;
    setSelectedExportIds((prev) => prev.filter((id) => !ids.has(id)));
  }, []);

  const removeExportItems = useCallback((items: CodexSessionExportPreviewItem[]) => {
    const ids = items.map((item) => item.sessionId);
    if (ids.length === 0) return;
    const idSet = new Set(ids);
    setRemovedExportIds((prev) => Array.from(new Set([...prev, ...ids])));
    setSelectedExportIds((prev) => prev.filter((id) => !idSet.has(id)));
  }, []);

  const toggleExportSession = useCallback((sessionId: string) => {
    setSelectedExportIds((prev) =>
      prev.includes(sessionId)
        ? prev.filter((id) => id !== sessionId)
        : [...prev, sessionId],
    );
  }, []);

  const keepFilteredExportItems = useCallback(() => {
    const visibleIds = new Set(exportFilteredItems.map((item) => item.sessionId));
    setRemovedExportIds((prev) =>
      Array.from(new Set([
        ...prev,
        ...exportAvailableItems
          .filter((item) => !visibleIds.has(item.sessionId))
          .map((item) => item.sessionId),
      ])),
    );
    setSelectedExportIds(exportFilteredItems.map((item) => item.sessionId));
  }, [exportAvailableItems, exportFilteredItems]);

  const removeUncheckedExportItems = useCallback(() => {
    removeExportItems(exportAvailableItems.filter((item) => !selectedExportIdSet.has(item.sessionId)));
  }, [exportAvailableItems, removeExportItems, selectedExportIdSet]);

  const restoreRemovedExportItems = useCallback(() => {
    setRemovedExportIds([]);
  }, []);

  const handleExportSessions = async () => {
    if (selectedIds.length === 0) {
      setMessage({ text: t('codex.sessionManager.messages.pickOne', '请至少选择一条会话'), tone: 'error' });
      return;
    }

    setMessage(null);
    setShowExportModal(true);
    setExportPreview(null);
    setExportPath('');
    resetExportModalSelection();
    setExportModalError(null);
    setLoadingExportPreview(true);
    try {
      const preview = await previewSessionExport(selectedIds);
      setExportPreview(preview);
      setSelectedExportIds(preview.items.map((item) => item.sessionId));
    } catch (error) {
      setExportModalError(String(error));
    } finally {
      setLoadingExportPreview(false);
    }
  };

  const handleCloseExportModal = () => {
    if (loadingExportPreview || exporting) return;
    setShowExportModal(false);
    setExportPreview(null);
    setExportPath('');
    resetExportModalSelection();
    setExportModalError(null);
  };

  useEscClose(showExportModal, handleCloseExportModal);

  const handleChooseExportPath = async () => {
    setExportModalError(null);
    const selected = await saveFileDialog({
      defaultPath: buildDefaultSessionExportName(),
      filters: [{ name: 'ZIP', extensions: ['zip'] }],
    });
    if (selected) {
      setExportPath(selected);
    }
  };

  const handleConfirmExportSessions = async () => {
    if (!exportPreview) {
      setExportModalError(t('codex.sessionManager.exportModal.noPreview', '请先完成导出预览'));
      return;
    }
    if (exportableSessionIds.length === 0) {
      setExportModalError(t('codex.sessionManager.exportModal.noExportable', '没有可导出的会话'));
      return;
    }
    if (!exportPath) {
      setExportModalError(t('codex.sessionManager.exportModal.pickPath', '请选择导出位置'));
      return;
    }

    setExporting(true);
    setExportModalError(null);
    setMessage(null);
    const transferId = createSessionTransferId();
    const sessionIdsToExport = [...exportableSessionIds];
    const targetExportPath = exportPath;
    transferTaskIdRef.current = transferId;
    setTransferTask({
      id: transferId,
      operation: 'export',
      status: 'running',
      progress: buildInitialTransferProgress(transferId, 'export', sessionIdsToExport.length),
    });
    setShowTransferModal(true);
    setShowExportModal(false);
    try {
      const summary = await exportSessions(sessionIdsToExport, targetExportPath, transferId);
      setTransferTask((current) =>
        current?.id === transferId
          ? {
              ...current,
              status: 'success',
              message: summary.message,
              progress: {
                ...current.progress,
                phase: 'done',
                current: current.progress.total,
                percent: 100,
                running: false,
              },
            }
          : current,
      );
      setMessage({ text: summary.message });
      setExportPreview(null);
      setExportPath('');
    } catch (error) {
      const errorText = String(error);
      setTransferTask((current) =>
        current?.id === transferId
          ? {
              ...current,
              status: 'error',
              error: errorText,
              progress: {
                ...current.progress,
                running: false,
              },
            }
          : current,
      );
      setMessage({ text: String(error), tone: 'error' });
    } finally {
      setExporting(false);
    }
  };

  const handleOpenImportModal = async () => {
    const selected = await openFileDialog({
      multiple: false,
      filters: [{ name: 'ZIP', extensions: ['zip'] }],
    });
    const filePath = Array.isArray(selected) ? selected[0] : selected;
    if (!filePath) return;

    setMessage(null);
    setShowImportModal(true);
    setImportPreview(null);
    setSelectedImportIds([]);
    setImportModalError(null);
    try {
      const latestInstances = await refreshInstances();
      const targetCandidates = sortInstancesForDisplay(
        latestInstances.length > 0 ? latestInstances : instances,
      );
      const defaultTarget = targetCandidates.find((instance) => instance.isDefault) ?? targetCandidates[0];
      const targetInstanceId = importTargetInstanceId || defaultTarget?.id || '';
      if (!targetInstanceId) {
        setImportModalError(t('codex.sessionManager.importModal.noTarget', '未发现可导入的 Codex 实例'));
        return;
      }
      await previewImportPackage(filePath, targetInstanceId);
    } catch (error) {
      setImportModalError(String(error));
    }
  };

  const handleCloseImportModal = () => {
    if (importing || loadingImportPreview) return;
    setShowImportModal(false);
    setImportPreview(null);
    setSelectedImportIds([]);
    setImportModalError(null);
  };

  useEscClose(showImportModal, handleCloseImportModal);

  const handleChangeImportTarget = async (targetInstanceId: string) => {
    setImportTargetInstanceId(targetInstanceId);
    setImportModalError(null);
    const filePath = importPreview?.importFilePath;
    if (!filePath || !targetInstanceId) return;
    try {
      await previewImportPackage(filePath, targetInstanceId);
    } catch (error) {
      setImportModalError(String(error));
    }
  };

  const toggleImportSession = (item: CodexSessionImportPreviewItem) => {
    if (item.status !== 'ready') return;
    setSelectedImportIds((prev) =>
      prev.includes(item.sessionId)
        ? prev.filter((id) => id !== item.sessionId)
        : [...prev, item.sessionId],
    );
  };

  const toggleAllImportReady = () => {
    if (importReadyItems.length === 0) return;
    setSelectedImportIds((prev) => {
      const next = new Set(prev);
      if (importReadyItems.every((item) => next.has(item.sessionId))) {
        importReadyItems.forEach((item) => next.delete(item.sessionId));
      } else {
        importReadyItems.forEach((item) => next.add(item.sessionId));
      }
      return Array.from(next);
    });
  };

  const handleImportSelectedSessions = async () => {
    if (!importPreview) {
      setImportModalError(t('codex.sessionManager.importModal.noPackage', '请先选择会话包'));
      return;
    }
    if (!importTargetInstanceId) {
      setImportModalError(t('codex.sessionManager.targetModal.pickTarget', '请选择目标实例'));
      return;
    }
    if (selectedImportIds.length === 0) {
      setImportModalError(t('codex.sessionManager.importModal.pickOne', '请至少选择一条可导入会话'));
      return;
    }

    setImporting(true);
    setImportModalError(null);
    const transferId = createSessionTransferId();
    transferTaskIdRef.current = transferId;
    setTransferTask({
      id: transferId,
      operation: 'import',
      status: 'running',
      progress: buildInitialTransferProgress(transferId, 'import', selectedImportIds.length),
    });
    setShowTransferModal(true);
    setShowImportModal(false);
    try {
      const summary = await importSessions(
        importPreview.importFilePath,
        importTargetInstanceId,
        selectedImportIds,
        transferId,
      );
      setTransferTask((current) =>
        current?.id === transferId
          ? {
              ...current,
              status: 'success',
              message: summary.message,
              progress: {
                ...current.progress,
                phase: 'done',
                current: current.progress.total,
                percent: 100,
                running: false,
              },
            }
          : current,
      );
      setMessage({ text: summary.message });
      setImportPreview(null);
      setSelectedImportIds([]);
      await loadSessions();
    } catch (error) {
      const errorText = String(error);
      setTransferTask((current) =>
        current?.id === transferId
          ? {
              ...current,
              status: 'error',
              error: errorText,
              progress: {
                ...current.progress,
                running: false,
              },
            }
          : current,
      );
    } finally {
      setImporting(false);
    }
  };

  const handleOpenSessionLocation = async (
    event: MouseEvent<HTMLButtonElement>,
    sessionId: string,
  ) => {
    event.preventDefault();
    event.stopPropagation();
    setMessage(null);
    try {
      await openSessionLocation(sessionId);
    } catch (error) {
      setMessage({ text: String(error), tone: 'error' });
    }
  };

  const getImportStatusLabel = (item: CodexSessionImportPreviewItem): string => {
    if (item.status === 'ready') {
      return t('codex.sessionManager.importModal.statusReady', '可导入');
    }
    if (item.status === 'duplicate') {
      return t('codex.sessionManager.importModal.statusDuplicate', '已存在');
    }
    if (item.status === 'conflict') {
      return t('codex.sessionManager.importModal.statusConflict', '冲突');
    }
    return t('codex.sessionManager.importModal.statusInvalid', '无效');
  };

  const handleCopySessionId = async (event: MouseEvent<HTMLButtonElement>, sessionId: string) => {
    event.preventDefault();
    event.stopPropagation();

    try {
      await navigator.clipboard.writeText(sessionId);
      setCopiedSessionId(sessionId);
      if (copyResetTimerRef.current !== null) {
        window.clearTimeout(copyResetTimerRef.current);
      }
      copyResetTimerRef.current = window.setTimeout(() => {
        setCopiedSessionId((current) => (current === sessionId ? null : current));
        copyResetTimerRef.current = null;
      }, 1200);
    } catch (error) {
      console.error('Failed to copy session id:', error);
      setMessage({
        text: t('common.shared.export.copyFailed', '复制失败，请手动复制'),
        tone: 'error',
      });
    }
  };

  return (
    <section className="codex-session-manager">
      <div className="codex-session-manager__header">
        <div className="codex-session-manager__search">
          <label className="codex-session-search-field">
            <div className="codex-session-search-field__control">
              <Search size={14} />
              <input
                type="text"
                value={titleSearchInput}
                onChange={(event) => setTitleSearchInput(event.target.value)}
                placeholder={t('codex.sessionManager.search.titlePlaceholder', '按标题搜索')}
                disabled={loading}
              />
            </div>
          </label>
          <button
            className="btn btn-secondary codex-session-manager__search-button"
            type="button"
            onClick={handleClearSearch}
            disabled={loading || (!hasSearchInput && !hasAppliedSearch)}
          >
            <X size={14} />
            {t('codex.sessionManager.search.clear', '清空')}
          </button>
        </div>
        <div className="codex-session-manager__actions">
          <button
            className="btn btn-secondary codex-session-manager__action-button"
            type="button"
            onClick={toggleAllSessions}
            disabled={loading || allSessionIds.length === 0}
            title={
              allSessionsSelected
                ? t('codex.sessionManager.actions.clearSelectedSessions', '取消全选')
                : t('codex.sessionManager.actions.selectAllSessions', '全选全部会话')
            }
            aria-label={
              allSessionsSelected
                ? t('codex.sessionManager.actions.clearSelectedSessions', '取消全选')
                : t('codex.sessionManager.actions.selectAllSessions', '全选全部会话')
            }
          >
            {allSessionsSelected ? <X size={14} /> : <Check size={14} />}
            {allSessionsSelected
              ? t('codex.sessionManager.actions.clearSelectedSessions', '取消全选')
              : t('codex.sessionManager.actions.selectAllSessions', '全选全部会话')}
          </button>
          <button
            className="btn btn-secondary codex-session-manager__action-button"
            type="button"
            onClick={() => void handleSyncSessions()}
            disabled={syncing || syncingToInstance || repairingVisibility || deleting || loading || instanceCount < 2}
            title={
              instanceCount < 2
                ? t('codex.sessionManager.messages.syncNeedTwo', '至少需要两个实例才能同步会话')
                : t('codex.sessionManager.actions.syncSessions', '同步会话')
            }
          >
            <RefreshCw size={14} className={syncing ? 'icon-spin' : undefined} />
            {t('codex.sessionManager.actions.syncSessions', '同步会话')}
          </button>
          <button
            className="btn btn-secondary codex-session-manager__action-button"
            type="button"
            onClick={() => void handleOpenSyncTargetModal()}
            disabled={syncing || syncingToInstance || repairingVisibility || deleting || loading || exporting || selectedIds.length === 0}
          >
            <Copy size={14} className={syncingToInstance ? 'icon-spin' : undefined} />
            {t('codex.sessionManager.actions.copyToInstance', '复制到实例')} ({selectedIds.length})
          </button>
          <button
            className="btn btn-secondary codex-session-manager__action-button"
	            type="button"
	            onClick={() => void handleExportSessions()}
	            disabled={syncing || syncingToInstance || repairingVisibility || deleting || loading || loadingExportPreview || exporting || selectedIds.length === 0}
	          >
	            <Download size={14} className={loadingExportPreview || exporting ? 'icon-spin' : undefined} />
	            {t('codex.sessionManager.actions.exportSessions', '导出会话')} ({selectedIds.length})
	          </button>
          <button
            className="btn btn-secondary codex-session-manager__action-button"
            type="button"
            onClick={() => void handleOpenImportModal()}
            disabled={loading || syncing || syncingToInstance || repairingVisibility || deleting || exporting || importing || loadingImportPreview}
          >
            <Upload size={14} className={loadingImportPreview ? 'icon-spin' : undefined} />
            {t('codex.sessionManager.actions.importSessions', '导入会话')}
          </button>
          <button
            className="btn btn-secondary codex-session-manager__action-button"
            type="button"
            onClick={() => void handleRepairVisibility()}
            disabled={repairingVisibility || loading || deleting || syncing || syncingToInstance || exporting}
          >
            <Eye size={14} />
            {t('codex.sessionManager.actions.repairVisibility', '修复可见性')}
          </button>
          <button
            className="btn btn-secondary codex-session-manager__action-button"
            type="button"
            onClick={() => void handleOpenRestoreModal()}
            disabled={loading || syncing || syncingToInstance || repairingVisibility || deleting || restoring || purgingTrash}
          >
            <Trash2 size={14} />
            {t('codex.sessionManager.actions.trash', '废纸篓')}
          </button>
          <button
            className="btn btn-secondary codex-session-manager__action-button"
            type="button"
            onClick={() => void handleRefresh()}
            disabled={loading || deleting || syncing || syncingToInstance || repairingVisibility}
          >
            <RefreshCw size={14} className={loading ? 'icon-spin' : undefined} />
            {t('common.refresh', '刷新')}
          </button>
          <button
            className="btn btn-danger codex-session-manager__action-button"
            type="button"
            onClick={() => void handleMoveToTrash()}
            disabled={deleting || loading || syncing || syncingToInstance || repairingVisibility || selectedIds.length === 0}
          >
            <Trash2 size={14} />
            {t('codex.sessionManager.actions.moveToTrash', '移到废纸篓')} ({selectedIds.length})
          </button>
        </div>
      </div>

      {message ? (
        <div className={`message-bar ${message.tone === 'error' ? 'error' : 'success'}`}>{message.text}</div>
      ) : null}

      {transferTask && !showTransferModal ? (
        <div className={`codex-session-transfer-task is-${transferTask.status}`}>
          <div className="codex-session-transfer-task__copy">
            <strong>{getTransferTitle(transferTask.operation)}</strong>
            <span>
              {transferTask.status === 'running'
                ? t('codex.sessionManager.transferModal.runningSummary', {
                    defaultValue: '正在处理 {{current}}/{{total}}',
                    current: transferProgress?.current ?? 0,
                    total: transferProgress?.total ?? 0,
                  })
                : transferTask.status === 'success'
                  ? t('codex.sessionManager.transferModal.completedSummary', '任务已完成')
                  : t('codex.sessionManager.transferModal.failedSummary', '任务失败')}
            </span>
            <div className="codex-session-transfer-task__progress" aria-hidden="true">
              <span style={{ width: `${transferPercent}%` }} />
            </div>
          </div>
          <div className="codex-session-transfer-task__actions">
            <button className="btn btn-secondary" type="button" onClick={() => setShowTransferModal(true)}>
              <Eye size={14} />
              {t('codex.sessionManager.transferModal.reopen', '查看进度')}
            </button>
            {transferTask.status !== 'running' ? (
              <button className="btn btn-secondary" type="button" onClick={handleClearTransferTask}>
                <X size={14} />
                {t('codex.sessionManager.transferModal.clear', '清除')}
              </button>
            ) : null}
          </div>
        </div>
      ) : null}

      {loading && sessions.length === 0 ? (
        <div className="empty-state">
          <h3>{t('common.loading', '加载中...')}</h3>
        </div>
      ) : null}

      {!loading && groupedSessions.length === 0 ? (
        <div className="empty-state codex-session-manager__empty">
          <Folder size={42} className="empty-icon" />
          <h3>
            {hasAppliedSearch
              ? t('codex.sessionManager.empty.searchTitle', '未找到匹配会话')
              : t('codex.sessionManager.empty.title', '还没有可管理的会话')}
          </h3>
          <p>
            {hasAppliedSearch
              ? t('codex.sessionManager.empty.searchDesc', '请调整标题关键词后再试。')
              : t('codex.sessionManager.empty.desc', '当前实例集合中还没有发现会话记录。')}
          </p>
        </div>
      ) : null}

      {groupedSessions.length > 0 ? (
        <div className="codex-session-manager__list">
          {groupedSessions.map((group) => {
            const groupSessionIds = group.sessions.map((item) => item.sessionId);
            const allSelected = groupSessionIds.every((id) => selectedIdSet.has(id));
            const isExpanded = expandedGroups.includes(group.cwd);
            const isTokenStatsLoading = loadingTokenGroupSet.has(group.cwd);
            return (
              <section className="codex-session-folder" key={group.cwd}>
                <div className="codex-session-folder__row">
                  <div className="codex-session-folder__left">
                    <button
                      className="codex-session-folder__expand"
                      type="button"
                      onClick={() => toggleGroupExpanded(group.cwd)}
                      aria-label={
                        isExpanded
                          ? t('codex.sessionManager.actions.collapse', '收起')
                          : t('codex.sessionManager.actions.expand', '展开')
                      }
                    >
                      {isExpanded ? <ChevronDown size={16} /> : <ChevronRight size={16} />}
                    </button>
                    <input
                      className="codex-session-folder__checkbox"
                      type="checkbox"
                      checked={allSelected && groupSessionIds.length > 0}
                      onChange={() => toggleGroupSelection(groupSessionIds)}
                    />
                    <Folder size={16} className="codex-session-folder__icon" />
                    <button
                      className="codex-session-folder__label"
                      type="button"
                      onClick={() => toggleGroupExpanded(group.cwd)}
                      title={group.cwd}
                    >
                      {resolveGroupLabel(group.cwd)}
                    </button>
                  </div>
                  <span className="codex-session-folder__time">
                    {formatRelativeTime(group.latestUpdatedAt, isZh)}
                  </span>
                </div>
                {isExpanded ? (
                  <div className="codex-session-folder__children">
                    {group.sessions.map((session) => {
                      const hasRunningLocation = session.locations.some((location) => location.running);
                      const tokenText = formatTokenStats(tokenStatsBySessionId[session.sessionId]);
                      return (
                        <div className="codex-session-row" key={session.sessionId}>
                          <label className="codex-session-row__left">
                            <input
                              className="codex-session-row__checkbox"
                              type="checkbox"
                              checked={selectedIdSet.has(session.sessionId)}
                              onChange={() => toggleSession(session.sessionId)}
                            />
                            <div className="codex-session-row__content">
                              <span className="codex-session-row__title" title={session.title}>
                                {session.title || t('codex.sessionManager.untitled', '未命名会话')}
                              </span>
                              <span className="codex-session-row__meta">
                                {session.locations.map((location) => location.instanceName).join(' / ')}
                                {hasRunningLocation
                                  ? t('codex.sessionManager.locationRunning', '（运行中）')
                                  : ''}
                              </span>
                              <span className="codex-session-row__meta codex-session-row__session-id" title={session.sessionId}>
                                {t('codex.sessionManager.labels.sessionId', '会话 ID')}: {formatSessionId(session.sessionId)}
                              </span>
                            </div>
                          </label>
                          <div className="codex-session-row__right">
                            <button
                              className={`codex-session-row__copy-button${copiedSessionId === session.sessionId ? ' is-copied' : ''}`}
                              type="button"
                              onClick={(event) => void handleCopySessionId(event, session.sessionId)}
                              title={t('codex.sessionManager.actions.copySessionId', '复制会话 ID')}
                              aria-label={t('codex.sessionManager.actions.copySessionId', '复制会话 ID')}
                            >
                              {copiedSessionId === session.sessionId ? <Check size={14} /> : <Copy size={14} />}
                            </button>
                            <button
                              className="codex-session-row__copy-button"
                              type="button"
                              onClick={(event) => void handleOpenSessionLocation(event, session.sessionId)}
                              title={t('codex.sessionManager.actions.openLocation', '打开位置')}
                              aria-label={t('codex.sessionManager.actions.openLocation', '打开位置')}
                            >
                              <FolderOpen size={14} />
                            </button>
                            {tokenText ? (
                              <span className="codex-session-row__tokens" title={t('codex.sessionManager.labels.tokenUsage', 'Token使用')}>
                                {tokenText}
                              </span>
                            ) : null}
                            {!tokenText && isTokenStatsLoading ? (
                              <span className="codex-session-row__tokens" title={t('common.loading', '加载中...')}>
                                <RefreshCw size={12} className="icon-spin" />
                              </span>
                            ) : null}
                            <span className="codex-session-row__time">
                              {formatRelativeTime(session.updatedAt, isZh)}
                            </span>
                          </div>
                        </div>
                      );
                    })}
                  </div>
                ) : null}
              </section>
            );
          })}
        </div>
      ) : null}

      {showSyncTargetModal ? (
        <div className="modal-overlay">
          <div className="modal codex-session-target-modal" onClick={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('codex.sessionManager.targetModal.title', '复制到实例')}</h2>
              <button
                className="modal-close"
                type="button"
                onClick={handleCloseSyncTargetModal}
                disabled={syncingToInstance}
                aria-label={t('common.close', '关闭')}
              >
                <X size={18} />
              </button>
            </div>
            <div className="modal-body">
              <ModalErrorMessage message={syncTargetModalError} scrollKey={syncTargetModalErrorScrollKey} />
              <p className="codex-session-target-modal__hint">
                {t(
                  'codex.sessionManager.targetModal.hint',
                  '会把所选会话的 rollout、session_index 条目和会话文件时间补到目标实例，并触发官方 Codex 重建会话索引；已有同 ID 会话会自动跳过。',
                )}
              </p>
              <label className="codex-session-target-modal__field">
                <span>{t('codex.sessionManager.targetModal.targetInstance', '目标实例')}</span>
                <SingleSelectDropdown
                  className="codex-session-target-modal__select"
                  value={syncTargetInstanceId}
                  options={targetInstanceOptions}
                  onChange={(value) => {
                    setSyncTargetInstanceId(value);
                    setSyncTargetModalError(null);
                  }}
                  disabled={syncingToInstance}
                  ariaLabel={t('codex.sessionManager.targetModal.targetInstance', '目标实例')}
                  menuMaxHeight={240}
                />
              </label>
              <div className="codex-session-target-modal__summary">
                <span>
                  {t('codex.sessionManager.targetModal.selectedCount', {
                    defaultValue: '已选择 {{count}} 条会话',
                    count: selectedIds.length,
                  })}
                </span>
                {syncTargetInstance ? (
                  <span>
                    {t('codex.sessionManager.targetModal.existingCount', {
                      defaultValue: '目标已存在 {{count}} 条',
                      count: syncTargetExistingCount,
                    })}
                  </span>
                ) : null}
              </div>
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                type="button"
                onClick={handleCloseSyncTargetModal}
                disabled={syncingToInstance}
              >
                {t('common.cancel', '取消')}
              </button>
              <button
                className="btn btn-primary"
                type="button"
                onClick={() => void handleSyncSelectedToInstance()}
                disabled={syncingToInstance || !syncTargetInstanceId || selectedIds.length === 0}
              >
                <Copy size={14} className={syncingToInstance ? 'icon-spin' : undefined} />
                {t('codex.sessionManager.targetModal.confirm', '复制会话')}
              </button>
            </div>
          </div>
        </div>
      ) : null}

      {showExportModal ? (
        <div className="modal-overlay">
          <div className="modal codex-session-export-modal" onClick={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('codex.sessionManager.exportModal.title', '导出会话')}</h2>
              <button
                className="modal-close"
                type="button"
                onClick={handleCloseExportModal}
                disabled={loadingExportPreview || exporting}
                aria-label={t('common.close', '关闭')}
              >
                <X size={18} />
              </button>
            </div>
            <div className="modal-body">
              <ModalErrorMessage message={exportModalError} scrollKey={exportModalErrorScrollKey} />
              <p className="codex-session-export-modal__hint">
                {t(
                  'codex.sessionManager.exportModal.hint',
                  '导出前可先确认会话列表、来源实例和文件大小；会话包只包含 rollout 文件和 session_index 条目，不包含账号、Token、API Key 或应用配置。',
                )}
              </p>
              {loadingExportPreview ? (
                <div className="codex-session-restore-modal__empty">
                  <RefreshCw size={28} className="icon-spin empty-icon" />
                  <h3>{t('codex.sessionManager.exportModal.previewing', '正在预览会话...')}</h3>
                </div>
              ) : null}
              {!loadingExportPreview && exportPreview ? (
                <>
                  <div className="codex-session-export-modal__summary">
                    <span>
                      {t('codex.sessionManager.exportModal.selectedExportCount', {
                        defaultValue: '已选 {{selected}} / 可导出 {{total}}',
                        selected: exportableSessionIds.length,
                        total: exportAvailableItems.length,
                      })}
                    </span>
                    <span>
                      {t('codex.sessionManager.exportModal.selectedSize', {
                        defaultValue: '已选大小 {{selected}} / 总计 {{total}}',
                        selected: formatBytes(exportSelectedSizeBytes),
                        total: formatBytes(exportAvailableSizeBytes),
                      })}
                    </span>
                    <span>
                      {t('codex.sessionManager.exportModal.missingCount', {
                        defaultValue: '缺失 {{count}} 条',
                        count: exportPreview.missingSessionCount,
                      })}
                    </span>
                  </div>
                  {removedExportIds.length > 0 ? (
                    <div className="codex-session-export-modal__notice is-neutral">
                      <span>
                        {t('codex.sessionManager.exportModal.removedNotice', {
                          defaultValue: '已从本次导出列表移出 {{count}} 条',
                          count: removedExportIds.length,
                        })}
                      </span>
                      <button
                        className="btn btn-secondary codex-session-export-modal__notice-action"
                        type="button"
                        onClick={restoreRemovedExportItems}
                        disabled={exporting}
                      >
                        <RotateCcw size={13} />
                        {t('codex.sessionManager.exportModal.restoreRemoved', '恢复列表')}
                      </button>
                    </div>
                  ) : null}
                  <div className="codex-session-export-modal__path">
                    <div className="codex-session-export-modal__path-copy">
                      <strong>{t('codex.sessionManager.exportModal.exportPath', '导出位置')}</strong>
                      <span title={exportPath}>
                        {exportPath || t('codex.sessionManager.exportModal.pathUnset', '尚未选择导出位置')}
                      </span>
                    </div>
                    <button
                      className="btn btn-secondary"
                      type="button"
                      onClick={() => void handleChooseExportPath()}
                      disabled={exporting}
                    >
                      <FolderOpen size={14} />
                      {t('codex.sessionManager.exportModal.choosePath', '选择位置')}
                    </button>
                  </div>
                  {exportPreview.missingSessionCount > 0 ? (
                    <div className="codex-session-export-modal__notice">
                      {t('codex.sessionManager.exportModal.missingNotice', {
                        defaultValue: '{{count}} 条会话已不在当前实例集合中，导出时会跳过。',
                        count: exportPreview.missingSessionCount,
                      })}
                    </div>
                  ) : null}
                  {exportAvailableItems.length > 0 ? (
                    <>
                      <div className="codex-session-export-filter">
                        <label className="codex-session-export-filter__field">
                          <span>{t('codex.sessionManager.exportModal.sourceLabel', '来源')}</span>
                          <SingleSelectDropdown
                            className="codex-session-export-filter__select"
                            value={exportSourceFilter}
                            options={exportSourceOptions}
                            onChange={setExportSourceFilter}
                            disabled={exporting}
                            ariaLabel={t('codex.sessionManager.exportModal.sourceLabel', '来源')}
                            menuMaxHeight={240}
                          />
                        </label>
                        <label className="codex-session-export-filter__field">
                          <span>{t('codex.sessionManager.exportModal.selectionLabel', '选择')}</span>
                          <SingleSelectDropdown
                            className="codex-session-export-filter__select"
                            value={exportSelectionFilter}
                            options={exportSelectionFilterOptions}
                            onChange={(value) => setExportSelectionFilter(value as ExportSelectionFilter)}
                            disabled={exporting}
                            ariaLabel={t('codex.sessionManager.exportModal.selectionLabel', '选择')}
                            menuMaxHeight={220}
                          />
                        </label>
                      </div>
                      <div className="codex-session-export-actions">
                        <span>
                          {t('codex.sessionManager.exportModal.visibleCount', {
                            defaultValue: '当前显示 {{visible}} / {{total}}',
                            visible: exportFilteredItems.length,
                            total: exportAvailableItems.length,
                          })}
                        </span>
                        <button
                          className="btn btn-secondary"
                          type="button"
                          onClick={() => addExportSelection(exportFilteredItems)}
                          disabled={exporting || exportFilteredItems.length === 0}
                        >
                          <Check size={13} />
                          {t('codex.sessionManager.exportModal.selectVisible', '选中筛选')}
                        </button>
                        <button
                          className="btn btn-secondary"
                          type="button"
                          onClick={() => removeExportSelection(exportFilteredItems)}
                          disabled={exporting || exportFilteredItems.length === 0}
                        >
                          <X size={13} />
                          {t('codex.sessionManager.exportModal.clearVisible', '取消筛选')}
                        </button>
                        <button
                          className="btn btn-secondary"
                          type="button"
                          onClick={keepFilteredExportItems}
                          disabled={exporting || exportFilteredItems.length === 0}
                        >
                          {t('codex.sessionManager.exportModal.keepVisibleOnly', '仅保留筛选')}
                        </button>
                        <button
                          className="btn btn-secondary"
                          type="button"
                          onClick={() => removeExportItems(exportFilteredItems)}
                          disabled={exporting || exportFilteredItems.length === 0}
                        >
                          <Trash2 size={13} />
                          {t('codex.sessionManager.exportModal.removeVisible', '移出筛选')}
                        </button>
                        <button
                          className="btn btn-secondary"
                          type="button"
                          onClick={removeUncheckedExportItems}
                          disabled={exporting || exportAvailableItems.length === exportableSessionIds.length}
                        >
                          {t('codex.sessionManager.exportModal.removeUnchecked', '移出未选')}
                        </button>
                      </div>
                      {exportFilteredItems.length > 0 ? (
                        <div className="codex-session-export-list">
                          {exportFilteredItems.map((item: CodexSessionExportPreviewItem) => {
                            const selected = selectedExportIdSet.has(item.sessionId);
                            return (
                              <div
                                className={`codex-session-export-row${selected ? ' is-selected' : ''}`}
                                key={item.sessionId}
                              >
                                <label className="codex-session-export-row__check">
                                  <input
                                    className="codex-session-row__checkbox"
                                    type="checkbox"
                                    checked={selected}
                                    disabled={exporting}
                                    onChange={() => toggleExportSession(item.sessionId)}
                                  />
                                  <span className="sr-only">
                                    {t('codex.sessionManager.exportModal.selectItem', '选择导出会话')}
                                  </span>
                                </label>
                                <div className="codex-session-export-row__content">
                                  <span className="codex-session-export-row__title" title={item.title}>
                                    {item.title || t('codex.sessionManager.untitled', '未命名会话')}
                                  </span>
                                  <span className="codex-session-export-row__meta" title={item.cwd}>
                                    {item.cwd}
                                  </span>
                                  <span className="codex-session-export-row__meta">
                                    {t('codex.sessionManager.exportModal.sourceInstance', {
                                      defaultValue: '来源：{{name}}',
                                      name: item.sourceInstanceName,
                                    })}
                                  </span>
                                </div>
                                <div className="codex-session-export-row__right">
                                  <span className="codex-session-import-row__size">{formatBytes(item.sizeBytes)}</span>
                                  <span className="codex-session-row__time">
                                    {formatRelativeTime(item.updatedAt, isZh)}
                                  </span>
                                  <button
                                    className="btn btn-secondary codex-session-export-row__remove"
                                    type="button"
                                    onClick={() => removeExportItems([item])}
                                    disabled={exporting}
                                  >
                                    <Trash2 size={13} />
                                    {t('codex.sessionManager.exportModal.removeItem', '移出')}
                                  </button>
                                </div>
                              </div>
                            );
                          })}
                        </div>
                      ) : (
                        <div className="codex-session-restore-modal__empty">
                          <Search size={32} className="empty-icon" />
                          <h3>{t('codex.sessionManager.exportModal.emptyFilteredTitle', '当前筛选无会话')}</h3>
                          <p>{t('codex.sessionManager.exportModal.emptyFilteredDesc', '调整搜索、来源或选择状态后再试。')}</p>
                        </div>
                      )}
                    </>
                  ) : (
                    <div className="codex-session-restore-modal__empty">
                      <Folder size={32} className="empty-icon" />
                      <h3>{t('codex.sessionManager.exportModal.emptyTitle', '没有可导出的会话')}</h3>
                    </div>
                  )}
                </>
              ) : null}
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                type="button"
                onClick={handleCloseExportModal}
                disabled={loadingExportPreview || exporting}
              >
                {t('common.cancel', '取消')}
              </button>
              <button
                className="btn btn-primary"
                type="button"
                onClick={() => void handleConfirmExportSessions()}
                disabled={loadingExportPreview || exporting || !exportPath || exportableSessionIds.length === 0}
              >
                <Download size={14} className={exporting ? 'icon-spin' : undefined} />
                {t('codex.sessionManager.exportModal.confirm', '确认导出')} ({exportableSessionIds.length})
              </button>
            </div>
          </div>
        </div>
      ) : null}

      {showImportModal ? (
        <div className="modal-overlay">
          <div className="modal codex-session-import-modal" onClick={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('codex.sessionManager.importModal.title', '导入会话')}</h2>
              <button
                className="modal-close"
                type="button"
                onClick={handleCloseImportModal}
                disabled={importing || loadingImportPreview}
                aria-label={t('common.close', '关闭')}
              >
                <X size={18} />
              </button>
            </div>
            <div className="modal-body">
              <ModalErrorMessage message={importModalError} scrollKey={importModalErrorScrollKey} />
              <p className="codex-session-import-modal__hint">
                {t(
                  'codex.sessionManager.importModal.hint',
                  '会话包只导入 rollout 文件和 session_index 条目，不包含账号、Token、API Key 或应用配置；目标实例已有同 ID 会话时会跳过。',
                )}
              </p>
              <label className="codex-session-target-modal__field">
                <span>{t('codex.sessionManager.targetModal.targetInstance', '目标实例')}</span>
                <SingleSelectDropdown
                  className="codex-session-target-modal__select"
                  value={importTargetInstanceId}
                  options={importTargetOptions}
                  onChange={(value) => void handleChangeImportTarget(value)}
                  disabled={importing || loadingImportPreview}
                  ariaLabel={t('codex.sessionManager.targetModal.targetInstance', '目标实例')}
                  menuMaxHeight={240}
                />
              </label>
              {importPreview ? (
                <div className="codex-session-import-modal__summary">
                  <span>
                    {t('codex.sessionManager.importModal.totalCount', {
                      defaultValue: '会话包 {{count}} 条',
                      count: importPreview.totalSessionCount,
                    })}
                  </span>
                  <span>
                    {t('codex.sessionManager.importModal.readyCount', {
                      defaultValue: '可导入 {{count}} 条',
                      count: importPreview.importableSessionCount,
                    })}
                  </span>
                  <button
                    className="btn btn-secondary codex-session-import-modal__select-all"
                    type="button"
                    onClick={toggleAllImportReady}
                    disabled={importing || loadingImportPreview || importReadyItems.length === 0}
                  >
                    {allImportReadySelected
                      ? t('codex.sessionManager.actions.clearSelectedSessions', '取消全选')
                      : t('codex.sessionManager.importModal.selectReady', '选择可导入')}
                  </button>
                </div>
              ) : null}
              {loadingImportPreview ? (
                <div className="codex-session-restore-modal__empty">
                  <RefreshCw size={28} className="icon-spin empty-icon" />
                  <h3>{t('codex.sessionManager.importModal.previewing', '正在预览会话包...')}</h3>
                </div>
              ) : null}
              {!loadingImportPreview && importPreview ? (
                <div className="codex-session-import-list">
                  {importPreview.items.map((item) => (
                    <label
                      className={`codex-session-import-row is-${item.status}`}
                      key={item.sessionId}
                    >
                      <div className="codex-session-import-row__left">
                        <input
                          className="codex-session-row__checkbox"
                          type="checkbox"
                          checked={selectedImportIdSet.has(item.sessionId)}
                          disabled={item.status !== 'ready' || importing}
                          onChange={() => toggleImportSession(item)}
                        />
                        <div className="codex-session-import-row__content">
                          <span className="codex-session-import-row__title" title={item.title}>
                            {item.title || t('codex.sessionManager.untitled', '未命名会话')}
                          </span>
                          <span className="codex-session-import-row__meta" title={item.cwd}>
                            {item.cwd}
                          </span>
                          {item.existingInstanceNames.length > 0 ? (
                            <span className="codex-session-import-row__meta">
                              {t('codex.sessionManager.importModal.existsIn', {
                                defaultValue: '已存在于：{{names}}',
                                names: item.existingInstanceNames.join(' / '),
                              })}
                            </span>
                          ) : null}
                          {item.reason ? (
                            <span className="codex-session-import-row__reason">{item.reason}</span>
                          ) : null}
                        </div>
                      </div>
                      <div className="codex-session-import-row__right">
                        <span className={`codex-session-import-row__status is-${item.status}`}>
                          {getImportStatusLabel(item)}
                        </span>
                        <span className="codex-session-import-row__size">{formatBytes(item.sizeBytes)}</span>
                      </div>
                    </label>
                  ))}
                </div>
              ) : null}
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                type="button"
                onClick={handleCloseImportModal}
                disabled={importing || loadingImportPreview}
              >
                {t('common.cancel', '取消')}
              </button>
              <button
                className="btn btn-primary"
                type="button"
                onClick={() => void handleImportSelectedSessions()}
                disabled={importing || loadingImportPreview || selectedImportIds.length === 0}
              >
                <Upload size={14} className={importing ? 'icon-spin' : undefined} />
                {t('codex.sessionManager.importModal.confirm', '导入选中会话')} ({selectedImportIds.length})
              </button>
            </div>
          </div>
        </div>
      ) : null}

      {transferTask && showTransferModal ? (
        <div className="modal-overlay">
          <div
            className={`modal codex-session-transfer-modal is-${transferTask.status}`}
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <h2>{getTransferTitle(transferTask.operation)}</h2>
              <button
                className="modal-close"
                type="button"
                onClick={transferRunning ? handleMinimizeTransferModal : handleClearTransferTask}
                aria-label={
                  transferRunning
                    ? t('codex.sessionManager.transferModal.minimize', '最小化')
                    : t('common.close', '关闭')
                }
              >
                {transferRunning ? <Minimize2 size={18} /> : <X size={18} />}
              </button>
            </div>
            <div className="modal-body">
              <div className="codex-session-transfer-progress" role="status">
                <div className="codex-session-transfer-progress__head">
                  <strong>{getTransferPhaseText(transferProgress)}</strong>
                  <span>{transferPercent}%</span>
                </div>
                <div className="codex-session-transfer-progress__bar">
                  <span style={{ width: `${transferPercent}%` }} />
                </div>
                <div className="codex-session-transfer-current">
                  <span>
                    {t('codex.sessionManager.transferModal.current', {
                      defaultValue: '{{current}} / {{total}}',
                      current: transferProgress?.current ?? 0,
                      total: transferProgress?.total ?? 0,
                    })}
                  </span>
                  {transferProgress?.currentLabel ? <span>{transferProgress.currentLabel}</span> : null}
                </div>
              </div>
              {transferTask.message ? (
                <div className="codex-session-transfer-result is-success">{transferTask.message}</div>
              ) : null}
              {transferTask.error ? (
                <div className="codex-session-transfer-result is-error">{transferTask.error}</div>
              ) : null}
            </div>
            <div className="modal-footer">
              {transferRunning ? (
                <button className="btn btn-secondary" type="button" onClick={handleMinimizeTransferModal}>
                  <Minimize2 size={14} />
                  {t('codex.sessionManager.transferModal.minimize', '最小化')}
                </button>
              ) : (
                <button className="btn btn-primary" type="button" onClick={handleClearTransferTask}>
                  <X size={14} />
                  {t('common.close', '关闭')}
                </button>
              )}
            </div>
          </div>
        </div>
      ) : null}

      <CodexSessionVisibilityRepairModal
        open={showRepairVisibilityModal}
        selectedSessionIds={selectedIds}
        totalSessionCount={allSessionIds.length}
        onClose={() => setShowRepairVisibilityModal(false)}
        onRunningChange={setRepairingVisibility}
        onRepaired={() => loadSessions()}
      />

      {showRestoreModal ? (
        <div className="modal-overlay">
          <div className="modal codex-session-restore-modal" onClick={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('codex.sessionManager.restoreModal.title', '废纸篓')}</h2>
              <button
                className="modal-close"
                type="button"
                onClick={handleCloseRestoreModal}
                disabled={restoring || purgingTrash}
                aria-label={t('common.close', '关闭')}
              >
                <X size={18} />
              </button>
            </div>
            <div className="modal-body">
              <ModalErrorMessage message={restoreModalError} scrollKey={restoreModalErrorScrollKey} />
              {loadingTrash ? (
                <div className="codex-session-restore-modal__empty">
                  <h3>{t('common.loading', '加载中...')}</h3>
                </div>
              ) : null}
              {!loadingTrash && trashedSessions.length === 0 ? (
                <div className="codex-session-restore-modal__empty">
                  <Folder size={36} className="empty-icon" />
                  <h3>{t('codex.sessionManager.restoreModal.emptyTitle', '废纸篓里还没有会话')}</h3>
                  <p>{t('codex.sessionManager.restoreModal.emptyDesc', '已移到废纸篓的会话会显示在这里。')}</p>
                </div>
              ) : null}
              {!loadingTrash && trashedSessions.length > 0 ? (
                <>
                  <div className="codex-session-restore-modal__summary">
                    <span>
                      {t('codex.sessionManager.restoreModal.summary', '共 {{count}} 条，{{size}}', {
                        count: trashedSessions.length,
                        size: formatBytes(trashTotalSizeBytes),
                      })}
                    </span>
                    <span>
                      {t('codex.sessionManager.restoreModal.selectedSummary', '已选 {{count}} 条，{{size}}', {
                        count: selectedTrashIds.length,
                        size: formatBytes(selectedTrashSizeBytes),
                      })}
                    </span>
                  </div>
                  <div className="codex-session-restore-actions">
                    <button
                      className="btn btn-secondary"
                      type="button"
                      onClick={toggleAllTrashedSessions}
                      disabled={trashBusy}
                    >
                      {allTrashSelected
                        ? t('codex.sessionManager.restoreModal.clearSelected', '取消选择')
                        : t('codex.sessionManager.restoreModal.selectAll', '全选')}
                    </button>
                    <button
                      className="btn btn-danger"
                      type="button"
                      onClick={() => void handleEmptySessionTrash()}
                      disabled={trashBusy || trashedSessions.length === 0}
                    >
                      <Trash2 size={14} className={purgingTrash && selectedTrashIds.length === 0 ? 'icon-spin' : undefined} />
                      {t('codex.sessionManager.restoreModal.emptyTrash', '清空废纸篓')}
                    </button>
                  </div>
                  <p className="codex-session-restore-modal__hint">
                    {t(
                      'codex.sessionManager.restoreModal.hint',
                      '废纸篓中的会话可以恢复到原实例，也可以永久删除以释放磁盘空间。',
                    )}
                  </p>
                  <div className="codex-session-restore-list">
                    {trashedSessions.map((session) => (
                      <div className="codex-session-restore-row" key={session.sessionId}>
                        <div className="codex-session-restore-row__left">
                          <input
                            className="codex-session-row__checkbox"
                            type="checkbox"
                            checked={selectedTrashIdSet.has(session.sessionId)}
                            disabled={trashBusy}
                            onChange={() => toggleTrashedSession(session.sessionId)}
                          />
                          <div className="codex-session-restore-row__content">
                            <span className="codex-session-restore-row__title" title={session.title}>
                              {session.title || t('codex.sessionManager.untitled', '未命名会话')}
                            </span>
                            <span className="codex-session-restore-row__meta">
                              {session.locations.map((location) => location.instanceName).join(' / ')}
                            </span>
                            <span className="codex-session-restore-row__meta">
                              {t('codex.sessionManager.restoreModal.itemSize', '大小 {{size}}', {
                                size: formatBytes(session.sizeBytes ?? 0),
                              })}
                            </span>
                            <span className="codex-session-restore-row__meta codex-session-restore-row__cwd">
                              {session.cwd}
                            </span>
                          </div>
                        </div>
                        <div className="codex-session-restore-row__side">
                          <span className="codex-session-row__time">
                            {formatRelativeTime(session.deletedAt, isZh)}
                          </span>
                          <button
                            className="btn btn-danger codex-session-restore-row__delete"
                            type="button"
                            onClick={() => void handleDeleteTrashedSessions([session.sessionId])}
                            disabled={trashBusy}
                          >
                            <Trash2 size={13} />
                            {t('codex.sessionManager.restoreModal.deleteOne', '永久删除')}
                          </button>
                        </div>
                      </div>
                    ))}
                  </div>
                </>
              ) : null}
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                type="button"
                onClick={handleCloseRestoreModal}
                disabled={restoring || purgingTrash}
              >
                {t('common.cancel', '取消')}
              </button>
              <button
                className="btn btn-danger"
                type="button"
                onClick={() => void handleDeleteTrashedSessions(selectedTrashIds)}
                disabled={trashBusy || selectedTrashIds.length === 0}
              >
                <Trash2 size={14} className={purgingTrash && selectedTrashIds.length > 0 ? 'icon-spin' : undefined} />
                {t('codex.sessionManager.restoreModal.deleteSelected', '永久删除选中')} ({selectedTrashIds.length})
              </button>
              <button
                className="btn btn-primary"
                type="button"
                onClick={() => void handleRestoreFromTrash()}
                disabled={trashBusy || selectedTrashIds.length === 0}
              >
                <RotateCcw size={14} className={restoring ? 'icon-spin' : undefined} />
                {t('codex.sessionManager.restoreModal.restoreAction', '恢复选中会话')} ({selectedTrashIds.length})
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </section>
  );
}
