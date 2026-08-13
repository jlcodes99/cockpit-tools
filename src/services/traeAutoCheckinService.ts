/**
 * Trae SOLO CN 自动签到服务
 *
 * 基于 workbuddyAutoCheckinService 模式，适配 Trae 的签到 API。
 */

import { listTraeAccounts, getTraeCheckinStatus, claimTraeCheckin } from './traeService';
import { useTraeAccountStore } from '../stores/useTraeAccountStore';

export interface TraeAutoCheckinConfig {
  enabled: boolean;
  startTime: string; // HH:mm
  endTime: string;   // HH:mm
  lastCheckedDate?: string; // "YYYY-MM-DD"
}

export const DEFAULT_TRAE_AUTO_CHECKIN_CONFIG: TraeAutoCheckinConfig = {
  enabled: false,
  startTime: '06:00',
  endTime: '12:00',
};

const CONFIG_KEY = 'agtools.trae.auto_checkin_config';
export const TRAE_AUTO_CHECKIN_CONFIG_CHANGED_EVENT = 'trae-auto-checkin-config-changed';
const AUTO_CHECKIN_RETRY_DELAY_MS = 5 * 60 * 1000;
const AUTO_CHECKIN_IDLE_RECHECK_DELAY_MS = 60 * 60 * 1000;
const THIRTY_DAYS_MS = 30 * 24 * 60 * 60 * 1000;

export type TraeAutoCheckinCycleResult = 'disabled' | 'waiting' | 'completed' | 'retry';

function isValidTime(time: unknown): time is string {
  return typeof time === 'string' && /^([01]\d|2[0-3]):[0-5]\d$/.test(time);
}

export function getTraeAutoCheckinConfig(): TraeAutoCheckinConfig {
  if (typeof window === 'undefined') {
    return DEFAULT_TRAE_AUTO_CHECKIN_CONFIG;
  }
  try {
    const raw = localStorage.getItem(CONFIG_KEY);
    if (!raw) {
      return DEFAULT_TRAE_AUTO_CHECKIN_CONFIG;
    }
    const parsed = JSON.parse(raw);
    return {
      enabled: typeof parsed.enabled === 'boolean' ? parsed.enabled : false,
      startTime: isValidTime(parsed.startTime) ? parsed.startTime : '06:00',
      endTime: isValidTime(parsed.endTime) ? parsed.endTime : '12:00',
      lastCheckedDate: typeof parsed.lastCheckedDate === 'string' ? parsed.lastCheckedDate : undefined,
    };
  } catch {
    return DEFAULT_TRAE_AUTO_CHECKIN_CONFIG;
  }
}

export function saveTraeAutoCheckinConfig(config: TraeAutoCheckinConfig): void {
  if (typeof window === 'undefined') {
    return;
  }
  try {
    localStorage.setItem(CONFIG_KEY, JSON.stringify(config));
    window.dispatchEvent(new Event(TRAE_AUTO_CHECKIN_CONFIG_CHANGED_EVENT));
  } catch (err) {
    console.warn('[TraeAutoCheckin] 保存配置失败:', err);
  }
}

export function parseTimeToMinutes(timeStr: string): number {
  const parts = timeStr.split(':').map(Number);
  const h = parts[0] ?? 0;
  const m = parts[1] ?? 0;
  return h * 60 + m;
}

export function getTodayDateString(): string {
  const now = new Date();
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

export function formatTimeOnly(date: Date = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

export function formatFormattedTimestamp(date: Date = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(
    date.getHours(),
  )}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

let isAutoCheckinCycleRunning = false;

function getMillisecondsUntilNextLocalDay(now: Date): number {
  const nextDay = new Date(now);
  nextDay.setHours(24, 0, 1, 0);
  return Math.max(1_000, nextDay.getTime() - now.getTime());
}

export function getTraeAutoCheckinNextDelayMs(
  result?: TraeAutoCheckinCycleResult,
  _accounts: Array<{ id: string }> = [],
): number {
  const config = getTraeAutoCheckinConfig();
  if (!config.enabled) {
    return AUTO_CHECKIN_IDLE_RECHECK_DELAY_MS;
  }

  if (result === 'retry') {
    return AUTO_CHECKIN_RETRY_DELAY_MS;
  }

  const now = new Date();
  const todayStr = getTodayDateString();
  const currentMinute = now.getHours() * 60 + now.getMinutes();

  if (config.lastCheckedDate === todayStr) {
    return getMillisecondsUntilNextLocalDay(now);
  }

  const startMin = parseTimeToMinutes(config.startTime);
  const endMin = Math.max(startMin, parseTimeToMinutes(config.endTime));

  if (currentMinute < startMin) {
    const scheduledAt = new Date(now);
    scheduledAt.setHours(Math.floor(startMin / 60), startMin % 60, 0, 0);
    return Math.max(1_000, scheduledAt.getTime() - now.getTime());
  }

  if (currentMinute >= endMin) {
    return getMillisecondsUntilNextLocalDay(now);
  }

  return 1000;
}

export interface TraeAutoCheckinAccountDetail {
  accountId: string;
  email: string;
  status: 'success' | 'already_checked' | 'failed' | 'inactive';
  time?: string;
  message?: string;
  credit?: number;
}

export interface TraeAutoCheckinLogRecord {
  id: string;
  timestamp: string;
  date: string;
  durationMs: number;
  totalAccounts: number;
  successCount: number;
  alreadyCheckedCount: number;
  failedCount: number;
  status: 'success' | 'partial' | 'failed' | 'no_accounts';
  details: TraeAutoCheckinAccountDetail[];
}

const LOGS_KEY = 'agtools.trae.auto_checkin_logs';
export const TRAE_AUTO_CHECKIN_LOGS_CHANGED_EVENT = 'trae-auto-checkin-logs-changed';

export function getTraeAutoCheckinLogs(): TraeAutoCheckinLogRecord[] {
  if (typeof window === 'undefined') {
    return [];
  }
  try {
    const raw = localStorage.getItem(LOGS_KEY);
    if (!raw) {
      return [];
    }
    const parsed = JSON.parse(raw) as TraeAutoCheckinLogRecord[];
    if (!Array.isArray(parsed)) {
      return [];
    }

    const now = Date.now();
    const validLogs = parsed.filter((log) => {
      const logTime = new Date(log.timestamp.replace(' ', 'T')).getTime();
      return !isNaN(logTime) && now - logTime <= THIRTY_DAYS_MS;
    });

    if (validLogs.length !== parsed.length) {
      localStorage.setItem(LOGS_KEY, JSON.stringify(validLogs));
    }
    return validLogs;
  } catch {
    return [];
  }
}

export function saveTraeAutoCheckinLogs(logs: TraeAutoCheckinLogRecord[]): void {
  if (typeof window === 'undefined') {
    return;
  }
  try {
    const now = Date.now();
    const validLogs = logs.filter((log) => {
      const logTime = new Date(log.timestamp.replace(' ', 'T')).getTime();
      return !isNaN(logTime) && now - logTime <= THIRTY_DAYS_MS;
    });
    localStorage.setItem(LOGS_KEY, JSON.stringify(validLogs));
    window.dispatchEvent(new Event(TRAE_AUTO_CHECKIN_LOGS_CHANGED_EVENT));
  } catch (err) {
    console.warn('[TraeAutoCheckin] 保存自动签到日志失败:', err);
  }
}

export function addTraeAutoCheckinLog(record: TraeAutoCheckinLogRecord): void {
  const currentLogs = getTraeAutoCheckinLogs();
  const existingIndex = currentLogs.findIndex((l) => l.date === record.date);

  if (existingIndex < 0) {
    saveTraeAutoCheckinLogs([record, ...currentLogs]);
    return;
  }

  const existing = currentLogs[existingIndex];
  if (!existing) {
    saveTraeAutoCheckinLogs([record, ...currentLogs]);
    return;
  }

  const mergedDetailsMap = new Map<string, TraeAutoCheckinAccountDetail>();
  for (const d of existing.details) {
    mergedDetailsMap.set(d.accountId, d);
  }
  for (const d of record.details) {
    mergedDetailsMap.set(d.accountId, d);
  }

  const mergedDetails = Array.from(mergedDetailsMap.values());
  let successCount = 0;
  let alreadyCheckedCount = 0;
  let failedCount = 0;

  for (const d of mergedDetails) {
    if (d.status === 'success') successCount++;
    else if (d.status === 'already_checked') alreadyCheckedCount++;
    else if (d.status === 'failed') failedCount++;
  }

  const totalAccounts = mergedDetails.length;
  const overallStatus: TraeAutoCheckinLogRecord['status'] =
    totalAccounts === 0
      ? 'no_accounts'
      : failedCount === 0
        ? 'success'
        : successCount > 0 || alreadyCheckedCount > 0
          ? 'partial'
          : 'failed';

  const mergedRecord: TraeAutoCheckinLogRecord = {
    id: existing.id,
    timestamp: record.timestamp,
    date: record.date,
    durationMs: existing.durationMs + record.durationMs,
    totalAccounts,
    successCount,
    alreadyCheckedCount,
    failedCount,
    status: overallStatus,
    details: mergedDetails,
  };

  const updatedLogs = [...currentLogs];
  updatedLogs[existingIndex] = mergedRecord;
  saveTraeAutoCheckinLogs(updatedLogs);
}

export function clearTraeAutoCheckinLogs(): void {
  saveTraeAutoCheckinLogs([]);
}

export async function runTraeAutoCheckinCycleIfNeeded(
  force = false,
): Promise<TraeAutoCheckinCycleResult> {
  const config = getTraeAutoCheckinConfig();
  if (!config.enabled && !force) {
    return 'disabled';
  }

  if (isAutoCheckinCycleRunning) {
    return 'waiting';
  }

  isAutoCheckinCycleRunning = true;
  const startTime = Date.now();
  const startTimestampStr = formatFormattedTimestamp(new Date(startTime));
  const todayStr = getTodayDateString();

  try {
    console.log('[TraeAutoCheckin] 检查签到任务...');
    const accounts = await listTraeAccounts();
    if (accounts.length === 0) {
      if (force) {
        addTraeAutoCheckinLog({
          id: `log_${startTime}_${Math.random().toString(36).substring(2, 7)}`,
          timestamp: startTimestampStr,
          date: todayStr,
          durationMs: Date.now() - startTime,
          totalAccounts: 0,
          successCount: 0,
          alreadyCheckedCount: 0,
          failedCount: 0,
          status: 'no_accounts',
          details: [],
        });
      }
      return 'completed';
    }

    // 筛选今日需签到的账号：全部签到（简单模式，不设独立随机时间）
    const targetAccounts = accounts;

    let retryNeeded = false;
    let successCount = 0;
    let alreadyCheckedCount = 0;
    let failedCount = 0;
    let didCheckinAny = false;
    const details: TraeAutoCheckinAccountDetail[] = [];

    for (const account of targetAccounts) {
      const emailDisplay = account.email || account.id;
      const accountCheckinTime = formatTimeOnly(new Date());
      try {
        const status = await getTraeCheckinStatus(account.id);
        if (status.checked_in) {
          alreadyCheckedCount++;
          details.push({
            accountId: account.id,
            email: emailDisplay,
            status: 'already_checked',
            time: accountCheckinTime,
            message: '今日已完成签到',
          });
        } else {
          console.log(`[TraeAutoCheckin] 为账号 ${emailDisplay} 执行签到...`);
          const result = await claimTraeCheckin(account.id);
          didCheckinAny = true;
          successCount++;
          details.push({
            accountId: account.id,
            email: emailDisplay,
            status: 'success',
            time: accountCheckinTime,
            message: result.message || '签到成功',
            credit: result.total_credits,
          });
        }
      } catch (accountErr) {
        const errMsg = accountErr instanceof Error ? accountErr.message : String(accountErr);
        console.warn(`[TraeAutoCheckin] 账号 ${account.id} 签到异常:`, accountErr);
        retryNeeded = true;
        failedCount++;
        details.push({
          accountId: account.id,
          email: emailDisplay,
          status: 'failed',
          time: accountCheckinTime,
          message: errMsg,
        });
      }
    }

    // 更新最后检查日期
    saveTraeAutoCheckinConfig({
      ...config,
      lastCheckedDate: todayStr,
    });

    const durationMs = Date.now() - startTime;
    const overallStatus: TraeAutoCheckinLogRecord['status'] =
      failedCount === 0
        ? 'success'
        : successCount > 0 || alreadyCheckedCount > 0
          ? 'partial'
          : 'failed';

    addTraeAutoCheckinLog({
      id: `log_${startTime}_${Math.random().toString(36).substring(2, 7)}`,
      timestamp: startTimestampStr,
      date: todayStr,
      durationMs,
      totalAccounts: targetAccounts.length,
      successCount,
      alreadyCheckedCount,
      failedCount,
      status: overallStatus,
      details,
    });

    if (didCheckinAny) {
      void useTraeAccountStore.getState().fetchAccounts().catch((err) => {
        console.warn('[TraeAutoCheckin] 刷新账号列表失败:', err);
      });
    }
    return retryNeeded ? 'retry' : 'completed';
  } catch (err) {
    console.error('[TraeAutoCheckin] 自动签到周期失败:', err);
    return 'retry';
  } finally {
    isAutoCheckinCycleRunning = false;
  }
}