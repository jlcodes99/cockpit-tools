import { type MouseEvent, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { confirm as nativeConfirmDialog } from '@tauri-apps/plugin-dialog';
import { Check, ChevronDown, ChevronRight, Copy, Eye, Folder, RefreshCw, RotateCcw, Trash2, X } from 'lucide-react';
import { ModalErrorMessage, useModalErrorState } from '../ModalErrorMessage';
import { useEscClose } from '../../hooks/useEscClose';
import type {
  CodexSessionRecord,
  CodexSessionRolloutBackupBatch,
  CodexSessionRestorePreviewSummary,
  CodexSessionSourceRepairCandidate,
  CodexSessionTokenStats,
  CodexTrashedSessionRecord,
} from '../../types/codex';
import { useCodexInstanceStore } from '../../stores/useCodexInstanceStore';
import { formatCodexSessionVisibilityRepairMessage } from '../../utils/codexSessionVisibility';

type MessageState = { text: string; tone?: 'error' };
type SessionTokenStatsMap = Record<string, CodexSessionTokenStats>;
const SOURCE_REPAIR_PREVIEW_LIMIT = 20;
const SOURCE_REPAIR_TITLE_PREVIEW_LENGTH = 30;

type SessionGroup = {
  cwd: string;
  sessions: CodexSessionRecord[];
  latestUpdatedAt: number;
  archived: boolean;
  hasArchivedLocation: boolean;
  projectless: boolean;
  removed: boolean;
  hasRemovedLocation: boolean;
};

function buildGroups(sessions: CodexSessionRecord[]): SessionGroup[] {
  const groups = new Map<string, CodexSessionRecord[]>();
  sessions.forEach((session) => {
    const groupKey = session.projectless ? '__codex_projectless__' : session.cwd;
    const bucket = groups.get(groupKey) ?? [];
    bucket.push(session);
    groups.set(groupKey, bucket);
  });

  return Array.from(groups.entries())
    .map(([cwd, groupSessions]) => ({
      cwd,
      sessions: [...groupSessions].sort(
        (left, right) => (right.updatedAt ?? 0) - (left.updatedAt ?? 0) || left.title.localeCompare(right.title),
      ),
      latestUpdatedAt: Math.max(...groupSessions.map((item) => item.updatedAt ?? 0), 0),
      archived: groupSessions.every((item) => item.archived),
      hasArchivedLocation: groupSessions.some((item) => item.hasArchivedLocation || item.archived),
      projectless: groupSessions.every((item) => item.projectless),
      removed: groupSessions.every((item) => item.removed),
      hasRemovedLocation: groupSessions.some((item) => item.hasRemovedLocation || item.removed),
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

function formatLargeNumber(value: number): string {
  if (value >= 1_000_000) {
    return `${(value / 1_000_000).toFixed(1)}M`;
  }
  if (value >= 1_000) {
    return `${(value / 1_000).toFixed(1)}K`;
  }
  return value.toLocaleString();
}

function truncateSourceRepairTitle(title: string, maxLength = SOURCE_REPAIR_TITLE_PREVIEW_LENGTH): string {
  const normalizedTitle = title.trim();
  const chars = Array.from(normalizedTitle);
  if (chars.length <= maxLength) {
    return normalizedTitle;
  }
  return `${chars.slice(0, Math.max(0, maxLength - 3)).join('')}...`;
}

function buildSourceRepairCandidatePreview(
  candidates: CodexSessionSourceRepairCandidate[],
  limit = SOURCE_REPAIR_PREVIEW_LIMIT,
): {
  visible: Array<{ key: string; title: string }>;
  hiddenCount: number;
} {
  const visible = candidates.slice(0, limit);
  return {
    visible: visible.map((candidate, index) => ({
      key: `${candidate.instanceId}:${candidate.sessionId}:${index}`,
      title: truncateSourceRepairTitle(candidate.title?.trim() || candidate.sessionId),
    })),
    hiddenCount: Math.max(0, candidates.length - limit),
  };
}

function formatTokenStats(stats?: CodexSessionTokenStats): string {
  if (stats) {
    return `${formatLargeNumber(stats.inputTokens)} / ${formatLargeNumber(stats.outputTokens)} tokens`;
  }

  return '';
}

export function CodexSessionManager() {
  const { t, i18n } = useTranslation();
  const instances = useCodexInstanceStore((state) => state.instances);
  const refreshInstances = useCodexInstanceStore((state) => state.refreshInstances);
  const syncThreadsAcrossInstances = useCodexInstanceStore((state) => state.syncThreadsAcrossInstances);
  const syncSessionsToInstance = useCodexInstanceStore((state) => state.syncSessionsToInstance);
  const repairSessionVisibilityAcrossInstances = useCodexInstanceStore(
    (state) => state.repairSessionVisibilityAcrossInstances,
  );
  const previewSessionVisibilitySourceRepairs = useCodexInstanceStore(
    (state) => state.previewSessionVisibilitySourceRepairs,
  );
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
  const previewRestoreSessionsFromTrashAcrossInstances = useCodexInstanceStore(
    (state) => state.previewRestoreSessionsFromTrashAcrossInstances,
  );
  const restoreSessionsFromTrashAcrossInstances = useCodexInstanceStore(
    (state) => state.restoreSessionsFromTrashAcrossInstances,
  );
  const listSessionRestoreRolloutBackups = useCodexInstanceStore(
    (state) => state.listSessionRestoreRolloutBackups,
  );
  const restoreSessionRestoreRolloutBackup = useCodexInstanceStore(
    (state) => state.restoreSessionRestoreRolloutBackup,
  );
  const [sessions, setSessions] = useState<CodexSessionRecord[]>([]);
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [expandedGroups, setExpandedGroups] = useState<string[]>([]);
  const [showSyncTargetModal, setShowSyncTargetModal] = useState(false);
  const [syncTargetInstanceId, setSyncTargetInstanceId] = useState('');
  const [showRestoreModal, setShowRestoreModal] = useState(false);
  const [trashedSessions, setTrashedSessions] = useState<CodexTrashedSessionRecord[]>([]);
  const [selectedTrashIds, setSelectedTrashIds] = useState<string[]>([]);
  const [rolloutBackupBatches, setRolloutBackupBatches] = useState<CodexSessionRolloutBackupBatch[]>([]);
  const [showRolloutBackups, setShowRolloutBackups] = useState(false);
  const [sourceRepairCandidates, setSourceRepairCandidates] = useState<CodexSessionSourceRepairCandidate[] | null>(
    null,
  );
  const [loading, setLoading] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [syncingToInstance, setSyncingToInstance] = useState(false);
  const [repairingVisibility, setRepairingVisibility] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [loadingTrash, setLoadingTrash] = useState(false);
  const [loadingRolloutBackups, setLoadingRolloutBackups] = useState(false);
  const [restoring, setRestoring] = useState(false);
  const [message, setMessage] = useState<MessageState | null>(null);
  const [copiedSessionId, setCopiedSessionId] = useState<string | null>(null);
  const [tokenStatsBySessionId, setTokenStatsBySessionId] = useState<SessionTokenStatsMap>({});
  const [loadingTokenGroupCwds, setLoadingTokenGroupCwds] = useState<string[]>([]);
  const [loadedTokenGroupCwds, setLoadedTokenGroupCwds] = useState<string[]>([]);
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
  const hasInitializedExpandedGroupsRef = useRef(false);
  const loadSessionsPromiseRef = useRef<Promise<void> | null>(null);
  const copyResetTimerRef = useRef<number | null>(null);
  const tokenStatsVersionRef = useRef(0);
  const sourceRepairConfirmationResolverRef = useRef<((confirmed: boolean) => void) | null>(null);
  const isZh = i18n.resolvedLanguage?.toLowerCase().startsWith('zh') ?? true;

  const groupedSessions = useMemo(() => buildGroups(sessions), [sessions]);
  const selectedIdSet = useMemo(() => new Set(selectedIds), [selectedIds]);
  const selectedTrashIdSet = useMemo(() => new Set(selectedTrashIds), [selectedTrashIds]);
  const loadingTokenGroupSet = useMemo(() => new Set(loadingTokenGroupCwds), [loadingTokenGroupCwds]);
  const loadedTokenGroupSet = useMemo(() => new Set(loadedTokenGroupCwds), [loadedTokenGroupCwds]);
  const selectedSessions = useMemo(
    () => sessions.filter((session) => selectedIdSet.has(session.sessionId)),
    [selectedIdSet, sessions],
  );
  const syncTargetInstance = useMemo(
    () => instances.find((instance) => instance.id === syncTargetInstanceId) ?? null,
    [instances, syncTargetInstanceId],
  );
  const syncTargetExistingCount = useMemo(() => {
    if (!syncTargetInstance) return 0;
    return selectedSessions.filter((session) =>
      session.locations.some((location) => location.instanceId === syncTargetInstance.id),
    ).length;
  }, [selectedSessions, syncTargetInstance]);
  const sourceRepairPreview = useMemo(
    () => buildSourceRepairCandidatePreview(sourceRepairCandidates ?? []),
    [sourceRepairCandidates],
  );
  const instanceCount = instances.length;

  const loadSessions = useCallback(async () => {
    if (loadSessionsPromiseRef.current) {
      return await loadSessionsPromiseRef.current;
    }

    const task = (async () => {
      setLoading(true);
      try {
        const nextSessions = await listSessionsAcrossInstances();
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
  }, [listSessionsAcrossInstances]);

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

  const loadRolloutBackupBatches = useCallback(async () => {
    setLoadingRolloutBackups(true);
    try {
      const batches = await listSessionRestoreRolloutBackups();
      setRolloutBackupBatches(batches);
      return batches;
    } catch (error) {
      setRestoreModalError(String(error));
      return [];
    } finally {
      setLoadingRolloutBackups(false);
    }
  }, [listSessionRestoreRolloutBackups, setRestoreModalError]);

  const showRestoreRestartMessage = useCallback((baseMessage: string) => {
    setMessage({
      text: `${baseMessage}；${t(
        'codex.sessionManager.messages.restartAfterRestore',
        '稍等 {{seconds}} 秒钟后，你必须手动彻底退出Codex进程后再启动',
        { seconds: 10 },
      )}`,
    });
  }, [t]);

  useEffect(() => {
    void loadSessions();
  }, [loadSessions]);

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

  const toggleGroupExpanded = (cwd: string) => {
    setExpandedGroups((prev) => (prev.includes(cwd) ? prev.filter((item) => item !== cwd) : [...prev, cwd]));
  };

  const toggleTrashedSession = (sessionId: string) => {
    setSelectedTrashIds((prev) =>
      prev.includes(sessionId) ? prev.filter((id) => id !== sessionId) : [...prev, sessionId],
    );
  };

  const handleOpenRestoreModal = async () => {
    setShowRestoreModal(true);
    setSelectedTrashIds([]);
    setShowRolloutBackups(false);
    await Promise.all([loadTrashedSessions(), loadRolloutBackupBatches()]);
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
      const targetCandidates = latestInstances.length > 0 ? latestInstances : instances;
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

  useEscClose(showSyncTargetModal && sourceRepairCandidates === null, handleCloseSyncTargetModal);

  const handleCloseRestoreModal = () => {
    if (restoring) return;
    setShowRestoreModal(false);
    setSelectedTrashIds([]);
    setShowRolloutBackups(false);
    setRestoreModalError(null);
  };

  useEscClose(showRestoreModal && sourceRepairCandidates === null, handleCloseRestoreModal);

  const resolveSourceRepairConfirmation = useCallback((confirmed: boolean) => {
    const resolver = sourceRepairConfirmationResolverRef.current;
    sourceRepairConfirmationResolverRef.current = null;
    setSourceRepairCandidates(null);
    resolver?.(confirmed);
  }, []);

  const requestSourceRepairConfirmation = useCallback(
    async (candidates: CodexSessionSourceRepairCandidate[]) => {
      if (candidates.length === 0) {
        return true;
      }
      sourceRepairConfirmationResolverRef.current?.(false);
      setSourceRepairCandidates(candidates);
      return await new Promise<boolean>((resolve) => {
        sourceRepairConfirmationResolverRef.current = resolve;
      });
    },
    [],
  );

  const requestForceRestoreConfirmation = useCallback(
    async (preview: CodexSessionRestorePreviewSummary) => {
      if (preview.conflictRolloutFileCount <= 0) {
        return true;
      }

      return await nativeConfirmDialog(
        t('codex.sessionManager.confirm.forceRestoreMessage', {
          defaultValue:
            '发现 {{count}} 个同名不同内容 rollout 文件冲突。继续会先备份 Codex sessions 目录中这些冲突 rollout 文件和对应 session_index.jsonl，再用废纸篓中的文件覆盖并重建索引。备份列表按这 {{count}} 个冲突 rollout 文件统计；本次预检共可恢复 {{restorable}} 条会话 / {{rollouts}} 个 rollout 文件。确认继续？',
          count: preview.conflictRolloutFileCount,
          restorable: preview.restorableSessionCount,
          rollouts: preview.restorableRolloutFileCount,
        }),
        {
          title: t('codex.sessionManager.confirm.forceRestoreTitle', '确认强制覆盖恢复'),
          okLabel: t('common.confirm', '确认'),
          cancelLabel: t('common.cancel', '取消'),
        },
      );
    },
    [t],
  );

  useEscClose(sourceRepairCandidates !== null, () => resolveSourceRepairConfirmation(false));

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

      const confirmed = await nativeConfirmDialog(
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

  const handleRepairVisibility = async () => {
    setMessage(null);
    setRepairingVisibility(true);
    try {
      const sourcePreview = await previewSessionVisibilitySourceRepairs();
      let normalizeSources = false;
      if (sourcePreview.sourceRepairCount > 0) {
        const sourceConfirmed = await requestSourceRepairConfirmation(sourcePreview.candidates);
        if (!sourceConfirmed) return;
        normalizeSources = true;
      }

      const summary = await repairSessionVisibilityAcrossInstances(normalizeSources);
      setMessage({ text: formatCodexSessionVisibilityRepairMessage(summary, t) });
      await loadSessions();
    } catch (error) {
      setMessage({ text: String(error), tone: 'error' });
    } finally {
      setRepairingVisibility(false);
    }
  };

  const handleMoveToTrash = async () => {
    if (selectedIds.length === 0) {
      setMessage({ text: t('codex.sessionManager.messages.pickOne', '请至少选择一条会话'), tone: 'error' });
      return;
    }

    const confirmed = await nativeConfirmDialog(
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
      const preview = await previewRestoreSessionsFromTrashAcrossInstances(selectedTrashIds);
      let forceOverwrite = false;
      let normalizeSources = false;
      if (preview.sourceRepairCount > 0) {
        const sourceConfirmed = await requestSourceRepairConfirmation(preview.sourceRepairCandidates);
        if (!sourceConfirmed) return;
        normalizeSources = true;
      }
      if (preview.conflictRolloutFileCount > 0) {
        const confirmed = await requestForceRestoreConfirmation(preview);
        if (!confirmed) return;
        forceOverwrite = true;
      }

      const summary = await restoreSessionsFromTrashAcrossInstances(
        selectedTrashIds,
        forceOverwrite,
        normalizeSources,
      );
      showRestoreRestartMessage(summary.message);
      setSelectedTrashIds([]);
      const [nextTrashedSessions] = await Promise.all([
        loadTrashedSessions(),
        loadRolloutBackupBatches(),
        loadSessions(),
      ]);
      if (nextTrashedSessions.length === 0) {
        setShowRestoreModal(false);
      }
    } catch (error) {
      setRestoreModalError(String(error));
    } finally {
      setRestoring(false);
    }
  };

  const handleRestoreRolloutBackup = async (batchId: string) => {
    const confirmed = await nativeConfirmDialog(
      t(
        'codex.sessionManager.restoreModal.rolloutBackupConfirm',
        '这里是恢复会话时Codex sessions目录原本存在且与废纸篓内产生冲突的rollout备份，不是cockpit tools内废纸篓的备份，确定要恢复吗',
      ),
      {
        title: t('codex.sessionManager.restoreModal.restoreRolloutBackup', '恢复 rollout 备份'),
        okLabel: t('common.confirm', '确认'),
        cancelLabel: t('common.cancel', '取消'),
      },
    );
    if (!confirmed) return;

    setRestoring(true);
    setRestoreModalError(null);
    try {
      const summary = await restoreSessionRestoreRolloutBackup(batchId);
      showRestoreRestartMessage(summary.message);
      await Promise.all([loadRolloutBackupBatches(), loadTrashedSessions(), loadSessions()]);
    } catch (error) {
      setRestoreModalError(String(error));
    } finally {
      setRestoring(false);
    }
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
        <div className="codex-session-manager__actions">
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
            disabled={syncing || syncingToInstance || repairingVisibility || deleting || loading || selectedIds.length === 0}
          >
            <Copy size={14} className={syncingToInstance ? 'icon-spin' : undefined} />
            {t('codex.sessionManager.actions.copyToInstance', '复制到实例')} ({selectedIds.length})
          </button>
          <button
            className="btn btn-secondary codex-session-manager__action-button"
            type="button"
            onClick={() => void handleRepairVisibility()}
            disabled={repairingVisibility || loading || deleting || syncing || syncingToInstance}
          >
            <Eye size={14} />
            {t('codex.sessionManager.actions.repairVisibility', '修复可见性')}
          </button>
          <button
            className="btn btn-secondary codex-session-manager__action-button"
            type="button"
            onClick={() => void handleOpenRestoreModal()}
            disabled={loading || syncing || syncingToInstance || repairingVisibility || deleting || restoring}
          >
            <RotateCcw size={14} />
            {t('codex.sessionManager.actions.restoreSessions', '恢复会话')}
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

      {loading && sessions.length === 0 ? (
        <div className="empty-state">
          <h3>{t('common.loading', '加载中...')}</h3>
        </div>
      ) : null}

      {!loading && groupedSessions.length === 0 ? (
        <div className="empty-state codex-session-manager__empty">
          <Folder size={42} className="empty-icon" />
          <h3>{t('codex.sessionManager.empty.title', '还没有可管理的会话')}</h3>
          <p>{t('codex.sessionManager.empty.desc', '当前实例集合中还没有发现会话记录。')}</p>
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
                      title={group.projectless ? t('codex.sessionManager.labels.projectlessHint', '无目录对话') : group.cwd}
                    >
                      {group.archived ? (
                        <span className="codex-session-archive-badge">
                          {t('codex.sessionManager.labels.archived', '「归档」')}
                        </span>
                      ) : null}
                      {!group.archived && group.removed ? (
                        <span className="codex-session-archive-badge">
                          {t('codex.sessionManager.labels.removed', '「移除」')}
                        </span>
                      ) : null}
                      {group.projectless ? (
                        <span className="codex-session-projectless-badge">
                          {t('codex.sessionManager.labels.projectless', '「对话」')}
                        </span>
                      ) : null}
                      {group.projectless
                        ? t('codex.sessionManager.labels.projectlessGroup', '对话')
                        : resolveGroupLabel(group.cwd)}
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
                                {session.archived ? (
                                  <span className="codex-session-archive-badge">
                                    {t('codex.sessionManager.labels.archived', '「归档」')}
                                  </span>
                                ) : null}
                                {!session.archived && session.removed ? (
                                  <span className="codex-session-archive-badge">
                                    {t('codex.sessionManager.labels.removed', '「移除」')}
                                  </span>
                                ) : null}
                                {session.projectless ? (
                                  <span className="codex-session-projectless-badge">
                                    {t('codex.sessionManager.labels.projectless', '「对话」')}
                                  </span>
                                ) : null}
                                {session.title || t('codex.sessionManager.untitled', '未命名会话')}
                              </span>
                              <span className="codex-session-row__meta">
                                {session.locations.map((location) => location.instanceName).join(' / ')}
                                {hasRunningLocation
                                  ? t('codex.sessionManager.locationRunning', '（运行中）')
                                  : ''}
                              </span>
                              <span className="codex-session-row__meta codex-session-row__session-id">
                                {t('codex.sessionManager.labels.sessionId', '会话 ID')}: {session.sessionId}
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
        <div className="modal-overlay" onClick={handleCloseSyncTargetModal}>
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
                <select
                  value={syncTargetInstanceId}
                  onChange={(event) => {
                    setSyncTargetInstanceId(event.target.value);
                    setSyncTargetModalError(null);
                  }}
                  disabled={syncingToInstance}
                >
                  <option value="">{t('codex.sessionManager.targetModal.pickTarget', '请选择目标实例')}</option>
                  {instances.map((instance) => (
                    <option key={instance.id} value={instance.id}>
                      {instance.isDefault
                        ? t('instances.defaultName', '默认实例')
                        : instance.name || t('instances.defaultName', '默认实例')}
                    </option>
                  ))}
                </select>
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

      {sourceRepairCandidates ? (
        <div className="modal-overlay" onClick={() => resolveSourceRepairConfirmation(false)}>
          <div
            className="modal codex-source-repair-modal"
            onClick={(event) => event.stopPropagation()}
            onMouseDown={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <h2>{t('codex.sessionManager.confirm.sourceRepairTitle', '恢复 Codex 左侧可见性')}</h2>
              <button
                className="modal-close"
                type="button"
                onClick={() => resolveSourceRepairConfirmation(false)}
                aria-label={t('common.close', '关闭')}
              >
                <X size={18} />
              </button>
            </div>
            <div className="modal-body">
              <p className="codex-source-repair-modal__summary">
                {t('codex.sessionManager.groupCount', '{{count}} 条会话', {
                  count: sourceRepairCandidates.length,
                })}
              </p>
              <div className="codex-source-repair-modal__list">
                {sourceRepairPreview.visible.map((candidate) => (
                  <div className="codex-session-row codex-source-repair-row" key={candidate.key}>
                    <div className="codex-session-row__content">
                      <span className="codex-session-row__title">{candidate.title}</span>
                    </div>
                  </div>
                ))}
                {sourceRepairPreview.hiddenCount > 0 ? (
                  <div className="codex-session-row codex-source-repair-row codex-source-repair-row--more">
                    <div className="codex-session-row__content">
                      <span className="codex-session-row__title">
                        {t('codex.sessionManager.labels.moreHidden', '另有 {{count}} 条未显示', {
                          count: sourceRepairPreview.hiddenCount,
                        })}
                      </span>
                    </div>
                  </div>
                ) : null}
              </div>
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                type="button"
                onClick={() => resolveSourceRepairConfirmation(false)}
              >
                {t('common.cancel', '取消')}
              </button>
              <button
                className="btn btn-primary"
                type="button"
                onClick={() => resolveSourceRepairConfirmation(true)}
              >
                {t('common.confirm', '确认')}
              </button>
            </div>
          </div>
        </div>
      ) : null}

      {showRestoreModal ? (
        <div className="modal-overlay" onClick={handleCloseRestoreModal}>
          <div className="modal codex-session-restore-modal" onClick={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('codex.sessionManager.restoreModal.title', '恢复会话')}</h2>
              <button
                className="modal-close"
                type="button"
                onClick={handleCloseRestoreModal}
                disabled={restoring}
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
                  <p className="codex-session-restore-modal__hint">
                    {t(
                      'codex.sessionManager.restoreModal.hint',
                      '恢复会把 rollout 文件、session_index 条目和会话文件时间放回原实例；如检测到来源会导致 Codex 左侧项目不可见，会先弹出会话列表要求确认，再触发官方 Codex 重建会话索引。',
                    )}
                  </p>
                  <div className="codex-session-restore-list">
                    {trashedSessions.map((session) => (
                      <label className="codex-session-restore-row" key={session.sessionId}>
                        <div className="codex-session-restore-row__left">
                          <input
                            className="codex-session-row__checkbox"
                            type="checkbox"
                            checked={selectedTrashIdSet.has(session.sessionId)}
                            onChange={() => toggleTrashedSession(session.sessionId)}
                          />
                          <div className="codex-session-restore-row__content">
                            <span className="codex-session-restore-row__title" title={session.title}>
                              {session.title || t('codex.sessionManager.untitled', '未命名会话')}
                            </span>
                            <span className="codex-session-restore-row__meta">
                              {session.locations.map((location) => location.instanceName).join(' / ')}
                            </span>
                            <span className="codex-session-restore-row__meta codex-session-restore-row__cwd">
                              {session.cwd}
                            </span>
                          </div>
                        </div>
                        <span className="codex-session-row__time">
                          {formatRelativeTime(session.deletedAt, isZh)}
                        </span>
                      </label>
                    ))}
                  </div>
                </>
              ) : null}
              {showRolloutBackups ? (
                <div className="codex-session-rollout-backups">
                  <div className="codex-session-rollout-backups__header">
                    <strong>{t('codex.sessionManager.restoreModal.rolloutBackupsTitle', 'rollout 恢复备份')}</strong>
                    <span>
                      {loadingRolloutBackups
                        ? t('common.loading', '加载中...')
                        : t('codex.sessionManager.restoreModal.rolloutBackupsCount', {
                            defaultValue: '{{count}} 个批次',
                            count: rolloutBackupBatches.length,
                          })}
                    </span>
                  </div>
                  {!loadingRolloutBackups && rolloutBackupBatches.length === 0 ? (
                    <p className="codex-session-rollout-backups__empty">
                      {t('codex.sessionManager.restoreModal.noRolloutBackups', '暂无可恢复的 rollout 备份。')}
                    </p>
                  ) : null}
                  {!loadingRolloutBackups && rolloutBackupBatches.length > 0 ? (
                    <div className="codex-session-rollout-backups__list">
                      {rolloutBackupBatches.map((batch) => (
                        <div className="codex-session-rollout-backup-row" key={batch.batchId}>
                          <div className="codex-session-rollout-backup-row__content">
                            <span className="codex-session-rollout-backup-row__title">
                              {batch.batchId}
                            </span>
                            <span className="codex-session-rollout-backup-row__meta">
                              {t('codex.sessionManager.restoreModal.rolloutBackupSummary', {
                                defaultValue: '{{sessions}} 条会话 / {{rollouts}} 个冲突 rollout 备份 / {{instances}} 个实例',
                                sessions: batch.sessionCount,
                                rollouts: batch.rolloutFileCount,
                                instances: batch.instanceCount,
                              })}
                            </span>
                            <span className="codex-session-rollout-backup-row__meta">
                              {batch.items.map((item) => item.instanceName).join(' / ')}
                            </span>
                          </div>
                          <button
                            className="btn btn-secondary"
                            type="button"
                            onClick={() => void handleRestoreRolloutBackup(batch.batchId)}
                            disabled={restoring}
                          >
                            <RotateCcw size={14} className={restoring ? 'icon-spin' : undefined} />
                            {t('codex.sessionManager.restoreModal.restoreRolloutBackup', '恢复 rollout 备份')}
                          </button>
                        </div>
                      ))}
                    </div>
                  ) : null}
                </div>
              ) : null}
            </div>
            <div className="modal-footer">
              <div className="codex-session-restore-modal__footer-left">
                <button
                  className="btn btn-secondary"
                  type="button"
                  onClick={() => {
                    setShowRolloutBackups((prev) => !prev);
                    if (!showRolloutBackups) {
                      void loadRolloutBackupBatches();
                    }
                  }}
                  disabled={restoring || loadingRolloutBackups}
                >
                  <RotateCcw size={14} className={loadingRolloutBackups ? 'icon-spin' : undefined} />
                  {t('codex.sessionManager.restoreModal.restoreRolloutBackup', '恢复 rollout 备份')}
                </button>
              </div>
              <div className="codex-session-restore-modal__footer-right">
                <button
                  className="btn btn-secondary"
                  type="button"
                  onClick={handleCloseRestoreModal}
                  disabled={restoring}
                >
                  {t('common.cancel', '取消')}
                </button>
                <button
                  className="btn btn-primary"
                  type="button"
                  onClick={() => void handleRestoreFromTrash()}
                  disabled={restoring || loadingTrash || selectedTrashIds.length === 0}
                >
                  <RotateCcw size={14} className={restoring ? 'icon-spin' : undefined} />
                  {t('codex.sessionManager.restoreModal.restoreAction', '恢复选中会话')} ({selectedTrashIds.length})
                </button>
              </div>
            </div>
          </div>
        </div>
      ) : null}
    </section>
  );
}
