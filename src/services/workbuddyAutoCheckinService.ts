import { invoke } from '@tauri-apps/api/core';

export interface WorkbuddyAccountScheduleState {
  scheduledDate: string;        // "YYYY-MM-DD"
  scheduledMinute: number;      // Minutes from midnight (0..1439)
  lastCheckedDate?: string;     // "YYYY-MM-DD" when checked in
}

export interface WorkbuddyAutoCheckinConfig {
  enabled: boolean;
  startTime: string; // HH:mm, e.g. "06:00"
  endTime: string;   // HH:mm, e.g. "12:00"
  lastCheckedDate?: string; // "YYYY-MM-DD"
  accountSchedules?: Record<string, WorkbuddyAccountScheduleState>;
}

export const DEFAULT_WORKBUDDY_AUTO_CHECKIN_CONFIG: WorkbuddyAutoCheckinConfig = {
  enabled: false,
  startTime: '06:00',
  endTime: '12:00',
};

const CONFIG_KEY = 'agtools.workbuddy.auto_checkin_config';
const LEGACY_LOGS_KEY = 'agtools.workbuddy.auto_checkin_logs';
export const WORKBUDDY_AUTO_CHECKIN_CONFIG_CHANGED_EVENT = 'workbuddy-auto-checkin-config-changed';
const AUTO_CHECKIN_RETRY_DELAY_MS = 5 * 60 * 1000;
const AUTO_CHECKIN_IDLE_RECHECK_DELAY_MS = 60 * 60 * 1000;

export type WorkbuddyAutoCheckinCycleResult = 'disabled' | 'waiting' | 'completed' | 'retry';

export function clearLegacyWorkbuddyAutoCheckinLogs(): void {
  if (typeof window === 'undefined') {
    return;
  }

  try {
    if (localStorage.getItem(LEGACY_LOGS_KEY) !== null) {
      localStorage.removeItem(LEGACY_LOGS_KEY);
      console.info('[WorkbuddyAutoCheckin] 已清理废弃的 WebView 自动签到日志缓存');
    }
  } catch (err) {
    console.warn('[WorkbuddyAutoCheckin] 清理废弃的自动签到日志缓存失败:', err);
  }
}

let cachedConfig: WorkbuddyAutoCheckinConfig | null = null;

function isValidTime(time: unknown): time is string {
  return typeof time === 'string' && /^([01]\d|2[0-3]):[0-5]\d$/.test(time);
}

function cacheConfigLocally(config: WorkbuddyAutoCheckinConfig, emitChange = false): void {
  cachedConfig = config;
  if (typeof window === 'undefined') {
    return;
  }
  try {
    localStorage.setItem(CONFIG_KEY, JSON.stringify(config));
    if (emitChange) {
      window.dispatchEvent(new Event(WORKBUDDY_AUTO_CHECKIN_CONFIG_CHANGED_EVENT));
    }
  } catch (err) {
    console.warn('[WorkbuddyAutoCheckin] 本地缓存保存失败:', err);
  }
}

export async function getWorkbuddyAutoCheckinConfigAsync(): Promise<WorkbuddyAutoCheckinConfig> {
  try {
    const config = await invoke<WorkbuddyAutoCheckinConfig>('get_workbuddy_auto_checkin_config');
    cacheConfigLocally(config);
    return config;
  } catch (err) {
    console.warn('[WorkbuddyAutoCheckin] 从 Rust 端获取配置失败，使用本地缓存或默认值:', err);
    return getWorkbuddyAutoCheckinConfig();
  }
}

export function getWorkbuddyAutoCheckinConfig(): WorkbuddyAutoCheckinConfig {
  if (cachedConfig) {
    return cachedConfig;
  }
  if (typeof window === 'undefined') {
    return DEFAULT_WORKBUDDY_AUTO_CHECKIN_CONFIG;
  }
  try {
    const raw = localStorage.getItem(CONFIG_KEY);
    if (!raw) {
      return DEFAULT_WORKBUDDY_AUTO_CHECKIN_CONFIG;
    }
    const parsed = JSON.parse(raw);
    const config: WorkbuddyAutoCheckinConfig = {
      enabled: typeof parsed.enabled === 'boolean' ? parsed.enabled : false,
      startTime: isValidTime(parsed.startTime) ? parsed.startTime : '06:00',
      endTime: isValidTime(parsed.endTime) ? parsed.endTime : '12:00',
      lastCheckedDate: typeof parsed.lastCheckedDate === 'string' ? parsed.lastCheckedDate : undefined,
      accountSchedules: typeof parsed.accountSchedules === 'object' && parsed.accountSchedules !== null ? parsed.accountSchedules : undefined,
    };
    cachedConfig = config;
    return config;
  } catch {
    return DEFAULT_WORKBUDDY_AUTO_CHECKIN_CONFIG;
  }
}

export async function migrateWorkbuddyAutoCheckinConfigAsync(
  legacyConfig: WorkbuddyAutoCheckinConfig,
): Promise<WorkbuddyAutoCheckinConfig> {
  const config = await invoke<WorkbuddyAutoCheckinConfig>(
    'migrate_workbuddy_auto_checkin_config',
    { legacyConfig },
  );
  cacheConfigLocally(config, true);
  return config;
}

export async function saveWorkbuddyAutoCheckinConfigAsync(config: WorkbuddyAutoCheckinConfig): Promise<void> {
  if (typeof window === 'undefined') {
    cacheConfigLocally(config);
    return;
  }
  await invoke('save_workbuddy_auto_checkin_config', { config });
  cacheConfigLocally(config, true);
}

export function saveWorkbuddyAutoCheckinConfig(config: WorkbuddyAutoCheckinConfig): void {
  void saveWorkbuddyAutoCheckinConfigAsync(config).catch((err) => {
    console.warn('[WorkbuddyAutoCheckin] 保存配置到 Rust 后端失败:', err);
  });
}

export function parseTimeToMinutes(timeStr: string): number {
  const parts = timeStr.split(':').map(Number);
  const h = parts[0] ?? 0;
  const m = parts[1] ?? 0;
  return h * 60 + m;
}

export function formatMinutesToTime(minutes: number): string {
  const h = Math.floor(minutes / 60) % 24;
  const m = minutes % 60;
  return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}`;
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

export function ensureAccountSchedules(
  config: WorkbuddyAutoCheckinConfig,
  accounts: Array<{ id: string; email?: string }>,
): WorkbuddyAutoCheckinConfig {
  const todayStr = getTodayDateString();
  const startMin = parseTimeToMinutes(config.startTime);
  let endMin = parseTimeToMinutes(config.endTime);
  if (endMin < startMin) {
    endMin = startMin;
  }
  const minRange = Math.max(0, endMin - startMin);

  const existingSchedules = config.accountSchedules || {};
  let changed = false;
  const updatedSchedules: Record<string, WorkbuddyAccountScheduleState> = { ...existingSchedules };

  for (const account of accounts) {
    const existing = existingSchedules[account.id];
    if (
      existing &&
      existing.scheduledDate === todayStr &&
      existing.scheduledMinute >= startMin &&
      existing.scheduledMinute <= endMin
    ) {
      continue;
    }

    const randomOffset = minRange > 0 ? Math.floor(Math.random() * (minRange + 1)) : 0;
    const scheduledMinute = startMin + randomOffset;

    updatedSchedules[account.id] = {
      scheduledDate: todayStr,
      scheduledMinute,
      lastCheckedDate: existing?.lastCheckedDate === todayStr ? todayStr : undefined,
    };
    changed = true;
  }

  if (changed) {
    const updatedConfig: WorkbuddyAutoCheckinConfig = {
      ...config,
      accountSchedules: updatedSchedules,
    };
    saveWorkbuddyAutoCheckinConfig(updatedConfig);
    return updatedConfig;
  }

  return config;
}

function getMillisecondsUntilNextLocalDay(now: Date): number {
  const nextDay = new Date(now);
  nextDay.setHours(24, 0, 1, 0);
  return Math.max(1_000, nextDay.getTime() - now.getTime());
}

export function getWorkbuddyAutoCheckinNextDelayMs(
  result?: WorkbuddyAutoCheckinCycleResult,
  accounts: Array<{ id: string }> = [],
): number {
  const config = getWorkbuddyAutoCheckinConfig();
  if (!config.enabled) {
    return AUTO_CHECKIN_IDLE_RECHECK_DELAY_MS;
  }

  if (result === 'retry') {
    return AUTO_CHECKIN_RETRY_DELAY_MS;
  }

  const now = new Date();
  const todayStr = getTodayDateString();
  const updatedConfig = ensureAccountSchedules(config, accounts);
  const schedules = updatedConfig.accountSchedules || {};

  const currentMinute = now.getHours() * 60 + now.getMinutes();
  let nextScheduledMinute: number | null = null;

  for (const accId of Object.keys(schedules)) {
    const sch = schedules[accId];
    if (!sch) continue;
    if (sch.lastCheckedDate !== todayStr) {
      if (nextScheduledMinute === null || sch.scheduledMinute < nextScheduledMinute) {
        nextScheduledMinute = sch.scheduledMinute;
      }
    }
  }

  if (nextScheduledMinute === null) {
    return getMillisecondsUntilNextLocalDay(now);
  }

  if (currentMinute >= nextScheduledMinute) {
    return 1000;
  }

  const scheduledAt = new Date(now);
  scheduledAt.setHours(Math.floor(nextScheduledMinute / 60), nextScheduledMinute % 60, 0, 0);
  return Math.max(1_000, scheduledAt.getTime() - now.getTime());
}

export interface WorkbuddyAutoCheckinAccountDetail {
  accountId: string;
  email: string;
  status: 'success' | 'already_checked' | 'failed' | 'inactive';
  time?: string;
  message?: string;
  credit?: number;
}

export interface WorkbuddyAutoCheckinLogRecord {
  id: string;
  timestamp: string;
  date: string;
  durationMs: number;
  totalAccounts: number;
  successCount: number;
  alreadyCheckedCount: number;
  failedCount: number;
  status: 'success' | 'partial' | 'failed' | 'no_accounts';
  details: WorkbuddyAutoCheckinAccountDetail[];
}

export const WORKBUDDY_AUTO_CHECKIN_LOGS_CHANGED_EVENT = 'workbuddy-auto-checkin-logs-changed';

export async function getWorkbuddyAutoCheckinLogsAsync(): Promise<WorkbuddyAutoCheckinLogRecord[]> {
  return await invoke<WorkbuddyAutoCheckinLogRecord[]>('get_workbuddy_auto_checkin_logs');
}

export async function clearWorkbuddyAutoCheckinLogs(): Promise<void> {
  await invoke('clear_workbuddy_auto_checkin_logs');
}

export function formatFormattedTimestamp(date: Date = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(
    date.getHours(),
  )}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

export async function runWorkbuddyAutoCheckinCycleIfNeeded(
  force = false,
): Promise<WorkbuddyAutoCheckinCycleResult> {
  const res = await invoke<string>('run_workbuddy_auto_checkin_now', { force });
  if (res === 'already_running') {
    return 'waiting';
  }
  if (res === 'disabled' || res === 'waiting' || res === 'completed' || res === 'retry') {
    return res as WorkbuddyAutoCheckinCycleResult;
  }
  return 'completed';
}
