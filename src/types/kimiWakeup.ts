export type KimiWakeupScheduleKind =
  | 'daily'
  | 'weekly'
  | 'interval'
  | 'quota_reset'
  | 'startup';

export type KimiWakeupQuotaResetWindow =
  | 'either'
  | 'primary_window'
  | 'secondary_window';

export interface KimiWakeupSchedule {
  kind: KimiWakeupScheduleKind;
  daily_time?: string;
  weekly_days: number[];
  weekly_time?: string;
  interval_hours?: number;
  quota_reset_window?: KimiWakeupQuotaResetWindow;
  startup_delay_minutes?: number;
}

export interface KimiWakeupTask {
  id: string;
  name: string;
  enabled: boolean;
  account_ids: string[];
  prompt?: string;
  model?: string;
  schedule: KimiWakeupSchedule;
  created_at: number;
  updated_at: number;
  last_run_at?: number;
  last_status?: string;
  last_message?: string;
  last_success_count?: number;
  last_failure_count?: number;
  last_duration_ms?: number;
}

export interface KimiWakeupState {
  enabled: boolean;
  tasks: KimiWakeupTask[];
}

export interface KimiCliStatus {
  available: boolean;
  binary_path?: string;
  configured_path?: string;
  version?: string;
  message?: string;
  checked_at: number;
}

export interface KimiWakeupHistoryItem {
  id: string;
  run_id: string;
  timestamp: number;
  trigger_type: string;
  task_id?: string;
  task_name?: string;
  account_id: string;
  account_email: string;
  success: boolean;
  prompt?: string;
  model?: string;
  reply?: string;
  error?: string;
  duration_ms?: number;
  cli_path?: string;
  injected?: boolean;
}

export interface KimiWakeupBatchResult {
  run_id: string;
  runtime: KimiCliStatus;
  records: KimiWakeupHistoryItem[];
  success_count: number;
  failure_count: number;
}

export interface KimiWakeupOverview {
  runtime: KimiCliStatus;
  state: KimiWakeupState;
  history: KimiWakeupHistoryItem[];
}

export interface KimiWakeupRuntimeConfig {
  kimi_cli_path?: string | null;
}

export const DEFAULT_KIMI_WAKEUP_PROMPT = 'hi';
/** Official alias used by `kimi -m` / config.toml default_model. */
export const DEFAULT_KIMI_WAKEUP_MODEL = 'kimi-code/kimi-for-coding';

/** Official built-in models from Kimi Code docs (config-files.md). */
export const KIMI_BUILTIN_MODELS: Array<{
  id: string;
  displayName: string;
  model: string;
}> = [
  { id: 'kimi-code/k3', displayName: 'K3', model: 'k3' },
  {
    id: 'kimi-code/kimi-for-coding',
    displayName: 'Kimi for Coding',
    model: 'kimi-for-coding',
  },
  {
    id: 'kimi-code/kimi-for-coding-highspeed',
    displayName: 'Kimi for Coding Highspeed',
    model: 'kimi-for-coding-highspeed',
  },
];

export function normalizeKimiModelId(raw?: string | null): string {
  const trimmed = (raw || '').trim();
  if (!trimmed) return DEFAULT_KIMI_WAKEUP_MODEL;
  if (trimmed.includes('/')) return trimmed;
  return `kimi-code/${trimmed}`;
}

export function createEmptyKimiWakeupTask(
  partial?: Partial<KimiWakeupTask>,
): KimiWakeupTask {
  const now = Math.floor(Date.now() / 1000);
  return {
    id: partial?.id || `kimi-task-${Date.now()}`,
    name: partial?.name || 'Kimi 唤醒',
    enabled: partial?.enabled ?? true,
    account_ids: partial?.account_ids || [],
    prompt: partial?.prompt ?? DEFAULT_KIMI_WAKEUP_PROMPT,
    model: normalizeKimiModelId(partial?.model ?? DEFAULT_KIMI_WAKEUP_MODEL),
    schedule: partial?.schedule || {
      kind: 'daily',
      daily_time: '08:00',
      weekly_days: [],
    },
    created_at: partial?.created_at ?? now,
    updated_at: partial?.updated_at ?? now,
  };
}
