export interface CodexCliInstallHint {
  label: string;
  command: string;
}

export interface CodexCliStatus {
  available: boolean;
  binary_path?: string;
  configured_codex_cli_path?: string;
  configured_node_path?: string;
  version?: string;
  source?: string;
  message?: string;
  required_runtime_paths: string[];
  checked_at: number;
  install_hints: CodexCliInstallHint[];
}

export type CodexWakeupScheduleKind = 'daily' | 'weekly' | 'interval' | 'quota_reset' | 'startup';
export type CodexWakeupQuotaResetWindow = 'either' | 'primary_window' | 'secondary_window';
export type CodexWakeupReasoningEffort = 'low' | 'medium' | 'high' | 'xhigh' | 'max' | 'ultra';
export type WakeupTaskExecutionMode = 'auto' | 'confirm';
export type WakeupTaskHistoryStatus =
  | 'success'
  | 'failed'
  | 'skipped_timeout'
  | 'skipped_app_closed'
  | 'skipped_manual';

export interface CodexWakeupSchedule {
  kind: CodexWakeupScheduleKind;
  daily_time?: string;
  weekly_days: number[];
  weekly_time?: string;
  interval_hours?: number;
  quota_reset_window?: CodexWakeupQuotaResetWindow;
  startup_delay_minutes?: number;
}

export interface CodexWakeupTask {
  id: string;
  name: string;
  enabled: boolean;
  account_ids: string[];
  prompt?: string;
  model?: string;
  model_display_name?: string;
  model_reasoning_effort?: CodexWakeupReasoningEffort;
  schedule: CodexWakeupSchedule;
  execution_mode?: WakeupTaskExecutionMode;
  confirm_timeout_minutes?: number;
  created_at: number;
  updated_at: number;
  last_run_at?: number;
  last_status?: string;
  last_message?: string;
  last_success_count?: number;
  last_failure_count?: number;
  last_duration_ms?: number;
  next_run_at?: number;
}

export interface CodexWakeupModelPreset {
  id: string;
  name: string;
  model: string;
  allowed_reasoning_efforts: CodexWakeupReasoningEffort[];
  default_reasoning_effort: CodexWakeupReasoningEffort;
}

export interface CodexWakeupState {
  enabled: boolean;
  tasks: CodexWakeupTask[];
  model_presets: CodexWakeupModelPreset[];
  model_preset_migrations: string[];
}

export interface CodexQuotaSnapshot {
  hourly_percentage?: number;
  hourly_reset_time?: number;
  weekly_percentage?: number;
  weekly_reset_time?: number;
}

export interface CodexWakeupHistoryItem {
  id: string;
  run_id: string;
  timestamp: number;
  trigger_type: string;
  task_id?: string;
  task_name?: string;
  account_id: string;
  account_email: string;
  account_context_text?: string;
  success: boolean;
  status?: WakeupTaskHistoryStatus;
  prompt?: string;
  model?: string;
  model_display_name?: string;
  model_reasoning_effort?: CodexWakeupReasoningEffort;
  reply?: string;
  error?: string;
  quota_refresh_error?: string;
  duration_ms?: number;
  cli_path?: string;
  quota_before?: CodexQuotaSnapshot;
  quota_after?: CodexQuotaSnapshot;
}

export interface CodexWakeupBatchResult {
  run_id: string;
  runtime: CodexCliStatus;
  records: CodexWakeupHistoryItem[];
  success_count: number;
  failure_count: number;
}

export interface CodexWakeupProgressPayload {
  run_id: string;
  trigger_type: string;
  task_id?: string;
  task_name?: string;
  total: number;
  completed: number;
  success_count: number;
  failure_count: number;
  running: boolean;
  phase: string;
  current_account_id?: string;
  item?: CodexWakeupHistoryItem;
}

export interface CodexWakeupOverview {
  runtime: CodexCliStatus;
  state: CodexWakeupState;
  history: CodexWakeupHistoryItem[];
}
