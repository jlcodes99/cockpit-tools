import { hydrateDurableUiSettings } from '../utils/durableUiSettings';
import { invoke } from '@tauri-apps/api/core';
import i18n from '../i18n';
import { DEFAULT_ANTIGRAVITY_WAKEUP_MODEL_ID } from '../utils/antigravityModels';

export type RepeatMode = 'daily' | 'weekly' | 'interval';
export type WakeupTransportMode = 'language_server' | 'clientless';

export interface ScheduleConfig {
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
  transportMode?: WakeupTransportMode;
  timeWindowEnabled?: boolean;
  timeWindowStart?: string;
  timeWindowEnd?: string;
  fallbackTimes?: string[];
  startupDelayMinutes?: number;
  executionMode?: 'auto' | 'confirm';
  confirmTimeoutMinutes?: number;
}

export interface WakeupTask {
  id: string;
  name: string;
  enabled: boolean;
  createdAt: number;
  lastRunAt?: number;
  schedule: ScheduleConfig;
  executionMode?: 'auto' | 'confirm';
  confirmTimeoutMinutes?: number;
  execution_mode?: 'auto' | 'confirm';
  confirm_timeout_minutes?: number;
}

export interface AntigravityWakeupState {
  enabled: boolean;
  tasks: WakeupTask[];
  rulesRevision: number;
}

export const WAKEUP_TASKS_UPDATED = 'wakeup-tasks-updated';
const LEGACY_TASKS = 'agtools.wakeup.tasks';
const LEGACY_ENABLED = 'agtools.wakeup.enabled';
const LEGACY_SCHEDULE = 'agtools.wakeup.schedule';
const LEGACY_DISABLE_MIGRATION = 'agtools.wakeup.migration.force_disable_0_8_14';

function readLegacyState(): Pick<AntigravityWakeupState, 'enabled' | 'tasks'> | undefined {
  const rawTasks = localStorage.getItem(LEGACY_TASKS);
  const rawEnabled = localStorage.getItem(LEGACY_ENABLED);
  const rawSchedule = rawTasks === null ? localStorage.getItem(LEGACY_SCHEDULE) : null;
  if (rawTasks === null && rawEnabled === null && rawSchedule === null) return undefined;

  let tasks: WakeupTask[] = [];
  if (rawTasks !== null) {
    const parsed: unknown = JSON.parse(rawTasks);
    if (!Array.isArray(parsed)) throw new Error('Invalid legacy wakeup tasks');
    tasks = parsed;
  } else if (rawSchedule !== null) {
    const parsed = JSON.parse(rawSchedule) as Partial<ScheduleConfig> & { enabled?: boolean };
    const { enabled, ...schedule } = parsed;
    tasks = [{
      id: 'legacy-wakeup-schedule',
      name: i18n.t('wakeup.defaultTaskName'),
      enabled: enabled === true,
      createdAt: 0,
      schedule: {
        repeatMode: 'daily',
        dailyTimes: ['08:00'],
        weeklyDays: [1, 2, 3, 4, 5],
        weeklyTimes: ['08:00'],
        intervalHours: 4,
        intervalStartTime: '07:00',
        intervalEndTime: '22:00',
        selectedModels: [DEFAULT_ANTIGRAVITY_WAKEUP_MODEL_ID],
        selectedAccounts: [],
        ...schedule,
      },
    }];
  }
  return {
    enabled: localStorage.getItem(LEGACY_DISABLE_MIGRATION) === '1' && rawEnabled === 'true',
    tasks,
  };
}

let initialization: Promise<AntigravityWakeupState> | undefined;

export function initializeAntigravityWakeupState(): Promise<AntigravityWakeupState> {
  if (initialization) return initialization;

  let legacyState: ReturnType<typeof readLegacyState>;
  try {
    legacyState = readLegacyState();
  } catch (error) {
    console.warn('[WakeupTasks] 读取旧任务配置失败:', error);
  }
  initialization = hydrateDurableUiSettings()
    .catch(error => console.warn('[WakeupTasks] 读取界面偏好失败:', error))
    .then(() => invoke<AntigravityWakeupState>('wakeup_initialize_state', {
      legacyState,
    })).catch((error) => {
    initialization = undefined;
    throw error;
  });
  return initialization;
}

export async function loadAntigravityWakeupState(): Promise<AntigravityWakeupState> {
  await initializeAntigravityWakeupState();
  return invoke<AntigravityWakeupState>('wakeup_load_state');
}

export async function saveAntigravityWakeupState(
  enabled: boolean,
  tasks: WakeupTask[],
  expectedRulesRevision: number,
): Promise<AntigravityWakeupState> {
  const saved = await invoke<AntigravityWakeupState>('wakeup_sync_state', {
    enabled, tasks, expectedRulesRevision,
  });
  window.dispatchEvent(new Event(WAKEUP_TASKS_UPDATED));
  return saved;
}

export function hasActiveWakeupQuotaReset(state: AntigravityWakeupState): boolean {
  return state.enabled && state.tasks.some((task) => task.enabled && task.schedule.wakeOnReset === true);
}
