import { invoke } from '@tauri-apps/api/core';
import {
  CodexCliStatus,
  CodexWakeupBatchResult,
  CodexWakeupHistoryItem,
  CodexWakeupModelPreset,
  CodexWakeupOverview,
  CodexWakeupProgressPayload,
  CodexWakeupReasoningEffort,
  CodexWakeupState,
  CodexWakeupTask,
} from '../types/codexWakeup';

interface RawCodexCliInstallHint {
  label: string;
  command: string;
}

interface RawCodexCliStatus {
  available: boolean;
  binaryPath?: string;
  configuredCodexCliPath?: string;
  configuredNodePath?: string;
  version?: string;
  source?: string;
  message?: string;
  requiredRuntimePaths?: string[];
  checkedAt: number;
  installHints?: RawCodexCliInstallHint[];
}

interface RawCodexWakeupSchedule {
  kind: CodexWakeupTask['schedule']['kind'];
  dailyTime?: string;
  weeklyDays?: number[];
  weeklyTime?: string;
  intervalHours?: number;
  quotaResetWindow?: CodexWakeupTask['schedule']['quota_reset_window'];
  startupDelayMinutes?: number;
}

interface RawCodexWakeupTask {
  id: string;
  name: string;
  enabled: boolean;
  accountIds: string[];
  prompt?: string;
  model?: string;
  modelDisplayName?: string;
  modelReasoningEffort?: CodexWakeupReasoningEffort;
  schedule: RawCodexWakeupSchedule;
  createdAt: number;
  updatedAt: number;
  lastRunAt?: number;
  lastStatus?: string;
  lastMessage?: string;
  lastSuccessCount?: number;
  lastFailureCount?: number;
  lastDurationMs?: number;
  nextRunAt?: number;
  executionMode?: string;
  confirmTimeoutMinutes?: number;
}

interface RawCodexWakeupModelPreset {
  id: string;
  name: string;
  model: string;
  allowedReasoningEfforts?: CodexWakeupReasoningEffort[];
  defaultReasoningEffort: CodexWakeupReasoningEffort;
}

interface RawCodexQuotaSnapshot {
  hourlyPercentage?: number;
  hourlyResetTime?: number;
  weeklyPercentage?: number;
  weeklyResetTime?: number;
}

interface RawCodexWakeupHistoryItem {
  id: string;
  runId: string;
  timestamp: number;
  triggerType: string;
  taskId?: string;
  taskName?: string;
  accountId: string;
  accountEmail: string;
  accountContextText?: string;
  success: boolean;
  prompt?: string;
  model?: string;
  modelDisplayName?: string;
  modelReasoningEffort?: CodexWakeupReasoningEffort;
  reply?: string;
  error?: string;
  quotaRefreshError?: string;
  durationMs?: number;
  cliPath?: string;
  quotaBefore?: RawCodexQuotaSnapshot;
  quotaAfter?: RawCodexQuotaSnapshot;
}

interface RawCodexWakeupState {
  enabled: boolean;
  tasks: RawCodexWakeupTask[];
  modelPresets?: RawCodexWakeupModelPreset[];
  modelPresetMigrations?: string[];
}

interface RawCodexWakeupBatchResult {
  runId: string;
  runtime: RawCodexCliStatus;
  records: RawCodexWakeupHistoryItem[];
  successCount: number;
  failureCount: number;
}

interface RawCodexWakeupOverview {
  runtime: RawCodexCliStatus;
  state: RawCodexWakeupState;
  history: RawCodexWakeupHistoryItem[];
}

interface RawCodexWakeupProgressPayload {
  runId: string;
  triggerType: string;
  taskId?: string;
  taskName?: string;
  total: number;
  completed: number;
  successCount: number;
  failureCount: number;
  running: boolean;
  phase: string;
  currentAccountId?: string;
  item?: RawCodexWakeupHistoryItem;
}

function fromRawCliStatus(raw: RawCodexCliStatus): CodexCliStatus {
  return {
    available: raw.available,
    binary_path: raw.binaryPath,
    configured_codex_cli_path: raw.configuredCodexCliPath,
    configured_node_path: raw.configuredNodePath,
    version: raw.version,
    source: raw.source,
    message: raw.message,
    required_runtime_paths: raw.requiredRuntimePaths ?? [],
    checked_at: raw.checkedAt,
    install_hints: raw.installHints ?? [],
  };
}

function toRawTask(task: CodexWakeupTask): RawCodexWakeupTask {
  return {
    id: task.id,
    name: task.name,
    enabled: task.enabled,
    accountIds: task.account_ids,
    prompt: task.prompt,
    model: task.model,
    modelDisplayName: task.model_display_name,
    modelReasoningEffort: task.model_reasoning_effort,
    schedule: {
      kind: task.schedule.kind,
      dailyTime: task.schedule.daily_time,
      weeklyDays: task.schedule.weekly_days,
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
    nextRunAt: task.next_run_at,
    executionMode: task.execution_mode,
    confirmTimeoutMinutes: task.confirm_timeout_minutes,
  };
}

function fromRawTask(raw: RawCodexWakeupTask): CodexWakeupTask {
  return {
    id: raw.id,
    name: raw.name,
    enabled: raw.enabled,
    account_ids: raw.accountIds ?? [],
    prompt: raw.prompt,
    model: raw.model,
    model_display_name: raw.modelDisplayName,
    model_reasoning_effort: raw.modelReasoningEffort,
    schedule: {
      kind: raw.schedule.kind,
      daily_time: raw.schedule.dailyTime,
      weekly_days: raw.schedule.weeklyDays ?? [],
      weekly_time: raw.schedule.weeklyTime,
      interval_hours: raw.schedule.intervalHours,
      quota_reset_window: raw.schedule.quotaResetWindow,
      startup_delay_minutes: raw.schedule.startupDelayMinutes,
    },
    created_at: raw.createdAt,
    updated_at: raw.updatedAt,
    last_run_at: raw.lastRunAt,
    last_status: raw.lastStatus,
    last_message: raw.lastMessage,
    last_success_count: raw.lastSuccessCount,
    last_failure_count: raw.lastFailureCount,
    last_duration_ms: raw.lastDurationMs,
    next_run_at: raw.nextRunAt,
    execution_mode: raw.executionMode as 'auto' | 'confirm' | undefined,
    confirm_timeout_minutes: raw.confirmTimeoutMinutes,
  };
}

function toRawModelPreset(preset: CodexWakeupModelPreset): RawCodexWakeupModelPreset {
  return {
    id: preset.id,
    name: preset.name,
    model: preset.model,
    allowedReasoningEfforts: preset.allowed_reasoning_efforts,
    defaultReasoningEffort: preset.default_reasoning_effort,
  };
}

function fromRawModelPreset(raw: RawCodexWakeupModelPreset): CodexWakeupModelPreset {
  return {
    id: raw.id,
    name: raw.name,
    model: raw.model,
    allowed_reasoning_efforts: raw.allowedReasoningEfforts ?? [],
    default_reasoning_effort: raw.defaultReasoningEffort,
  };
}

function fromRawQuotaSnapshot(raw?: RawCodexQuotaSnapshot) {
  if (!raw) return undefined;
  return {
    hourly_percentage: raw.hourlyPercentage,
    hourly_reset_time: raw.hourlyResetTime,
    weekly_percentage: raw.weeklyPercentage,
    weekly_reset_time: raw.weeklyResetTime,
  };
}

function fromRawHistoryItem(raw: RawCodexWakeupHistoryItem): CodexWakeupHistoryItem {
  return {
    id: raw.id,
    run_id: raw.runId,
    timestamp: raw.timestamp,
    trigger_type: raw.triggerType,
    task_id: raw.taskId,
    task_name: raw.taskName,
    account_id: raw.accountId,
    account_email: raw.accountEmail,
    account_context_text: raw.accountContextText,
    success: raw.success,
    prompt: raw.prompt,
    model: raw.model,
    model_display_name: raw.modelDisplayName,
    model_reasoning_effort: raw.modelReasoningEffort,
    reply: raw.reply,
    error: raw.error,
    quota_refresh_error: raw.quotaRefreshError,
    duration_ms: raw.durationMs,
    cli_path: raw.cliPath,
    quota_before: fromRawQuotaSnapshot(raw.quotaBefore),
    quota_after: fromRawQuotaSnapshot(raw.quotaAfter),
  };
}

function fromRawState(raw: RawCodexWakeupState): CodexWakeupState {
  return {
    enabled: raw.enabled,
    tasks: (raw.tasks ?? []).map(fromRawTask),
    model_presets: (raw.modelPresets ?? []).map(fromRawModelPreset),
    model_preset_migrations: raw.modelPresetMigrations ?? [],
  };
}

function fromRawBatchResult(raw: RawCodexWakeupBatchResult): CodexWakeupBatchResult {
  return {
    run_id: raw.runId,
    runtime: fromRawCliStatus(raw.runtime),
    records: (raw.records ?? []).map(fromRawHistoryItem),
    success_count: raw.successCount,
    failure_count: raw.failureCount,
  };
}

function fromRawOverview(raw: RawCodexWakeupOverview): CodexWakeupOverview {
  return {
    runtime: fromRawCliStatus(raw.runtime),
    state: fromRawState(raw.state),
    history: (raw.history ?? []).map(fromRawHistoryItem),
  };
}

export function fromRawWakeupProgressPayload(
  raw: RawCodexWakeupProgressPayload,
): CodexWakeupProgressPayload {
  return {
    run_id: raw.runId,
    trigger_type: raw.triggerType,
    task_id: raw.taskId,
    task_name: raw.taskName,
    total: raw.total,
    completed: raw.completed,
    success_count: raw.successCount,
    failure_count: raw.failureCount,
    running: raw.running,
    phase: raw.phase,
    current_account_id: raw.currentAccountId,
    item: raw.item ? fromRawHistoryItem(raw.item) : undefined,
  };
}

export async function getCodexWakeupCliStatus(): Promise<CodexCliStatus> {
  return fromRawCliStatus(await invoke<RawCodexCliStatus>('codex_wakeup_get_cli_status'));
}

export async function updateCodexWakeupRuntimeConfig(
  codexCliPath?: string,
  nodePath?: string,
): Promise<CodexCliStatus> {
  return fromRawCliStatus(
    await invoke<RawCodexCliStatus>('codex_wakeup_update_runtime_config', {
      codexCliPath: codexCliPath ?? null,
      nodePath: nodePath ?? null,
    }),
  );
}

export async function getCodexWakeupOverview(): Promise<CodexWakeupOverview> {
  return fromRawOverview(await invoke<RawCodexWakeupOverview>('codex_wakeup_get_overview'));
}

export async function getCodexWakeupState(): Promise<CodexWakeupState> {
  return fromRawState(await invoke<RawCodexWakeupState>('codex_wakeup_get_state'));
}

export async function saveCodexWakeupState(
  enabled: boolean,
  tasks: CodexWakeupTask[],
  modelPresets: CodexWakeupModelPreset[],
  modelPresetMigrations: string[] = [],
): Promise<CodexWakeupState> {
  return fromRawState(
    await invoke<RawCodexWakeupState>('codex_wakeup_save_state', {
      enabled,
      tasks: tasks.map(toRawTask),
      modelPresets: modelPresets.map(toRawModelPreset),
      modelPresetMigrations,
    }),
  );
}

export async function loadCodexWakeupHistory(): Promise<CodexWakeupHistoryItem[]> {
  return (await invoke<RawCodexWakeupHistoryItem[]>('codex_wakeup_load_history')).map(fromRawHistoryItem);
}

export async function clearCodexWakeupHistory(): Promise<void> {
  return await invoke('codex_wakeup_clear_history');
}

export async function testCodexWakeup(
  accountIds: string[],
  runId: string,
  prompt?: string,
  model?: string,
  modelDisplayName?: string,
  modelReasoningEffort?: CodexWakeupReasoningEffort,
  cancelScopeId?: string,
): Promise<CodexWakeupBatchResult> {
  return fromRawBatchResult(
    await invoke<RawCodexWakeupBatchResult>('codex_wakeup_test', {
      accountIds,
      runId,
      prompt: prompt ?? null,
      model: model ?? null,
      modelDisplayName: modelDisplayName ?? null,
      modelReasoningEffort: modelReasoningEffort ?? null,
      cancelScopeId: cancelScopeId ?? null,
    }),
  );
}

export async function runCodexWakeupTask(taskId: string, runId: string): Promise<CodexWakeupBatchResult> {
  return fromRawBatchResult(
    await invoke<RawCodexWakeupBatchResult>('codex_wakeup_run_task', { taskId, runId }),
  );
}

export async function cancelCodexWakeupScope(cancelScopeId: string): Promise<void> {
  return await invoke('codex_wakeup_cancel_scope', { cancelScopeId });
}

export async function releaseCodexWakeupScope(cancelScopeId: string): Promise<void> {
  return await invoke('codex_wakeup_release_scope', { cancelScopeId });
}
