import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  ChevronDown,
  ChevronRight,
  Copy,
  Check,
  Folder,
  RefreshCw,
  Search,
  X,
} from 'lucide-react';
import type {
  CodebuddySessionRecord,
  CodebuddySessionPlatform,
} from '../../services/codebuddySessionService';
import * as sessionService from '../../services/codebuddySessionService';
import type { CodebuddySuiteAccountBase } from '../../types/codebuddy-suite';
import { getAccountDisplayName } from '../../utils/codebuddy-suite/quota-model';
import {
  buildSessionGroups,
  formatRelativeTime,
  resolveGroupLabel,
  formatConversationId,
} from '../../utils/sessionHelpers';

// ---------------------------------------------------------------------------
// Local helpers
// ---------------------------------------------------------------------------

function formatUserId(id: string): string {
  if (id.length <= 12) return id;
  return `${id.slice(0, 8)}...${id.slice(-4)}`;
}

function formatCwdDisplay(cwd: string): string {
  const normalized = cwd.replace(/\\/g, '/').replace(/\/$/, '');
  if (normalized.length <= 45) return normalized;
  const parts = normalized.split('/').filter(Boolean);
  if (parts.length <= 2) return normalized;
  return `/${parts[0]}/.../${parts.slice(-2).join('/')}`;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

interface Props {
  platform: CodebuddySessionPlatform;
  accounts?: CodebuddySuiteAccountBase[];
}

export function CodebuddySessionManager({ platform, accounts = [] }: Props) {
  const { t } = useTranslation();

  // userId → displayName map
  const userIdMap = useMemo(() => {
    const map = new Map<string, string>();
    for (const acc of accounts) {
      if (acc.uid) map.set(acc.uid, getAccountDisplayName(acc));
    }
    return map;
  }, [accounts]);

  const resolveUserName = useCallback(
    (userId: string): string => userIdMap.get(userId) || formatUserId(userId),
    [userIdMap],
  );

  // Data
  const [sessions, setSessions] = useState<CodebuddySessionRecord[]>([]);
  const [expandedGroups, setExpandedGroups] = useState<string[]>([]);

  // Filter — keywordInput is the immediate input value; keyword is debounced
  const [keywordInput, setKeywordInput] = useState('');
  const [keyword, setKeyword] = useState('');
  const [statusFilter, setStatusFilter] = useState('');

  // UI state
  const [loading, setLoading] = useState(false);
  const [copiedId, setCopiedId] = useState<string | null>(null);

  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const copyTimerRef = useRef<number | null>(null);

  // Cleanup timers on unmount
  useEffect(() => {
    return () => {
      if (debounceRef.current) clearTimeout(debounceRef.current);
      if (copyTimerRef.current) clearTimeout(copyTimerRef.current);
    };
  }, []);

  // Derived
  const groupedSessions = useMemo(() => buildSessionGroups(sessions), [sessions]);

  // ---------------------------------------------------------------------------
  // Debounced keyword
  // ---------------------------------------------------------------------------

  const handleKeywordChange = useCallback((value: string) => {
    setKeywordInput(value);
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => setKeyword(value), 300);
  }, []);

  const handleClearKeyword = useCallback(() => {
    if (debounceRef.current) clearTimeout(debounceRef.current);
    setKeywordInput('');
    setKeyword('');
  }, []);

  // ---------------------------------------------------------------------------
  // Data loading
  // ---------------------------------------------------------------------------

  const loadSessions = useCallback(async () => {
    setLoading(true);
    try {
      const data = await sessionService.codebuddyListSessions(platform, {
        keyword: keyword || undefined,
        status: statusFilter || undefined,
      });
      setSessions(data);
    } catch (_e) {
      // silently ignore
    } finally {
      setLoading(false);
    }
  }, [platform, keyword, statusFilter]);

  useEffect(() => {
    loadSessions();
  }, [loadSessions]);

  // ---------------------------------------------------------------------------
  // Helpers
  // ---------------------------------------------------------------------------

  const toggleGroupExpanded = useCallback((cwd: string) => {
    setExpandedGroups((prev) =>
      prev.includes(cwd) ? prev.filter((x) => x !== cwd) : [...prev, cwd],
    );
  }, []);

  const handleCopyId = useCallback((id: string) => {
    navigator.clipboard.writeText(id);
    setCopiedId(id);
    if (copyTimerRef.current) clearTimeout(copyTimerRef.current);
    copyTimerRef.current = window.setTimeout(() => setCopiedId(null), 1200);
  }, []);

  return (
    <div className="codex-session-manager">
      {/* Search & Filter row */}
      <div className="codex-session-manager__header">
        <div className="codex-session-manager__filters">
          <div className="codex-session-manager__search-wrap">
            <Search size={13} className="codex-session-manager__search-icon" />
            <input
              type="text"
              className="codex-session-manager__search-input"
              placeholder={t('codebuddy.sessionManager.searchPlaceholder', '搜索标题或工作目录...')}
              value={keywordInput}
              onChange={(e) => handleKeywordChange(e.target.value)}
              disabled={loading}
            />
            {keywordInput && (
              <button
                className="codex-session-manager__search-clear"
                onClick={handleClearKeyword}
              >
                <X size={12} />
              </button>
            )}
          </div>

          <select
            className="codex-session-manager__status-select"
            value={statusFilter}
            onChange={(e) => setStatusFilter(e.target.value)}
            disabled={loading}
          >
            <option value="">{t('codebuddy.sessionManager.statusAll', '全部状态')}</option>
            <option value="Completed">{t('codebuddy.sessionManager.statusCompleted', '已完成')}</option>
            <option value="InProgress">{t('codebuddy.sessionManager.statusInProgress', '进行中')}</option>
          </select>
        </div>

        <div className="codex-session-manager__actions">
          <button
            className="codex-session-manager__action-button btn btn-ghost"
            onClick={loadSessions}
            disabled={loading}
            title={t('common.refresh', '刷新')}
          >
            <RefreshCw size={14} />
          </button>
        </div>
      </div>

      {/* Session list */}
      <div className="codex-session-manager__list">
        {loading && sessions.length === 0 && (
          <div className="codex-session-manager__empty">
            {t('common.loading', '加载中...')}
          </div>
        )}
        {!loading && sessions.length === 0 && (
          <div className="codex-session-manager__empty">
            {keyword || statusFilter
              ? t('codebuddy.sessionManager.noMatching', '没有匹配的会话')
              : t('codebuddy.sessionManager.noSessions', '暂无会话记录')}
          </div>
        )}
        {groupedSessions.map((group) => {
          const isExpanded = expandedGroups.includes(group.cwd);

          return (
            <div key={group.cwd} className="codex-session-folder">
              {/* Group header */}
              <div className="codex-session-folder__row">
                <div className="codex-session-folder__left">
                  <button className="codex-session-folder__expand" onClick={() => toggleGroupExpanded(group.cwd)}>
                    {isExpanded ? <ChevronDown size={16} /> : <ChevronRight size={16} />}
                  </button>
                  <Folder size={14} className="codex-session-folder__icon" />
                  <button className="codex-session-folder__label" onClick={() => toggleGroupExpanded(group.cwd)}>
                    {resolveGroupLabel(group.cwd)}
                  </button>
                  <span className="codex-session-group__count">{group.sessions.length}</span>
                </div>
                <span className="codex-session-folder__time">
                  {formatRelativeTime(group.latestUpdatedAt, t)}
                </span>
              </div>

              {/* Expanded sessions */}
              {isExpanded && (
                <div className="codex-session-folder__children">
                  {group.sessions.map((session) => (
                    <div key={session.conversationId} className="codex-session-row">
                      <div className="codex-session-row__left">
                        <div className="codex-session-row__content">
                          <span className="codex-session-row__title">
                            {session.title || t('codebuddy.sessionManager.untitled', '(untitled)')}
                          </span>
                          {session.locations.length > 0 && (
                            <span className="codex-session-row__meta">
                              {session.locations.map((loc, idx) => (
                                <span key={idx} className="codex-session-location">
                                  {loc.instanceName}
                                </span>
                              ))}
                            </span>
                          )}
                          <span className="codex-session-row__meta codex-session-row__session-id" title={session.conversationId}>
                            {t('codebuddy.sessionManager.labels.session', '会话')}: {formatConversationId(session.conversationId)}
                          </span>
                          {session.userId && (
                            <span className="codex-session-row__meta codex-session-row__session-id codex-session-row__user" title={session.userId}>
                              {t('codebuddy.sessionManager.labels.user', '用户')}: {resolveUserName(session.userId)}
                            </span>
                          )}
                          <span className="codex-session-row__meta codex-session-row__cwd" title={session.cwd}>
                            {t('codebuddy.sessionManager.labels.dir', '目录')}: {formatCwdDisplay(session.cwd)}
                          </span>
                        </div>
                      </div>
                      <div className="codex-session-row__right">
                        <span className="codex-session-row__status-badge">
                          {session.status}
                        </span>
                        <button
                          className={`codex-session-row__copy-button${copiedId === session.conversationId ? ' is-copied' : ''}`}
                          onClick={() => handleCopyId(session.conversationId)}
                          title={session.conversationId}
                        >
                          {copiedId === session.conversationId ? <Check size={12} /> : <Copy size={12} />}
                        </button>
                        <span className="codex-session-row__time">
                          {formatRelativeTime(session.updatedAt, t)}
                        </span>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
