import type { TFunction } from 'i18next';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface SessionLike {
  cwd: string;
  updatedAt?: number | null;
  title: string;
}

export interface SessionGroup<T extends SessionLike> {
  cwd: string;
  sessions: T[];
  latestUpdatedAt: number;
}

// ---------------------------------------------------------------------------
// Shared helpers for session manager components
// ---------------------------------------------------------------------------

export function buildSessionGroups<T extends SessionLike>(sessions: T[]): SessionGroup<T>[] {
  const groups = new Map<string, T[]>();
  sessions.forEach((s) => {
    const bucket = groups.get(s.cwd) ?? [];
    bucket.push(s);
    groups.set(s.cwd, bucket);
  });
  return Array.from(groups.entries())
    .map(([cwd, groupSessions]) => ({
      cwd,
      sessions: [...groupSessions].sort(
        (a, b) => (b.updatedAt ?? 0) - (a.updatedAt ?? 0) || a.title.localeCompare(b.title),
      ),
      latestUpdatedAt: Math.max(...groupSessions.map((s) => s.updatedAt ?? 0), 0),
    }))
    .sort(
      (a, b) => b.latestUpdatedAt - a.latestUpdatedAt || a.cwd.localeCompare(b.cwd, 'zh-CN'),
    );
}

export function formatRelativeTime(value: number | null | undefined, t: TFunction): string {
  if (!value) return t('sessionManager.time.unknown', '时间未知');
  const diffMs = Date.now() - value;
  const diffSeconds = Math.max(0, Math.floor(diffMs / 1000));
  const minute = 60;
  const hour = 60 * minute;
  const day = 24 * hour;
  const week = 7 * day;
  if (diffSeconds < hour) {
    const m = Math.max(1, Math.floor(diffSeconds / minute));
    return t('sessionManager.time.minutesAgo', '{{count}} 分钟前', { count: m });
  }
  if (diffSeconds < day) {
    const h = Math.floor(diffSeconds / hour);
    return t('sessionManager.time.hoursAgo', '{{count}} 小时前', { count: h });
  }
  if (diffSeconds < week) {
    const d = Math.floor(diffSeconds / day);
    return t('sessionManager.time.daysAgo', '{{count}} 天前', { count: d });
  }
  const w = Math.floor(diffSeconds / week);
  return t('sessionManager.time.weeksAgo', '{{count}} 周前', { count: w });
}

export function resolveGroupLabel(cwd: string): string {
  const normalized = cwd.replace(/\\/g, '/').replace(/\/$/, '');
  const parts = normalized.split('/').filter(Boolean);
  return parts[parts.length - 1] || cwd;
}

export function formatConversationId(id: string): string {
  if (id.length <= 18) return id;
  return `${id.slice(0, 8)}...${id.slice(-6)}`;
}
