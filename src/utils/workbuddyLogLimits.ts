import {
  scanWorkbuddyLogLimits,
  type WorkbuddyLogLimitRecord,
} from '../services/workbuddyService';
import {
  dismissWorkbuddyProbeRecord,
  getWorkbuddyProbeRecords,
  probeSharedIdentityKey,
  type WorkbuddyProbeStatusKind,
} from './workbuddyProbeStatus';

export type WorkbuddyNoticeSource = 'probe' | 'client_log' | 'both';

export interface WorkbuddyMergedNotice {
  key: string;
  accountId: string;
  model: string;
  status: WorkbuddyProbeStatusKind;
  source: WorkbuddyNoticeSource;
  resetAt: number | null;
  recoveredAt: number | null;
  timestamp: number;
  hitCount: number;
  message: string;
  hasProbeRecord: boolean;
  hasLogRecord: boolean;
  logResetAt: number | null;
}

const CHANGE_EVENT = 'agtools:workbuddy-log-limits';
const DISMISSED_STORAGE_KEY = 'agtools.workbuddy.log_limit_dismissed.v1';
const CLIENT_THROTTLE_MS = 30_000;

let logLimitsByIdentity: Record<string, WorkbuddyLogLimitRecord[]> = {};
let lastFetchedAt = 0;
let inFlightRefresh: Promise<void> | null = null;

function identityKeyFor(accountId: string): string {
  return probeSharedIdentityKey(accountId) ?? accountId;
}

function dismissEntryKey(identity: string, model: string, resetAt: number): string {
  return `${identity}\u0000${model}\u0000${resetAt}`;
}

function readDismissedMap(): Record<string, number> {
  try {
    const raw = localStorage.getItem(DISMISSED_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return {};
    const now = Date.now();
    const cleaned: Record<string, number> = {};
    for (const [key, resetAt] of Object.entries(parsed)) {
      if (typeof resetAt === 'number' && Number.isFinite(resetAt) && resetAt > now) {
        cleaned[key] = resetAt;
      }
    }
    return cleaned;
  } catch {
    return {};
  }
}

function writeDismissedMap(map: Record<string, number>) {
  try {
    localStorage.setItem(DISMISSED_STORAGE_KEY, JSON.stringify(map));
  } catch {
    /* localStorage 不可用时忽略 */
  }
  emitLogLimitsChanged();
}

function emitLogLimitsChanged() {
  try {
    window.dispatchEvent(new CustomEvent(CHANGE_EVENT));
  } catch {
    /* SSR/测试环境忽略 */
  }
}

/**
 * 触发一次被动日志限流扫描（复用后端 5 分钟缓存；force=true 时绕过缓存）。
 */
export async function refreshWorkbuddyLogLimits(force = false): Promise<void> {
  if (!force && inFlightRefresh) {
    return inFlightRefresh;
  }
  const now = Date.now();
  if (!force && lastFetchedAt > 0 && now - lastFetchedAt < CLIENT_THROTTLE_MS) {
    return;
  }

  let task: Promise<void> | null = null;
  task = (async () => {
    try {
      const items = await scanWorkbuddyLogLimits(force);
      const grouped: Record<string, WorkbuddyLogLimitRecord[]> = {};
      for (const item of items) {
        if (!item || typeof item.accountId !== 'string') continue;
        const key = identityKeyFor(item.accountId);
        if (!grouped[key]) grouped[key] = [];
        grouped[key].push(item);
      }
      logLimitsByIdentity = grouped;
      lastFetchedAt = Date.now();
      emitLogLimitsChanged();
    } catch (error) {
      console.warn('[WorkBuddy] scanWorkbuddyLogLimits failed:', error);
    } finally {
      if (inFlightRefresh === task) {
        inFlightRefresh = null;
      }
    }
  })();

  if (!force) {
    inFlightRefresh = task;
  }
  return task;
}

/**
 * 读取指定账号当前仍有效且未被用户关闭的日志限流记录。
 */
export function getWorkbuddyLogLimits(accountId: string): WorkbuddyLogLimitRecord[] {
  const key = identityKeyFor(accountId);
  const rows = logLimitsByIdentity[key] ?? logLimitsByIdentity[accountId] ?? [];
  if (rows.length === 0) return [];
  const now = Date.now();
  const dismissed = readDismissedMap();
  return rows.filter(
    (row) =>
      row.resetAt > now &&
      !Object.prototype.hasOwnProperty.call(
        dismissed,
        dismissEntryKey(key, row.model, row.resetAt),
      ),
  );
}

/**
 * 关闭（隐藏）指定日志限流提示；若后续出现新的 resetAt 会重新展示。
 */
export function dismissWorkbuddyLogLimit(
  accountId: string,
  model: string,
  resetAt: number,
) {
  const key = identityKeyFor(accountId);
  const dismissed = readDismissedMap();
  dismissed[dismissEntryKey(key, model, resetAt)] = resetAt;
  writeDismissedMap(dismissed);
}

/**
 * 合并「主动探测」与「客户端日志」两种限流来源，按模型去重并排序。
 */
export function getMergedWorkbuddyNotices(accountId: string): WorkbuddyMergedNotice[] {
  const probeRecords = getWorkbuddyProbeRecords(accountId);
  const logRecords = getWorkbuddyLogLimits(accountId);

  const probeByModel = new Map(probeRecords.map((record) => [record.model, record]));
  const merged: WorkbuddyMergedNotice[] = [];
  const handledModels = new Set<string>();

  for (const log of logRecords) {
    const model = log.model.trim();
    const matchedProbe = model ? probeByModel.get(model) : undefined;
    if (matchedProbe) {
      handledModels.add(model);
      if (matchedProbe.recoveredAt && matchedProbe.recoveredAt >= log.lastSeenAt) {
        merged.push({
          key: `probe:${model}`,
          accountId,
          model,
          status: matchedProbe.status,
          source: 'probe',
          resetAt: matchedProbe.resetAt,
          recoveredAt: matchedProbe.recoveredAt,
          timestamp: matchedProbe.recoveredAt,
          hitCount: 1,
          message: matchedProbe.message,
          hasProbeRecord: true,
          hasLogRecord: true,
          logResetAt: log.resetAt,
        });
      } else if (matchedProbe.recoveredAt && log.lastSeenAt > matchedProbe.recoveredAt) {
        merged.push({
          key: `log:${model}`,
          accountId,
          model,
          status: 'rate_limited',
          source: 'client_log',
          resetAt: log.resetAt,
          recoveredAt: null,
          timestamp: log.lastSeenAt,
          hitCount: log.hitCount,
          message: log.message,
          hasProbeRecord: true,
          hasLogRecord: true,
          logResetAt: log.resetAt,
        });
      } else {
        const bestResetAt =
          Math.max(matchedProbe.resetAt ?? 0, log.resetAt) || null;
        const useLogMessage = log.lastSeenAt >= matchedProbe.checkedAt;
        merged.push({
          key: `both:${model}`,
          accountId,
          model,
          status: 'rate_limited',
          source: 'both',
          resetAt: bestResetAt,
          recoveredAt: null,
          timestamp: Math.max(matchedProbe.checkedAt, log.lastSeenAt),
          hitCount: log.hitCount,
          message: useLogMessage
            ? log.message || matchedProbe.message
            : matchedProbe.message || log.message,
          hasProbeRecord: true,
          hasLogRecord: true,
          logResetAt: log.resetAt,
        });
      }
    } else {
      merged.push({
        key: model ? `log:${model}` : `log:__unknown__:${log.sessionId}:${log.resetAt}`,
        accountId,
        model,
        status: 'rate_limited',
        source: 'client_log',
        resetAt: log.resetAt,
        recoveredAt: null,
        timestamp: log.lastSeenAt,
        hitCount: log.hitCount,
        message: log.message,
        hasProbeRecord: false,
        hasLogRecord: true,
        logResetAt: log.resetAt,
      });
    }
  }

  for (const probe of probeRecords) {
    if (handledModels.has(probe.model)) continue;
    merged.push({
      key: `probe:${probe.model}`,
      accountId,
      model: probe.model,
      status: probe.status,
      source: 'probe',
      resetAt: probe.resetAt,
      recoveredAt: probe.recoveredAt ?? null,
      timestamp: probe.recoveredAt ?? probe.checkedAt,
      hitCount: 1,
      message: probe.message,
      hasProbeRecord: true,
      hasLogRecord: false,
      logResetAt: null,
    });
  }

  merged.sort((a, b) => {
    const aRecovered = a.recoveredAt ? 1 : 0;
    const bRecovered = b.recoveredAt ? 1 : 0;
    if (aRecovered !== bRecovered) return aRecovered - bRecovered;
    return b.timestamp - a.timestamp;
  });

  return merged;
}

export function dismissWorkbuddyMergedNotice(notice: WorkbuddyMergedNotice) {
  if (notice.hasProbeRecord && notice.model) {
    dismissWorkbuddyProbeRecord(notice.accountId, notice.model);
  }
  if (notice.hasLogRecord && notice.logResetAt != null) {
    dismissWorkbuddyLogLimit(notice.accountId, notice.model, notice.logResetAt);
  }
}

export function subscribeWorkbuddyLogLimits(listener: () => void): () => void {
  window.addEventListener(CHANGE_EVENT, listener);
  const onStorage = (event: StorageEvent) => {
    if (event.key === DISMISSED_STORAGE_KEY) listener();
  };
  window.addEventListener('storage', onStorage);
  return () => {
    window.removeEventListener('storage', onStorage);
    window.removeEventListener(CHANGE_EVENT, listener);
  };
}
