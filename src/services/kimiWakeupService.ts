import { invoke } from '@tauri-apps/api/core';
import type {
  KimiCliStatus,
  KimiWakeupBatchResult,
  KimiWakeupHistoryItem,
  KimiWakeupOverview,
  KimiWakeupRuntimeConfig,
  KimiWakeupState,
  KimiWakeupTask,
} from '../types/kimiWakeup';

export interface KimiWakeupModelInfo {
  id: string;
  displayName: string;
  model: string;
}

/** Wire format matches Rust `#[serde(rename_all = "camelCase")]`. */
interface RawKimiCliStatus {
  available: boolean;
  binaryPath?: string;
  configuredPath?: string;
  version?: string;
  message?: string;
  checkedAt: number;
}

interface RawKimiWakeupSchedule {
  kind: KimiWakeupTask['schedule']['kind'];
  dailyTime?: string;
  weeklyDays?: number[];
  weeklyTime?: string;
  intervalHours?: number;
  quotaResetWindow?: KimiWakeupTask['schedule']['quota_reset_window'];
  startupDelayMinutes?: number;
}

interface RawKimiWakeupTask {
  id: string;
  name: string;
  enabled: boolean;
  accountIds: string[];
  prompt?: string;
  model?: string;
  schedule: RawKimiWakeupSchedule;
  createdAt: number;
  updatedAt: number;
  lastRunAt?: number;
  lastStatus?: string;
  lastMessage?: string;
  lastSuccessCount?: number;
  lastFailureCount?: number;
  lastDurationMs?: number;
}

interface RawKimiWakeupState {
  enabled: boolean;
  tasks: RawKimiWakeupTask[];
}

interface RawKimiWakeupHistoryItem {
  id: string;
  runId: string;
  timestamp: number;
  triggerType: string;
  taskId?: string;
  taskName?: string;
  accountId: string;
  accountEmail: string;
  success: boolean;
  prompt?: string;
  model?: string;
  reply?: string;
  error?: string;
  durationMs?: number;
  cliPath?: string;
  injected?: boolean;
}

interface RawKimiWakeupBatchResult {
  runId: string;
  runtime: RawKimiCliStatus;
  records: RawKimiWakeupHistoryItem[];
  successCount: number;
  failureCount: number;
}

interface RawKimiWakeupOverview {
  runtime: RawKimiCliStatus;
  state: RawKimiWakeupState;
  history: RawKimiWakeupHistoryItem[];
}

interface RawKimiWakeupRuntimeConfig {
  kimiCliPath?: string | null;
}

interface RawKimiWakeupModelInfo {
  id: string;
  displayName: string;
  model: string;
}

function fromRawCliStatus(raw: RawKimiCliStatus): KimiCliStatus {
  return {
    available: raw.available,
    binary_path: raw.binaryPath,
    configured_path: raw.configuredPath,
    version: raw.version,
    message: raw.message,
    checked_at: raw.checkedAt,
  };
}

function toRawTask(task: KimiWakeupTask): RawKimiWakeupTask {
  return {
    id: task.id,
    name: task.name,
    enabled: task.enabled,
    accountIds: task.account_ids ?? [],
    prompt: task.prompt,
    model: task.model,
    schedule: {
      kind: task.schedule.kind,
      dailyTime: task.schedule.daily_time,
      weeklyDays: task.schedule.weekly_days ?? [],
      weeklyTime: task.schedule.weekly_time,
      intervalHours: task.schedule.interval_hours,
      quotaResetWindow: task.schedule.quota_reset_window,
      startupDelayMinutes: task.schedule.startup_delay_minutes,
    },
    createdAt: task.created_at,
    updatedAt: task.updated_at,
    lastRunAt: task.last_run_at,
    lastStatus: task.last_status,
    lastMessage: task.last_message,
    lastSuccessCount: task.last_success_count,
    lastFailureCount: task.last_failure_count,
    lastDurationMs: task.last_duration_ms,
  };
}

function fromRawTask(raw: RawKimiWakeupTask): KimiWakeupTask {
  return {
    id: raw.id,
    name: raw.name,
    enabled: raw.enabled,
    account_ids: raw.accountIds ?? [],
    prompt: raw.prompt,
    model: raw.model,
    schedule: {
      kind: raw.schedule?.kind || 'daily',
      daily_time: raw.schedule?.dailyTime,
      weekly_days: raw.schedule?.weeklyDays ?? [],
      weekly_time: raw.schedule?.weeklyTime,
      interval_hours: raw.schedule?.intervalHours,
      quota_reset_window: raw.schedule?.quotaResetWindow,
      startup_delay_minutes: raw.schedule?.startupDelayMinutes,
    },
    created_at: raw.createdAt,
    updated_at: raw.updatedAt,
    last_run_at: raw.lastRunAt,
    last_status: raw.lastStatus,
    last_message: raw.lastMessage,
    last_success_count: raw.lastSuccessCount,
    last_failure_count: raw.lastFailureCount,
    last_duration_ms: raw.lastDurationMs,
  };
}

function fromRawState(raw: RawKimiWakeupState): KimiWakeupState {
  return {
    enabled: !!raw.enabled,
    tasks: (raw.tasks ?? []).map(fromRawTask),
  };
}

/** Exported for contract tests (must include `accountIds` on tasks). */
export function toRawState(state: KimiWakeupState): RawKimiWakeupState {
  return {
    enabled: state.enabled,
    tasks: (state.tasks ?? []).map(toRawTask),
  };
}

function fromRawHistoryItem(raw: RawKimiWakeupHistoryItem): KimiWakeupHistoryItem {
  return {
    id: raw.id,
    run_id: raw.runId,
    timestamp: raw.timestamp,
    trigger_type: raw.triggerType,
    task_id: raw.taskId,
    task_name: raw.taskName,
    account_id: raw.accountId,
    account_email: raw.accountEmail,
    success: raw.success,
    prompt: raw.prompt,
    model: raw.model,
    reply: raw.reply,
    error: raw.error,
    duration_ms: raw.durationMs,
    cli_path: raw.cliPath,
    injected: raw.injected,
  };
}

function fromRawBatchResult(raw: RawKimiWakeupBatchResult): KimiWakeupBatchResult {
  return {
    run_id: raw.runId,
    runtime: fromRawCliStatus(raw.runtime),
    records: (raw.records ?? []).map(fromRawHistoryItem),
    success_count: raw.successCount ?? 0,
    failure_count: raw.failureCount ?? 0,
  };
}

function fromRawOverview(raw: RawKimiWakeupOverview): KimiWakeupOverview {
  return {
    runtime: fromRawCliStatus(raw.runtime),
    state: fromRawState(raw.state),
    history: (raw.history ?? []).map(fromRawHistoryItem),
  };
}

export async function listKimiWakeupModels(): Promise<KimiWakeupModelInfo[]> {
  const raw = await invoke<RawKimiWakeupModelInfo[]>('kimi_wakeup_list_models');
  return (raw ?? []).map((item) => ({
    id: item.id,
    displayName: item.displayName,
    model: item.model,
  }));
}

export async function getKimiWakeupCliStatus(): Promise<KimiCliStatus> {
  return fromRawCliStatus(await invoke<RawKimiCliStatus>('kimi_wakeup_get_cli_status'));
}

/** System discovery only — ignores saved custom path. */
export async function detectKimiWakeupCli(): Promise<KimiCliStatus> {
  return fromRawCliStatus(await invoke<RawKimiCliStatus>('kimi_wakeup_detect_cli'));
}

export async function updateKimiWakeupRuntimeConfig(
  config: KimiWakeupRuntimeConfig,
): Promise<KimiWakeupRuntimeConfig> {
  const raw = await invoke<RawKimiWakeupRuntimeConfig>('kimi_wakeup_update_runtime_config', {
    config: {
      kimiCliPath: config.kimi_cli_path ?? null,
    },
  });
  return {
    kimi_cli_path: raw?.kimiCliPath ?? null,
  };
}

export async function getKimiWakeupOverview(): Promise<KimiWakeupOverview> {
  return fromRawOverview(await invoke<RawKimiWakeupOverview>('kimi_wakeup_get_overview'));
}

export async function getKimiWakeupState(): Promise<KimiWakeupState> {
  return fromRawState(await invoke<RawKimiWakeupState>('kimi_wakeup_get_state'));
}

export async function saveKimiWakeupState(
  state: KimiWakeupState,
): Promise<KimiWakeupState> {
  return fromRawState(
    await invoke<RawKimiWakeupState>('kimi_wakeup_save_state', {
      state: toRawState(state),
    }),
  );
}

export async function loadKimiWakeupHistory(): Promise<KimiWakeupHistoryItem[]> {
  const raw = await invoke<RawKimiWakeupHistoryItem[]>('kimi_wakeup_load_history');
  return (raw ?? []).map(fromRawHistoryItem);
}

export async function clearKimiWakeupHistory(): Promise<void> {
  return invoke('kimi_wakeup_clear_history');
}

export async function testKimiWakeup(
  accountIds: string[],
  prompt?: string,
  model?: string,
): Promise<KimiWakeupBatchResult> {
  return fromRawBatchResult(
    await invoke<RawKimiWakeupBatchResult>('kimi_wakeup_test', {
      accountIds,
      prompt: prompt ?? null,
      model: model ?? null,
    }),
  );
}

export async function runKimiWakeupTask(
  taskId: string,
): Promise<KimiWakeupBatchResult> {
  return fromRawBatchResult(
    await invoke<RawKimiWakeupBatchResult>('kimi_wakeup_run_task', { taskId }),
  );
}

export async function runEnabledKimiWakeupTasks(): Promise<KimiWakeupBatchResult[]> {
  const raw = await invoke<RawKimiWakeupBatchResult[]>('kimi_wakeup_run_enabled_tasks');
  return (raw ?? []).map(fromRawBatchResult);
}
