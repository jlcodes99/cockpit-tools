import { type MouseEvent, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { confirm as confirmDialog } from '@tauri-apps/plugin-dialog';
import { Check, ChevronDown, ChevronRight, Copy, Eye, Folder, RefreshCw, RotateCcw, Trash2, X } from 'lucide-react';
import { ModalErrorMessage, useModalErrorState } from '../ModalErrorMessage';
import type { CodexSessionRecord, CodexSessionTokenStats, CodexTrashedSessionRecord } from '../../types/codex';
import { useCodexInstanceStore } from '../../stores/useCodexInstanceStore';
import { formatCodexSessionVisibilityRepairMessage } from '../../utils/codexSessionVisibility';

type MessageState = { text: string; tone?: 'error' };
type SessionTokenStatsMap = Record<string, CodexSessionTokenStats>;

type SessionGroup = {
  cwd: string;
  sessions: CodexSessionRecord[];
  latestUpdatedAt: number;
};

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

export function CodexSessionManager() {
  const { t, i18n } = useTranslation();
  const instances = useCodexInstanceStore((state) => state.instances);
  const refreshInstances = useCodexInstanceStore((state) => state.refreshInstances);
  const syncThreadsAcrossInstances = useCodexInstanceStore((state) => state.syncThreadsAcrossInstances);
  const repairSessionVisibilityAcrossInstances = useCodexInstanceStore(
    (state) => state.repairSessionVisibilityAcrossInstances,
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
  const restoreSessionsFromTrashAcrossInstances = useCodexInstanceStore(
    (state) => state.restoreSessionsFromTrashAcrossInstances,
  );
  const [sessions, setSessions] = useState<CodexSessionRecord[]>([]);
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [expandedGroups, setExpandedGroups] = useState<string[]>([]);
  const [showRestoreModal, setShowRestoreModal] = useState(false);
  const [trashedSessions, setTrashedSessions] = useState<CodexTrashedSessionRecord[]>([]);
  const [selectedTrashIds, setSelectedTrashIds] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [repairingVisibility, setRepairingVisibility] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [loadingTrash, setLoadingTrash] = useState(false);
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
  const hasInitializedExpandedGroupsRef = useRef(false);
  const loadSessionsPromiseRef = useRef<Promise<void> | null>(null);
  const copyResetTimerRef = useRef<number | null>(null);
  const tokenStatsVersionRef = useRef(0);
  const isZh = i18n.resolvedLanguage?.toLowerCase().startsWith('zh') ?? true;

  const groupedSessions = useMemo(() => buildGroups(sessions), [sessions]);
  const selectedIdSet = useMemo(() => new Set(selectedIds), [selectedIds]);
  const selectedTrashIdSet = useMemo(() => new Set(selectedTrashIds), [selectedTrashIds]);
  const loadingTokenGroupSet = useMemo(() => new Set(loadingTokenGroupCwds), [loadingTokenGroupCwds]);
  const loadedTokenGroupSet = useMemo(() => new Set(loadedTokenGroupCwds), [loadedTokenGroupCwds]);
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
    await loadTrashedSessions();
  };

  const handleCloseRestoreModal = () => {
    if (restoring) return;
    setShowRestoreModal(false);
    setSelectedTrashIds([]);
    setRestoreModalError(null);
  };

  const handleSyncSessions = async () => {
    setMessage(null);
    try {
      const latestInstances = await refreshInstances();
      if (latestInstances.length < 2) {
        setMessage({
          text: t('codex.sessionManager.messages.syncNeedTwo', "At least two instances are required to sync sessions"),
          tone: 'error',
        });
        return;
      }

      const confirmed = await confirmDialog(
        t(
          'codex.sessionManager.confirm.syncMessage',
          "Missing threads and their session data will be synced to all instances. Existing content will not be written again, and target instance files will be backed up before any write occurs. Continue?",
        ),
        {
          title: t('codex.sessionManager.actions.syncSessions', "Sync Sessions"),
          okLabel: t('common.confirm', "Confirm"),
          cancelLabel: t('common.cancel', "Cancel"),
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
    const confirmed = await confirmDialog(
      t(
        'codex.sessionManager.confirm.repairVisibilityMessage',
        "This will repair provider metadata in rollout files and state_5.sqlite using each instance's root config.toml model_provider (defaulting to openai when missing). Files that will be changed are backed up before writing. Running instances may need a restart before the change appears. Continue?",
      ),
      {
        title: t('codex.sessionManager.actions.repairVisibility', "Repair Visibility"),
        okLabel: t('common.confirm', "Confirm"),
        cancelLabel: t('common.cancel', "Cancel"),
      },
    );
    if (!confirmed) return;

    setRepairingVisibility(true);
    try {
      const summary = await repairSessionVisibilityAcrossInstances();
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
      setMessage({ text: t('codex.sessionManager.messages.pickOne', "Please select at least one session"), tone: 'error' });
      return;
    }

    const confirmed = await confirmDialog(
      t(
        'codex.sessionManager.confirm.message',
        "Selected sessions will be moved to Trash from their instances so they can be restored later. Running instances may need a restart before the change appears. Continue?",
      ),
      {
        title: t('codex.sessionManager.confirm.title', "Move to Trash"),
        okLabel: t('common.confirm', "Confirm"),
        cancelLabel: t('common.cancel', "Cancel"),
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
      setRestoreModalError(t('codex.sessionManager.messages.pickRestoreOne', "Please select at least one session to restore"));
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
        text: t('common.shared.export.copyFailed', "Copy failed, please copy manually"),
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
            disabled={syncing || repairingVisibility || deleting || loading || instanceCount < 2}
            title={
              instanceCount < 2
                ? t('codex.sessionManager.messages.syncNeedTwo', "At least two instances are required to sync sessions")
                : t('codex.sessionManager.actions.syncSessions', "Sync Sessions")
            }
          >
            <RefreshCw size={14} className={syncing ? 'icon-spin' : undefined} />
            {t('codex.sessionManager.actions.syncSessions', "Sync Sessions")}
          </button>
          <button
            className="btn btn-secondary codex-session-manager__action-button"
            type="button"
            onClick={() => void handleRepairVisibility()}
            disabled={repairingVisibility || loading || deleting || syncing}
          >
            <Eye size={14} />
            {t('codex.sessionManager.actions.repairVisibility', "Repair Visibility")}
          </button>
          <button
            className="btn btn-secondary codex-session-manager__action-button"
            type="button"
            onClick={() => void handleOpenRestoreModal()}
            disabled={loading || syncing || repairingVisibility || deleting || restoring}
          >
            <RotateCcw size={14} />
            {t('codex.sessionManager.actions.restoreSessions', "Restore Sessions")}
          </button>
          <button
            className="btn btn-secondary codex-session-manager__action-button"
            type="button"
            onClick={() => void handleRefresh()}
            disabled={loading || deleting || syncing || repairingVisibility}
          >
            <RefreshCw size={14} className={loading ? 'icon-spin' : undefined} />
            {t('common.refresh', "Refresh")}
          </button>
          <button
            className="btn btn-danger codex-session-manager__action-button"
            type="button"
            onClick={() => void handleMoveToTrash()}
            disabled={deleting || loading || syncing || repairingVisibility || selectedIds.length === 0}
          >
            <Trash2 size={14} />
            {t('codex.sessionManager.actions.moveToTrash', "Move to Trash")} ({selectedIds.length})
          </button>
        </div>
      </div>

      {message ? (
        <div className={`message-bar ${message.tone === 'error' ? 'error' : 'success'}`}>{message.text}</div>
      ) : null}

      {loading && sessions.length === 0 ? (
        <div className="empty-state">
          <h3>{t('common.loading', "Loading...")}</h3>
        </div>
      ) : null}

      {!loading && groupedSessions.length === 0 ? (
        <div className="empty-state codex-session-manager__empty">
          <Folder size={42} className="empty-icon" />
          <h3>{t('codex.sessionManager.empty.title', "No sessions to manage")}</h3>
          <p>{t('codex.sessionManager.empty.desc', "No session records were found in the current instance set.")}</p>
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
                          ? t('codex.sessionManager.actions.collapse', "Collapse")
                          : t('codex.sessionManager.actions.expand', "Expand")
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
                                {session.title || t('codex.sessionManager.untitled', "Untitled session")}
                              </span>
                              <span className="codex-session-row__meta">
                                {session.locations.map((location) => location.instanceName).join(' / ')}
                                {hasRunningLocation
                                  ? t('codex.sessionManager.locationRunning', " (running)")
                                  : ''}
                              </span>
                              <span className="codex-session-row__meta codex-session-row__session-id" title={session.sessionId}>
                                {t('codex.sessionManager.labels.sessionId', "Session ID")}: {formatSessionId(session.sessionId)}
                              </span>
                            </div>
                          </label>
                          <div className="codex-session-row__right">
                            <button
                              className={`codex-session-row__copy-button${copiedSessionId === session.sessionId ? ' is-copied' : ''}`}
                              type="button"
                              onClick={(event) => void handleCopySessionId(event, session.sessionId)}
                              title={t('codex.sessionManager.actions.copySessionId', "Copy Session ID")}
                              aria-label={t('codex.sessionManager.actions.copySessionId', "Copy Session ID")}
                            >
                              {copiedSessionId === session.sessionId ? <Check size={14} /> : <Copy size={14} />}
                            </button>
                            {tokenText ? (
                              <span className="codex-session-row__tokens" title={t('codex.sessionManager.labels.tokenUsage', "Token Usage")}>
                                {tokenText}
                              </span>
                            ) : null}
                            {!tokenText && isTokenStatsLoading ? (
                              <span className="codex-session-row__tokens" title={t('common.loading', "Loading...")}>
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

      {showRestoreModal ? (
        <div className="modal-overlay" onClick={handleCloseRestoreModal}>
          <div className="modal codex-session-restore-modal" onClick={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <h2>{t('codex.sessionManager.restoreModal.title', "Restore Sessions")}</h2>
              <button
                className="modal-close"
                type="button"
                onClick={handleCloseRestoreModal}
                disabled={restoring}
                aria-label={t('common.close', "Close")}
              >
                <X size={18} />
              </button>
            </div>
            <div className="modal-body">
              <ModalErrorMessage message={restoreModalError} scrollKey={restoreModalErrorScrollKey} />
              {loadingTrash ? (
                <div className="codex-session-restore-modal__empty">
                  <h3>{t('common.loading', "Loading...")}</h3>
                </div>
              ) : null}
              {!loadingTrash && trashedSessions.length === 0 ? (
                <div className="codex-session-restore-modal__empty">
                  <Folder size={36} className="empty-icon" />
                  <h3>{t('codex.sessionManager.restoreModal.emptyTitle', "Trash is empty")}</h3>
                  <p>{t('codex.sessionManager.restoreModal.emptyDesc', "Sessions moved to Trash will appear here.")}</p>
                </div>
              ) : null}
              {!loadingTrash && trashedSessions.length > 0 ? (
                <>
                  <p className="codex-session-restore-modal__hint">
                    {t(
                      'codex.sessionManager.restoreModal.hint',
                      "Restoring puts the rollout file, SQLite thread row, and session_index entry back into the original instance together.",
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
                              {session.title || t('codex.sessionManager.untitled', "Untitled session")}
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
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                type="button"
                onClick={handleCloseRestoreModal}
                disabled={restoring}
              >
                {t('common.cancel', "Cancel")}
              </button>
              <button
                className="btn btn-primary"
                type="button"
                onClick={() => void handleRestoreFromTrash()}
                disabled={restoring || loadingTrash || selectedTrashIds.length === 0}
              >
                <RotateCcw size={14} className={restoring ? 'icon-spin' : undefined} />
                {t('codex.sessionManager.restoreModal.restoreAction', "Restore Selected Sessions")} ({selectedTrashIds.length})
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </section>
  );
}
