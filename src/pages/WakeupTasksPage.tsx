import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { confirm as confirmDialog } from '@tauri-apps/plugin-dialog';
import { openUrl } from '@tauri-apps/plugin-opener';
import { ChevronLeft, Plus, Pencil, Trash2, Power, X } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { useAccountStore } from '../stores/useAccountStore';
import { Page } from '../types/navigation';
import { MultiSelectFilterDropdown, type MultiSelectFilterOption } from '../components/MultiSelectFilterDropdown';
import {
  collectAntigravityQuotaModelKeys,
  filterAntigravityModelOptions,
  getAntigravityModelDisplayName,
  type AntigravityModelOption,
} from '../utils/antigravityModels';
import {
  accountMatchesTagFilters,
  accountMatchesTypeFilters,
  buildAccountTierCounts,
  buildAccountTierFilterOptions,
  collectAvailableAccountTags,
  normalizeAccountTag,
  type AccountFilterType,
} from '../utils/accountFilters';
import { getAccountGroups, type AccountGroup } from '../services/accountGroupService';
import {
  isPrivacyModeEnabledByDefault,
  maskSensitiveValue,
  PRIVACY_MODE_CHANGED_EVENT,
} from '../utils/privacy';
import {
  loadWakeupOfficialLsVersionMode,
  saveWakeupOfficialLsVersionMode,
  WAKEUP_OFFICIAL_LS_VERSION_CHANGED_EVENT,
  type WakeupOfficialLsVersionMode,
} from '../utils/wakeupOfficialLsVersion';
import { ModalErrorMessage, useModalErrorState } from '../components/ModalErrorMessage';
import { useEscClose } from '../hooks/useEscClose';
import { OverviewTabsHeader } from '../components/OverviewTabsHeader';

const TASKS_STORAGE_KEY = 'agtools.wakeup.tasks';
const WAKEUP_ENABLED_KEY = 'agtools.wakeup.enabled';
const WAKEUP_FORCE_DISABLE_MIGRATION_KEY = 'agtools.wakeup.migration.force_disable_0_8_14';
const LEGACY_SCHEDULE_KEY = 'agtools.wakeup.schedule';
const MAX_HISTORY_ITEMS = 100;
const WAKEUP_ERROR_JSON_PREFIX = 'AG_WAKEUP_ERROR_JSON:';
const APP_PATH_NOT_FOUND_PREFIX = 'APP_PATH_NOT_FOUND:';
const UNGROUPED_ACCOUNT_GROUP_FILTER_KEY = '__ungrouped__';

const BASE_TIME_OPTIONS = [
  '06:00',
  '07:00',
  '08:00',
  '09:00',
  '10:00',
  '11:00',
  '12:00',
  '14:00',
  '16:00',
  '18:00',
  '20:00',
  '22:00',
];

const WEEKDAY_KEYS = ['sun', 'mon', 'tue', 'wed', 'thu', 'fri', 'sat'];
const DEFAULT_PROMPT = 'hi';
const buildWakeupTestScopeId = () =>
  typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function'
    ? `wakeup-test-${crypto.randomUUID()}`
    : `wakeup-test-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;

type Translator = (key: string, options?: Record<string, unknown>) => string;
const getReadableModelLabel = (id: string) => getAntigravityModelDisplayName(id);

type TriggerMode = 'scheduled' | 'crontab' | 'quota_reset' | 'startup';
type RepeatMode = 'daily' | 'weekly' | 'interval';

type TriggerSource = 'scheduled' | 'crontab' | 'quota_reset' | 'startup';
type HistoryTriggerSource = TriggerSource | 'manual';
type HistoryTriggerType = 'manual' | 'auto';

type NoticeTone = 'error' | 'warning' | 'success';

interface WakeupPageProps {
  onNavigate?: (page: Page) => void;
}

type AvailableModel = AntigravityModelOption;

interface ScheduleConfig {
  repeatMode: RepeatMode;
  dailyTimes: string[];
  weeklyDays: number[];
  weeklyTimes: string[];
  intervalHours: number;
  intervalStartTime: string;
  intervalEndTime: string;
  selectedModels: string[];
  selectedAccounts: string[];
  crontab?: string;
  wakeOnReset?: boolean;
  customPrompt?: string;
  maxOutputTokens?: number;
  timeWindowEnabled?: boolean;
  timeWindowStart?: string;
  timeWindowEnd?: string;
  fallbackTimes?: string[];
  startupDelayMinutes?: number;
}

interface WakeupTask {
  id: string;
  name: string;
  enabled: boolean;
  createdAt: number;
  lastRunAt?: number;
  schedule: ScheduleConfig;
}

interface WakeupGeneralConfig {
  language?: string;
  theme?: string;
  auto_refresh_minutes: number;
  codex_auto_refresh_minutes?: number;
  close_behavior?: string;
  opencode_app_path?: string;
  antigravity_app_path?: string;
  codex_app_path?: string;
  vscode_app_path?: string;
  opencode_sync_on_switch?: boolean;
  opencode_auth_overwrite_on_switch?: boolean;
  codex_launch_on_switch?: boolean;
}

interface WakeupHistoryRecord {
  id: string;
  timestamp: number;
  triggerType: HistoryTriggerType;
  triggerSource: HistoryTriggerSource;
  taskName?: string;
  accountEmail: string;
  modelId: string;
  prompt?: string;
  success: boolean;
  message?: string;
  duration?: number;
}

type WakeupStructuredErrorKind = 'verification_required' | 'quota' | 'temporary' | 'generic';

interface WakeupStructuredErrorPayload {
  version?: number;
  kind?: WakeupStructuredErrorKind;
  message?: string;
  errorCode?: number | null;
  validationUrl?: string | null;
  trajectoryId?: string | null;
  errorMessageJson?: string | null;
  stepJson?: string | null;
}

const parseWakeupStructuredError = (message?: string | null): WakeupStructuredErrorPayload | null => {
  if (!message || typeof message !== 'string') return null;
  if (!message.startsWith(WAKEUP_ERROR_JSON_PREFIX)) return null;
  const payloadText = message.slice(WAKEUP_ERROR_JSON_PREFIX.length).trim();
  if (!payloadText) return null;
  try {
    const parsed = JSON.parse(payloadText) as WakeupStructuredErrorPayload;
    if (!parsed || typeof parsed !== 'object') return null;
    return parsed;
  } catch {
    return null;
  }
};

const getWakeupErrorDisplayText = (message?: string | null): string => {
  if (!message) return '';
  const payload = parseWakeupStructuredError(message);
  if (!payload) return message;
  return (payload.message || '').trim() || message;
};

interface WakeupInvokeResult {
  reply: string;
  promptTokens?: number;
  completionTokens?: number;
  totalTokens?: number;
  traceId?: string;
  responseId?: string;
  durationMs?: number;
}

interface WakeupTaskResultPayload {
  taskId: string;
  lastRunAt: number;
  records: WakeupHistoryRecord[];
}

const DEFAULT_SCHEDULE: ScheduleConfig = {
  repeatMode: 'daily',
  dailyTimes: ['08:00'],
  weeklyDays: [1, 2, 3, 4, 5],
  weeklyTimes: ['08:00'],
  intervalHours: 4,
  intervalStartTime: '07:00',
  intervalEndTime: '22:00',
  selectedModels: ['gemini-3-flash'],
  selectedAccounts: [],
  maxOutputTokens: 0,
};

const MAX_STARTUP_DELAY_MINUTES = 1440;

const normalizeStartupDelayMinutes = (value?: number): number | undefined => {
  if (typeof value !== 'number' || Number.isNaN(value)) {
    return undefined;
  }
  return Math.min(MAX_STARTUP_DELAY_MINUTES, Math.max(0, Math.floor(value)));
};

const normalizeSchedule = (schedule: ScheduleConfig): ScheduleConfig => {
  const dailyTimes = schedule.dailyTimes?.length ? schedule.dailyTimes : ['08:00'];
  const weeklyDays = schedule.weeklyDays?.length ? schedule.weeklyDays : [1, 2, 3, 4, 5];
  const weeklyTimes = schedule.weeklyTimes?.length ? schedule.weeklyTimes : ['08:00'];
  const intervalHours = schedule.intervalHours && schedule.intervalHours > 0 ? schedule.intervalHours : 4;
  const intervalStartTime = schedule.intervalStartTime || '07:00';
  const intervalEndTime = schedule.intervalEndTime || '22:00';
  const maxOutputTokens = typeof schedule.maxOutputTokens === 'number' ? schedule.maxOutputTokens : 0;
  const fallbackTimes = schedule.fallbackTimes?.length ? schedule.fallbackTimes : ['07:00'];

  return {
    ...schedule,
    dailyTimes,
    weeklyDays,
    weeklyTimes,
    intervalHours,
    intervalStartTime,
    intervalEndTime,
    maxOutputTokens,
    fallbackTimes,
    startupDelayMinutes: normalizeStartupDelayMinutes(schedule.startupDelayMinutes),
  };
};

const normalizeTask = (task: WakeupTask): WakeupTask => ({
  ...task,
  schedule: normalizeSchedule({ ...DEFAULT_SCHEDULE, ...task.schedule }),
});

const parseTasks = (raw: string | null): WakeupTask[] => {
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw) as WakeupTask[];
    return parsed.map((task) => normalizeTask(task));
  } catch {
    return [];
  }
};

const loadTasks = (defaultTaskName: string): WakeupTask[] => {
  const rawTasks = localStorage.getItem(TASKS_STORAGE_KEY);
  if (rawTasks) return parseTasks(rawTasks);

  const legacyRaw = localStorage.getItem(LEGACY_SCHEDULE_KEY);
  if (!legacyRaw) return [];
  try {
    const legacySchedule = JSON.parse(legacyRaw) as Partial<ScheduleConfig> & { enabled?: boolean };
    const task: WakeupTask = {
      id: crypto.randomUUID ? crypto.randomUUID() : String(Date.now()),
      name: defaultTaskName,
      enabled: legacySchedule.enabled ?? false,
      createdAt: Date.now(),
      schedule: normalizeSchedule({ ...DEFAULT_SCHEDULE, ...legacySchedule }),
    };
    return [task];
  } catch {
    return [];
  }
};

const loadHistory = async (): Promise<WakeupHistoryRecord[]> => {
  try {
    const records = await invoke<WakeupHistoryRecord[]>('wakeup_load_history');
    if (!Array.isArray(records)) return [];
    return records
      .filter((item) => item && typeof item.timestamp === 'number')
      .sort((a, b) => b.timestamp - a.timestamp)
      .slice(0, MAX_HISTORY_ITEMS);
  } catch (error) {
    console.error('加载唤醒历史失败:', error);
    return [];
  }
};

const formatErrorMessage = (error: unknown) => {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  try {
    return JSON.stringify(error);
  } catch {
    return String(error);
  }
};

const isAntigravityPathMissingError = (message: string) =>
  message.startsWith(`${APP_PATH_NOT_FOUND_PREFIX}antigravity`);

const formatWakeupMessage = (
  modelId: string,
  result: WakeupInvokeResult,
  durationMs: number | undefined,
  t: Translator
) => {
  const reply = result.reply && result.reply.trim() ? result.reply.trim() : t('wakeup.format.noReply');
  const details: string[] = [];
  if (typeof durationMs === 'number') {
    details.push(t('wakeup.format.durationMs', { ms: durationMs }));
  }
  if (result.promptTokens !== undefined || result.totalTokens !== undefined) {
    const promptTokens = result.promptTokens ?? '?';
    const completionTokens = result.completionTokens ?? '?';
    const totalTokens = result.totalTokens ?? '?';
    details.push(
      t('wakeup.format.tokens', { prompt: promptTokens, completion: completionTokens, total: totalTokens })
    );
  }
  if (result.traceId) {
    details.push(t('wakeup.format.traceId', { traceId: result.traceId }));
  }
  const joiner = t('wakeup.format.detailJoiner');
  const suffix = details.length ? ` (${details.join(joiner)})` : '';
  return t('wakeup.format.message', { model: modelId, reply, suffix });
};

const normalizeTimeInput = (value: string) => {
  const trimmed = String(value || '').trim();
  if (!trimmed) return null;
  const match = trimmed.match(/^(\d{1,2}):(\d{2})$/);
  if (!match) return null;
  const hour = Number.parseInt(match[1], 10);
  const minute = Number.parseInt(match[2], 10);
  if (Number.isNaN(hour) || Number.isNaN(minute)) return null;
  if (hour < 0 || hour > 23 || minute < 0 || minute > 59) return null;
  return `${String(hour).padStart(2, '0')}:${String(minute).padStart(2, '0')}`;
};

const parseTimeToMinutes = (value: string | undefined, fallbackMinutes: number) => {
  const normalized = normalizeTimeInput(value || '');
  if (!normalized) return fallbackMinutes;
  const [hour, minute] = normalized.split(':').map(Number);
  return hour * 60 + minute;
};

const calculateNextRuns = (config: ScheduleConfig, count: number) => {
  const now = new Date();
  const results: Date[] = [];

  if (config.repeatMode === 'daily' && config.dailyTimes?.length) {
    for (let dayOffset = 0; dayOffset < 7 && results.length < count; dayOffset += 1) {
      for (const time of [...config.dailyTimes].sort()) {
        const [h, m] = time.split(':').map(Number);
        const date = new Date(now);
        date.setDate(date.getDate() + dayOffset);
        date.setHours(h, m, 0, 0);
        if (date > now) {
          results.push(date);
          if (results.length >= count) break;
        }
      }
    }
  } else if (config.repeatMode === 'weekly' && config.weeklyDays?.length && config.weeklyTimes?.length) {
    for (let dayOffset = 0; dayOffset < 14 && results.length < count; dayOffset += 1) {
      const date = new Date(now);
      date.setDate(date.getDate() + dayOffset);
      const dayOfWeek = date.getDay();
      if (config.weeklyDays.includes(dayOfWeek)) {
        for (const time of [...config.weeklyTimes].sort()) {
          const [h, m] = time.split(':').map(Number);
          const candidate = new Date(date);
          candidate.setHours(h, m, 0, 0);
          if (candidate > now) {
            results.push(candidate);
            if (results.length >= count) break;
          }
        }
      }
    }
  } else if (config.repeatMode === 'interval') {
    const startMinutes = parseTimeToMinutes(config.intervalStartTime, 7 * 60);
    const endMinutes = parseTimeToMinutes(config.intervalEndTime, 22 * 60);
    const intervalHours = Math.max(1, config.intervalHours || 4);
    const intervalMs = intervalHours * 60 * 60 * 1000;

    for (let dayOffset = 0; dayOffset < 7 && results.length < count; dayOffset += 1) {
      const baseDate = new Date(now);
      baseDate.setDate(baseDate.getDate() + dayOffset);

      const windowStart = new Date(baseDate);
      windowStart.setHours(Math.floor(startMinutes / 60), startMinutes % 60, 0, 0);
      const windowEnd = new Date(baseDate);
      windowEnd.setHours(Math.floor(endMinutes / 60), endMinutes % 60, 0, 0);
      if (startMinutes > endMinutes) {
        windowEnd.setDate(windowEnd.getDate() + 1);
      }

      for (
        let candidate = new Date(windowStart);
        candidate <= windowEnd && results.length < count;
        candidate = new Date(candidate.getTime() + intervalMs)
      ) {
        if (candidate > now) {
          results.push(new Date(candidate));
        }
      }
    }
  }

  return results.slice(0, count);
};

interface ParsedCronField {
  values: Set<number>;
  wildcard: boolean;
}

interface ParsedCrontab {
  minute: ParsedCronField;
  hour: ParsedCronField;
  dayOfMonth: ParsedCronField;
  month: ParsedCronField;
  dayOfWeek: ParsedCronField;
}

const normalizeCrontabDayOfWeek = (value: number) => (value === 7 ? 0 : value);

const parseCrontabNumber = (raw: string): number => {
  const parsed = Number.parseInt(raw.trim(), 10);
  if (Number.isNaN(parsed)) {
    throw new Error('invalid_crontab_number');
  }
  return parsed;
};

const validateCrontabValue = (
  value: number,
  min: number,
  max: number,
  normalizeDayOfWeek: boolean,
) => {
  if (normalizeDayOfWeek && value === 7) return;
  if (value < min || value > max) {
    throw new Error('crontab_value_out_of_range');
  }
};

const insertCrontabRange = (
  target: Set<number>,
  start: number,
  end: number,
  step: number,
  normalizeDayOfWeek: boolean,
) => {
  for (let value = start; value <= end; value += step) {
    target.add(normalizeDayOfWeek ? normalizeCrontabDayOfWeek(value) : value);
  }
};

const parseCrontabSegment = (
  segment: string,
  min: number,
  max: number,
  normalizeDayOfWeek: boolean,
  target: Set<number>,
) => {
  const [rawRange, rawStep] = segment.split('/');
  const rangePart = rawRange.trim();
  const step = rawStep ? parseCrontabNumber(rawStep) : 1;
  if (step <= 0) {
    throw new Error('crontab_step_must_be_positive');
  }

  if (rangePart === '*') {
    insertCrontabRange(target, min, max, step, normalizeDayOfWeek);
    return;
  }

  if (rangePart.includes('-')) {
    const [rawStart, rawEnd] = rangePart.split('-');
    const start = parseCrontabNumber(rawStart);
    const end = parseCrontabNumber(rawEnd);
    validateCrontabValue(start, min, max, normalizeDayOfWeek);
    validateCrontabValue(end, min, max, normalizeDayOfWeek);
    if (end < start) {
      throw new Error('crontab_range_invalid');
    }
    insertCrontabRange(target, start, end, step, normalizeDayOfWeek);
    return;
  }

  const single = parseCrontabNumber(rangePart);
  validateCrontabValue(single, min, max, normalizeDayOfWeek);
  if (step === 1) {
    target.add(normalizeDayOfWeek ? normalizeCrontabDayOfWeek(single) : single);
    return;
  }
  insertCrontabRange(target, single, max, step, normalizeDayOfWeek);
};

const parseCrontabField = (
  field: string,
  min: number,
  max: number,
  normalizeDayOfWeek: boolean,
): ParsedCronField => {
  const trimmed = field.trim();
  if (!trimmed) {
    throw new Error('crontab_field_empty');
  }

  if (trimmed === '*') {
    const values = new Set<number>();
    insertCrontabRange(values, min, max, 1, normalizeDayOfWeek);
    return { values, wildcard: true };
  }

  const values = new Set<number>();
  const segments = trimmed.split(',');
  if (segments.length === 0) {
    throw new Error('crontab_field_empty');
  }
  segments.forEach((segment) => {
    const normalizedSegment = segment.trim();
    if (!normalizedSegment) {
      throw new Error('crontab_segment_empty');
    }
    parseCrontabSegment(normalizedSegment, min, max, normalizeDayOfWeek, values);
  });

  if (values.size === 0) {
    throw new Error('crontab_no_values');
  }
  return { values, wildcard: false };
};

const parseCrontabExpression = (expr: string): ParsedCrontab => {
  const parts = expr.trim().split(/\s+/);
  if (parts.length !== 5) {
    throw new Error('crontab_parts_must_be_five');
  }

  return {
    minute: parseCrontabField(parts[0], 0, 59, false),
    hour: parseCrontabField(parts[1], 0, 23, false),
    dayOfMonth: parseCrontabField(parts[2], 1, 31, false),
    month: parseCrontabField(parts[3], 1, 12, false),
    dayOfWeek: parseCrontabField(parts[4], 0, 7, true),
  };
};

const isCrontabMatch = (parsed: ParsedCrontab, date: Date): boolean => {
  const minute = date.getMinutes();
  const hour = date.getHours();
  const dayOfMonth = date.getDate();
  const month = date.getMonth() + 1;
  const dayOfWeek = date.getDay();

  if (
    !parsed.minute.values.has(minute)
    || !parsed.hour.values.has(hour)
    || !parsed.month.values.has(month)
  ) {
    return false;
  }

  const dayOfMonthMatch = parsed.dayOfMonth.values.has(dayOfMonth);
  const dayOfWeekMatch = parsed.dayOfWeek.values.has(dayOfWeek);
  if (parsed.dayOfMonth.wildcard && parsed.dayOfWeek.wildcard) {
    return true;
  }
  if (parsed.dayOfMonth.wildcard) {
    return dayOfWeekMatch;
  }
  if (parsed.dayOfWeek.wildcard) {
    return dayOfMonthMatch;
  }
  return dayOfMonthMatch || dayOfWeekMatch;
};

const calculateCrontabNextRuns = (crontab: string, count: number) => {
  try {
    const parsed = parseCrontabExpression(crontab);
    const results: Date[] = [];
    const candidate = new Date();
    candidate.setSeconds(0, 0);
    candidate.setMinutes(candidate.getMinutes() + 1);

    const maxLookaheadMinutes = 366 * 24 * 60;
    for (let i = 0; i < maxLookaheadMinutes && results.length < count; i += 1) {
      if (isCrontabMatch(parsed, candidate)) {
        results.push(new Date(candidate));
      }
      candidate.setMinutes(candidate.getMinutes() + 1);
    }

    return results;
  } catch {
    return [];
  }
};

const formatDateTime = (timestamp: number | undefined, locale: string, t: Translator) => {
  if (!timestamp) return t('wakeup.format.none');
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) return t('wakeup.format.none');
  return date.toLocaleString(locale, { hour12: false });
};

const formatRunTime = (date: Date, locale: string, t: Translator) => {
  const now = new Date();
  const tomorrow = new Date(now);
  tomorrow.setDate(tomorrow.getDate() + 1);
  const timeStr = date.toLocaleTimeString(locale, { hour: '2-digit', minute: '2-digit', hour12: false });

  if (date.toDateString() === now.toDateString()) {
    return t('wakeup.format.today', { time: timeStr });
  }
  if (date.toDateString() === tomorrow.toDateString()) {
    return t('wakeup.format.tomorrow', { time: timeStr });
  }
  const weekdayKey = WEEKDAY_KEYS[date.getDay()] || WEEKDAY_KEYS[0];
  return t('wakeup.format.weekdayWithTime', { day: t(`wakeup.weekdays.${weekdayKey}`), time: timeStr });
};

const formatSelectionPreview = (items: string[], maxItems: number, t: Translator) => {
  if (items.length === 0) return t('wakeup.format.notSelected');
  const joiner = t('wakeup.format.joiner');
  if (items.length <= maxItems) return items.join(joiner);
  return t('wakeup.format.moreItems', {
    items: items.slice(0, maxItems).join(joiner),
    total: items.length,
  });
};

const filterAvailableModels = (
  models: AvailableModel[],
  allowedModelKeys?: Iterable<string>,
) =>
  filterAntigravityModelOptions(models, {
    allowedModelKeys,
    includeNonRecommended: false,
  });

const getTriggerMode = (task: WakeupTask): TriggerMode => {
  if (typeof task.schedule.startupDelayMinutes === 'number') return 'startup';
  if (task.schedule.wakeOnReset) return 'quota_reset';
  if (task.schedule.crontab) return 'crontab';
  return 'scheduled';
};

export function WakeupTasksPage({ onNavigate }: WakeupPageProps) {
  const { t, i18n } = useTranslation();
  const { accounts, currentAccount, fetchAccounts, fetchCurrentAccount } = useAccountStore();
  const locale = i18n.language || 'zh-CN';
  const [tasks, setTasks] = useState<WakeupTask[]>(() => loadTasks(t('wakeup.defaultTaskName')));
  const [wakeupEnabled, setWakeupEnabled] = useState(() => {
    if (localStorage.getItem(WAKEUP_FORCE_DISABLE_MIGRATION_KEY) !== '1') {
      localStorage.setItem(WAKEUP_ENABLED_KEY, 'false');
      localStorage.setItem(WAKEUP_FORCE_DISABLE_MIGRATION_KEY, '1');
      return false;
    }
    const raw = localStorage.getItem(WAKEUP_ENABLED_KEY);
    return raw ? raw === 'true' : false;
  });
  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [notice, setNotice] = useState<{ text: string; tone?: NoticeTone } | null>(null);
  const [historyRecords, setHistoryRecords] = useState<WakeupHistoryRecord[]>([]);
  const [testing, setTesting] = useState(false);
  const [showTestModal, setShowTestModal] = useState(false);
  const {
    message: testModalError,
    scrollKey: testModalErrorScrollKey,
    report: reportTestModalError,
    clear: clearTestModalError,
  } = useModalErrorState('');
  const [showHistoryModal, setShowHistoryModal] = useState(false);
  const [privacyModeEnabled, setPrivacyModeEnabled] = useState<boolean>(() =>
    isPrivacyModeEnabledByDefault(),
  );

  const [showModal, setShowModal] = useState(false);
  const [editingTaskId, setEditingTaskId] = useState<string | null>(null);

  const [formName, setFormName] = useState('');
  const [formEnabled, setFormEnabled] = useState(true);
  const [formTriggerMode, setFormTriggerMode] = useState<TriggerMode>('scheduled');
  const [formRepeatMode, setFormRepeatMode] = useState<RepeatMode>('daily');
  const [formDailyTimes, setFormDailyTimes] = useState<string[]>(['08:00']);
  const [formWeeklyDays, setFormWeeklyDays] = useState<number[]>([1, 2, 3, 4, 5]);
  const [formWeeklyTimes, setFormWeeklyTimes] = useState<string[]>(['08:00']);
  const [formIntervalHours, setFormIntervalHours] = useState(4);
  const [formIntervalStart, setFormIntervalStart] = useState('07:00');
  const [formIntervalEnd, setFormIntervalEnd] = useState('22:00');
  const [formSelectedModels, setFormSelectedModels] = useState<string[]>([]);
  const [formSelectedAccounts, setFormSelectedAccounts] = useState<string[]>([]);
  const [formCustomPrompt, setFormCustomPrompt] = useState('');
  const [formMaxOutputTokens, setFormMaxOutputTokens] = useState(0);
  const [formCrontab, setFormCrontab] = useState('');
  const [formCrontabError, setFormCrontabError] = useState('');
  const [formStartupDelayMode, setFormStartupDelayMode] = useState<'immediate' | 'delayed'>(
    'immediate',
  );
  const [formStartupDelayMinutes, setFormStartupDelayMinutes] = useState('1');
  const {
    message: formError,
    scrollKey: formErrorScrollKey,
    report: reportFormError,
    clear: clearFormError,
  } = useModalErrorState('');
  const [formTimeWindowEnabled, setFormTimeWindowEnabled] = useState(false);
  const [formTimeWindowStart, setFormTimeWindowStart] = useState('09:00');
  const [formTimeWindowEnd, setFormTimeWindowEnd] = useState('18:00');
  const [formFallbackTimes, setFormFallbackTimes] = useState<string[]>(['07:00']);
  const [customDailyTime, setCustomDailyTime] = useState('');
  const [customWeeklyTime, setCustomWeeklyTime] = useState('');
  const [customFallbackTime, setCustomFallbackTime] = useState('');
  const [testSelectedModels, setTestSelectedModels] = useState<string[]>([]);
  const [testSelectedAccounts, setTestSelectedAccounts] = useState<string[]>([]);
  const [testCustomPrompt, setTestCustomPrompt] = useState('');
  const [testMaxOutputTokens, setTestMaxOutputTokens] = useState(0);
  const [accountGroups, setAccountGroups] = useState<AccountGroup[]>([]);
  const [testAccountSearchQuery, setTestAccountSearchQuery] = useState('');
  const [testTypeFilter, setTestTypeFilter] = useState<AccountFilterType[]>([]);
  const [testTagFilter, setTestTagFilter] = useState<string[]>([]);
  const [testGroupFilter, setTestGroupFilter] = useState<string[]>([]);
  const [formAccountSearchQuery, setFormAccountSearchQuery] = useState('');
  const [formTypeFilter, setFormTypeFilter] = useState<AccountFilterType[]>([]);
  const [formTagFilter, setFormTagFilter] = useState<string[]>([]);
  const [formGroupFilter, setFormGroupFilter] = useState<string[]>([]);
  const [officialLsVersionMode, setOfficialLsVersionMode] = useState<WakeupOfficialLsVersionMode>(
    () => loadWakeupOfficialLsVersionMode(),
  );

  const tasksRef = useRef(tasks);
  const wakeupEnabledRef = useRef(wakeupEnabled);
  const activeTestRunTokenRef = useRef(0);
  const activeTestScopeIdRef = useRef<string | null>(null);
  const testAccountSelectAllRef = useRef<HTMLInputElement | null>(null);
  const formAccountSelectAllRef = useRef<HTMLInputElement | null>(null);
  const accountEmails = useMemo(() => accounts.map((account) => account.email), [accounts]);
  const accountByEmail = useMemo(() => {
    const map = new Map<string, (typeof accounts)[number]>();
    accounts.forEach((account) => {
      map.set(account.email.toLowerCase(), account);
    });
    return map;
  }, [accounts]);
  const accountById = useMemo(() => {
    const map = new Map<string, (typeof accounts)[number]>();
    accounts.forEach((account) => {
      map.set(account.id, account);
    });
    return map;
  }, [accounts]);
  const availableFilterTags = useMemo(() => collectAvailableAccountTags(accounts), [accounts]);
  const availableFilterTagOptions = useMemo<MultiSelectFilterOption[]>(
    () =>
      availableFilterTags.map((tag) => ({
        value: tag,
        label: tag,
      })),
    [availableFilterTags],
  );
  const tierCounts = useMemo(
    () => buildAccountTierCounts(accounts, {}),
    [accounts],
  );
  const typeFilterOptions = useMemo<MultiSelectFilterOption[]>(
    () => buildAccountTierFilterOptions(t, tierCounts),
    [t, tierCounts],
  );
  const groupIdsByAccountId = useMemo(() => {
    const map = new Map<string, string[]>();
    accountGroups.forEach((group) => {
      group.accountIds.forEach((accountId) => {
        const current = map.get(accountId);
        if (!current) {
          map.set(accountId, [group.id]);
          return;
        }
        current.push(group.id);
      });
    });
    return map;
  }, [accountGroups]);
  const accountIdsInAnyGroup = useMemo(() => {
    const ids = new Set<string>();
    accountGroups.forEach((group) => {
      group.accountIds.forEach((accountId) => ids.add(accountId));
    });
    return ids;
  }, [accountGroups]);
  const groupFilterOptions = useMemo<MultiSelectFilterOption[]>(() => {
    const groupOptions = accountGroups
      .map((group) => {
        const count = group.accountIds.filter((id) => accountById.has(id)).length;
        return {
          value: group.id,
          label: `${group.name} (${count})`,
        };
      })
      .sort((left, right) => left.label.localeCompare(right.label));
    const ungroupedCount = accounts.reduce(
      (count, account) => (accountIdsInAnyGroup.has(account.id) ? count : count + 1),
      0,
    );
    return [
      ...groupOptions,
      {
        value: UNGROUPED_ACCOUNT_GROUP_FILTER_KEY,
        label: `${t('accounts.groups.ungrouped')} (${ungroupedCount})`,
      },
    ];
  }, [accountById, accountGroups, accountIdsInAnyGroup, accounts, t]);

  const activeAccountEmail = currentAccount?.email || accountEmails[0] || '';
  const filteredTestAccounts = useMemo(() => {
    const query = testAccountSearchQuery.trim().toLowerCase();
    const selectedTypes = new Set<AccountFilterType>(testTypeFilter);
    const selectedTags = new Set(testTagFilter.map(normalizeAccountTag));
    const selectedGroups = new Set(testGroupFilter);

    return accounts
      .filter((account) => {
        const email = (account.email || '').toLowerCase();
        if (query && !email.includes(query)) {
          return false;
        }
        if (!accountMatchesTypeFilters(account, selectedTypes, {})) {
          return false;
        }
        if (!accountMatchesTagFilters(account, selectedTags)) {
          return false;
        }
        if (selectedGroups.size > 0) {
          const groupIds = groupIdsByAccountId.get(account.id) || [];
          const matchesGrouped = groupIds.some((groupId) => selectedGroups.has(groupId));
          const matchesUngrouped =
            groupIds.length === 0 && selectedGroups.has(UNGROUPED_ACCOUNT_GROUP_FILTER_KEY);
          if (!matchesGrouped && !matchesUngrouped) {
            return false;
          }
        }
        return true;
      })
      .sort((left, right) => left.email.localeCompare(right.email));
  }, [accounts, groupIdsByAccountId, testAccountSearchQuery, testGroupFilter, testTagFilter, testTypeFilter]);
  const filteredTestAccountEmails = useMemo(
    () => filteredTestAccounts.map((account) => account.email),
    [filteredTestAccounts],
  );
  const testSelectedAccountSet = useMemo(() => new Set(testSelectedAccounts), [testSelectedAccounts]);
  const testSelectedVisibleAccountsCount = useMemo(
    () =>
      filteredTestAccountEmails.reduce(
        (count, email) => (testSelectedAccountSet.has(email) ? count + 1 : count),
        0,
      ),
    [filteredTestAccountEmails, testSelectedAccountSet],
  );
  const allVisibleTestAccountsSelected = useMemo(
    () =>
      filteredTestAccountEmails.length > 0
      && testSelectedVisibleAccountsCount === filteredTestAccountEmails.length,
    [filteredTestAccountEmails.length, testSelectedVisibleAccountsCount],
  );
  const partiallyVisibleTestAccountsSelected = useMemo(
    () =>
      testSelectedVisibleAccountsCount > 0
      && testSelectedVisibleAccountsCount < filteredTestAccountEmails.length,
    [filteredTestAccountEmails.length, testSelectedVisibleAccountsCount],
  );

  const filteredFormAccounts = useMemo(() => {
    const query = formAccountSearchQuery.trim().toLowerCase();
    const selectedTypes = new Set<AccountFilterType>(formTypeFilter);
    const selectedTags = new Set(formTagFilter.map(normalizeAccountTag));
    const selectedGroups = new Set(formGroupFilter);

    return accounts
      .filter((account) => {
        const email = (account.email || '').toLowerCase();
        if (query && !email.includes(query)) {
          return false;
        }
        if (!accountMatchesTypeFilters(account, selectedTypes, {})) {
          return false;
        }
        if (!accountMatchesTagFilters(account, selectedTags)) {
          return false;
        }
        if (selectedGroups.size > 0) {
          const groupIds = groupIdsByAccountId.get(account.id) || [];
          const matchesGrouped = groupIds.some((groupId) => selectedGroups.has(groupId));
          const matchesUngrouped =
            groupIds.length === 0 && selectedGroups.has(UNGROUPED_ACCOUNT_GROUP_FILTER_KEY);
          if (!matchesGrouped && !matchesUngrouped) {
            return false;
          }
        }
        return true;
      })
      .sort((left, right) => left.email.localeCompare(right.email));
  }, [accounts, formAccountSearchQuery, formGroupFilter, formTagFilter, formTypeFilter, groupIdsByAccountId]);
  const filteredFormAccountEmails = useMemo(
    () => filteredFormAccounts.map((account) => account.email),
    [filteredFormAccounts],
  );
  const formSelectedAccountSet = useMemo(() => new Set(formSelectedAccounts), [formSelectedAccounts]);
  const formSelectedVisibleAccountsCount = useMemo(
    () =>
      filteredFormAccountEmails.reduce(
        (count, email) => (formSelectedAccountSet.has(email) ? count + 1 : count),
        0,
      ),
    [filteredFormAccountEmails, formSelectedAccountSet],
  );
  const allVisibleFormAccountsSelected = useMemo(
    () =>
      filteredFormAccountEmails.length > 0
      && formSelectedVisibleAccountsCount === filteredFormAccountEmails.length,
    [filteredFormAccountEmails.length, formSelectedVisibleAccountsCount],
  );
  const partiallyVisibleFormAccountsSelected = useMemo(
    () =>
      formSelectedVisibleAccountsCount > 0
      && formSelectedVisibleAccountsCount < filteredFormAccountEmails.length,
    [filteredFormAccountEmails.length, formSelectedVisibleAccountsCount],
  );

  const quotaModelKeys = useMemo(() => collectAntigravityQuotaModelKeys(accounts), [accounts]);
  const filteredModels = useMemo(
    () => filterAvailableModels(availableModels, quotaModelKeys),
    [availableModels, quotaModelKeys],
  );

  const cancelImmediateTest = useCallback(() => {
    const scopeId = activeTestScopeIdRef.current;
    activeTestRunTokenRef.current = 0;
    activeTestScopeIdRef.current = null;
    clearTestModalError();
    setTesting(false);
    setShowTestModal(false);
    setNotice({ text: t('wakeup.notice.testCancelled'), tone: 'warning' });
    if (scopeId) {
      invoke('wakeup_cancel_scope', { cancelScopeId: scopeId }).catch((error) => {
        console.error('取消唤醒测试失败:', error);
      });
    }
  }, [clearTestModalError, t]);

  const closeTestModal = useCallback(() => {
    if (testing) {
      cancelImmediateTest();
      return;
    }
    clearTestModalError();
    setShowTestModal(false);
  }, [cancelImmediateTest, clearTestModalError, testing]);

  useEscClose(showTestModal, closeTestModal);
  useEscClose(showHistoryModal, () => setShowHistoryModal(false));
  useEscClose(showModal, () => setShowModal(false));
  const modelById = useMemo(() => {
    const map = new Map<string, AvailableModel>();
    filteredModels.forEach((model) => map.set(model.id, model));
    return map;
  }, [filteredModels]);
  const modelConstantById = useMemo(() => {
    const map = new Map<string, string>();
    filteredModels.forEach((model) => {
      map.set(model.id, model.modelConstant || model.id);
    });
    return map;
  }, [filteredModels]);
  const modelConstantRef = useRef(modelConstantById);
  const maskAccountText = useCallback(
    (value?: string | null) => maskSensitiveValue(value, privacyModeEnabled),
    [privacyModeEnabled],
  );
  const handleOfficialLsVersionModeChange = useCallback((value: string) => {
    const nextMode =
      value === 'lt_1_21_6'
        ? 'lt_1_21_6'
        : 'gte_1_21_6';
    setOfficialLsVersionMode(nextMode);
    saveWakeupOfficialLsVersionMode(nextMode);
  }, []);

  useEffect(() => {
    tasksRef.current = tasks;
  }, [tasks]);

  useEffect(() => {
    modelConstantRef.current = modelConstantById;
  }, [modelConstantById]);

  useEffect(() => {
    wakeupEnabledRef.current = wakeupEnabled;
  }, [wakeupEnabled]);

  useEffect(() => {
    fetchAccounts();
    fetchCurrentAccount();
  }, [fetchAccounts, fetchCurrentAccount]);

  useEffect(() => {
    const syncMode = () => {
      setOfficialLsVersionMode(loadWakeupOfficialLsVersionMode());
    };
    const handleModeChanged = (event: Event) => {
      const detail = (event as CustomEvent<WakeupOfficialLsVersionMode>).detail;
      if (detail === 'lt_1_21_6' || detail === 'gte_1_21_6') {
        setOfficialLsVersionMode(detail);
        return;
      }
      syncMode();
    };

    window.addEventListener(
      WAKEUP_OFFICIAL_LS_VERSION_CHANGED_EVENT,
      handleModeChanged as EventListener,
    );
    window.addEventListener('focus', syncMode);
    return () => {
      window.removeEventListener(
        WAKEUP_OFFICIAL_LS_VERSION_CHANGED_EVENT,
        handleModeChanged as EventListener,
      );
      window.removeEventListener('focus', syncMode);
    };
  }, []);

  useEffect(() => {
    if (!showModal && !showTestModal) return;
    let active = true;
    const loadAccountGroups = async () => {
      try {
        const groups = await getAccountGroups();
        if (!active) return;
        setAccountGroups(groups || []);
      } catch (error) {
        console.error('加载账号分组失败:', error);
        if (!active) return;
        setAccountGroups([]);
      }
    };
    void loadAccountGroups();
    return () => {
      active = false;
    };
  }, [showModal, showTestModal]);

  useEffect(() => {
    localStorage.setItem(TASKS_STORAGE_KEY, JSON.stringify(tasks));
    // 触发事件通知设置页面
    window.dispatchEvent(new Event('wakeup-tasks-updated'));
  }, [tasks]);

  useEffect(() => {
    localStorage.setItem(WAKEUP_ENABLED_KEY, String(wakeupEnabled));
  }, [wakeupEnabled]);

  // 唤醒历史现在由后端存储，组件加载时异步加载
  useEffect(() => {
    loadHistory().then(setHistoryRecords);
  }, []);

  useEffect(() => {
    invoke('set_wakeup_override', { enabled: wakeupEnabled }).catch((error) => {
      console.error('唤醒互斥通知失败:', error);
    });
  }, [wakeupEnabled]);

  useEffect(() => {
    invoke('wakeup_set_official_ls_version_mode', { mode: officialLsVersionMode }).catch((error) => {
      console.error('同步官方 LS 版本模式失败:', error);
    });
  }, [officialLsVersionMode]);

  useEffect(() => {
    const validGroupIds = new Set(accountGroups.map((group) => group.id));
    setTestGroupFilter((prev) =>
      prev.filter((value) => value === UNGROUPED_ACCOUNT_GROUP_FILTER_KEY || validGroupIds.has(value)),
    );
    setFormGroupFilter((prev) =>
      prev.filter((value) => value === UNGROUPED_ACCOUNT_GROUP_FILTER_KEY || validGroupIds.has(value)),
    );
  }, [accountGroups]);

  useEffect(() => {
    const syncPrivacyMode = () => {
      setPrivacyModeEnabled(isPrivacyModeEnabledByDefault());
    };

    const handlePrivacyModeChanged = (event: Event) => {
      const detail = (event as CustomEvent<boolean>).detail;
      if (typeof detail === 'boolean') {
        setPrivacyModeEnabled(detail);
      } else {
        syncPrivacyMode();
      }
    };

    window.addEventListener(PRIVACY_MODE_CHANGED_EVENT, handlePrivacyModeChanged as EventListener);
    window.addEventListener('focus', syncPrivacyMode);
    return () => {
      window.removeEventListener(PRIVACY_MODE_CHANGED_EVENT, handlePrivacyModeChanged as EventListener);
      window.removeEventListener('focus', syncPrivacyMode);
    };
  }, []);

  useEffect(() => {
    invoke('wakeup_sync_state', {
      enabled: wakeupEnabled,
      tasks,
      officialLsVersionMode,
    }).catch((error) => {
      console.error('[WakeupTasks] 同步唤醒任务状态失败:', error);
    });
  }, [officialLsVersionMode, tasks, wakeupEnabled]);

  useEffect(() => {
    const handleTaskResult = (event: Event) => {
      const custom = event as CustomEvent<WakeupTaskResultPayload>;
      if (!custom.detail) return;
      const { taskId, lastRunAt, records } = custom.detail;
      setTasks((prev) => prev.map((task) => (task.id === taskId ? { ...task, lastRunAt } : task)));
      if (records?.length) {
        loadHistory().then(setHistoryRecords);
      }
    };

    window.addEventListener('wakeup-task-result', handleTaskResult as EventListener);
    return () => {
      window.removeEventListener('wakeup-task-result', handleTaskResult as EventListener);
    };
  }, []);

  useEffect(() => {
    const loadModels = async () => {
      if (accounts.length === 0) {
        setAvailableModels([]);
        setModelsLoading(false);
        return;
      }
      setModelsLoading(true);
      try {
        const models = await invoke<AvailableModel[]>('fetch_available_models');
        const filtered = filterAvailableModels(models || [], quotaModelKeys);
        if (filtered.length > 0) {
          setAvailableModels(filtered);
        } else {
          setNotice({ text: t('wakeup.notice.modelsFetchFailed'), tone: 'warning' });
          setAvailableModels([]);
        }
      } catch (error) {
        console.error('获取模型列表失败:', error);
        setNotice({ text: t('wakeup.notice.modelsFetchFailed'), tone: 'warning' });
        setAvailableModels([]);
      } finally {
        setModelsLoading(false);
      }
    };
    loadModels();
  }, [accounts, currentAccount?.id, quotaModelKeys, t]);

  useEffect(() => {
    if (tasks.length === 0) return;
    if (accountEmails.length === 0 && filteredModels.length === 0) return;

    let changed = false;
    const modelIds = filteredModels.map((model) => model.id);
    const nextTasks = tasks.map((task) => {
      let nextSchedule = normalizeSchedule({ ...DEFAULT_SCHEDULE, ...task.schedule });
      if (accountEmails.length > 0) {
        const nextAccounts = nextSchedule.selectedAccounts.filter((email) =>
          accountEmails.includes(email)
        );
        if (nextAccounts.length === 0) {
          nextAccounts.push(accountEmails[0]);
        }
        if (nextAccounts.join('|') !== nextSchedule.selectedAccounts.join('|')) {
          nextSchedule = { ...nextSchedule, selectedAccounts: nextAccounts };
          changed = true;
        }
      }

      if (modelIds.length > 0) {
        const nextModels = nextSchedule.selectedModels.filter((id) => modelIds.includes(id));
        if (nextModels.length === 0) {
          nextModels.push(modelIds[0]);
        }
        if (nextModels.join('|') !== nextSchedule.selectedModels.join('|')) {
          nextSchedule = { ...nextSchedule, selectedModels: nextModels };
          changed = true;
        }
      }

      if (nextSchedule !== task.schedule) {
        return { ...task, schedule: nextSchedule };
      }
      return task;
    });

    if (changed) {
      setTasks(nextTasks);
    }
  }, [tasks, accountEmails, filteredModels]);

  useEffect(() => {
    if (filteredModels.length === 0) {
      setTestSelectedModels([]);
      return;
    }
    setTestSelectedModels((prev) => {
      const next = prev.filter((id) => filteredModels.some((model) => model.id === id));
      if (next.length === 0) {
        return [filteredModels[0].id];
      }
      return next;
    });
  }, [filteredModels]);

  useEffect(() => {
    if (accountEmails.length === 0) {
      setTestSelectedAccounts([]);
      return;
    }
    setTestSelectedAccounts((prev) => {
      const next = prev.filter((email) => accountEmails.includes(email));
      if (next.length === 0) {
        return [activeAccountEmail || accountEmails[0]];
      }
      return next;
    });
  }, [accountEmails, activeAccountEmail]);

  useEffect(() => {
    if (!testAccountSelectAllRef.current) return;
    testAccountSelectAllRef.current.indeterminate = partiallyVisibleTestAccountsSelected;
  }, [partiallyVisibleTestAccountsSelected]);

  useEffect(() => {
    if (!formAccountSelectAllRef.current) return;
    formAccountSelectAllRef.current.indeterminate = partiallyVisibleFormAccountsSelected;
  }, [partiallyVisibleFormAccountsSelected]);

  function appendHistoryRecords(records: WakeupHistoryRecord[]) {
    if (records.length === 0) return;
    setHistoryRecords((prev) => {
      const next = [...records, ...prev];
      next.sort((a, b) => b.timestamp - a.timestamp);
      return next.slice(0, MAX_HISTORY_ITEMS);
    });
  }

  const clearHistoryRecords = async () => {
    try {
      await invoke('wakeup_clear_history');
      setHistoryRecords([]);
    } catch (error) {
      console.error('清空唤醒历史失败:', error);
    }
  };

  const getHistoryModelLabel = (modelId: string) =>
    modelById.get(modelId)?.displayName || getReadableModelLabel(modelId) || modelId;

  const resolveAccounts = (emails: string[]) =>
    emails
      .map((email) => accountByEmail.get(email.toLowerCase()))
      .filter((account): account is (typeof accounts)[number] => Boolean(account));

  const copyWakeupErrorText = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setNotice({ text: t('wakeup.errorUi.copySuccess'), tone: 'success' });
    } catch (error) {
      console.error('复制唤醒错误信息失败:', error);
      setNotice({ text: t('wakeup.errorUi.copyFailed'), tone: 'error' });
    }
  };

  const openWakeupErrorUrl = async (url: string) => {
    try {
      await openUrl(url);
    } catch (error) {
      console.error('打开验证链接失败:', error);
      setNotice({ text: t('wakeup.errorUi.openFailed'), tone: 'error' });
      window.open(url, '_blank', 'noopener,noreferrer');
    }
  };

  const ensureWakeupRuntimeReady = async (options?: { reportToTestModal?: boolean }): Promise<boolean> => {
    try {
      await invoke('wakeup_ensure_runtime_ready', { officialLsVersionMode });
      return true;
    } catch (error) {
      const message = formatErrorMessage(error);
      if (isAntigravityPathMissingError(message)) {
        window.dispatchEvent(
          new CustomEvent('app-path-missing', {
            detail: { app: 'antigravity', retry: { kind: 'default' } },
          }),
        );
        const pathErrorText = t('appPath.modal.desc', { app: 'Antigravity' });
        if (options?.reportToTestModal) {
          reportTestModalError(pathErrorText);
        } else {
          setNotice({ text: pathErrorText, tone: 'warning' });
        }
        return false;
      }
      if (options?.reportToTestModal) {
        reportTestModalError(message);
      } else {
        setNotice({ text: message, tone: 'error' });
      }
      return false;
    }
  };

  const buildWakeupDebugText = (payload: WakeupStructuredErrorPayload, record: WakeupHistoryRecord) => {
    const lines: string[] = [];
    if (payload.trajectoryId) lines.push(`Trajectory ID: ${payload.trajectoryId}`);
    if (typeof payload.errorCode === 'number') lines.push(`Error Code: ${payload.errorCode}`);
    if (payload.message) lines.push(`Message: ${payload.message}`);
    if (record.accountEmail) lines.push(`Account: ${maskAccountText(record.accountEmail)}`);
    if (record.modelId) lines.push(`Model: ${record.modelId}`);
    if (record.prompt) lines.push(`Prompt: ${record.prompt}`);
    if (payload.validationUrl) lines.push(`Validation URL: ${payload.validationUrl}`);
    if (payload.errorMessageJson) lines.push(`Error JSON: ${payload.errorMessageJson}`);
    if (payload.stepJson) lines.push(`Step JSON: ${payload.stepJson}`);
    return lines.join('\n');
  };

  const renderWakeupHistoryMessage = (record: WakeupHistoryRecord) => {
    const rawMessage = record.message || '';
    const payload = parseWakeupStructuredError(rawMessage);
    const plainText = getWakeupErrorDisplayText(rawMessage);
    if (!payload || record.success) {
      return plainText;
    }

    const kind = payload.kind || 'generic';
    const titleKey =
      kind === 'verification_required'
        ? 'wakeup.errorUi.verificationRequiredTitle'
        : kind === 'quota'
          ? 'wakeup.errorUi.quotaTitle'
          : kind === 'temporary'
            ? 'wakeup.errorUi.temporaryTitle'
            : 'wakeup.errorUi.genericTitle';
    const summaryText =
      kind === 'verification_required'
        ? t('wakeup.errorUi.errorCode', {
            code: typeof payload.errorCode === 'number' ? payload.errorCode : 403,
          })
        : plainText;
    const shouldShowErrorCodeMeta = typeof payload.errorCode === 'number' && kind !== 'verification_required';
    const shouldShowTrajectoryMeta = Boolean(payload.trajectoryId);

    return (
      <div className={`wakeup-error-panel is-${kind}`}>
        <div className="wakeup-error-title">{t(titleKey)}</div>
        <div className="wakeup-error-text">{summaryText}</div>
        {shouldShowErrorCodeMeta || shouldShowTrajectoryMeta ? (
          <div className="wakeup-error-meta">
            {shouldShowErrorCodeMeta && (
              <span>{t('wakeup.errorUi.errorCode', { code: payload.errorCode })}</span>
            )}
            {shouldShowTrajectoryMeta && (
              <span>{t('wakeup.errorUi.trajectoryId', { id: payload.trajectoryId })}</span>
            )}
          </div>
        ) : null}
        {payload.validationUrl ? (
          <div className="wakeup-error-link-box">
            <div className="wakeup-error-link-label">{t('wakeup.errorUi.validationUrlLabel')}</div>
            <div className="wakeup-error-link-value">{payload.validationUrl}</div>
          </div>
        ) : null}
        <div className="wakeup-error-actions">
          {payload.validationUrl ? (
            <>
              <button
                type="button"
                className="btn btn-primary wakeup-error-btn"
                onClick={() => openWakeupErrorUrl(payload.validationUrl!)}
              >
                {t('wakeup.errorUi.completeVerification')}
              </button>
              <button
                type="button"
                className="btn btn-secondary wakeup-error-btn"
                onClick={() => copyWakeupErrorText(payload.validationUrl!)}
              >
                {t('wakeup.errorUi.copyValidationUrl')}
              </button>
            </>
          ) : null}
          <button
            type="button"
            className="btn btn-secondary wakeup-error-btn"
            onClick={() => copyWakeupErrorText(buildWakeupDebugText(payload, record))}
          >
            {t('wakeup.errorUi.copyDebugInfo')}
          </button>
        </div>
      </div>
    );
  };

  const runImmediateTest = async () => {
    if (testing) return;
    clearTestModalError();
    const models = testSelectedModels;
    if (models.length === 0) {
      reportTestModalError(t('wakeup.notice.testMissingModel'));
      return;
    }
    const selectedAccounts = resolveAccounts(testSelectedAccounts);
    if (selectedAccounts.length === 0) {
      reportTestModalError(t('wakeup.notice.testMissingAccount'));
      return;
    }

    const runtimeReady = await ensureWakeupRuntimeReady({ reportToTestModal: true });
    if (!runtimeReady) {
      return;
    }

    const runToken = activeTestRunTokenRef.current + 1;
    const cancelScopeId = buildWakeupTestScopeId();
    activeTestRunTokenRef.current = runToken;
    activeTestScopeIdRef.current = cancelScopeId;
    setTesting(true);
    const trimmedPrompt = testCustomPrompt && testCustomPrompt.trim()
      ? testCustomPrompt.trim()
      : undefined;
    const promptText = trimmedPrompt || DEFAULT_PROMPT;
    const fallbackTokens =
      tasksRef.current.find((task) => task.enabled)?.schedule.maxOutputTokens ?? 0;
    const resolvedMaxTokens = normalizeMaxOutputTokens(testMaxOutputTokens, fallbackTokens);
    const actions: {
      promise: Promise<WakeupInvokeResult>;
      accountEmail: string;
      modelId: string;
      startedAt: number;
    }[] = [];
    selectedAccounts.forEach((account) => {
      models.forEach((model) => {
        actions.push({
          accountEmail: account.email,
          modelId: model,
          startedAt: Date.now(),
          promise: invoke<WakeupInvokeResult>('trigger_wakeup', {
            accountId: account.id,
            model,
            prompt: trimmedPrompt,
            maxOutputTokens: resolvedMaxTokens,
            cancelScopeId,
            officialLsVersionMode,
          }),
        });
      });
    });

    try {
      const results = await Promise.allSettled(actions.map((action) => action.promise));
      if (activeTestRunTokenRef.current !== runToken) {
        return;
      }

      const failed = results.filter((res) => res.status === 'rejected');
      const timestamp = Date.now();
      const historyItems = results.map((result, index) => {
        const action = actions[index];
        let duration = Date.now() - action.startedAt;
        let message: string | undefined;
        if (result.status === 'fulfilled') {
          const value = result.value;
          if (typeof value.durationMs === 'number') {
            duration = value.durationMs;
          }
          message = formatWakeupMessage(action.modelId, value, duration, t);
        } else {
          message = formatErrorMessage(result.reason);
        }
        return {
          id: crypto.randomUUID ? crypto.randomUUID() : `${timestamp}-${index}`,
          timestamp,
          triggerType: 'manual' as HistoryTriggerType,
          triggerSource: 'manual' as HistoryTriggerSource,
          taskName: '',
          accountEmail: action.accountEmail,
          modelId: action.modelId,
          prompt: promptText,
          success: result.status === 'fulfilled',
          message,
          duration,
        };
      });
      if (historyItems.length > 0) {
        try {
          await invoke('wakeup_add_history', { items: historyItems });
          if (activeTestRunTokenRef.current !== runToken) {
            return;
          }
          const latest = await loadHistory();
          if (activeTestRunTokenRef.current !== runToken) {
            return;
          }
          setHistoryRecords(latest);
        } catch (error) {
          console.error('写入唤醒历史失败:', error);
          if (activeTestRunTokenRef.current !== runToken) {
            return;
          }
          appendHistoryRecords(historyItems);
        }
      }
      if (activeTestRunTokenRef.current !== runToken) {
        return;
      }
      if (failed.length > 0) {
        reportTestModalError(t('wakeup.notice.testFailed', { count: failed.length }));
      } else {
        clearTestModalError();
        setShowTestModal(false);
        setNotice({ text: t('wakeup.notice.testCompleted'), tone: 'success' });
      }
    } finally {
      if (activeTestRunTokenRef.current === runToken) {
        activeTestRunTokenRef.current = 0;
        activeTestScopeIdRef.current = null;
        setTesting(false);
      }
      invoke('wakeup_release_scope', { cancelScopeId }).catch((error) => {
        console.error('释放唤醒测试取消作用域失败:', error);
      });
    }
  };

  const describeTask = (task: WakeupTask) => {
    const schedule = task.schedule;
    if (typeof schedule.startupDelayMinutes === 'number') {
      if (schedule.startupDelayMinutes <= 0) {
        return t('settings.general.startupWakeupImmediate');
      }
      return `${t('wakeup.triggerSource.startup')} +${schedule.startupDelayMinutes}${t('settings.general.minutes')}`;
    }
    if (schedule.wakeOnReset) {
      return t('wakeup.format.quotaReset');
    }
    if (schedule.crontab) {
      return t('wakeup.format.crontab', { expr: schedule.crontab });
    }
    if (schedule.repeatMode === 'daily') {
      const times = schedule.dailyTimes.slice(0, 3).join(', ');
      const suffix = schedule.dailyTimes.length > 3 ? '...' : '';
      return t('wakeup.format.daily', { times, suffix });
    }
    if (schedule.repeatMode === 'weekly') {
      const dayLabels = schedule.weeklyDays.map((day) => {
        const key = WEEKDAY_KEYS[day] || WEEKDAY_KEYS[0];
        return t(`wakeup.weekdays.${key}`);
      });
      const days = dayLabels.join(', ');
      const times = schedule.weeklyTimes.slice(0, 3).join(', ');
      const suffix = schedule.weeklyTimes.length > 3 ? '...' : '';
      return t('wakeup.format.weekly', { days, times, suffix });
    }
    return t('wakeup.format.interval', {
      hours: schedule.intervalHours || 4,
      start: schedule.intervalStartTime,
      end: schedule.intervalEndTime,
    });
  };

  const getNextRunLabel = (task: WakeupTask) => {
    const mode = getTriggerMode(task);
    if (mode === 'startup') {
      const delayMinutes = normalizeStartupDelayMinutes(task.schedule.startupDelayMinutes) ?? 0;
      if (delayMinutes <= 0) {
        return t('settings.general.startupWakeupImmediate');
      }
      return `${t('wakeup.triggerSource.startup')} +${delayMinutes}${t('settings.general.minutes')}`;
    }
    if (mode === 'quota_reset') return t('wakeup.format.none');
    if (mode === 'crontab') {
      const nextRuns = calculateCrontabNextRuns(task.schedule.crontab || '', 1);
      if (!task.schedule.crontab) return t('wakeup.format.none');
      if (nextRuns.length === 0) return t('wakeup.format.invalidCrontab');
      return formatRunTime(nextRuns[0], locale, t);
    }
    const nextRuns = calculateNextRuns(task.schedule, 1);
    if (!nextRuns.length) return t('wakeup.format.none');
    return formatRunTime(nextRuns[0], locale, t);
  };

  const openCreateModal = async () => {
    // 先检查路径是否已配置
    const runtimeReady = await ensureWakeupRuntimeReady();
    if (!runtimeReady) return;

    setEditingTaskId(null);
    setFormName(t('wakeup.newTaskName'));
    setFormEnabled(true);
    setFormTriggerMode('scheduled');
    setFormRepeatMode('daily');
    setFormDailyTimes(['08:00']);
    setFormWeeklyDays([1, 2, 3, 4, 5]);
    setFormWeeklyTimes(['08:00']);
    setFormIntervalHours(4);
    setFormIntervalStart('07:00');
    setFormIntervalEnd('22:00');
    setFormSelectedModels(filteredModels.length ? [filteredModels[0].id] : []);
    setFormSelectedAccounts(accountEmails.length ? [accountEmails[0]] : []);
    setFormAccountSearchQuery('');
    setFormTypeFilter([]);
    setFormTagFilter([]);
    setFormGroupFilter([]);
    setFormCustomPrompt('');
    setFormMaxOutputTokens(0);
    setFormCrontab('');
    setFormCrontabError('');
    setFormStartupDelayMode('immediate');
    setFormStartupDelayMinutes('1');
    setFormTimeWindowEnabled(false);
    setFormTimeWindowStart('09:00');
    setFormTimeWindowEnd('18:00');
    setFormFallbackTimes(['07:00']);
    setCustomDailyTime('');
    setCustomWeeklyTime('');
    setCustomFallbackTime('');
    clearFormError();
    setShowModal(true);
  };

  const openEditModal = (task: WakeupTask) => {
    const schedule = normalizeSchedule({ ...DEFAULT_SCHEDULE, ...task.schedule });
    const triggerMode = getTriggerMode(task);

    setEditingTaskId(task.id);
    setFormName(task.name);
    setFormEnabled(task.enabled);
    setFormTriggerMode(triggerMode);
    setFormRepeatMode(schedule.repeatMode);
    setFormDailyTimes([...schedule.dailyTimes]);
    setFormWeeklyDays([...schedule.weeklyDays]);
    setFormWeeklyTimes([...schedule.weeklyTimes]);
    setFormIntervalHours(schedule.intervalHours || 4);
    setFormIntervalStart(schedule.intervalStartTime || '07:00');
    setFormIntervalEnd(schedule.intervalEndTime || '22:00');
    setFormSelectedModels(
      schedule.selectedModels.length ? schedule.selectedModels.filter((id) => modelById.has(id)) : []
    );
    setFormSelectedAccounts(
      schedule.selectedAccounts.length ? schedule.selectedAccounts.filter((email) => accountEmails.includes(email)) : []
    );
    setFormAccountSearchQuery('');
    setFormTypeFilter([]);
    setFormTagFilter([]);
    setFormGroupFilter([]);
    setFormCustomPrompt(schedule.customPrompt || '');
    setFormMaxOutputTokens(schedule.maxOutputTokens ?? 0);
    setFormCrontab(schedule.crontab || '');
    setFormCrontabError('');
    const startupDelayMinutes = normalizeStartupDelayMinutes(schedule.startupDelayMinutes) ?? 0;
    setFormStartupDelayMode(startupDelayMinutes > 0 ? 'delayed' : 'immediate');
    setFormStartupDelayMinutes(String(startupDelayMinutes > 0 ? startupDelayMinutes : 1));
    setFormTimeWindowEnabled(Boolean(schedule.timeWindowEnabled));
    setFormTimeWindowStart(schedule.timeWindowStart || '09:00');
    setFormTimeWindowEnd(schedule.timeWindowEnd || '18:00');
    setFormFallbackTimes(schedule.fallbackTimes?.length ? [...schedule.fallbackTimes] : ['07:00']);
    setCustomDailyTime('');
    setCustomWeeklyTime('');
    setCustomFallbackTime('');
    clearFormError();
    setShowModal(true);
  };

  const toggleListValue = (
    list: string[],
    value: string,
    options?: { allowEmpty?: boolean }
  ) => {
    if (list.includes(value)) {
      const next = list.filter((item) => item !== value);
      if (next.length === 0 && !options?.allowEmpty) return list;
      return next;
    }
    return [...list, value];
  };

  const toggleTestTypeFilterValue = useCallback((value: string) => {
    setTestTypeFilter((prev) => {
      if (prev.includes(value as AccountFilterType)) {
        return prev.filter((item) => item !== value);
      }
      return [...prev, value as AccountFilterType];
    });
  }, []);

  const clearTestTypeFilter = useCallback(() => {
    setTestTypeFilter([]);
  }, []);

  const toggleTestTagFilterValue = useCallback((value: string) => {
    setTestTagFilter((prev) => {
      if (prev.includes(value)) {
        return prev.filter((item) => item !== value);
      }
      return [...prev, value];
    });
  }, []);

  const clearTestTagFilter = useCallback(() => {
    setTestTagFilter([]);
  }, []);

  const toggleTestGroupFilterValue = useCallback((value: string) => {
    setTestGroupFilter((prev) => {
      if (prev.includes(value)) {
        return prev.filter((item) => item !== value);
      }
      return [...prev, value];
    });
  }, []);

  const clearTestGroupFilter = useCallback(() => {
    setTestGroupFilter([]);
  }, []);

  const selectVisibleTestAccounts = useCallback(() => {
    setTestSelectedAccounts((prev) => {
      const next = new Set(prev);
      filteredTestAccountEmails.forEach((email) => next.add(email));
      return Array.from(next);
    });
  }, [filteredTestAccountEmails]);

  const clearVisibleTestAccounts = useCallback(() => {
    setTestSelectedAccounts((prev) => {
      if (prev.length === 0) return prev;
      const next = new Set(prev);
      filteredTestAccountEmails.forEach((email) => next.delete(email));
      return Array.from(next);
    });
  }, [filteredTestAccountEmails]);

  const toggleAllTestAccountsSelection = useCallback(() => {
    if (filteredTestAccountEmails.length === 0) return;
    if (allVisibleTestAccountsSelected) {
      clearVisibleTestAccounts();
      return;
    }
    selectVisibleTestAccounts();
  }, [
    allVisibleTestAccountsSelected,
    clearVisibleTestAccounts,
    filteredTestAccountEmails.length,
    selectVisibleTestAccounts,
  ]);

  const toggleFormTypeFilterValue = useCallback((value: string) => {
    setFormTypeFilter((prev) => {
      if (prev.includes(value as AccountFilterType)) {
        return prev.filter((item) => item !== value);
      }
      return [...prev, value as AccountFilterType];
    });
  }, []);

  const clearFormTypeFilter = useCallback(() => {
    setFormTypeFilter([]);
  }, []);

  const toggleFormTagFilterValue = useCallback((value: string) => {
    setFormTagFilter((prev) => {
      if (prev.includes(value)) {
        return prev.filter((item) => item !== value);
      }
      return [...prev, value];
    });
  }, []);

  const clearFormTagFilter = useCallback(() => {
    setFormTagFilter([]);
  }, []);

  const toggleFormGroupFilterValue = useCallback((value: string) => {
    setFormGroupFilter((prev) => {
      if (prev.includes(value)) {
        return prev.filter((item) => item !== value);
      }
      return [...prev, value];
    });
  }, []);

  const clearFormGroupFilter = useCallback(() => {
    setFormGroupFilter([]);
  }, []);

  const selectVisibleFormAccounts = useCallback(() => {
    setFormSelectedAccounts((prev) => {
      const next = new Set(prev);
      filteredFormAccountEmails.forEach((email) => next.add(email));
      return Array.from(next);
    });
  }, [filteredFormAccountEmails]);

  const clearVisibleFormAccounts = useCallback(() => {
    setFormSelectedAccounts((prev) => {
      if (prev.length === 0) return prev;
      const next = new Set(prev);
      filteredFormAccountEmails.forEach((email) => next.delete(email));
      return Array.from(next);
    });
  }, [filteredFormAccountEmails]);

  const toggleAllFormAccountsSelection = useCallback(() => {
    if (filteredFormAccountEmails.length === 0) return;
    if (allVisibleFormAccountsSelected) {
      clearVisibleFormAccounts();
      return;
    }
    selectVisibleFormAccounts();
  }, [
    allVisibleFormAccountsSelected,
    clearVisibleFormAccounts,
    filteredFormAccountEmails.length,
    selectVisibleFormAccounts,
  ]);

  const getPendingCustomTime = (mode: 'daily' | 'weekly' | 'fallback') => {
    if (mode === 'daily') return normalizeTimeInput(customDailyTime);
    if (mode === 'weekly') return normalizeTimeInput(customWeeklyTime);
    return normalizeTimeInput(customFallbackTime);
  };

  const hasPendingCustomTime = (mode: 'daily' | 'weekly' | 'fallback') =>
    Boolean(getPendingCustomTime(mode));

  const toggleTimeSelection = (time: string, mode: 'daily' | 'weekly' | 'fallback') => {
    if (mode === 'daily') {
      const hasPending = hasPendingCustomTime('daily');
      setFormDailyTimes((prev) => {
        if (prev.includes(time)) {
          if (prev.length <= 1 && !hasPending) return prev;
          return prev.filter((item) => item !== time).sort();
        }
        return [...prev, time].sort();
      });
      return;
    }
    if (mode === 'weekly') {
      const hasPending = hasPendingCustomTime('weekly');
      setFormWeeklyTimes((prev) => {
        if (prev.includes(time)) {
          if (prev.length <= 1 && !hasPending) return prev;
          return prev.filter((item) => item !== time).sort();
        }
        return [...prev, time].sort();
      });
      return;
    }
    const hasPending = hasPendingCustomTime('fallback');
    setFormFallbackTimes((prev) => {
      if (prev.includes(time)) {
        if (prev.length <= 1 && !hasPending) return prev;
        return prev.filter((item) => item !== time).sort();
      }
      return [...prev, time].sort();
    });
  };

  const addCustomTime = (value: string, mode: 'daily' | 'weekly' | 'fallback') => {
    const normalized = normalizeTimeInput(value);
    if (!normalized) return;
    if (mode === 'daily') {
      setFormDailyTimes((prev) => Array.from(new Set([...prev, normalized])).sort());
    } else if (mode === 'weekly') {
      setFormWeeklyTimes((prev) => Array.from(new Set([...prev, normalized])).sort());
    } else {
      setFormFallbackTimes((prev) => Array.from(new Set([...prev, normalized])).sort());
    }
  };

  const toggleDaySelection = (day: number) => {
    setFormWeeklyDays((prev) => {
      if (prev.includes(day)) {
        if (prev.length <= 1) return prev;
        return prev.filter((item) => item !== day);
      }
      return [...prev, day];
    });
  };

  const applyQuickDays = (preset: 'workdays' | 'weekend' | 'all') => {
    if (preset === 'workdays') setFormWeeklyDays([1, 2, 3, 4, 5]);
    if (preset === 'weekend') setFormWeeklyDays([0, 6]);
    if (preset === 'all') setFormWeeklyDays([0, 1, 2, 3, 4, 5, 6]);
  };

  const getEffectiveTimesForPreview = (mode: 'daily' | 'weekly') => {
    const base = mode === 'daily' ? [...formDailyTimes] : [...formWeeklyTimes];
    const pending = getPendingCustomTime(mode);
    if (pending && !base.includes(pending)) {
      base.push(pending);
    }
    return base.sort();
  };

  const normalizeMaxOutputTokens = (value?: number, fallback: number = 0) => {
    if (typeof value === 'number' && Number.isFinite(value) && value > 0) {
      return Math.floor(value);
    }
    if (typeof fallback === 'number' && Number.isFinite(fallback) && fallback >= 0) {
      return Math.floor(fallback);
    }
    return 0;
  };

  /**
   * 确保配额刷新间隔满足最小要求
   * 用于配额重置模式，确保数据足够实时
   */
  const ensureMinRefreshInterval = async (minMinutes: number) => {
    try {
      const config = await invoke<WakeupGeneralConfig>('get_general_config');
      
      // 如果刷新间隔大于最小值（或禁用），自动调整
      if (config.auto_refresh_minutes < 0 || config.auto_refresh_minutes > minMinutes) {
        const oldValue = config.auto_refresh_minutes;
        
        // 更新配置
        await invoke('save_general_config', {
          language: config.language,
          theme: config.theme,
          autoRefreshMinutes: minMinutes,
          codexAutoRefreshMinutes: config.codex_auto_refresh_minutes ?? 10,
          closeBehavior: config.close_behavior || 'ask',
          opencodeAppPath: config.opencode_app_path ?? '',
          antigravityAppPath: config.antigravity_app_path ?? '',
          codexAppPath: config.codex_app_path ?? '',
          vscodeAppPath: config.vscode_app_path ?? '',
          opencodeSyncOnSwitch: config.opencode_sync_on_switch ?? false,
          opencodeAuthOverwriteOnSwitch: config.opencode_auth_overwrite_on_switch ?? false,
          codexLaunchOnSwitch: config.codex_launch_on_switch ?? true,
        });
        
        // 触发配置更新事件（让 useAutoRefresh 重新设置定时器）
        window.dispatchEvent(new Event('config-updated'));
        
        // 通知用户
        const oldText = oldValue < 0 ? t('wakeup.refreshInterval.disabled') : `${oldValue} ${t('wakeup.refreshInterval.minutes')}`;
        const newText = `${minMinutes} ${t('wakeup.refreshInterval.minutes')}`;
        
        setNotice({
          text: t('wakeup.notice.refreshIntervalAdjusted', { old: oldText, new: newText }),
          tone: 'success',
        });
      }
    } catch (error) {
      console.error('[WakeupTasks] 调整刷新间隔失败:', error);
    }
  };

  const validateCrontabInput = async (expr: string, options?: { showSuccess?: boolean }) => {
    const showSuccess = options?.showSuccess ?? false;
    const trimmed = expr.trim();
    if (!trimmed) {
      setFormCrontabError(t('wakeup.notice.crontabRequired'));
      return false;
    }
    try {
      await invoke('wakeup_validate_crontab', { expr: trimmed });
      setFormCrontabError(showSuccess ? t('wakeup.notice.crontabValid') : '');
      return true;
    } catch (error) {
      console.error('[WakeupTasks] 校验 crontab 失败:', error);
      setFormCrontabError(t('wakeup.notice.crontabInvalid'));
      return false;
    }
  };

  const handleSaveTask = async () => {
    const name = formName.trim();
    if (!name) {
      reportFormError(t('wakeup.notice.nameRequired'));
      return;
    }
    if (formSelectedAccounts.length === 0) {
      reportFormError(t('wakeup.notice.accountRequired'));
      return;
    }
    if (formSelectedModels.length === 0) {
      reportFormError(t('wakeup.notice.modelRequired'));
      return;
    }
    if (formTriggerMode === 'crontab' && !formCrontab.trim()) {
      setFormCrontabError(t('wakeup.notice.crontabRequired'));
      return;
    }
    if (formTriggerMode === 'crontab') {
      const valid = await validateCrontabInput(formCrontab, { showSuccess: false });
      if (!valid) {
        return;
      }
    }

    const resolvedDailyTimes = [...formDailyTimes];
    const pendingDailyTime = getPendingCustomTime('daily');
    if (
      formTriggerMode === 'scheduled' &&
      formRepeatMode === 'daily' &&
      pendingDailyTime &&
      !resolvedDailyTimes.includes(pendingDailyTime)
    ) {
      resolvedDailyTimes.push(pendingDailyTime);
    }
    resolvedDailyTimes.sort();

    const resolvedWeeklyTimes = [...formWeeklyTimes];
    const pendingWeeklyTime = getPendingCustomTime('weekly');
    if (
      formTriggerMode === 'scheduled' &&
      formRepeatMode === 'weekly' &&
      pendingWeeklyTime &&
      !resolvedWeeklyTimes.includes(pendingWeeklyTime)
    ) {
      resolvedWeeklyTimes.push(pendingWeeklyTime);
    }
    resolvedWeeklyTimes.sort();

    const resolvedFallbackTimes = [...formFallbackTimes];
    const pendingFallbackTime = getPendingCustomTime('fallback');
    if (
      formTriggerMode === 'quota_reset' &&
      formTimeWindowEnabled &&
      pendingFallbackTime &&
      !resolvedFallbackTimes.includes(pendingFallbackTime)
    ) {
      resolvedFallbackTimes.push(pendingFallbackTime);
    }
    resolvedFallbackTimes.sort();

    const startupDelayMinutes =
      formTriggerMode === 'startup'
        ? formStartupDelayMode === 'delayed'
          ? Math.min(
              MAX_STARTUP_DELAY_MINUTES,
              Math.max(1, Number(formStartupDelayMinutes) || 1),
            )
          : 0
        : undefined;

    const schedule = normalizeSchedule({
      ...DEFAULT_SCHEDULE,
      repeatMode: formRepeatMode,
      dailyTimes: resolvedDailyTimes,
      weeklyDays: formWeeklyDays,
      weeklyTimes: resolvedWeeklyTimes,
      intervalHours: formIntervalHours,
      intervalStartTime: formIntervalStart,
      intervalEndTime: formIntervalEnd,
      selectedModels: formSelectedModels,
      selectedAccounts: formSelectedAccounts,
      crontab: formTriggerMode === 'crontab' ? formCrontab.trim() : undefined,
      wakeOnReset: formTriggerMode === 'quota_reset',
      startupDelayMinutes,
      customPrompt: formCustomPrompt.trim() || undefined,
      maxOutputTokens: normalizeMaxOutputTokens(formMaxOutputTokens, 0),
      timeWindowEnabled: formTriggerMode === 'quota_reset' ? formTimeWindowEnabled : false,
      timeWindowStart:
        formTriggerMode === 'quota_reset' && formTimeWindowEnabled
          ? formTimeWindowStart
          : undefined,
      timeWindowEnd:
        formTriggerMode === 'quota_reset' && formTimeWindowEnabled
          ? formTimeWindowEnd
          : undefined,
      fallbackTimes:
        formTriggerMode === 'quota_reset' && formTimeWindowEnabled
          ? resolvedFallbackTimes
          : undefined,
    });

    const now = Date.now();
    const baseTask: WakeupTask = {
      id: editingTaskId || (crypto.randomUUID ? crypto.randomUUID() : String(Date.now())),
      name,
      enabled: formEnabled,
      createdAt: editingTaskId
        ? tasksRef.current.find((task) => task.id === editingTaskId)?.createdAt || now
        : now,
      lastRunAt: editingTaskId
        ? tasksRef.current.find((task) => task.id === editingTaskId)?.lastRunAt
        : undefined,
      schedule,
    };

    setTasks((prev) => {
      const exists = prev.some((task) => task.id === baseTask.id);
      if (exists) {
        return prev.map((task) => (task.id === baseTask.id ? baseTask : task));
      }
      return [baseTask, ...prev];
    });

    // 如果启用了配额重置模式，确保刷新间隔满足最小要求
    if (formEnabled && formTriggerMode === 'quota_reset') {
      await ensureMinRefreshInterval(2);
    }

    setShowModal(false);
    setNotice({ text: t('wakeup.notice.taskSaved', { name }), tone: 'success' });
  };

  const openTestModal = async () => {
    // 先检查路径是否已配置
    const runtimeReady = await ensureWakeupRuntimeReady();
    if (!runtimeReady) return;

    setTestAccountSearchQuery('');
    setTestTypeFilter([]);
    setTestTagFilter([]);
    setTestGroupFilter([]);
    setShowTestModal(true);
  };

  const openHistoryModal = () => {
    setShowHistoryModal(true);
  };

  const handleDeleteTask = async (taskId: string) => {
    const task = tasks.find((item) => item.id === taskId);
    if (!task) return;
    const confirmed = await confirmDialog(t('wakeup.dialogs.deleteConfirm', { name: task.name }));
    if (!confirmed) return;
    setTasks((prev) => prev.filter((item) => item.id !== taskId));
  };

  const handleToggleTask = (taskId: string) => {
    setTasks((prev) =>
      prev.map((task) =>
        task.id === taskId ? { ...task, enabled: !task.enabled } : task
      )
    );
  };

  const handleToggleWakeup = async (event?: React.MouseEvent) => {
    event?.preventDefault();
    if (!wakeupEnabled) {
      setWakeupEnabled(true);
      setNotice({ text: t('wakeup.notice.featureOn') });
      return;
    }
    setWakeupEnabled(false);
    setNotice({ text: t('wakeup.notice.featureOff') });
  };

  const previewSchedule = useMemo(() => {
    if (formTriggerMode !== 'scheduled') return [];
    const config = normalizeSchedule({
      ...DEFAULT_SCHEDULE,
      repeatMode: formRepeatMode,
      dailyTimes: getEffectiveTimesForPreview('daily'),
      weeklyDays: formWeeklyDays,
      weeklyTimes: getEffectiveTimesForPreview('weekly'),
      intervalHours: formIntervalHours,
      intervalStartTime: formIntervalStart,
      intervalEndTime: formIntervalEnd,
    });
    return calculateNextRuns(config, 5);
  }, [
    formTriggerMode,
    formRepeatMode,
    formDailyTimes,
    customDailyTime,
    formWeeklyDays,
    formWeeklyTimes,
    customWeeklyTime,
    formIntervalHours,
    formIntervalStart,
    formIntervalEnd,
  ]);

  const previewCrontab = useMemo(() => {
    if (formTriggerMode !== 'crontab') return [];
    if (!formCrontab.trim()) return [];
    return calculateCrontabNextRuns(formCrontab, 5);
  }, [formTriggerMode, formCrontab]);

  const triggerSourceLabel = (source: HistoryTriggerSource) => {
    switch (source) {
      case 'scheduled':
        return t('wakeup.triggerSource.scheduled');
      case 'crontab':
        return t('wakeup.triggerSource.crontab');
      case 'quota_reset':
        return t('wakeup.triggerSource.quotaReset');
      case 'startup':
        return t('wakeup.triggerSource.startup');
      case 'manual':
        return t('wakeup.triggerSource.manual');
      default:
        return t('wakeup.triggerSource.unknown');
    }
  };

  return (
    <main className="main-content wakeup-page accounts-page">
      <OverviewTabsHeader
        active="wakeup"
        onNavigate={onNavigate}
        subtitle={t('wakeup.subtitle')}
      />
      <div className="toolbar wakeup-toolbar">
        <div className="toolbar-left">
          <div className={`wakeup-global-toggle ${wakeupEnabled ? 'is-on' : 'is-off'}`}>
            <span className="toggle-label">{t('wakeup.globalToggle')}</span>
            <span className={`pill ${wakeupEnabled ? 'pill-success' : 'pill-secondary'}`}>
              {wakeupEnabled ? t('wakeup.statusEnabled') : t('wakeup.statusDisabled')}
            </span>
            <label className="wakeup-switch" onClick={handleToggleWakeup}>
              <input type="checkbox" checked={wakeupEnabled} readOnly />
              <span className="wakeup-slider" />
            </label>
          </div>
        </div>
        <div className="toolbar-right">
          <button className="btn btn-primary" onClick={openCreateModal}>
            <Plus size={16} /> {t('wakeup.newTask')}
          </button>
          <button
            className="btn btn-secondary"
            onClick={openTestModal}
          >
            {t('wakeup.runTest')}
          </button>
          <button className="btn btn-secondary" onClick={openHistoryModal}>
            {historyRecords.length > 0
              ? t('wakeup.historyCount', { count: historyRecords.length })
              : t('wakeup.history')}
          </button>
          {accounts.length === 0 && (
            <button className="btn btn-secondary" onClick={() => onNavigate?.('overview')}>
              {t('wakeup.gotoAddAccount')}
            </button>
          )}
        </div>
      </div>

      {notice && (
        <div className={`action-message${notice.tone ? ` ${notice.tone}` : ''}`}>
          <span className="action-message-text">{notice.text}</span>
          <button className="action-message-close" onClick={() => setNotice(null)} aria-label={t('common.close')}>
            <X size={14} />
          </button>
        </div>
      )}

      {tasks.length === 0 ? (
        <div className="empty-state">
          <div className="icon">
            <Power size={40} />
          </div>
          <h3>{t('wakeup.emptyTitle')}</h3>
          <p>{t('wakeup.emptyDesc')}</p>
          <button className="btn btn-primary" onClick={openCreateModal}>
            <Plus size={18} /> {t('wakeup.newTask')}
          </button>
        </div>
      ) : (
        <div className="wakeup-task-grid">
          {tasks.map((task) => {
            const modelLabels = task.schedule.selectedModels.map(
              (id) => modelById.get(id)?.displayName || getReadableModelLabel(id)
            );
            const accountLabels = task.schedule.selectedAccounts.map((email) => maskAccountText(email));
            return (
              <div
                key={task.id}
                className={`wakeup-task-card ${task.enabled ? '' : 'is-disabled'}`}
              >
                <div className="wakeup-task-header">
                  <div className="wakeup-task-title">
                    <span>{task.name}</span>
                    {task.enabled ? (
                      <span className="pill pill-success">{t('wakeup.statusEnabled')}</span>
                    ) : (
                      <span className="pill pill-secondary">{t('wakeup.statusDisabled')}</span>
                    )}
                  </div>
                  <div className="wakeup-task-actions">
                    <button
                      className="btn btn-secondary icon-only"
                      onClick={() => openEditModal(task)}
                      title={t('wakeup.edit')}
                    >
                      <Pencil size={14} />
                    </button>
                    <button
                      className="btn btn-secondary icon-only"
                      onClick={() => handleToggleTask(task.id)}
                      title={task.enabled ? t('wakeup.statusDisabled') : t('wakeup.statusEnabled')}
                    >
                      <Power size={14} />
                    </button>
                    <button
                      className="btn btn-danger icon-only"
                      onClick={() => handleDeleteTask(task.id)}
                      title={t('common.delete')}
                    >
                      <Trash2 size={14} />
                    </button>
                  </div>
                </div>
                <div className="wakeup-task-meta">
                  <span>{describeTask(task)}</span>
                </div>
                <div className="wakeup-task-meta">
                  <span>{t('wakeup.taskCard.accountsCount', { count: task.schedule.selectedAccounts.length })}</span>
                  <span>{t('wakeup.taskCard.modelsCount', { count: task.schedule.selectedModels.length })}</span>
                </div>
                <div className="wakeup-task-meta">
                  <span>
                    {t('wakeup.taskCard.accountsLabel', {
                      preview: formatSelectionPreview(accountLabels, 2, t),
                    })}
                  </span>
                  <span>
                    {t('wakeup.taskCard.modelsLabel', {
                      preview: formatSelectionPreview(modelLabels, 2, t),
                    })}
                  </span>
                </div>
                <div className="wakeup-task-meta">
                  <span>
                    {t('wakeup.taskCard.lastRun', { time: formatDateTime(task.lastRunAt, locale, t) })}
                  </span>
                  <span>{t('wakeup.taskCard.nextRun', { time: getNextRunLabel(task) })}</span>
                </div>
              </div>
            );
          })}
        </div>
      )}

      {showTestModal && (
        <div className="modal-overlay" onClick={closeTestModal}>
          <div
            className="modal wakeup-modal wakeup-test-modal"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <button className="btn btn-secondary icon-only" onClick={closeTestModal} title={t('common.back', '返回')} aria-label={t('common.back', '返回')}><ChevronLeft size={14} /></button>
              <h2>{t('wakeup.dialogs.testTitle')}</h2>
              <button
                className="modal-close"
                onClick={closeTestModal}
                aria-label={t('common.close', '关闭')}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <div className="wakeup-form-group">
                <label>Antigravity Version</label>
                <select
                  className="wakeup-input wakeup-select"
                  value={officialLsVersionMode}
                  onChange={(event) => handleOfficialLsVersionModeChange(event.target.value)}
                >
                  <option value="gte_1_21_6">&gt;=1.21.6</option>
                  <option value="lt_1_21_6">&lt;1.21.6</option>
                </select>
              </div>
              <div className="wakeup-form-group">
                <label>{t('wakeup.test.modelsLabel')}</label>
                <div className="wakeup-chip-list">
                  {modelsLoading && <span className="wakeup-hint">{t('common.loading')}</span>}
                  {!modelsLoading && filteredModels.length === 0 && (
                    <span className="wakeup-hint">{t('wakeup.form.modelsEmpty')}</span>
                  )}
                  {!modelsLoading &&
                    filteredModels.map((model) => (
                      <button
                        key={model.id}
                        type="button"
                        className={`wakeup-chip ${testSelectedModels.includes(model.id) ? 'selected' : ''}`}
                        onClick={() =>
                          setTestSelectedModels((prev) => toggleListValue(prev, model.id, { allowEmpty: true }))
                        }
                      >
                        {model.displayName}
                      </button>
                    ))}
                </div>
              </div>
              <div className="wakeup-form-group">
                <label>{t('wakeup.test.accountsLabel')}</label>
                <p className="wakeup-hint">{t('wakeup.test.accountsHint')}</p>
                <div className="wakeup-account-selector">
                  <div className="verification-account-select-all wakeup-account-select-toolbar">
                    <label className="verification-checkbox-row verification-checkbox-row-head">
                      <input
                        ref={testAccountSelectAllRef}
                        type="checkbox"
                        className="verification-checkbox-input"
                        checked={allVisibleTestAccountsSelected}
                        disabled={filteredTestAccountEmails.length === 0}
                        onChange={toggleAllTestAccountsSelection}
                      />
                      <span className="verification-checkbox-ui" aria-hidden="true" />
                      <span className="verification-checkbox-label">{t('wakeup.verification.actions.selectAllAccounts')}</span>
                    </label>
                    <div className="verification-account-search-wrap wakeup-account-search-wrap">
                      <input
                        type="text"
                        className="verification-account-search"
                        placeholder={t('accounts.search')}
                        value={testAccountSearchQuery}
                        onChange={(event) => setTestAccountSearchQuery(event.target.value)}
                      />
                    </div>
                    <div className="verification-account-filters wakeup-account-filter-row">
                      <MultiSelectFilterDropdown
                        options={typeFilterOptions}
                        selectedValues={testTypeFilter}
                        allLabel={t('wakeup.verification.filters.typeShort')}
                        filterLabel={t('wakeup.verification.filters.typeShort')}
                        clearLabel={t('accounts.clearFilter')}
                        emptyLabel={t('common.none')}
                        ariaLabel={t('wakeup.verification.filters.typeShort')}
                        onToggleValue={toggleTestTypeFilterValue}
                        onClear={clearTestTypeFilter}
                      />
                      <MultiSelectFilterDropdown
                        options={availableFilterTagOptions}
                        selectedValues={testTagFilter}
                        allLabel={t('wakeup.verification.filters.tagsShort')}
                        filterLabel={t('wakeup.verification.filters.tagsShort')}
                        clearLabel={t('accounts.clearFilter')}
                        emptyLabel={t('accounts.noAvailableTags')}
                        ariaLabel={t('wakeup.verification.filters.tagsShort')}
                        onToggleValue={toggleTestTagFilterValue}
                        onClear={clearTestTagFilter}
                      />
                      <MultiSelectFilterDropdown
                        options={groupFilterOptions}
                        selectedValues={testGroupFilter}
                        allLabel={t('wakeup.verification.filters.groupsShort')}
                        filterLabel={t('wakeup.verification.filters.groupsShort')}
                        clearLabel={t('accounts.clearFilter')}
                        emptyLabel={t('accounts.groups.noGroups')}
                        ariaLabel={t('wakeup.verification.filters.groupsShort')}
                        onToggleValue={toggleTestGroupFilterValue}
                        onClear={clearTestGroupFilter}
                      />
                    </div>
                    <span className="verification-account-select-count wakeup-account-select-count">
                      {testSelectedVisibleAccountsCount}/{filteredTestAccountEmails.length}
                      {filteredTestAccountEmails.length !== accountEmails.length
                        ? ` · ${testSelectedAccounts.length}/${accountEmails.length}`
                        : ''}
                    </span>
                  </div>
                  <div className="verification-account-list wakeup-account-list">
                    {accountEmails.length === 0 ? (
                      <span className="wakeup-hint wakeup-account-empty">{t('wakeup.form.accountsEmpty')}</span>
                    ) : filteredTestAccounts.length === 0 ? (
                      <span className="wakeup-hint wakeup-account-empty">{t('accounts.noMatch.title')}</span>
                    ) : (
                      filteredTestAccounts.map((account) => {
                        const maskedEmail = maskAccountText(account.email);
                        const isSelected = testSelectedAccountSet.has(account.email);
                        return (
                          <label
                            key={account.id}
                            className={`verification-account-item ${isSelected ? 'selected' : ''}`}
                          >
                            <input
                              type="checkbox"
                              className="verification-checkbox-input"
                              checked={isSelected}
                              onChange={() =>
                                setTestSelectedAccounts((prev) =>
                                  toggleListValue(prev, account.email, { allowEmpty: true }),
                                )
                              }
                            />
                            <span className="verification-checkbox-ui" aria-hidden="true" />
                            <span className="verification-account-item-email" title={maskedEmail}>
                              {maskedEmail}
                            </span>
                          </label>
                        );
                      })
                    )}
                  </div>
                </div>
              </div>
              <div className="wakeup-form-group">
                <label>{t('wakeup.form.customPrompt')}</label>
                <input
                  className="wakeup-input"
                  value={testCustomPrompt}
                  onChange={(event) => setTestCustomPrompt(event.target.value)}
                  placeholder={t('wakeup.form.promptPlaceholder', { word: DEFAULT_PROMPT })}
                  maxLength={100}
                />
                <p className="wakeup-hint">{t('wakeup.form.promptHint', { word: DEFAULT_PROMPT })}</p>
              </div>
              <div className="wakeup-form-group">
                <label>{t('wakeup.form.maxTokens')}</label>
                <input
                  className="wakeup-input wakeup-input-small"
                  type="number"
                  min={0}
                  value={testMaxOutputTokens}
                  onChange={(event) => setTestMaxOutputTokens(Number(event.target.value))}
                />
                <p className="wakeup-hint">{t('wakeup.form.maxTokensHint')}</p>
              </div>
              <ModalErrorMessage
                message={testModalError}
                position="bottom"
                scrollKey={testModalErrorScrollKey}
              />
            </div>
            <div className="modal-footer">
              <button className="btn btn-secondary" onClick={closeTestModal}>
                {t('common.cancel')}
              </button>
              <button
                className="btn btn-primary"
                onClick={runImmediateTest}
                disabled={testing || filteredModels.length === 0 || accountEmails.length === 0}
              >
                {testing ? t('wakeup.test.testing') : t('wakeup.test.start')}
              </button>
            </div>
          </div>
        </div>
      )}

      {showHistoryModal && (
        <div className="modal-overlay" onClick={() => setShowHistoryModal(false)}>
          <div
            className="modal wakeup-modal wakeup-history-modal"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <button className="btn btn-secondary icon-only" onClick={() => setShowHistoryModal(false)} title={t('common.back', '返回')} aria-label={t('common.back', '返回')}><ChevronLeft size={14} /></button>
              <h2>{t('wakeup.dialogs.historyTitle')}</h2>
              <button
                className="modal-close"
                onClick={() => setShowHistoryModal(false)}
                aria-label={t('common.close', '关闭')}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              {historyRecords.length === 0 ? (
                <p className="wakeup-hint">{t('wakeup.historyEmpty')}</p>
              ) : (
                <ul className="wakeup-history-list">
                  {historyRecords.map((record) => (
                    <li
                      key={record.id}
                      className={`wakeup-history-item ${record.success ? 'is-success' : 'is-failed'}`}
                    >
                      <div className="wakeup-history-main">
                        <span className="wakeup-history-status">
                          {record.success ? t('common.success') : t('common.failed')}
                        </span>
                        <span className="wakeup-history-time">
                          {formatDateTime(record.timestamp, locale, t)}
                        </span>
                        <span className={`wakeup-history-badge ${record.triggerType}`}>
                          {triggerSourceLabel(record.triggerSource)}
                        </span>
                        {record.taskName && record.triggerSource !== 'manual' && (
                          <span className="wakeup-history-task">{record.taskName}</span>
                        )}
                      </div>
                      <div className="wakeup-history-meta">
                        <span>{getHistoryModelLabel(record.modelId)}</span>
                        <span>{maskAccountText(record.accountEmail)}</span>
                        {record.duration ? <span>{record.duration}ms</span> : null}
                      </div>
                      {record.prompt && (
                        <div className="wakeup-history-prompt">
                          {t('wakeup.historyPromptLabel', { prompt: record.prompt })}
                        </div>
                      )}
                      {record.message && (
                        <div className="wakeup-history-message">
                          {renderWakeupHistoryMessage(record)}
                        </div>
                      )}
                    </li>
                  ))}
                </ul>
              )}
            </div>
            <div className="modal-footer">
              <button className="btn btn-secondary" onClick={() => setShowHistoryModal(false)}>
                {t('common.close')}
              </button>
              <button
                className="btn btn-secondary"
                onClick={clearHistoryRecords}
                disabled={historyRecords.length === 0}
              >
                {t('wakeup.historyClear')}
              </button>
            </div>
          </div>
        </div>
      )}

      {showModal && (
        <div className="modal-overlay" onClick={() => setShowModal(false)}>
          <div className="modal modal-lg wakeup-modal" onClick={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <button className="btn btn-secondary icon-only" onClick={() => setShowModal(false)} title={t('common.back', '返回')} aria-label={t('common.back', '返回')}><ChevronLeft size={14} /></button>
              <h2>{editingTaskId ? t('wakeup.dialogs.taskTitleEdit') : t('wakeup.dialogs.taskTitleNew')}</h2>
              <button
                className="modal-close"
                onClick={() => setShowModal(false)}
                aria-label={t('common.close', '关闭')}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <div className="wakeup-form-group">
                <label>Antigravity Version</label>
                <select
                  className="wakeup-input wakeup-select"
                  value={officialLsVersionMode}
                  onChange={(event) => handleOfficialLsVersionModeChange(event.target.value)}
                >
                  <option value="gte_1_21_6">&gt;=1.21.6</option>
                  <option value="lt_1_21_6">&lt;1.21.6</option>
                </select>
              </div>
              <div className="wakeup-form-group">
                <label>{t('wakeup.form.taskName')}</label>
                <input
                  className="wakeup-input"
                  value={formName}
                  onChange={(event) => setFormName(event.target.value)}
                />
              </div>

              <div className="wakeup-form-group">
                <label>{t('wakeup.form.taskStatus')}</label>
                <div className="wakeup-toggle-group">
                  <button
                    className={`btn btn-secondary ${formEnabled ? 'is-active' : ''}`}
                    onClick={() => setFormEnabled(true)}
                  >
                    {t('common.enable')}
                  </button>
                  <button
                    className={`btn btn-secondary ${!formEnabled ? 'is-active' : ''}`}
                    onClick={() => setFormEnabled(false)}
                  >
                    {t('common.disable')}
                  </button>
                </div>
              </div>

              <div className="wakeup-form-group">
                <label>{t('wakeup.form.triggerMode')}</label>
                <p className="wakeup-hint">
                  {t('wakeup.form.triggerModeHint')}
                </p>
                <div className="wakeup-segmented">
                  <button
                    type="button"
                    className={`wakeup-segment-btn ${formTriggerMode === 'scheduled' ? 'active' : ''}`}
                    onClick={() => setFormTriggerMode('scheduled')}
                  >
                    {t('wakeup.form.modeScheduled')}
                  </button>
                  <button
                    type="button"
                    className={`wakeup-segment-btn ${formTriggerMode === 'crontab' ? 'active' : ''}`}
                    onClick={() => setFormTriggerMode('crontab')}
                  >
                    {t('wakeup.form.modeCrontab')}
                  </button>
                  <button
                    type="button"
                    className={`wakeup-segment-btn ${formTriggerMode === 'quota_reset' ? 'active' : ''}`}
                    onClick={() => setFormTriggerMode('quota_reset')}
                  >
                    {t('wakeup.form.modeQuotaReset')}
                  </button>
                  <button
                    type="button"
                    className={`wakeup-segment-btn ${formTriggerMode === 'startup' ? 'active' : ''}`}
                    onClick={() => setFormTriggerMode('startup')}
                  >
                    {t('wakeup.triggerSource.startup')}
                  </button>
                </div>
              </div>

              <div className="wakeup-form-group">
                <label>{t('wakeup.form.modelSelect')}</label>
                <p className="wakeup-hint">{t('wakeup.form.modelHint')}</p>
                <div className="wakeup-chip-list">
                  {modelsLoading && <span className="wakeup-hint">{t('common.loading')}</span>}
                  {!modelsLoading && filteredModels.length === 0 && (
                    <span className="wakeup-hint">{t('wakeup.form.modelsEmpty')}</span>
                  )}
                  {!modelsLoading &&
                    filteredModels.map((model) => (
                      <button
                        key={model.id}
                        type="button"
                        className={`wakeup-chip ${formSelectedModels.includes(model.id) ? 'selected' : ''}`}
                        onClick={() =>
                          setFormSelectedModels((prev) => toggleListValue(prev, model.id, { allowEmpty: true }))
                        }
                      >
                        {model.displayName}
                      </button>
                    ))}
                </div>
              </div>
              <div className="wakeup-form-group">
                <label>{t('wakeup.form.accountSelect')}</label>
                <p className="wakeup-hint">{t('wakeup.form.accountHint')}</p>
                <div className="wakeup-account-selector">
                  <div className="verification-account-select-all wakeup-account-select-toolbar">
                    <label className="verification-checkbox-row verification-checkbox-row-head">
                      <input
                        ref={formAccountSelectAllRef}
                        type="checkbox"
                        className="verification-checkbox-input"
                        checked={allVisibleFormAccountsSelected}
                        disabled={filteredFormAccountEmails.length === 0}
                        onChange={toggleAllFormAccountsSelection}
                      />
                      <span className="verification-checkbox-ui" aria-hidden="true" />
                      <span className="verification-checkbox-label">{t('wakeup.verification.actions.selectAllAccounts')}</span>
                    </label>
                    <div className="verification-account-search-wrap wakeup-account-search-wrap">
                      <input
                        type="text"
                        className="verification-account-search"
                        placeholder={t('accounts.search')}
                        value={formAccountSearchQuery}
                        onChange={(event) => setFormAccountSearchQuery(event.target.value)}
                      />
                    </div>
                    <div className="verification-account-filters wakeup-account-filter-row">
                      <MultiSelectFilterDropdown
                        options={typeFilterOptions}
                        selectedValues={formTypeFilter}
                        allLabel={t('wakeup.verification.filters.typeShort')}
                        filterLabel={t('wakeup.verification.filters.typeShort')}
                        clearLabel={t('accounts.clearFilter')}
                        emptyLabel={t('common.none')}
                        ariaLabel={t('wakeup.verification.filters.typeShort')}
                        onToggleValue={toggleFormTypeFilterValue}
                        onClear={clearFormTypeFilter}
                      />
                      <MultiSelectFilterDropdown
                        options={availableFilterTagOptions}
                        selectedValues={formTagFilter}
                        allLabel={t('wakeup.verification.filters.tagsShort')}
                        filterLabel={t('wakeup.verification.filters.tagsShort')}
                        clearLabel={t('accounts.clearFilter')}
                        emptyLabel={t('accounts.noAvailableTags')}
                        ariaLabel={t('wakeup.verification.filters.tagsShort')}
                        onToggleValue={toggleFormTagFilterValue}
                        onClear={clearFormTagFilter}
                      />
                      <MultiSelectFilterDropdown
                        options={groupFilterOptions}
                        selectedValues={formGroupFilter}
                        allLabel={t('wakeup.verification.filters.groupsShort')}
                        filterLabel={t('wakeup.verification.filters.groupsShort')}
                        clearLabel={t('accounts.clearFilter')}
                        emptyLabel={t('accounts.groups.noGroups')}
                        ariaLabel={t('wakeup.verification.filters.groupsShort')}
                        onToggleValue={toggleFormGroupFilterValue}
                        onClear={clearFormGroupFilter}
                      />
                    </div>
                    <span className="verification-account-select-count wakeup-account-select-count">
                      {formSelectedVisibleAccountsCount}/{filteredFormAccountEmails.length}
                      {filteredFormAccountEmails.length !== accountEmails.length
                        ? ` · ${formSelectedAccounts.length}/${accountEmails.length}`
                        : ''}
                    </span>
                  </div>
                  <div className="verification-account-list wakeup-account-list">
                    {accountEmails.length === 0 ? (
                      <span className="wakeup-hint wakeup-account-empty">{t('wakeup.form.accountsEmpty')}</span>
                    ) : filteredFormAccounts.length === 0 ? (
                      <span className="wakeup-hint wakeup-account-empty">{t('accounts.noMatch.title')}</span>
                    ) : (
                      filteredFormAccounts.map((account) => {
                        const maskedEmail = maskAccountText(account.email);
                        const isSelected = formSelectedAccountSet.has(account.email);
                        return (
                          <label
                            key={account.id}
                            className={`verification-account-item ${isSelected ? 'selected' : ''}`}
                          >
                            <input
                              type="checkbox"
                              className="verification-checkbox-input"
                              checked={isSelected}
                              onChange={() =>
                                setFormSelectedAccounts((prev) =>
                                  toggleListValue(prev, account.email, { allowEmpty: true }),
                                )
                              }
                            />
                            <span className="verification-checkbox-ui" aria-hidden="true" />
                            <span className="verification-account-item-email" title={maskedEmail}>
                              {maskedEmail}
                            </span>
                          </label>
                        );
                      })
                    )}
                  </div>
                </div>
              </div>

              <div className="wakeup-form-group">
                <label>{t('wakeup.form.customPrompt')}</label>
                <input
                  className="wakeup-input"
                  value={formCustomPrompt}
                  onChange={(event) => setFormCustomPrompt(event.target.value)}
                  placeholder={t('wakeup.form.promptPlaceholder', { word: DEFAULT_PROMPT })}
                  maxLength={100}
                />
                <p className="wakeup-hint">{t('wakeup.form.promptHint', { word: DEFAULT_PROMPT })}</p>
              </div>

              <div className="wakeup-form-group">
                <label>{t('wakeup.form.maxTokens')}</label>
                <input
                  className="wakeup-input wakeup-input-small"
                  type="number"
                  min={0}
                  value={formMaxOutputTokens}
                  onChange={(event) => setFormMaxOutputTokens(Number(event.target.value))}
                />
                <p className="wakeup-hint">{t('wakeup.form.maxTokensHint')}</p>
              </div>

              {formTriggerMode === 'scheduled' && (
                <div className="wakeup-mode-panel">
                  <div className="wakeup-form-group">
                    <label>{t('wakeup.form.repeatMode')}</label>
                    <select
                      className="wakeup-input wakeup-select"
                      value={formRepeatMode}
                      onChange={(event) => setFormRepeatMode(event.target.value as RepeatMode)}
                    >
                      <option value="daily">{t('wakeup.form.repeatDaily')}</option>
                      <option value="weekly">{t('wakeup.form.repeatWeekly')}</option>
                      <option value="interval">{t('wakeup.form.repeatInterval')}</option>
                    </select>
                  </div>

                  {formRepeatMode === 'daily' && (
                    <div className="wakeup-form-group">
                      <label>{t('wakeup.form.selectTime')}</label>
                      <div className="wakeup-chip-grid">
                        {BASE_TIME_OPTIONS.map((time) => (
                          <button
                            key={time}
                            type="button"
                            className={`wakeup-chip ${formDailyTimes.includes(time) ? 'selected' : ''}`}
                            onClick={() => toggleTimeSelection(time, 'daily')}
                          >
                            {time}
                          </button>
                        ))}
                        {formDailyTimes
                          .filter((time) => !BASE_TIME_OPTIONS.includes(time))
                          .map((time) => (
                            <button
                              key={time}
                              type="button"
                              className={`wakeup-chip selected`}
                              onClick={() => toggleTimeSelection(time, 'daily')}
                            >
                              {time}
                            </button>
                          ))}
                      </div>
                      <div className="wakeup-custom-row">
                        <span>{t('wakeup.form.customTime')}</span>
                        <input
                          className="wakeup-input wakeup-input-time wakeup-input-time-compact"
                          type="time"
                          step={60}
                          value={customDailyTime || ''}
                          onChange={(event) => setCustomDailyTime(event.target.value)}
                          onKeyDown={(event) => {
                            if (event.key !== 'Enter') return;
                            event.preventDefault();
                            addCustomTime(customDailyTime, 'daily');
                            setCustomDailyTime('');
                          }}
                        />
                        <button
                          className="btn btn-secondary"
                          onClick={() => {
                            addCustomTime(customDailyTime, 'daily');
                            setCustomDailyTime('');
                          }}
                        >
                          {t('common.add')}
                        </button>
                      </div>
                    </div>
                  )}

                  {formRepeatMode === 'weekly' && (
                    <div className="wakeup-form-group">
                      <label>{t('wakeup.form.selectWeekday')}</label>
                      <div className="wakeup-chip-grid">
                        {[1, 2, 3, 4, 5, 6, 0].map((day) => (
                          <button
                            key={day}
                            type="button"
                            className={`wakeup-chip ${formWeeklyDays.includes(day) ? 'selected' : ''}`}
                            onClick={() => toggleDaySelection(day)}
                          >
                            {t(`wakeup.weekdays.${WEEKDAY_KEYS[day]}`)}
                          </button>
                        ))}
                      </div>
                      <div className="wakeup-quick-actions">
                        <button className="btn btn-secondary" onClick={() => applyQuickDays('workdays')}>
                          {t('wakeup.form.quickWorkdays')}
                        </button>
                        <button className="btn btn-secondary" onClick={() => applyQuickDays('weekend')}>
                          {t('wakeup.form.quickWeekend')}
                        </button>
                        <button className="btn btn-secondary" onClick={() => applyQuickDays('all')}>
                          {t('wakeup.form.quickAll')}
                        </button>
                      </div>
                      <label>{t('wakeup.form.selectTime')}</label>
                      <div className="wakeup-chip-grid">
                        {BASE_TIME_OPTIONS.map((time) => (
                          <button
                            key={time}
                            type="button"
                            className={`wakeup-chip ${formWeeklyTimes.includes(time) ? 'selected' : ''}`}
                            onClick={() => toggleTimeSelection(time, 'weekly')}
                          >
                            {time}
                          </button>
                        ))}
                        {formWeeklyTimes
                          .filter((time) => !BASE_TIME_OPTIONS.includes(time))
                          .map((time) => (
                            <button
                              key={time}
                              type="button"
                              className="wakeup-chip selected"
                              onClick={() => toggleTimeSelection(time, 'weekly')}
                            >
                              {time}
                            </button>
                          ))}
                      </div>
                      <div className="wakeup-custom-row">
                        <span>{t('wakeup.form.customTime')}</span>
                        <input
                          className="wakeup-input wakeup-input-time wakeup-input-time-compact"
                          type="time"
                          step={60}
                          value={customWeeklyTime || ''}
                          onChange={(event) => setCustomWeeklyTime(event.target.value)}
                          onKeyDown={(event) => {
                            if (event.key !== 'Enter') return;
                            event.preventDefault();
                            addCustomTime(customWeeklyTime, 'weekly');
                            setCustomWeeklyTime('');
                          }}
                        />
                        <button
                          className="btn btn-secondary"
                          onClick={() => {
                            addCustomTime(customWeeklyTime, 'weekly');
                            setCustomWeeklyTime('');
                          }}
                        >
                          {t('common.add')}
                        </button>
                      </div>
                    </div>
                  )}

                  {formRepeatMode === 'interval' && (
                    <div className="wakeup-form-group">
                      <label>{t('wakeup.form.intervalSetting')}</label>
                      <div className="wakeup-inline-row">
                        <span>{t('wakeup.form.intervalEvery')}</span>
                        <input
                          className="wakeup-input wakeup-input-small"
                          type="number"
                          min={1}
                          max={12}
                          value={formIntervalHours}
                          onChange={(event) => setFormIntervalHours(Number(event.target.value))}
                        />
                        <span>{t('wakeup.form.intervalHours')}</span>
                      </div>
                      <div className="wakeup-inline-row">
                        <span>{t('wakeup.form.intervalStart')}</span>
                        <input
                          className="wakeup-input wakeup-input-time"
                          type="time"
                          value={formIntervalStart}
                          onChange={(event) => setFormIntervalStart(event.target.value)}
                        />
                        <span>{t('wakeup.form.intervalEnd')}</span>
                        <input
                          className="wakeup-input wakeup-input-time"
                          type="time"
                          value={formIntervalEnd}
                          onChange={(event) => setFormIntervalEnd(event.target.value)}
                        />
                      </div>
                    </div>
                  )}

                  <div className="wakeup-form-group">
                    <label>{t('wakeup.form.nextRuns')}</label>
                    <ul className="wakeup-preview-list">
                      {previewSchedule.length === 0 && <li>{t('wakeup.form.nextRunsEmpty')}</li>}
                      {previewSchedule.map((date, idx) => (
                        <li key={`${date.toISOString()}-${idx}`}>
                          {idx + 1}. {formatRunTime(date, locale, t)}
                        </li>
                      ))}
                    </ul>
                  </div>
                </div>
              )}

              {formTriggerMode === 'crontab' && (
                <div className="wakeup-mode-panel">
                  <div className="wakeup-form-group">
                    <label>{t('wakeup.form.crontab')}</label>
                    <div className="wakeup-cron-row">
                      <input
                        className="wakeup-input"
                        value={formCrontab}
                        onChange={(event) => {
                          setFormCrontab(event.target.value);
                          setFormCrontabError('');
                        }}
                        placeholder={t('wakeup.form.crontabPlaceholder')}
                      />
                      <button
                        className="btn btn-secondary"
                        onClick={() => {
                          void validateCrontabInput(formCrontab, { showSuccess: true });
                        }}
                      >
                        {t('wakeup.form.crontabValidate')}
                      </button>
                    </div>
                    {formCrontabError && <p className="wakeup-hint">{formCrontabError}</p>}
                  </div>
                  <div className="wakeup-form-group">
                    <label>{t('wakeup.form.nextRuns')}</label>
                    <ul className="wakeup-preview-list">
                      {previewCrontab.length === 0 && <li>{t('wakeup.form.crontabPreviewEmpty')}</li>}
                      {previewCrontab.map((date, idx) => (
                        <li key={`${date.toISOString()}-${idx}`}>
                          {idx + 1}. {formatRunTime(date, locale, t)}
                        </li>
                      ))}
                    </ul>
                  </div>
                </div>
              )}

              {formTriggerMode === 'quota_reset' && (
                <div className="wakeup-mode-panel">
                  <div className="wakeup-form-group">
                    <div className="wakeup-form-row">
                      <label>{t('wakeup.form.timeWindowEnabled')}</label>
                      <label className="wakeup-switch">
                        <input
                          type="checkbox"
                          checked={formTimeWindowEnabled}
                          onChange={(event) => setFormTimeWindowEnabled(event.target.checked)}
                        />
                        <span className="wakeup-slider" />
                      </label>
                    </div>
                    <p className="wakeup-hint">
                      {t('wakeup.form.timeWindowHint')}
                    </p>
                  </div>

                  {formTimeWindowEnabled && (
                    <div className="wakeup-form-group">
                      <label>{t('wakeup.form.timeWindow')}</label>
                      <div className="wakeup-inline-row">
                        <span>{t('wakeup.form.timeWindowFrom')}</span>
                        <input
                          className="wakeup-input wakeup-input-time"
                          type="time"
                          value={formTimeWindowStart}
                          onChange={(event) => setFormTimeWindowStart(event.target.value)}
                        />
                        <span>{t('wakeup.form.timeWindowTo')}</span>
                        <input
                          className="wakeup-input wakeup-input-time"
                          type="time"
                          value={formTimeWindowEnd}
                          onChange={(event) => setFormTimeWindowEnd(event.target.value)}
                        />
                      </div>
                      <label>{t('wakeup.form.fallbackTimes')}</label>
                      <div className="wakeup-chip-grid">
                        {['06:00', '07:00', '08:00'].map((time) => (
                          <button
                            key={time}
                            type="button"
                            className={`wakeup-chip ${formFallbackTimes.includes(time) ? 'selected' : ''}`}
                            onClick={() => toggleTimeSelection(time, 'fallback')}
                          >
                            {time}
                          </button>
                        ))}
                        {formFallbackTimes
                          .filter((time) => !['06:00', '07:00', '08:00'].includes(time))
                          .map((time) => (
                            <button
                              key={time}
                              type="button"
                              className="wakeup-chip selected"
                              onClick={() => toggleTimeSelection(time, 'fallback')}
                            >
                              {time}
                            </button>
                          ))}
                      </div>
                      <div className="wakeup-custom-row">
                        <span>{t('wakeup.form.customTime')}</span>
                        <input
                          className="wakeup-input wakeup-input-time wakeup-input-time-compact"
                          type="time"
                          step={60}
                          value={customFallbackTime || ''}
                          onChange={(event) => setCustomFallbackTime(event.target.value)}
                          onKeyDown={(event) => {
                            if (event.key !== 'Enter') return;
                            event.preventDefault();
                            addCustomTime(customFallbackTime, 'fallback');
                            setCustomFallbackTime('');
                          }}
                        />
                        <button
                          className="btn btn-secondary"
                          onClick={() => {
                            addCustomTime(customFallbackTime, 'fallback');
                            setCustomFallbackTime('');
                          }}
                        >
                          {t('common.add')}
                        </button>
                      </div>
                    </div>
                  )}
                </div>
              )}

              {formTriggerMode === 'startup' && (
                <div className="wakeup-mode-panel">
                  <div className="wakeup-form-group">
                    <label>{t('wakeup.triggerSource.startup')}</label>
                    <div className="wakeup-toggle-group">
                      <button
                        type="button"
                        className={`btn btn-secondary ${formStartupDelayMode === 'immediate' ? 'is-active' : ''}`}
                        onClick={() => setFormStartupDelayMode('immediate')}
                      >
                        {t('settings.general.startupWakeupImmediate')}
                      </button>
                      <button
                        type="button"
                        className={`btn btn-secondary ${formStartupDelayMode === 'delayed' ? 'is-active' : ''}`}
                        onClick={() => setFormStartupDelayMode('delayed')}
                      >
                        {t('settings.general.startupWakeupDelayed')}
                      </button>
                    </div>
                  </div>
                  {formStartupDelayMode === 'delayed' && (
                    <div className="wakeup-form-group">
                      <label>{t('settings.general.startupWakeupDelayed')}</label>
                      <div className="wakeup-inline-row">
                        <input
                          className="wakeup-input wakeup-input-small"
                          type="number"
                          min={1}
                          max={MAX_STARTUP_DELAY_MINUTES}
                          value={formStartupDelayMinutes}
                          onChange={(event) =>
                            setFormStartupDelayMinutes(event.target.value.replace(/[^\d]/g, ''))
                          }
                        />
                        <span>{t('settings.general.minutes')}</span>
                      </div>
                    </div>
                  )}
                </div>
              )}

              <ModalErrorMessage
                message={formError}
                position="bottom"
                scrollKey={formErrorScrollKey}
              />
              <div className="modal-actions">
                <button className="btn btn-secondary" onClick={() => setShowModal(false)}>
                  {t('common.cancel')}
                </button>
                <button className="btn btn-primary" onClick={handleSaveTask}>
                  {t('wakeup.form.saveTask')}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
    </main>
  );
}
