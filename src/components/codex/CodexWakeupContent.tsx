import {
  type Dispatch,
  type SetStateAction,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { confirm as confirmDialog } from '@tauri-apps/plugin-dialog';
import {
  Check,
  ChevronDown,
  ChevronLeft,
  CircleAlert,
  Copy,
  Eye,
  Pencil,
  Play,
  Plus,
  Power,
  RefreshCw,
  Search,
  Settings,
  Trash2,
  X,
} from 'lucide-react';
import { useEscClose } from '../../hooks/useEscClose';
import {
  CodexAccount,
  getCodexAuthMetadata,
  isCodexApiKeyAccount,
  isCodexTeamLikePlan,
} from '../../types/codex';
import { buildCodexAccountPresentation } from '../../presentation/platformAccountPresentation';
import {
  CodexWakeupBatchResult,
  CodexWakeupHistoryItem,
  CodexWakeupModelPreset,
  CodexWakeupProgressPayload,
  CodexWakeupQuotaResetWindow,
  CodexWakeupReasoningEffort,
  CodexWakeupScheduleKind,
  CodexWakeupTask,
} from '../../types/codexWakeup';
import { useCodexWakeupStore } from '../../stores/useCodexWakeupStore';
import { fromRawWakeupProgressPayload } from '../../services/codexWakeupService';
import { AccountTagFilterDropdown } from '../AccountTagFilterDropdown';
import { ModalErrorMessage, useModalErrorState } from '../ModalErrorMessage';
import {
  MultiSelectFilterDropdown,
  type MultiSelectFilterOption,
} from '../MultiSelectFilterDropdown';
import {
  isPrivacyModeEnabledByDefault,
  maskSensitiveValue,
  PRIVACY_MODE_CHANGED_EVENT,
} from '../../utils/privacy';

interface CodexWakeupGeneralConfig {
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
  codex_launch_on_switch?: boolean;
}

interface CodexWakeupContentProps {
  accounts: CodexAccount[];
  onRefreshAccounts: () => Promise<void>;
  openPresetManagerSignal?: number;
}

interface TaskDraft {
  id?: string;
  createdAt?: number;
  name: string;
  enabled: boolean;
  accountIds: string[];
  prompt: string;
  modelPresetId: string;
  model: string;
  modelDisplayName: string;
  modelReasoningEffort: CodexWakeupReasoningEffort | '';
  scheduleKind: CodexWakeupScheduleKind;
  dailyTime: string;
  weeklyDays: number[];
  weeklyTime: string;
  intervalHours: string;
  quotaResetWindow: CodexWakeupQuotaResetWindow;
  startupDelayMode: 'immediate' | 'delayed';
  startupDelayMinutes: string;
}

interface PresetDraft {
  id?: string;
  name: string;
  model: string;
  allowedReasoningEfforts: CodexWakeupReasoningEffort[];
  defaultReasoningEffort: CodexWakeupReasoningEffort | '';
}

interface AccountPickerFilters {
  query: string;
  planTypes: string[];
  tags: string[];
}

interface WakeupSingleSelectOption {
  value: string;
  label: string;
}

interface WakeupModelSelectionMemory {
  modelPresetId: string;
  model: string;
  modelDisplayName: string;
  modelReasoningEffort: CodexWakeupReasoningEffort | '';
}

interface RuntimeConfigDraft {
  codexCliPath: string;
  nodePath: string;
}

interface WakeupQuotaBadge {
  key: 'primary' | 'secondary';
  valueText: string;
  quotaClass: string;
}

type ExecutionRecordStatus = 'pending' | 'running' | 'success' | 'error';

interface ExecutionRecordState {
  id: string;
  accountId: string;
  accountEmail: string;
  accountContextText?: string;
  triggerType: string;
  status: ExecutionRecordStatus;
  prompt?: string;
  model?: string;
  modelDisplayName?: string;
  modelReasoningEffort?: CodexWakeupReasoningEffort;
  reply?: string;
  error?: string;
  timestamp?: number;
  durationMs?: number;
}

interface ExecutionSessionState {
  runId: string;
  taskId?: string;
  triggerType: string;
  title: string;
  runtime: CodexWakeupBatchResult['runtime'] | null;
  startedAt: number;
  durationMs?: number;
  total: number;
  completed: number;
  successCount: number;
  failureCount: number;
  taskName?: string;
  running: boolean;
  preview: boolean;
  errorText?: string;
  records: ExecutionRecordState[];
}

interface HistoryBatchSummary {
  runId: string;
  triggerType: string;
  taskId?: string;
  taskName?: string;
  timestamp: number;
  total: number;
  successCount: number;
  failureCount: number;
  durationMs?: number;
  cliPath?: string;
  records: CodexWakeupHistoryItem[];
}

type ExecutionRecordFilter = 'all' | ExecutionRecordStatus;

const WEEKDAY_OPTIONS = [
  { value: 1, short: 'Mon' },
  { value: 2, short: 'Tue' },
  { value: 3, short: 'Wed' },
  { value: 4, short: 'Thu' },
  { value: 5, short: 'Fri' },
  { value: 6, short: 'Sat' },
  { value: 0, short: 'Sun' },
];

const DEFAULT_PROMPT = 'hi';
const QUICK_TIME_OPTIONS = ['07:00', '08:00', '09:00', '10:00', '14:00', '18:00', '22:00'];
const REASONING_EFFORT_OPTIONS: CodexWakeupReasoningEffort[] = ['low', 'medium', 'high', 'xhigh'];
const DEFAULT_WAKEUP_MODEL = 'gpt-5.3-codex';
const DEFAULT_WAKEUP_REASONING_EFFORT: CodexWakeupReasoningEffort = 'medium';
const QUOTA_RESET_MIN_REFRESH_MINUTES = 2;
const MAX_STARTUP_DELAY_MINUTES = 1440;
const CODEX_WAKEUP_MODEL_SELECTION_STORAGE_KEY = 'agtools.codex.wakeup.model_selection';
const buildWakeupTestScopeId = () =>
  typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function'
    ? `codex-wakeup-test-${crypto.randomUUID()}`
    : `codex-wakeup-test-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;

function createEmptyAccountPickerFilters(): AccountPickerFilters {
  return {
    query: '',
    planTypes: [],
    tags: [],
  };
}

function createRuntimeConfigDraft(status?: CodexWakeupBatchResult['runtime'] | null): RuntimeConfigDraft {
  return {
    codexCliPath: status?.configured_codex_cli_path ?? '',
    nodePath: status?.configured_node_path ?? '',
  };
}

function toggleStringSelection(values: string[], target: string) {
  return values.includes(target)
    ? values.filter((item) => item !== target)
    : [...values, target];
}

function normalizeWakeupTag(value: string) {
  return value.trim().toLowerCase();
}

function resolveWakeupPlanBucket(planClass?: string) {
  const upper = (planClass || '').trim().toUpperCase();
  if (!upper || upper === 'FREE') return 'FREE';
  if (upper.includes('ENTERPRISE')) return 'ENTERPRISE';
  if (upper.includes('TEAM') || upper.includes('BUSINESS') || upper.includes('EDU')) return 'TEAM';
  if (upper.includes('PLUS')) return 'PLUS';
  if (upper.includes('PRO')) return 'PRO';
  return 'OTHER';
}

function resolveWakeupQuotaBadges(
  presentation: ReturnType<typeof buildCodexAccountPresentation>,
): WakeupQuotaBadge[] {
  const standardQuotaItems = presentation.quotaItems.filter((item) => item.key !== 'code_review');
  const primary = standardQuotaItems.find((item) => item.key === 'primary') ?? standardQuotaItems[0];
  const secondary =
    standardQuotaItems.find((item) => item.key === 'secondary') ??
    standardQuotaItems.find((item) => item.key !== primary?.key);

  return [
    {
      key: 'primary',
      valueText: primary?.valueText ?? '--',
      quotaClass: primary?.quotaClass ?? 'unknown',
    },
    {
      key: 'secondary',
      valueText: secondary?.valueText ?? '--',
      quotaClass: secondary?.quotaClass ?? 'unknown',
    },
  ];
}

function createEmptyTaskDraft(defaultPreset?: CodexWakeupModelPreset | null): TaskDraft {
  const defaultReasoningEffort =
    defaultPreset?.allowed_reasoning_efforts.includes(DEFAULT_WAKEUP_REASONING_EFFORT)
      ? DEFAULT_WAKEUP_REASONING_EFFORT
      : (defaultPreset?.default_reasoning_effort ?? '');
  return {
    name: '',
    enabled: true,
    accountIds: [],
    prompt: '',
    modelPresetId: defaultPreset?.id ?? '',
    model: defaultPreset?.model ?? '',
    modelDisplayName: defaultPreset?.name ?? '',
    modelReasoningEffort: defaultReasoningEffort,
    scheduleKind: 'daily',
    dailyTime: '09:00',
    weeklyDays: [1, 2, 3, 4, 5],
    weeklyTime: '10:00',
    intervalHours: '6',
    quotaResetWindow: 'either',
    startupDelayMode: 'immediate',
    startupDelayMinutes: '1',
  };
}

function normalizeStartupDelayMinutes(value?: number): number {
  if (typeof value !== 'number' || Number.isNaN(value)) {
    return 0;
  }
  return Math.min(MAX_STARTUP_DELAY_MINUTES, Math.max(0, Math.floor(value)));
}

function createEmptyPresetDraft(): PresetDraft {
  return {
    name: '',
    model: '',
    allowedReasoningEfforts: ['low', 'medium', 'high', 'xhigh'],
    defaultReasoningEffort: 'medium',
  };
}

function buildPresetDraft(preset: CodexWakeupModelPreset): PresetDraft {
  return {
    id: preset.id,
    name: preset.name,
    model: preset.model,
    allowedReasoningEfforts: preset.allowed_reasoning_efforts,
    defaultReasoningEffort: preset.default_reasoning_effort,
  };
}

function resolveTaskPreset(task: CodexWakeupTask, presets: CodexWakeupModelPreset[]) {
  if (!task.model) return undefined;
  return (
    presets.find(
      (preset) => preset.model === task.model && preset.name === (task.model_display_name || preset.name),
    ) ||
    presets.find((preset) => preset.model === task.model)
  );
}

function buildTaskDraft(task: CodexWakeupTask, presets: CodexWakeupModelPreset[]): TaskDraft {
  const matchedPreset = resolveTaskPreset(task, presets);
  const startupDelayMinutes = normalizeStartupDelayMinutes(task.schedule.startup_delay_minutes);
  return {
    id: task.id,
    createdAt: task.created_at,
    name: task.name,
    enabled: task.enabled,
    accountIds: task.account_ids,
    prompt: task.prompt ?? '',
    modelPresetId: matchedPreset?.id ?? '',
    model: task.model ?? matchedPreset?.model ?? '',
    modelDisplayName: task.model_display_name ?? matchedPreset?.name ?? '',
    modelReasoningEffort:
      task.model_reasoning_effort ?? matchedPreset?.default_reasoning_effort ?? '',
    scheduleKind: task.schedule.kind,
    dailyTime: task.schedule.daily_time ?? '09:00',
    weeklyDays: task.schedule.weekly_days.length > 0 ? task.schedule.weekly_days : [1, 2, 3, 4, 5],
    weeklyTime: task.schedule.weekly_time ?? '10:00',
    intervalHours: String(task.schedule.interval_hours ?? 6),
    quotaResetWindow: task.schedule.quota_reset_window ?? 'either',
    startupDelayMode: startupDelayMinutes > 0 ? 'delayed' : 'immediate',
    startupDelayMinutes: String(startupDelayMinutes > 0 ? startupDelayMinutes : 1),
  };
}

function formatWakeupModelLabel(model?: string, modelDisplayName?: string) {
  if (modelDisplayName?.trim()) return modelDisplayName.trim();
  if (model?.trim()) return model.trim();
  return '';
}

function formatDateTime(value?: number) {
  if (!value) return '—';
  return new Date(value * 1000).toLocaleString();
}

function formatHistoryTimestamp(value?: number) {
  if (!value) return '—';
  return new Date(value).toLocaleString();
}

function formatDuration(value?: number) {
  if (!value && value !== 0) return '—';
  if (value < 1000) return `${value}ms`;
  return `${(value / 1000).toFixed(1)}s`;
}

function reasoningEffortLabel(
  value: CodexWakeupReasoningEffort | '',
  t: ReturnType<typeof useTranslation>['t'],
) {
  if (!value) return '—';
  return t(`codex.wakeup.reasoningEfforts.${value}`);
}

function formatTaskLastResult(
  task: CodexWakeupTask,
  t: ReturnType<typeof useTranslation>['t'],
) {
  const successCount = task.last_success_count ?? 0;
  const failureCount = task.last_failure_count ?? 0;
  if (successCount > 0 || failureCount > 0) {
    if (failureCount === 0) {
      return t('codex.wakeup.lastStatusSuccessSummary', { count: successCount });
    }
    if (successCount === 0) {
      return t('codex.wakeup.lastStatusFailedSummary', { count: failureCount });
    }
    return t('codex.wakeup.lastStatusMixedSummary', {
      success: successCount,
      failed: failureCount,
    });
  }
  if (task.last_status === 'success') {
    return t('common.success');
  }
  if (task.last_status === 'error') {
    return t('codex.wakeup.historyFailed');
  }
  return task.last_message || t('codex.wakeup.lastStatusIdle');
}

function executionStatusFromRecord(record: CodexWakeupHistoryItem): ExecutionRecordStatus {
  return record.success ? 'success' : 'error';
}

function quotaResetWindowLabel(
  value: CodexWakeupQuotaResetWindow | undefined,
  t: ReturnType<typeof useTranslation>['t'],
) {
  const normalized = value ?? 'either';
  return t(`codex.wakeup.quotaResetWindowOptions.${normalized}`);
}

function scheduleSummary(task: CodexWakeupTask, t: ReturnType<typeof useTranslation>['t']) {
  const schedule = task.schedule;
  if (schedule.kind === 'startup') {
    const delayMinutes = normalizeStartupDelayMinutes(schedule.startup_delay_minutes);
    if (delayMinutes <= 0) {
      return t('settings.general.startupWakeupImmediate');
    }
    return `${t('wakeup.triggerSource.startup')} +${delayMinutes}${t('settings.general.minutes')}`;
  }
  if (schedule.kind === 'daily') {
    return t('codex.wakeup.scheduleDailySummary', { time: schedule.daily_time || '09:00' });
  }
  if (schedule.kind === 'weekly') {
    const days = (schedule.weekly_days || [])
      .map((day) => t(`codex.wakeup.weekdays.${day}`))
      .join(' / ');
    return t('codex.wakeup.scheduleWeeklySummary', {
      days: days || t('codex.wakeup.weekdaysFallback'),
      time: schedule.weekly_time || '10:00',
    });
  }
  if (schedule.kind === 'quota_reset') {
    return t('codex.wakeup.scheduleQuotaResetSummary', {
      window: quotaResetWindowLabel(schedule.quota_reset_window, t),
    });
  }
  return t('codex.wakeup.scheduleIntervalSummary', {
    hours: schedule.interval_hours ?? 6,
  });
}

function triggerLabel(triggerType: string, t: ReturnType<typeof useTranslation>['t']) {
  if (triggerType === 'scheduled') return t('codex.wakeup.triggerScheduled');
  if (triggerType === 'quota_reset') return t('codex.wakeup.triggerQuotaReset');
  if (triggerType === 'startup') return t('wakeup.triggerSource.startup');
  if (triggerType === 'manual_task') return t('codex.wakeup.triggerManualTask');
  return t('codex.wakeup.triggerTest');
}

function executionStatusLabel(
  status: ExecutionRecordStatus,
  t: ReturnType<typeof useTranslation>['t'],
) {
  if (status === 'running') return t('codex.wakeup.executionStatusRunning');
  if (status === 'pending') return t('codex.wakeup.executionStatusPending');
  if (status === 'success') return t('codex.wakeup.historySuccess');
  return t('codex.wakeup.historyFailed');
}

function filterAllLabel(t: ReturnType<typeof useTranslation>['t']) {
  return t('common.shared.filter.all', { count: 0 }).replace(/\s*[（(]\s*\d+\s*[)）]\s*$/u, '');
}

function deriveStateEnabled(tasks: CodexWakeupTask[]) {
  return tasks.some((task) => task.enabled);
}

function formatSelectionPreview(values: string[], limit: number = 2) {
  if (values.length === 0) return '—';
  if (values.length <= limit) return values.join(' / ');
  return `${values.slice(0, limit).join(' / ')} +${values.length - limit}`;
}

function resolveDefaultWakeupPreset(presets: CodexWakeupModelPreset[]) {
  return (
    presets.find((preset) => preset.model.trim().toLowerCase() === DEFAULT_WAKEUP_MODEL) ||
    presets[0] ||
    null
  );
}

function resolveDefaultWakeupReasoningEffort(defaultPreset?: CodexWakeupModelPreset | null) {
  if (!defaultPreset) return '';
  if (defaultPreset.allowed_reasoning_efforts.includes(DEFAULT_WAKEUP_REASONING_EFFORT)) {
    return DEFAULT_WAKEUP_REASONING_EFFORT;
  }
  return (
    defaultPreset.default_reasoning_effort ||
    defaultPreset.allowed_reasoning_efforts[0] ||
    ''
  );
}

function resolveWakeupReasoningForPreset(
  preset: CodexWakeupModelPreset,
  preferred?: CodexWakeupReasoningEffort | '',
): CodexWakeupReasoningEffort | '' {
  if (
    preferred &&
    REASONING_EFFORT_OPTIONS.includes(preferred) &&
    preset.allowed_reasoning_efforts.includes(preferred)
  ) {
    return preferred;
  }
  if (preset.allowed_reasoning_efforts.includes(preset.default_reasoning_effort)) {
    return preset.default_reasoning_effort;
  }
  return preset.allowed_reasoning_efforts[0] || '';
}

function buildWakeupModelSelectionFromPreset(
  preset: CodexWakeupModelPreset,
  preferredReasoning?: CodexWakeupReasoningEffort | '',
): WakeupModelSelectionMemory {
  return {
    modelPresetId: preset.id,
    model: preset.model,
    modelDisplayName: preset.name,
    modelReasoningEffort: resolveWakeupReasoningForPreset(preset, preferredReasoning),
  };
}

function isWakeupModelSelectionEqual(
  left: WakeupModelSelectionMemory | null,
  right: WakeupModelSelectionMemory,
) {
  if (!left) return false;
  return (
    left.modelPresetId === right.modelPresetId &&
    left.model === right.model &&
    left.modelDisplayName === right.modelDisplayName &&
    left.modelReasoningEffort === right.modelReasoningEffort
  );
}

function readWakeupModelSelectionMemory(): WakeupModelSelectionMemory | null {
  try {
    const raw = localStorage.getItem(CODEX_WAKEUP_MODEL_SELECTION_STORAGE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<WakeupModelSelectionMemory> | null;
    if (!parsed || typeof parsed !== 'object') return null;
    const modelPresetId = (parsed.modelPresetId || '').trim();
    const model = (parsed.model || '').trim();
    const modelDisplayName = (parsed.modelDisplayName || '').trim();
    const rawReasoning = (parsed.modelReasoningEffort || '').trim();
    const modelReasoningEffort = REASONING_EFFORT_OPTIONS.includes(
      rawReasoning as CodexWakeupReasoningEffort,
    )
      ? (rawReasoning as CodexWakeupReasoningEffort)
      : '';
    if (!modelPresetId && !model && !modelDisplayName && !modelReasoningEffort) {
      return null;
    }
    return {
      modelPresetId,
      model,
      modelDisplayName,
      modelReasoningEffort,
    };
  } catch {
    return null;
  }
}

function persistWakeupModelSelectionMemory(selection: WakeupModelSelectionMemory): void {
  try {
    localStorage.setItem(CODEX_WAKEUP_MODEL_SELECTION_STORAGE_KEY, JSON.stringify(selection));
  } catch {
    // ignore storage write failures
  }
}

function WakeupSingleSelectDropdown({
  value,
  options,
  placeholder,
  onSelect,
  disabled = false,
}: {
  value: string;
  options: WakeupSingleSelectOption[];
  placeholder: string;
  onSelect: (value: string) => void;
  disabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const panelRef = useRef<HTMLDivElement | null>(null);
  const [panelPosition, setPanelPosition] = useState<{ top: number; left: number; width: number } | null>(null);
  const selectedOption = options.find((option) => option.value === value);

  useEffect(() => {
    if (!open || disabled) return;
    const updatePanelPosition = () => {
      const rect = rootRef.current?.getBoundingClientRect();
      if (!rect) {
        setPanelPosition(null);
        return;
      }
      setPanelPosition({
        top: rect.bottom + 8,
        left: rect.left,
        width: rect.width,
      });
    };
    updatePanelPosition();
    const handlePointerDown = (event: MouseEvent) => {
      const target = event.target as Node | null;
      if (!target) return;
      if (rootRef.current?.contains(target)) return;
      if (panelRef.current?.contains(target)) return;
      setOpen(false);
    };
    document.addEventListener('mousedown', handlePointerDown);
    window.addEventListener('resize', updatePanelPosition);
    window.addEventListener('scroll', updatePanelPosition, true);
    return () => {
      document.removeEventListener('mousedown', handlePointerDown);
      window.removeEventListener('resize', updatePanelPosition);
      window.removeEventListener('scroll', updatePanelPosition, true);
    };
  }, [disabled, open]);

  useEffect(() => {
    if (!disabled) return;
    setOpen(false);
  }, [disabled]);

  useEffect(() => {
    if (!open) {
      setPanelPosition(null);
    }
  }, [open]);

  const panel = open ? (
    <div
      ref={panelRef}
      className={`codex-wakeup-single-select-panel ${panelPosition ? 'codex-wakeup-single-select-panel-portal' : ''}`}
      style={
        panelPosition
          ? {
              position: 'fixed',
              top: `${panelPosition.top}px`,
              left: `${panelPosition.left}px`,
              width: `${panelPosition.width}px`,
              zIndex: 13060,
            }
          : undefined
      }
    >
      {options.map((option) => {
        const active = option.value === value;
        return (
          <button
            key={option.value}
            type="button"
            className={`codex-wakeup-single-select-option ${active ? 'active' : ''}`}
            onClick={() => {
              onSelect(option.value);
              setOpen(false);
            }}
          >
            <span className="codex-wakeup-single-select-option-main">
              <span>{option.label}</span>
            </span>
            {active ? <Check size={16} /> : null}
          </button>
        );
      })}
    </div>
  ) : null;

  return (
    <div
      className={`codex-wakeup-single-select ${open ? 'open' : ''} ${disabled ? 'disabled' : ''}`}
      ref={rootRef}
    >
      <button
        type="button"
        className={`codex-wakeup-single-select-trigger ${selectedOption ? 'selected' : ''}`}
        onClick={() => {
          if (disabled) return;
          setOpen((current) => !current);
        }}
        aria-expanded={open}
        disabled={disabled}
      >
        <span className="codex-wakeup-single-select-value">
          <span
            className={
              selectedOption ? 'codex-wakeup-single-select-text' : 'codex-wakeup-single-select-placeholder'
            }
          >
            {selectedOption?.label || placeholder}
          </span>
        </span>
        <ChevronDown
          size={16}
          className={`codex-wakeup-single-select-chevron ${open ? 'open' : ''}`}
        />
      </button>
      {open && typeof document !== 'undefined' && panelPosition
        ? createPortal(panel, document.body)
        : panel}
    </div>
  );
}

function resolveAccountContextText(
  account: CodexAccount,
  t: ReturnType<typeof useTranslation>['t'],
) {
  const metadata = getCodexAuthMetadata(account);
  const organizationId = (account.organization_id || '').trim();
  const matchedWorkspace = organizationId
    ? metadata.workspaces.find((workspace) => (workspace.id || '').trim() === organizationId)
    : null;
  const defaultWorkspace = metadata.workspaces.find((workspace) => workspace.is_default);
  const fallbackWorkspace = matchedWorkspace || defaultWorkspace || metadata.workspaces[0] || null;
  const workspaceTitle = fallbackWorkspace?.title?.trim() || '';
  const accountName = (account.account_name || '').trim();
  const structure = (account.account_structure || '').trim().toLowerCase();
  const isTeamLikePlan = isCodexTeamLikePlan(account.plan_type);
  const isPersonalStructure = structure.includes('personal');

  if (isPersonalStructure || (!structure && !isTeamLikePlan)) {
    return t('codex.account.personal', '个人账户');
  }

  return accountName || workspaceTitle || '';
}

function parseTimeValue(value: string) {
  const parts = value.trim().split(':');
  if (parts.length !== 2) return null;
  const hour = Number(parts[0]);
  const minute = Number(parts[1]);
  if (!Number.isFinite(hour) || !Number.isFinite(minute)) return null;
  if (hour < 0 || hour > 23 || minute < 0 || minute > 59) return null;
  return { hour, minute };
}

function atLocalTime(base: Date, time: string) {
  const parsed = parseTimeValue(time);
  if (!parsed) return null;
  const next = new Date(base);
  next.setHours(parsed.hour, parsed.minute, 0, 0);
  return next;
}

function calculatePreviewRuns(taskDraft: TaskDraft, count: number = 5) {
  const runs: Date[] = [];
  const now = new Date();

  if (taskDraft.scheduleKind === 'daily') {
    for (let offset = 0; offset < 14 && runs.length < count; offset += 1) {
      const date = new Date(now);
      date.setDate(now.getDate() + offset);
      const candidate = atLocalTime(date, taskDraft.dailyTime);
      if (candidate && candidate.getTime() > now.getTime()) {
        runs.push(candidate);
      }
    }
    return runs;
  }

  if (taskDraft.scheduleKind === 'weekly') {
    for (let offset = 0; offset < 21 && runs.length < count; offset += 1) {
      const date = new Date(now);
      date.setDate(now.getDate() + offset);
      if (!taskDraft.weeklyDays.includes(date.getDay())) continue;
      const candidate = atLocalTime(date, taskDraft.weeklyTime);
      if (candidate && candidate.getTime() > now.getTime()) {
        runs.push(candidate);
      }
    }
    return runs;
  }

  if (taskDraft.scheduleKind === 'quota_reset') {
    return runs;
  }

  if (taskDraft.scheduleKind === 'startup') {
    return runs;
  }

  const intervalHours = Math.max(1, Number(taskDraft.intervalHours) || 1);
  for (let index = 1; index <= count; index += 1) {
    runs.push(new Date(now.getTime() + intervalHours * index * 60 * 60 * 1000));
  }
  return runs;
}

export function CodexWakeupContent({
  accounts,
  onRefreshAccounts,
  openPresetManagerSignal = 0,
}: CodexWakeupContentProps) {
  const { t } = useTranslation();
  const {
    runtime,
    state,
    history,
    loading,
    saving,
    runningTaskId,
    testing,
    error,
    loadAll,
    refreshRuntime,
    saveState,
    runTask,
    runTest,
    cancelTestScope,
    releaseTestScope,
    clearHistory,
  } = useCodexWakeupStore();

  const oauthAccounts = useMemo(
    () => accounts.filter((account) => !isCodexApiKeyAccount(account)),
    [accounts],
  );
  const modelPresetMap = useMemo(
    () => new Map(state.model_presets.map((preset) => [preset.id, preset])),
    [state.model_presets],
  );
  const defaultModelPreset = useMemo(
    () => resolveDefaultWakeupPreset(state.model_presets),
    [state.model_presets],
  );
  const defaultModelReasoningEffort = useMemo(
    () => resolveDefaultWakeupReasoningEffort(defaultModelPreset),
    [defaultModelPreset],
  );
  const accountMap = useMemo(
    () => new Map(accounts.map((account) => [account.id, account])),
    [accounts],
  );
  const wakeupAccountMetaMap = useMemo(() => {
    const map = new Map<
      string,
      {
        email: string;
        contextText: string;
        planLabel: string;
        planClass: string;
        planBucket: string;
        quotaBadges: WakeupQuotaBadge[];
      }
    >();
    oauthAccounts.forEach((account) => {
      const presentation = buildCodexAccountPresentation(account, t);
      map.set(account.id, {
        email: (account.email || account.id).trim() || account.id,
        contextText: resolveAccountContextText(account, t),
        planLabel: presentation.planLabel,
        planClass: presentation.planClass || 'unknown',
        planBucket: resolveWakeupPlanBucket(presentation.planClass),
        quotaBadges: resolveWakeupQuotaBadges(presentation),
      });
    });
    return map;
  }, [oauthAccounts, t]);
  const wakeupAvailableTags = useMemo(() => {
    const uniqueTags = new Set<string>();
    oauthAccounts.forEach((account) => {
      (account.tags || [])
        .map((tag) => tag.trim())
        .filter(Boolean)
        .forEach((tag) => uniqueTags.add(tag));
    });
    return Array.from(uniqueTags).sort((left, right) => left.localeCompare(right));
  }, [oauthAccounts]);
  const wakeupTierCounts = useMemo(() => {
    const counts = {
      all: oauthAccounts.length,
      FREE: 0,
      PLUS: 0,
      PRO: 0,
      TEAM: 0,
      ENTERPRISE: 0,
      OTHER: 0,
    };
    oauthAccounts.forEach((account) => {
      const bucket = wakeupAccountMetaMap.get(account.id)?.planBucket || 'FREE';
      if (bucket in counts) {
        counts[bucket as keyof typeof counts] += 1;
      }
    });
    return counts;
  }, [oauthAccounts, wakeupAccountMetaMap]);
  const wakeupTierFilterOptions = useMemo<MultiSelectFilterOption[]>(() => {
    const options: MultiSelectFilterOption[] = [
      { value: 'FREE', label: `FREE (${wakeupTierCounts.FREE})` },
      { value: 'PLUS', label: `PLUS (${wakeupTierCounts.PLUS})` },
      { value: 'PRO', label: `PRO (${wakeupTierCounts.PRO})` },
      { value: 'TEAM', label: `TEAM (${wakeupTierCounts.TEAM})` },
      { value: 'ENTERPRISE', label: `ENTERPRISE (${wakeupTierCounts.ENTERPRISE})` },
    ];
    if (wakeupTierCounts.OTHER > 0) {
      options.push({ value: 'OTHER', label: `OTHER (${wakeupTierCounts.OTHER})` });
    }
    return options;
  }, [wakeupTierCounts]);
  const [modelSelectionMemory, setModelSelectionMemory] = useState<WakeupModelSelectionMemory | null>(() =>
    readWakeupModelSelectionMemory(),
  );
  const resolvedModelSelection = useMemo<WakeupModelSelectionMemory>(() => {
    if (!defaultModelPreset) {
      return {
        modelPresetId: '',
        model: '',
        modelDisplayName: '',
        modelReasoningEffort: '',
      };
    }
    const rememberedPresetId = (modelSelectionMemory?.modelPresetId || '').trim();
    const rememberedModel = (modelSelectionMemory?.model || '').trim();
    const rememberedPreset =
      (rememberedPresetId ? modelPresetMap.get(rememberedPresetId) : undefined) ||
      (rememberedModel
        ? state.model_presets.find((preset) => preset.model.trim() === rememberedModel)
        : undefined) ||
      defaultModelPreset;
    return buildWakeupModelSelectionFromPreset(
      rememberedPreset,
      modelSelectionMemory?.modelReasoningEffort || defaultModelReasoningEffort,
    );
  }, [
    defaultModelPreset,
    defaultModelReasoningEffort,
    modelPresetMap,
    modelSelectionMemory,
    state.model_presets,
  ]);

  const [notice, setNotice] = useState<{ tone: 'success' | 'error'; text: string } | null>(null);
  const [showTaskModal, setShowTaskModal] = useState(false);
  const [taskDraft, setTaskDraft] = useState<TaskDraft>(createEmptyTaskDraft(defaultModelPreset));
  const {
    message: taskModalError,
    scrollKey: taskModalErrorScrollKey,
    set: setTaskModalError,
  } = useModalErrorState();
  const [taskAccountFilters, setTaskAccountFilters] = useState<AccountPickerFilters>(createEmptyAccountPickerFilters());
  const [showPresetModal, setShowPresetModal] = useState(false);
  const [presetModalSource, setPresetModalSource] = useState<'task' | 'test' | 'page'>('page');
  const [presetDraft, setPresetDraft] = useState<PresetDraft>(createEmptyPresetDraft());
  const {
    message: presetModalError,
    scrollKey: presetModalErrorScrollKey,
    set: setPresetModalError,
    clear: clearPresetModalError,
  } = useModalErrorState();

  useEffect(() => {
    if (openPresetManagerSignal <= 0) return;
    setPresetDraft(createEmptyPresetDraft());
    clearPresetModalError();
    setPresetModalSource('page');
    setShowPresetModal(true);
  }, [clearPresetModalError, openPresetManagerSignal]);
  const [showTestModal, setShowTestModal] = useState(false);
  const [showHistoryModal, setShowHistoryModal] = useState(false);
  const [testAccountIds, setTestAccountIds] = useState<string[]>([]);
  const [testPrompt, setTestPrompt] = useState('');
  const [testModelPresetId, setTestModelPresetId] = useState(defaultModelPreset?.id ?? '');
  const [testModel, setTestModel] = useState(defaultModelPreset?.model ?? '');
  const [testModelReasoningEffort, setTestModelReasoningEffort] = useState<
    CodexWakeupReasoningEffort | ''
  >(defaultModelReasoningEffort);
  const {
    message: testModalError,
    scrollKey: testModalErrorScrollKey,
    set: setTestModalError,
  } = useModalErrorState();
  const [testAccountFilters, setTestAccountFilters] = useState<AccountPickerFilters>(createEmptyAccountPickerFilters());
  const activeTestRunTokenRef = useRef(0);
  const activeTestScopeIdRef = useRef<string | null>(null);
  const [executionSession, setExecutionSession] = useState<ExecutionSessionState | null>(null);
  const [executionSessionFromHistory, setExecutionSessionFromHistory] = useState(false);
  const [executionFilter, setExecutionFilter] = useState<ExecutionRecordFilter>('all');
  const [copiedCommand, setCopiedCommand] = useState<string | null>(null);
  const [showRuntimeGuideModal, setShowRuntimeGuideModal] = useState(false);
  const [runtimeGuideRefreshing, setRuntimeGuideRefreshing] = useState(false);
  const [runtimeGuideAutoShown, setRuntimeGuideAutoShown] = useState(false);
  const [runtimeConfigDraft, setRuntimeConfigDraft] = useState<RuntimeConfigDraft>(() =>
    createRuntimeConfigDraft(runtime),
  );
  const [runtimeConfigDirty, setRuntimeConfigDirty] = useState(false);
  const [runtimeGuideConfigError, setRuntimeGuideConfigError] = useState<string | null>(null);
  const showCodexCliInput = true;
  const showNodeInput = true;
  const showRuntimeConfigCard = true;
  const runtimeGuideNeedInstall = Boolean(runtime && !runtime.available);
  const runtimeGuideTitle = runtimeGuideNeedInstall
    ? t('codex.wakeup.installTitle')
    : t('codex.wakeup.runtimeConfigTitle');
  const rememberModelSelection = useCallback((selection: WakeupModelSelectionMemory) => {
    setModelSelectionMemory(selection);
    persistWakeupModelSelectionMemory(selection);
  }, []);
  const createEmptyTaskDraftWithRememberedModel = useCallback(() => {
    const draft = createEmptyTaskDraft(defaultModelPreset);
    if (!resolvedModelSelection.modelPresetId) {
      return draft;
    }
    return {
      ...draft,
      modelPresetId: resolvedModelSelection.modelPresetId,
      model: resolvedModelSelection.model,
      modelDisplayName: resolvedModelSelection.modelDisplayName,
      modelReasoningEffort: resolvedModelSelection.modelReasoningEffort,
    };
  }, [defaultModelPreset, resolvedModelSelection]);
  const [privacyModeEnabled, setPrivacyModeEnabled] = useState<boolean>(() =>
    isPrivacyModeEnabledByDefault(),
  );
  const maskAccountText = useCallback(
    (value?: string | null) => maskSensitiveValue(value, privacyModeEnabled),
    [privacyModeEnabled],
  );

  useEffect(() => {
    void loadAll();
  }, [loadAll]);

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
    if (error) {
      if (showTaskModal) {
        setTaskModalError(error);
      } else if (showTestModal) {
        setTestModalError(error);
      } else if (showRuntimeGuideModal) {
        setRuntimeGuideConfigError(error);
      } else if (executionSession) {
        setExecutionSession((current) =>
          current ? { ...current, running: false, errorText: error } : current,
        );
      } else {
        setNotice({ tone: 'error', text: error });
      }
    }
  }, [error, executionSession, showRuntimeGuideModal, showTaskModal, showTestModal, setTaskModalError, setTestModalError]);

  useEffect(() => {
    if (loading || runtime === null) {
      return;
    }
    if (runtime.available) {
      setRuntimeGuideAutoShown(false);
      return;
    }
    if (!runtimeGuideAutoShown) {
      setShowRuntimeGuideModal(true);
      setRuntimeGuideAutoShown(true);
    }
  }, [loading, runtime, runtimeGuideAutoShown]);

  useEffect(() => {
    if (!runtime || runtimeConfigDirty) {
      return;
    }
    setRuntimeConfigDraft(createRuntimeConfigDraft(runtime));
  }, [runtime, runtimeConfigDirty]);

  useEffect(() => {
    if (!resolvedModelSelection.modelPresetId) {
      return;
    }
    if (!isWakeupModelSelectionEqual(modelSelectionMemory, resolvedModelSelection)) {
      setModelSelectionMemory(resolvedModelSelection);
      persistWakeupModelSelectionMemory(resolvedModelSelection);
    }
    setTaskDraft((current) => {
      if (current.model || current.modelPresetId) {
        return current;
      }
      return {
        ...current,
        modelPresetId: resolvedModelSelection.modelPresetId,
        model: resolvedModelSelection.model,
        modelDisplayName: resolvedModelSelection.modelDisplayName,
        modelReasoningEffort: resolvedModelSelection.modelReasoningEffort,
      };
    });
    setTestModelPresetId((current) => current || resolvedModelSelection.modelPresetId);
    setTestModel((current) => current || resolvedModelSelection.model);
    setTestModelReasoningEffort((current) => current || resolvedModelSelection.modelReasoningEffort);
  }, [modelSelectionMemory, resolvedModelSelection]);

  const selectedTaskPreset = useMemo(
    () => (taskDraft.modelPresetId ? modelPresetMap.get(taskDraft.modelPresetId) : undefined),
    [modelPresetMap, taskDraft.modelPresetId],
  );
  const selectedTestPreset = useMemo(
    () => (testModelPresetId ? modelPresetMap.get(testModelPresetId) : undefined),
    [modelPresetMap, testModelPresetId],
  );
  const taskAllowedReasoningEfforts = selectedTaskPreset?.allowed_reasoning_efforts ?? [];
  const testAllowedReasoningEfforts = selectedTestPreset?.allowed_reasoning_efforts ?? [];
  const modelPresetOptions = useMemo<WakeupSingleSelectOption[]>(
    () =>
      state.model_presets.map((preset) => ({
        value: preset.id,
        label: preset.name,
      })),
    [state.model_presets],
  );
  const taskReasoningOptions = useMemo<WakeupSingleSelectOption[]>(
    () =>
      taskAllowedReasoningEfforts.map((effort) => ({
        value: effort,
        label: reasoningEffortLabel(effort, t),
      })),
    [t, taskAllowedReasoningEfforts],
  );
  const testReasoningOptions = useMemo<WakeupSingleSelectOption[]>(
    () =>
      testAllowedReasoningEfforts.map((effort) => ({
        value: effort,
        label: reasoningEffortLabel(effort, t),
      })),
    [t, testAllowedReasoningEfforts],
  );

  const sortedTasks = useMemo(() => {
    const tasks = [...state.tasks];
    tasks.sort((left, right) => {
      if (left.enabled !== right.enabled) {
        return left.enabled ? -1 : 1;
      }
      const leftNext = left.next_run_at ?? Number.MAX_SAFE_INTEGER;
      const rightNext = right.next_run_at ?? Number.MAX_SAFE_INTEGER;
      if (leftNext !== rightNext) {
        return leftNext - rightNext;
      }
      return right.updated_at - left.updated_at;
    });
    return tasks;
  }, [state.tasks]);

  const historyBatches = useMemo<HistoryBatchSummary[]>(() => {
    const grouped = new Map<string, CodexWakeupHistoryItem[]>();
    history.forEach((item) => {
      const runId = item.run_id || item.id;
      const bucket = grouped.get(runId);
      if (bucket) {
        bucket.push(item);
      } else {
        grouped.set(runId, [item]);
      }
    });

    return Array.from(grouped.entries())
      .map(([runId, records]) => {
        const sorted = [...records].sort((left, right) => left.timestamp - right.timestamp);
        const latest = sorted[sorted.length - 1];
        const durationMs = sorted.reduce((sum, item) => sum + (item.duration_ms || 0), 0);
        const successCount = sorted.filter((item) => item.success).length;
        return {
          runId,
          triggerType: latest?.trigger_type || 'test',
          taskId: latest?.task_id,
          taskName: latest?.task_name,
          timestamp: latest?.timestamp || 0,
          total: sorted.length,
          successCount,
          failureCount: sorted.length - successCount,
          durationMs: durationMs > 0 ? durationMs : undefined,
          cliPath: sorted.find((item) => item.cli_path)?.cli_path,
          records: sorted.reverse(),
        };
      })
      .sort((left, right) => right.timestamp - left.timestamp);
  }, [history]);

  const buildExecutionSession = useCallback(
    (
      runId: string,
      triggerType: string,
      accountIds: string[],
      prompt?: string,
      taskId?: string,
      taskName?: string,
      model?: string,
      modelDisplayName?: string,
      modelReasoningEffort?: CodexWakeupReasoningEffort,
    ): ExecutionSessionState => ({
      runId,
      taskId,
      triggerType,
      title:
        triggerType === 'test'
          ? t('codex.wakeup.testTitle')
          : taskName || t('codex.wakeup.resultsTitle'),
      runtime: runtime,
      startedAt: Date.now(),
      durationMs: undefined,
      total: accountIds.length,
      completed: 0,
      successCount: 0,
      failureCount: 0,
      taskName,
      running: true,
      preview: false,
      errorText: undefined,
      records: accountIds.map((accountId, index) => {
        const account = accountMap.get(accountId);
        const meta = wakeupAccountMetaMap.get(accountId);
        return {
          id: `${runId}-${accountId}-${index}`,
          accountId,
          accountEmail: meta?.email || (account?.email || accountId),
          accountContextText: meta?.contextText || (account ? resolveAccountContextText(account, t) : undefined),
          triggerType,
          status: 'pending',
          prompt,
          model,
          modelDisplayName,
          modelReasoningEffort,
        };
      }),
    }),
    [accountMap, runtime, t, wakeupAccountMetaMap],
  );

  const buildExecutionSessionFromHistory = useCallback(
    (batch: HistoryBatchSummary): ExecutionSessionState => ({
      runId: batch.runId,
      taskId: batch.taskId,
      triggerType: batch.triggerType,
      title:
        batch.taskName ||
        (batch.triggerType === 'test'
          ? t('codex.wakeup.testTitle')
          : t('codex.wakeup.resultsTitle')),
      runtime: batch.cliPath
        ? {
            available: true,
            binary_path: batch.cliPath,
            checked_at: batch.timestamp,
            required_runtime_paths: [],
            install_hints: [],
          }
        : runtime,
      startedAt: batch.timestamp,
      durationMs: batch.durationMs,
      total: batch.total,
      completed: batch.total,
      successCount: batch.successCount,
      failureCount: batch.failureCount,
      taskName: batch.taskName,
      running: false,
      preview: false,
      errorText: undefined,
      records: batch.records.map((item) => ({
        id: item.id,
        accountId: item.account_id,
        accountEmail: item.account_email,
        accountContextText: item.account_context_text,
        triggerType: item.trigger_type,
        status: executionStatusFromRecord(item),
        prompt: item.prompt,
        model: item.model,
        modelDisplayName: item.model_display_name,
        modelReasoningEffort: item.model_reasoning_effort,
        reply: item.reply,
        error: item.error,
        timestamp: item.timestamp,
        durationMs: item.duration_ms,
      })),
    }),
    [runtime, t],
  );

  const buildTaskPreviewSession = useCallback(
    (task: CodexWakeupTask): ExecutionSessionState => ({
      runId: `preview:${task.id}`,
      taskId: task.id,
      triggerType: 'scheduled',
      title: task.name,
      runtime: runtime,
      startedAt: 0,
      durationMs: undefined,
      total: task.account_ids.length,
      completed: 0,
      successCount: 0,
      failureCount: 0,
      taskName: task.name,
      running: false,
      preview: true,
      errorText: undefined,
      records: task.account_ids.map((accountId, index) => {
        const account = accountMap.get(accountId);
        const meta = wakeupAccountMetaMap.get(accountId);
        return {
          id: `preview:${task.id}:${accountId}:${index}`,
          accountId,
          accountEmail: meta?.email || (account?.email || accountId),
          accountContextText:
            meta?.contextText || (account ? resolveAccountContextText(account, t) : undefined),
          triggerType: 'scheduled',
          status: 'pending' as const,
          prompt: task.prompt,
          model: task.model,
          modelDisplayName: task.model_display_name,
          modelReasoningEffort: task.model_reasoning_effort,
        };
      }),
    }),
    [accountMap, runtime, t, wakeupAccountMetaMap],
  );

  const openTaskExecutionDetails = useCallback(
    (task: CodexWakeupTask) => {
      setExecutionSession((current) => {
        if (current?.taskId === task.id) {
          return current;
        }
        return buildTaskPreviewSession(task);
      });
    },
    [buildTaskPreviewSession],
  );

  useEffect(() => {
    setExecutionFilter('all');
  }, [executionSession?.runId]);

  const applyProgressPayload = useCallback((payload: CodexWakeupProgressPayload) => {
    setExecutionSession((current) => {
      if (!current) {
        return current;
      }

      const sameRun = current.runId === payload.run_id;
      const attachPreview =
        current.preview && !!payload.task_id && !!current.taskId && current.taskId === payload.task_id;

      if (!sameRun && !attachPreview) {
        return current;
      }

      const nextRecords = current.records.map<ExecutionRecordState>((record) => {
        if (payload.current_account_id && record.accountId === payload.current_account_id) {
          if (payload.phase === 'account_started' && record.status === 'pending') {
            return { ...record, status: 'running' as const };
          }
          if (payload.phase === 'account_completed' && payload.item) {
            return {
              ...record,
              accountEmail: payload.item.account_email || record.accountEmail,
              accountContextText: payload.item.account_context_text || record.accountContextText,
              status: executionStatusFromRecord(payload.item),
              prompt: payload.item.prompt || record.prompt,
              model: payload.item.model || record.model,
              modelDisplayName: payload.item.model_display_name || record.modelDisplayName,
              modelReasoningEffort:
                payload.item.model_reasoning_effort || record.modelReasoningEffort,
              reply: payload.item.reply,
              error: payload.item.error,
              timestamp: payload.item.timestamp,
              durationMs: payload.item.duration_ms,
              triggerType: payload.item.trigger_type || record.triggerType,
            };
          }
        }

        if (payload.phase === 'account_started' && record.status === 'running') {
          return { ...record, status: 'pending' as const };
        }

        return record;
      });

      return {
        ...current,
        runId: payload.run_id,
        triggerType: payload.trigger_type || current.triggerType,
        taskId: payload.task_id || current.taskId,
        taskName: payload.task_name || current.taskName,
        total: payload.total,
        completed: payload.completed,
        successCount: payload.success_count,
        failureCount: payload.failure_count,
        running: payload.running,
        preview: false,
        records: nextRecords,
      };
    });
  }, []);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;

    void listen<Record<string, unknown>>('codex://wakeup-progress', (event) => {
      const payload = fromRawWakeupProgressPayload(event.payload as never);
      applyProgressPayload(payload);
      if (payload.phase === 'batch_completed') {
        void loadAll();
      }
    }).then((fn) => {
      unlisten = fn;
    });

    return () => {
      if (unlisten) {
        unlisten();
      }
    };
  }, [applyProgressPayload, loadAll]);

  const previewRuns = useMemo(() => calculatePreviewRuns(taskDraft), [taskDraft]);
  const executionCounts = useMemo(() => {
    if (!executionSession) {
      return {
        pending: 0,
        running: 0,
      };
    }
    return {
      pending: executionSession.records.filter((item) => item.status === 'pending').length,
      running: executionSession.records.filter((item) => item.status === 'running').length,
    };
  }, [executionSession]);
  const filteredExecutionRecords = useMemo(() => {
    if (!executionSession) return [];
    if (executionFilter === 'all') return executionSession.records;
    return executionSession.records.filter((record) => record.status === executionFilter);
  }, [executionFilter, executionSession]);
  const executionDuration = useMemo(() => {
    if (!executionSession) return undefined;
    if (executionSession.durationMs !== undefined) return executionSession.durationMs;
    const totalDuration = executionSession.records.reduce(
      (sum, item) => sum + (item.durationMs || 0),
      0,
    );
    if (totalDuration > 0) return totalDuration;
    return undefined;
  }, [executionSession]);
  const executionFilterOptions = useMemo(
    () =>
      executionSession
        ? [
            {
              key: 'all' as const,
              label: filterAllLabel(t),
              count: executionSession.total,
              tone: 'all' as const,
            },
            {
              key: 'success' as const,
              label: t('codex.wakeup.resultsSuccess'),
              count: executionSession.successCount,
              tone: 'success' as const,
            },
            {
              key: 'error' as const,
              label: t('codex.wakeup.resultsFailed'),
              count: executionSession.failureCount,
              tone: 'error' as const,
            },
            {
              key: 'pending' as const,
              label: t('codex.wakeup.executionStatusPending'),
              count: executionCounts.pending,
              tone: 'pending' as const,
            },
            {
              key: 'running' as const,
              label: t('codex.wakeup.executionStatusRunning'),
              count: executionCounts.running,
              tone: 'running' as const,
            },
          ]
        : [],
    [executionCounts.pending, executionCounts.running, executionSession, t],
  );
  const filterWakeupAccounts = useCallback(
    (filters: AccountPickerFilters) => {
      const query = filters.query.trim().toLowerCase();
      const selectedPlanTypes = new Set(filters.planTypes);
      const selectedTags = new Set(filters.tags.map(normalizeWakeupTag));

      return oauthAccounts.filter((account) => {
        const meta = wakeupAccountMetaMap.get(account.id);
        const email = (meta?.email || account.email || account.id).toLowerCase();
        if (query && !email.includes(query)) {
          return false;
        }
        if (selectedPlanTypes.size > 0 && !selectedPlanTypes.has(meta?.planBucket || 'FREE')) {
          return false;
        }
        if (selectedTags.size > 0) {
          const accountTags = (account.tags || []).map(normalizeWakeupTag).filter(Boolean);
          if (!accountTags.some((tag) => selectedTags.has(tag))) {
            return false;
          }
        }
        return true;
      });
    },
    [oauthAccounts, wakeupAccountMetaMap],
  );
  const filteredTaskAccounts = useMemo(
    () => filterWakeupAccounts(taskAccountFilters),
    [filterWakeupAccounts, taskAccountFilters],
  );
  const filteredTestAccounts = useMemo(
    () => filterWakeupAccounts(testAccountFilters),
    [filterWakeupAccounts, testAccountFilters],
  );
  const allFilteredTaskSelected = useMemo(
    () =>
      filteredTaskAccounts.length > 0 &&
      filteredTaskAccounts.every((account) => taskDraft.accountIds.includes(account.id)),
    [filteredTaskAccounts, taskDraft.accountIds],
  );
  const allFilteredTestSelected = useMemo(
    () =>
      filteredTestAccounts.length > 0 &&
      filteredTestAccounts.every((account) => testAccountIds.includes(account.id)),
    [filteredTestAccounts, testAccountIds],
  );

  const hasEnabledQuotaResetTask = useCallback(
    (tasks: CodexWakeupTask[]) =>
      tasks.some((task) => task.enabled && task.schedule.kind === 'quota_reset'),
    [],
  );

  const ensureCodexRefreshIntervalForQuotaReset = useCallback(async () => {
    const config = await invoke<CodexWakeupGeneralConfig>('get_general_config');
    if (config.codex_auto_refresh_minutes === QUOTA_RESET_MIN_REFRESH_MINUTES) {
      return false;
    }
    await invoke('save_general_config', {
      language: config.language,
      theme: config.theme,
      autoRefreshMinutes: config.auto_refresh_minutes,
      codexAutoRefreshMinutes: QUOTA_RESET_MIN_REFRESH_MINUTES,
      closeBehavior: config.close_behavior || 'ask',
      opencodeAppPath: config.opencode_app_path ?? '',
      antigravityAppPath: config.antigravity_app_path ?? '',
      codexAppPath: config.codex_app_path ?? '',
      vscodeAppPath: config.vscode_app_path ?? '',
      opencodeSyncOnSwitch: config.opencode_sync_on_switch ?? false,
      codexLaunchOnSwitch: config.codex_launch_on_switch ?? true,
    });
    window.dispatchEvent(new Event('config-updated'));
    return true;
  }, []);

  const copyCommand = useCallback(async (command: string) => {
    await navigator.clipboard.writeText(command);
    setCopiedCommand(command);
    window.setTimeout(() => setCopiedCommand(null), 1500);
  }, []);

  const renderInstallCommands = useCallback(
    (commands: { label: string; command: string }[]) => {
      if (commands.length === 0) return null;
      return (
        <div className="codex-wakeup-install-grid">
          {commands.map((hint) => (
            <div key={`${hint.label}:${hint.command}`} className="codex-wakeup-install-command">
              <div className="codex-wakeup-install-command-head">
                <span>{hint.label}</span>
                <button className="btn btn-secondary" onClick={() => void copyCommand(hint.command)}>
                  {copiedCommand === hint.command ? <Check size={14} /> : <Copy size={14} />}
                  {copiedCommand === hint.command ? t('codex.wakeup.copied') : t('codex.wakeup.copyCommand')}
                </button>
              </div>
              <div className="codex-wakeup-install-terminal">
                <code>{hint.command}</code>
              </div>
            </div>
          ))}
        </div>
      );
    },
    [copiedCommand, copyCommand, t],
  );

  const renderWakeupAccountOption = useCallback(
    (account: CodexAccount, checked: boolean, onToggle: () => void) => {
      const presentation = buildCodexAccountPresentation(account, t);
      const meta = wakeupAccountMetaMap.get(account.id) ?? {
        email: (account.email || account.id).trim() || account.id,
        contextText: resolveAccountContextText(account, t),
        planLabel: presentation.planLabel,
        planClass: presentation.planClass || 'unknown',
        planBucket: resolveWakeupPlanBucket(presentation.planClass),
        quotaBadges: resolveWakeupQuotaBadges(presentation),
      };
      const maskedEmail = maskAccountText(meta.email);
      return (
        <button
          key={account.id}
          type="button"
          className={`wakeup-chip codex-wakeup-account-chip ${checked ? 'selected' : ''}`}
          onClick={onToggle}
          title={[maskedEmail, meta.planLabel, meta.contextText].filter(Boolean).join(' · ')}
        >
          <div className="codex-wakeup-account-chip-head">
            <span className="codex-wakeup-account-chip-email">{maskedEmail}</span>
            <span className={`tier-badge ${meta.planClass}`}>{meta.planLabel}</span>
          </div>
          <div className="codex-wakeup-account-chip-meta">
            {meta.contextText && (
              <span className="codex-wakeup-account-chip-context">{meta.contextText}</span>
            )}
            <div className="codex-wakeup-account-chip-quotas">
              {meta.quotaBadges.map((item) => (
                <span
                  key={`${account.id}-${item.key}`}
                  className={`codex-wakeup-account-chip-quota codex-wakeup-account-chip-quota-${item.key}`}
                >
                  <span className="codex-wakeup-account-chip-quota-dot" />
                  <span className={`codex-wakeup-account-chip-quota-value ${item.quotaClass}`}>
                    {item.valueText}
                  </span>
                </span>
              ))}
            </div>
          </div>
        </button>
      );
    },
    [maskAccountText, t, wakeupAccountMetaMap],
  );

  const renderAccountPickerFilters = useCallback(
    (
      filters: AccountPickerFilters,
      setFilters: Dispatch<SetStateAction<AccountPickerFilters>>,
      filteredAccounts: CodexAccount[],
      allSelected: boolean,
      onToggleSelectAll: () => void,
    ) => (
      <>
        <div className="codex-wakeup-account-filter-toolbar">
          <label className="codex-wakeup-account-search">
            <Search size={16} className="codex-wakeup-account-search-icon" />
            <input
              type="text"
              value={filters.query}
              onChange={(event) =>
                setFilters((current) => ({ ...current, query: event.target.value }))
              }
              placeholder={t('codex.wakeup.accountSearchPlaceholder')}
            />
          </label>
          <div className="codex-wakeup-account-filter-actions">
            <MultiSelectFilterDropdown
              options={wakeupTierFilterOptions}
              selectedValues={filters.planTypes}
              allLabel={t('common.shared.filter.all', { count: wakeupTierCounts.all })}
              filterLabel={t('common.shared.filterLabel', '筛选')}
              clearLabel={t('accounts.clearFilter', '清空筛选')}
              emptyLabel={t('common.none', '暂无')}
              ariaLabel={t('common.shared.filterLabel', '筛选')}
              onToggleValue={(value) =>
                setFilters((current) => ({
                  ...current,
                  planTypes: toggleStringSelection(current.planTypes, value),
                }))
              }
              onClear={() =>
                setFilters((current) => ({
                  ...current,
                  planTypes: [],
                }))
              }
            />
            <AccountTagFilterDropdown
              availableTags={wakeupAvailableTags}
              selectedTags={filters.tags}
              onToggleTag={(tag) =>
                setFilters((current) => ({
                  ...current,
                  tags: toggleStringSelection(current.tags, tag),
                }))
              }
              onClear={() =>
                setFilters((current) => ({
                  ...current,
                  tags: [],
                }))
              }
            />
          </div>
        </div>
        <div className="codex-wakeup-account-selection-bar">
          <span className="codex-wakeup-account-selection-summary">
            {filteredAccounts.length === 0
              ? t('codex.wakeup.accountFilterEmpty')
              : t('codex.wakeup.accountFilteredCount', { count: filteredAccounts.length })}
          </span>
          <button
            type="button"
            className="btn btn-secondary"
            onClick={onToggleSelectAll}
            disabled={filteredAccounts.length === 0}
          >
            {allSelected
              ? `${t('wakeup.verification.actions.clearSelectedAccounts')} (${filteredAccounts.length})`
              : `${t('wakeup.verification.actions.selectAllAccounts')} (${filteredAccounts.length})`}
          </button>
        </div>
      </>
    ),
    [t, wakeupAvailableTags, wakeupTierCounts.all, wakeupTierFilterOptions],
  );

  const openRuntimeGuideModal = useCallback(() => {
    setRuntimeGuideConfigError(null);
    setRuntimeConfigDirty(false);
    setRuntimeConfigDraft(createRuntimeConfigDraft(runtime));
    setShowRuntimeGuideModal(true);
  }, [runtime]);

  const closeRuntimeGuideModal = useCallback(() => {
    if (runtimeGuideRefreshing) return;
    setShowRuntimeGuideModal(false);
    setRuntimeGuideConfigError(null);
  }, [runtimeGuideRefreshing]);

  const handleRuntimeConfigInputChange = useCallback((field: keyof RuntimeConfigDraft, value: string) => {
    setRuntimeConfigDirty(true);
    setRuntimeGuideConfigError(null);
    setRuntimeConfigDraft((current) => ({
      ...current,
      [field]: value,
    }));
  }, []);

  const handleRefreshRuntimeGuide = useCallback(async () => {
    setRuntimeGuideRefreshing(true);
    setRuntimeGuideConfigError(null);
    try {
      await refreshRuntime({
        codexCliPath: runtimeConfigDraft.codexCliPath.trim(),
        nodePath: runtimeConfigDraft.nodePath.trim(),
      });
      setRuntimeConfigDirty(false);
    } catch (error) {
      setRuntimeGuideConfigError(String(error));
    } finally {
      setRuntimeGuideRefreshing(false);
    }
  }, [refreshRuntime, runtimeConfigDraft.codexCliPath, runtimeConfigDraft.nodePath]);

  const openPresetModal = useCallback((source: 'task' | 'test' | 'page' = 'page') => {
    setPresetDraft(createEmptyPresetDraft());
    setPresetModalError(null);
    setPresetModalSource(source);
    setShowPresetModal(true);
  }, [setPresetModalError]);

  const closePresetModal = useCallback(() => {
    if (saving) return;
    setShowPresetModal(false);
    setPresetDraft(createEmptyPresetDraft());
    setPresetModalError(null);
  }, [saving, setPresetModalError]);

  const handleSelectTaskPreset = useCallback(
    (presetId: string) => {
      const preset = modelPresetMap.get(presetId);
      if (!preset) {
        setTaskDraft((current) => ({
          ...current,
          modelPresetId: '',
          model: '',
          modelDisplayName: '',
          modelReasoningEffort: '',
        }));
        return;
      }
      const nextReasoning = resolveWakeupReasoningForPreset(preset, taskDraft.modelReasoningEffort);
      setTaskDraft((current) => ({
        ...current,
        modelPresetId: preset.id,
        model: preset.model,
        modelDisplayName: preset.name,
        modelReasoningEffort: nextReasoning,
      }));
      rememberModelSelection(buildWakeupModelSelectionFromPreset(preset, nextReasoning));
    },
    [modelPresetMap, rememberModelSelection, taskDraft.modelReasoningEffort],
  );

  const handleSelectTestPreset = useCallback(
    (presetId: string) => {
      const preset = modelPresetMap.get(presetId);
      if (!preset) {
        setTestModelPresetId('');
        setTestModel('');
        setTestModelReasoningEffort('');
        return;
      }
      const nextReasoning = resolveWakeupReasoningForPreset(preset, testModelReasoningEffort);
      setTestModelPresetId(preset.id);
      setTestModel(preset.model);
      setTestModelReasoningEffort(nextReasoning);
      rememberModelSelection(buildWakeupModelSelectionFromPreset(preset, nextReasoning));
    },
    [modelPresetMap, rememberModelSelection, testModelReasoningEffort],
  );

  const handleSavePreset = useCallback(async () => {
    const trimmedName = presetDraft.name.trim();
    const trimmedModel = presetDraft.model.trim();
    if (!trimmedName) {
      setPresetModalError(t('codex.wakeup.presetNameRequired'));
      return;
    }
    if (!trimmedModel) {
      setPresetModalError(t('codex.wakeup.presetModelRequired'));
      return;
    }
    if (presetDraft.allowedReasoningEfforts.length === 0) {
      setPresetModalError(t('codex.wakeup.presetReasoningEffortsRequired'));
      return;
    }
    if (
      !presetDraft.defaultReasoningEffort ||
      !presetDraft.allowedReasoningEfforts.includes(presetDraft.defaultReasoningEffort)
    ) {
      setPresetModalError(t('codex.wakeup.presetDefaultReasoningRequired'));
      return;
    }
    const duplicatedModel = state.model_presets.find(
      (item) => item.model.trim() === trimmedModel && item.id !== presetDraft.id,
    );
    if (duplicatedModel) {
      setPresetModalError(t('codex.wakeup.presetModelDuplicate'));
      return;
    }

    const nextPreset: CodexWakeupModelPreset = {
      id: presetDraft.id ?? crypto.randomUUID(),
      name: trimmedName,
      model: trimmedModel,
      allowed_reasoning_efforts: presetDraft.allowedReasoningEfforts,
      default_reasoning_effort: presetDraft.defaultReasoningEffort,
    };
    const nextPresets = presetDraft.id
      ? state.model_presets.map((item) => (item.id === presetDraft.id ? nextPreset : item))
      : [nextPreset, ...state.model_presets];

    try {
      await saveState(state.enabled, state.tasks, nextPresets);
      setNotice({
        tone: 'success',
        text: presetDraft.id
          ? t('codex.wakeup.noticePresetUpdated')
          : t('codex.wakeup.noticePresetCreated'),
      });
      setPresetModalError(null);
      setPresetDraft(buildPresetDraft(nextPreset));
    } catch (error) {
      setPresetModalError(String(error));
    }
  }, [presetDraft, saveState, setPresetModalError, state.enabled, state.model_presets, state.tasks, t]);

  const handleDeletePreset = useCallback(
    async (preset: CodexWakeupModelPreset) => {
      const confirmed = await confirmDialog(t('codex.wakeup.presetDeleteConfirm', { name: preset.name }), {
        title: t('common.confirm', '确认'),
        kind: 'warning',
      });
      if (!confirmed) return;
      const nextPresets = state.model_presets.filter((item) => item.id !== preset.id);
      try {
        await saveState(state.enabled, state.tasks, nextPresets);
        setNotice({ tone: 'success', text: t('codex.wakeup.noticePresetDeleted') });
        if (presetDraft.id === preset.id) {
          setPresetDraft(createEmptyPresetDraft());
        }
      } catch (error) {
        setPresetModalError(String(error));
      }
    },
    [presetDraft.id, saveState, state.enabled, state.model_presets, state.tasks, t],
  );

  const openNewTaskModal = useCallback(async () => {
    if (runtime && !runtime.available) {
      openRuntimeGuideModal();
      return;
    }
    setTaskDraft(createEmptyTaskDraftWithRememberedModel());
    setTaskModalError(null);
    setTaskAccountFilters(createEmptyAccountPickerFilters());
    setShowTaskModal(true);
  }, [createEmptyTaskDraftWithRememberedModel, openRuntimeGuideModal, runtime]);

  const openEditTaskModal = useCallback((task: CodexWakeupTask) => {
    setTaskDraft(buildTaskDraft(task, state.model_presets));
    setTaskModalError(null);
    setTaskAccountFilters(createEmptyAccountPickerFilters());
    setShowTaskModal(true);
  }, [state.model_presets]);

  const openTestModal = useCallback(async () => {
    if (runtime && !runtime.available) {
      openRuntimeGuideModal();
      return;
    }
    setTestModalError(null);
    setTestAccountFilters(createEmptyAccountPickerFilters());
    setTestModelPresetId(resolvedModelSelection.modelPresetId);
    setTestModel(resolvedModelSelection.model);
    setTestModelReasoningEffort(resolvedModelSelection.modelReasoningEffort);
    setShowTestModal(true);
  }, [openRuntimeGuideModal, resolvedModelSelection, runtime]);

  const closeTaskModal = useCallback(() => {
    if (saving) return;
    setShowTaskModal(false);
    setTaskModalError(null);
    setTaskDraft(createEmptyTaskDraftWithRememberedModel());
  }, [createEmptyTaskDraftWithRememberedModel, saving]);

  const cancelRunningTest = useCallback(() => {
    const scopeId = activeTestScopeIdRef.current;
    if (!scopeId) {
      return;
    }
    activeTestRunTokenRef.current = 0;
    activeTestScopeIdRef.current = null;
    const cancelledMessage = t('wakeup.notice.testCancelled');
    setExecutionSession((current) => {
      if (!current || current.triggerType !== 'test' || !current.running) {
        return current;
      }
      const cancelledCount = current.records.filter(
        (item) => item.status === 'pending' || item.status === 'running',
      ).length;
      const nextFailureCount = current.failureCount + cancelledCount;
      return {
        ...current,
        running: false,
        completed: current.total,
        failureCount: nextFailureCount,
        records: current.records.map((record) =>
          record.status === 'pending' || record.status === 'running'
            ? { ...record, status: 'error', error: cancelledMessage }
            : record,
        ),
      };
    });
    setShowTestModal(false);
    setTestModalError(null);
    setNotice({ tone: 'error', text: cancelledMessage });
    void cancelTestScope(scopeId).catch((error) => {
      console.error('取消 Codex 唤醒测试失败:', error);
    });
  }, [cancelTestScope, t]);

  const closeTestModal = useCallback(() => {
    if (testing) {
      cancelRunningTest();
      return;
    }
    setShowTestModal(false);
    setTestModalError(null);
    setTestAccountIds([]);
    setTestPrompt('');
    setTestModelPresetId(resolvedModelSelection.modelPresetId);
    setTestModel(resolvedModelSelection.model);
    setTestModelReasoningEffort(resolvedModelSelection.modelReasoningEffort);
  }, [cancelRunningTest, resolvedModelSelection, testing]);

  useEscClose(showRuntimeGuideModal, closeRuntimeGuideModal);
  useEscClose(showPresetModal, closePresetModal);
  useEscClose(showTaskModal, closeTaskModal);
  useEscClose(showTestModal, closeTestModal);
  useEscClose(showHistoryModal, () => setShowHistoryModal(false));

  const persistTasks = useCallback(
    async (
      enabled: boolean,
      tasks: CodexWakeupTask[],
      modelPresets: CodexWakeupModelPreset[] = state.model_presets,
    ) => {
      const refreshAdjusted =
        enabled && hasEnabledQuotaResetTask(tasks)
          ? await ensureCodexRefreshIntervalForQuotaReset()
          : false;
      const next = await saveState(enabled, tasks, modelPresets);
      setNotice({
        tone: 'success',
        text: enabled
          ? refreshAdjusted
            ? t('codex.wakeup.noticeSavedEnabledWithQuotaReset', {
                count: next.tasks.length,
                minutes: QUOTA_RESET_MIN_REFRESH_MINUTES,
              })
            : t('codex.wakeup.noticeSavedEnabled', { count: next.tasks.length })
          : t('codex.wakeup.noticeSavedDisabled', { count: next.tasks.length }),
      });
      return next;
    },
    [ensureCodexRefreshIntervalForQuotaReset, hasEnabledQuotaResetTask, saveState, state.model_presets, t],
  );

  const toggleAllTasks = useCallback(async () => {
    if (state.tasks.length === 0) return;
    const nextEnabled = !state.enabled;
    const nextTasks = state.tasks.map((task) => ({ ...task, enabled: nextEnabled }));
    await persistTasks(nextEnabled, nextTasks);
  }, [persistTasks, state.enabled, state.tasks]);

  const handleDeleteTask = useCallback(
    async (task: CodexWakeupTask) => {
      const confirmed = await confirmDialog(
        t('codex.wakeup.deleteConfirm', { name: task.name }),
        {
          title: t('common.confirm', '确认'),
          kind: 'warning',
        },
      );
      if (!confirmed) return;
      const nextTasks = state.tasks.filter((item) => item.id !== task.id);
      await persistTasks(deriveStateEnabled(nextTasks), nextTasks);
    },
    [persistTasks, state.tasks, t],
  );

  const handleToggleTask = useCallback(
    async (task: CodexWakeupTask) => {
      const nextTasks = state.tasks.map((item) =>
        item.id === task.id ? { ...item, enabled: !item.enabled } : item,
      );
      await persistTasks(deriveStateEnabled(nextTasks), nextTasks);
    },
    [persistTasks, state.tasks],
  );

  const handleSaveTask = useCallback(async () => {
    const trimmedName = taskDraft.name.trim();
    if (!trimmedName) {
      setTaskModalError(t('codex.wakeup.taskNameRequired'));
      return;
    }
    if (taskDraft.accountIds.length === 0) {
      setTaskModalError(t('codex.wakeup.taskAccountsRequired'));
      return;
    }
    if (!selectedTaskPreset) {
      setTaskModalError(t('codex.wakeup.taskModelRequired'));
      return;
    }
    if (
      !taskDraft.modelReasoningEffort ||
      !selectedTaskPreset.allowed_reasoning_efforts.includes(taskDraft.modelReasoningEffort)
    ) {
      setTaskModalError(t('codex.wakeup.taskReasoningEffortRequired'));
      return;
    }
    if (taskDraft.scheduleKind === 'weekly' && taskDraft.weeklyDays.length === 0) {
      setTaskModalError(t('codex.wakeup.weeklyDaysRequired'));
      return;
    }
    setTaskModalError(null);

    const now = Math.floor(Date.now() / 1000);
    const existingTask = taskDraft.id
      ? state.tasks.find((item) => item.id === taskDraft.id)
      : undefined;
    const startupDelayMinutes =
      taskDraft.scheduleKind === 'startup'
        ? taskDraft.startupDelayMode === 'delayed'
          ? Math.min(
              MAX_STARTUP_DELAY_MINUTES,
              Math.max(1, Number(taskDraft.startupDelayMinutes) || 1),
            )
          : 0
        : undefined;
    const nextTask: CodexWakeupTask = {
      id: taskDraft.id ?? crypto.randomUUID(),
      name: trimmedName,
      enabled: taskDraft.enabled,
      account_ids: taskDraft.accountIds,
      prompt: taskDraft.prompt.trim() || undefined,
      model: selectedTaskPreset.model,
      model_display_name: selectedTaskPreset.name,
      model_reasoning_effort: taskDraft.modelReasoningEffort || undefined,
      schedule: {
        kind: taskDraft.scheduleKind,
        daily_time: taskDraft.scheduleKind === 'daily' ? taskDraft.dailyTime : undefined,
        weekly_days: taskDraft.scheduleKind === 'weekly' ? taskDraft.weeklyDays : [],
        weekly_time: taskDraft.scheduleKind === 'weekly' ? taskDraft.weeklyTime : undefined,
        interval_hours:
          taskDraft.scheduleKind === 'interval'
            ? Math.max(1, Number(taskDraft.intervalHours) || 1)
            : undefined,
        quota_reset_window:
          taskDraft.scheduleKind === 'quota_reset' ? taskDraft.quotaResetWindow : undefined,
        startup_delay_minutes: startupDelayMinutes,
      },
      created_at: existingTask?.created_at ?? taskDraft.createdAt ?? now,
      updated_at: now,
      last_run_at: existingTask?.last_run_at,
      last_status: existingTask?.last_status,
      last_message: existingTask?.last_message,
      last_success_count: existingTask?.last_success_count,
      last_failure_count: existingTask?.last_failure_count,
      last_duration_ms: existingTask?.last_duration_ms,
      next_run_at: existingTask?.next_run_at,
    };

    const nextTasks = taskDraft.id
      ? state.tasks.map((item) => (item.id === taskDraft.id ? { ...item, ...nextTask } : item))
      : [nextTask, ...state.tasks];
    try {
      await persistTasks(deriveStateEnabled(nextTasks), nextTasks);
      setShowTaskModal(false);
      setTaskModalError(null);
    } catch (error) {
      setTaskModalError(String(error));
    }
  }, [persistTasks, selectedTaskPreset, state.tasks, t, taskDraft]);

  const handleRunTask = useCallback(
    async (task: CodexWakeupTask) => {
      const confirmed = await confirmDialog(
        t('codex.wakeup.manualRunConfirm', {
          name: task.name,
          count: task.account_ids.length,
        }),
        {
          title: t('common.confirm', '确认'),
          kind: 'warning',
          okLabel: t('common.confirm', '确认'),
          cancelLabel: t('common.cancel', '取消'),
        },
      );
      if (!confirmed) {
        return;
      }

      const runId = crypto.randomUUID();
      setExecutionSessionFromHistory(false);
      setExecutionSession(
        buildExecutionSession(
          runId,
          'manual_task',
          task.account_ids,
          task.prompt,
          task.id,
          task.name,
          task.model,
          task.model_display_name,
          task.model_reasoning_effort,
        ),
      );
      try {
        const result = await runTask(task.id, runId);
        await onRefreshAccounts();
        setExecutionSession((current) =>
          current && current.runId === runId
            ? {
                ...current,
                runtime: result.runtime,
                completed: result.records.length,
                total: result.records.length,
                successCount: result.success_count,
                failureCount: result.failure_count,
                durationMs: result.records.reduce((sum, item) => sum + (item.duration_ms || 0), 0),
                running: false,
                records: current.records.map((record) => {
                  const matched = result.records.find((item) => item.account_id === record.accountId);
                  if (!matched) {
                    return record;
                  }
                  return {
                    ...record,
                    accountEmail: matched.account_email || record.accountEmail,
                    accountContextText: matched.account_context_text || record.accountContextText,
                    status: executionStatusFromRecord(matched),
                    prompt: matched.prompt || record.prompt,
                    model: matched.model || record.model,
                    modelDisplayName: matched.model_display_name || record.modelDisplayName,
                    modelReasoningEffort:
                      matched.model_reasoning_effort || record.modelReasoningEffort,
                    reply: matched.reply,
                    error: matched.error,
                    timestamp: matched.timestamp,
                    durationMs: matched.duration_ms,
                    triggerType: matched.trigger_type || record.triggerType,
                  };
                }),
              }
            : current,
        );
        setNotice({
          tone: result.failure_count > 0 ? 'error' : 'success',
          text:
            result.failure_count > 0
              ? t('codex.wakeup.noticeTaskFinishedWithError', {
                  success: result.success_count,
                  failed: result.failure_count,
                })
              : t('codex.wakeup.noticeTaskFinished', { count: result.success_count }),
        });
      } catch (error) {
        setExecutionSession((current) =>
          current && current.runId === runId
            ? { ...current, running: false, errorText: String(error) }
            : current,
        );
      }
    },
    [buildExecutionSession, onRefreshAccounts, runTask, t],
  );

  const handleRunTest = useCallback(async () => {
    if (testAccountIds.length === 0) {
      setTestModalError(t('codex.wakeup.testAccountsRequired'));
      return;
    }
    if (!selectedTestPreset) {
      setTestModalError(t('codex.wakeup.testModelRequired'));
      return;
    }
    if (
      !testModelReasoningEffort ||
      !selectedTestPreset.allowed_reasoning_efforts.includes(testModelReasoningEffort)
    ) {
      setTestModalError(t('codex.wakeup.testReasoningEffortRequired'));
      return;
    }
    setTestModalError(null);
    const runId = crypto.randomUUID();
    const runToken = activeTestRunTokenRef.current + 1;
    const cancelScopeId = buildWakeupTestScopeId();
    activeTestRunTokenRef.current = runToken;
    activeTestScopeIdRef.current = cancelScopeId;
    const promptValue = testPrompt.trim() || undefined;
    setExecutionSessionFromHistory(false);
    setExecutionSession(
      buildExecutionSession(
        runId,
        'test',
        testAccountIds,
        promptValue,
        undefined,
        undefined,
        selectedTestPreset.model,
        selectedTestPreset.name,
        testModelReasoningEffort,
      ),
    );
    setShowTestModal(false);
    try {
      const result = await runTest(
        testAccountIds,
        runId,
        promptValue,
        selectedTestPreset.model,
        selectedTestPreset.name,
        testModelReasoningEffort,
        cancelScopeId,
      );
      if (activeTestRunTokenRef.current !== runToken) {
        return;
      }
      await onRefreshAccounts();
      if (activeTestRunTokenRef.current !== runToken) {
        return;
      }
      setExecutionSession((current) =>
        current && current.runId === runId
          ? {
              ...current,
              runtime: result.runtime,
              completed: result.records.length,
              total: result.records.length,
              successCount: result.success_count,
              failureCount: result.failure_count,
              durationMs: result.records.reduce((sum, item) => sum + (item.duration_ms || 0), 0),
              running: false,
              records: current.records.map((record) => {
                const matched = result.records.find((item) => item.account_id === record.accountId);
                if (!matched) {
                  return record;
                }
                  return {
                    ...record,
                    accountEmail: matched.account_email || record.accountEmail,
                    accountContextText: matched.account_context_text || record.accountContextText,
                    status: executionStatusFromRecord(matched),
                    prompt: matched.prompt || record.prompt,
                    model: matched.model || record.model,
                    modelDisplayName: matched.model_display_name || record.modelDisplayName,
                    modelReasoningEffort:
                      matched.model_reasoning_effort || record.modelReasoningEffort,
                    reply: matched.reply,
                    error: matched.error,
                    timestamp: matched.timestamp,
                  durationMs: matched.duration_ms,
                  triggerType: matched.trigger_type || record.triggerType,
                };
              }),
            }
          : current,
      );
      if (activeTestRunTokenRef.current !== runToken) {
        return;
      }
      setTestAccountIds([]);
      setTestPrompt('');
      setTestModelPresetId(resolvedModelSelection.modelPresetId);
      setTestModel(resolvedModelSelection.model);
      setTestModelReasoningEffort(resolvedModelSelection.modelReasoningEffort);
      setNotice({
        tone: result.failure_count > 0 ? 'error' : 'success',
        text:
          result.failure_count > 0
            ? t('codex.wakeup.noticeTestFinishedWithError', {
                success: result.success_count,
                failed: result.failure_count,
              })
            : t('codex.wakeup.noticeTestFinished', { count: result.success_count }),
      });
    } catch (error) {
      if (activeTestRunTokenRef.current !== runToken) {
        return;
      }
      setExecutionSession((current) =>
        current && current.runId === runId
          ? { ...current, running: false, errorText: String(error) }
          : current,
      );
    } finally {
      if (activeTestRunTokenRef.current === runToken) {
        activeTestRunTokenRef.current = 0;
        activeTestScopeIdRef.current = null;
      }
      void releaseTestScope(cancelScopeId).catch((error) => {
        console.error('释放 Codex 唤醒测试取消作用域失败:', error);
      });
    }
  }, [
    buildExecutionSession,
    onRefreshAccounts,
    releaseTestScope,
    resolvedModelSelection,
    runTest,
    selectedTestPreset,
    t,
    testAccountIds,
    testModelReasoningEffort,
    testPrompt,
  ]);

  const handleClearHistory = useCallback(async () => {
    const confirmed = await confirmDialog(t('codex.wakeup.clearHistoryConfirm'), {
      title: t('common.confirm', '确认'),
      kind: 'warning',
    });
    if (!confirmed) return;
    await clearHistory();
    setNotice({ tone: 'success', text: t('codex.wakeup.historyCleared') });
  }, [clearHistory, t]);

  return (
    <div className="wakeup-page codex-wakeup-content">
      {notice && (
        <div className={`action-message ${notice.tone}`}>
          <span className="action-message-text">{notice.text}</span>
          <button className="action-message-close" onClick={() => setNotice(null)} aria-label={t('common.close')}>
            <X size={14} />
          </button>
        </div>
      )}

      <div className="toolbar wakeup-toolbar">
        <div className="toolbar-left">
          <div className={`wakeup-global-toggle ${state.enabled ? 'is-on' : 'is-off'}`}>
            <span className="toggle-label">{t('codex.wakeup.tab')}</span>
            <span className={`pill ${state.enabled ? 'pill-success' : 'pill-secondary'}`}>
              {state.enabled ? t('codex.wakeup.taskEnabled') : t('codex.wakeup.taskPaused')}
            </span>
            <label
              className="wakeup-switch"
              onClick={(event) => {
                event.preventDefault();
                void toggleAllTasks();
              }}
            >
              <input type="checkbox" checked={state.enabled} readOnly />
              <span className="wakeup-slider" />
            </label>
          </div>
        </div>
        <div className="toolbar-right">
          <button className="btn btn-primary" onClick={() => void openNewTaskModal()} disabled={oauthAccounts.length === 0}>
            <Plus size={16} /> {t('codex.wakeup.addTask')}
          </button>
          <button className="btn btn-secondary" onClick={() => openPresetModal('page')}>
            {t('codex.wakeup.managePresets')}
          </button>
          <button className="btn btn-secondary" onClick={() => void openTestModal()} disabled={oauthAccounts.length === 0}>
            {t('codex.wakeup.testNow')}
          </button>
          <button className="btn btn-secondary" onClick={() => setShowHistoryModal(true)}>
            {historyBatches.length > 0
              ? `${t('codex.wakeup.historyTitle')} (${historyBatches.length})`
              : t('codex.wakeup.historyTitle')}
          </button>
          <button className="btn btn-secondary" onClick={() => void refreshRuntime().catch(() => undefined)}>
            <RefreshCw size={16} /> {t('codex.wakeup.refreshRuntime')}
          </button>
          <button
            className="btn btn-secondary icon-only"
            onClick={openRuntimeGuideModal}
            title={t('codex.wakeup.runtimeConfigTitle')}
            aria-label={t('codex.wakeup.runtimeConfigTitle')}
          >
            <Settings size={14} />
          </button>
        </div>
      </div>

      {loading ? (
        <div className="loading-container">
          <RefreshCw size={24} className="loading-spinner" />
          <p>{t('common.loading', '加载中...')}</p>
        </div>
      ) : sortedTasks.length === 0 ? (
        <div className="empty-state">
          <div className="icon">
            <Power size={40} />
          </div>
          <h3>{t('codex.wakeup.emptyTitle')}</h3>
          <p>{t('codex.wakeup.emptyDesc')}</p>
          <button className="btn btn-primary" onClick={() => void openNewTaskModal()} disabled={oauthAccounts.length === 0}>
            <Plus size={18} /> {t('codex.wakeup.addTask')}
          </button>
        </div>
      ) : (
        <div className="wakeup-task-grid">
          {sortedTasks.map((task) => {
            const accountLabels = task.account_ids.map((accountId) => {
              const meta = wakeupAccountMetaMap.get(accountId);
              const value = meta?.email || accountMap.get(accountId)?.email || accountId;
              return maskAccountText(value);
            });
            return (
              <div key={task.id} className={`wakeup-task-card ${task.enabled ? 'is-enabled' : 'is-disabled'}`}>
                <div className="wakeup-task-header">
                  <div className="wakeup-task-title">
                    <span>{task.name}</span>
                    <span className={`pill ${task.enabled ? 'pill-success' : 'pill-secondary'}`}>
                      {task.enabled ? t('codex.wakeup.taskEnabled') : t('codex.wakeup.taskPaused')}
                    </span>
                  </div>
                  <div className="wakeup-task-actions">
                    <button
                      className="btn btn-secondary icon-only"
                      onClick={() => openTaskExecutionDetails(task)}
                      title={t('common.detail')}
                      aria-label={t('common.detail')}
                    >
                      <Eye size={14} />
                    </button>
                    <button
                      className="btn btn-secondary icon-only"
                      onClick={() => void handleRunTask(task)}
                      disabled={runningTaskId === task.id}
                      title={t('codex.wakeup.testNow')}
                    >
                      {runningTaskId === task.id ? <RefreshCw size={14} className="loading-spinner" /> : <Play size={14} />}
                    </button>
                    <button
                      className="btn btn-secondary icon-only"
                      onClick={() => openEditTaskModal(task)}
                      title={t('common.edit', '编辑')}
                    >
                      <Pencil size={14} />
                    </button>
                    <button
                      className="btn btn-secondary icon-only"
                      onClick={() => void handleToggleTask(task)}
                      title={task.enabled ? t('codex.wakeup.pauseOne') : t('codex.wakeup.resumeOne')}
                    >
                      <Power size={14} />
                    </button>
                    <button
                      className="btn btn-danger icon-only"
                      onClick={() => void handleDeleteTask(task)}
                      title={t('common.delete')}
                    >
                      <Trash2 size={14} />
                    </button>
                  </div>
                </div>
                <div className="wakeup-task-section wakeup-task-section-primary">
                  <div className="wakeup-task-meta wakeup-task-meta-schedule">
                    <span>{scheduleSummary(task, t)}</span>
                  </div>
                </div>

                <div className="wakeup-task-section">
                  <div className="wakeup-task-meta wakeup-task-meta-accounts">
                    <span>{t('codex.wakeup.taskAccountsLabel')}: {formatSelectionPreview(accountLabels)}</span>
                  </div>
                  {(task.model || task.model_reasoning_effort) && (
                    <div className="wakeup-task-meta wakeup-task-meta-prompt">
                      <span>
                        {t('codex.wakeup.modelSummaryLabel', {
                          model: formatWakeupModelLabel(task.model, task.model_display_name) || t('codex.wakeup.modelDefault'),
                          reasoning: task.model_reasoning_effort
                            ? reasoningEffortLabel(task.model_reasoning_effort, t)
                            : t('codex.wakeup.modelDefault'),
                        })}
                      </span>
                    </div>
                  )}
                  {task.prompt && (
                    <div className="wakeup-task-meta wakeup-task-meta-prompt">
                      <span>{t('codex.wakeup.promptLabel')}: {task.prompt}</span>
                    </div>
                  )}
                </div>

                <div className="wakeup-task-section wakeup-task-section-muted">
                  <div className="wakeup-task-meta wakeup-task-meta-status">
                    <span>{t('codex.wakeup.lastStatusLabel')}: {formatTaskLastResult(task, t)}</span>
                    <span>{t('codex.wakeup.lastDurationLabel')}: {formatDuration(task.last_duration_ms)}</span>
                  </div>
                  <div className="wakeup-task-meta wakeup-task-meta-timeline">
                    <span>{t('codex.wakeup.lastRunLabel', { time: formatDateTime(task.last_run_at) })}</span>
                    <span>{t('codex.wakeup.nextRunLabel', { time: formatDateTime(task.next_run_at) })}</span>
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      )}

      {showRuntimeGuideModal && (
        <div className="modal-overlay" onClick={closeRuntimeGuideModal}>
          <div
            className="modal wakeup-modal codex-wakeup-runtime-guide-modal"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              <button
                className="btn btn-secondary icon-only"
                onClick={closeRuntimeGuideModal}
                title={t('common.back', '返回')}
                aria-label={t('common.back', '返回')}
                disabled={runtimeGuideRefreshing}
              >
                <ChevronLeft size={14} />
              </button>
              <h2>{runtimeGuideTitle}</h2>
              <button className="modal-close" onClick={closeRuntimeGuideModal} disabled={runtimeGuideRefreshing}>
                <X />
              </button>
            </div>
            <div className="modal-body codex-wakeup-runtime-guide-body">
              <div className="codex-wakeup-runtime-guide-hero">
                <div className="codex-wakeup-runtime-guide-icon">
                  {runtimeGuideNeedInstall ? <CircleAlert size={20} /> : <Check size={20} />}
                </div>
                <div className="codex-wakeup-runtime-guide-copy">
                  <span className="codex-wakeup-runtime-guide-kicker">
                    {runtimeGuideNeedInstall ? t('codex.wakeup.runtimeMissing') : t('codex.wakeup.runtimeReady')}
                  </span>
                  <h3>{runtimeGuideTitle}</h3>
                  <p>
                    {runtimeGuideNeedInstall
                      ? runtime?.message || t('codex.wakeup.installSubtitle')
                      : runtime?.binary_path || runtime?.message || t('codex.wakeup.runtimeConfigHint')}
                  </p>
                </div>
              </div>
              {showRuntimeConfigCard && (
                <section className="codex-wakeup-runtime-config-card">
                  <div className="codex-wakeup-runtime-config-header">
                    <h4>{t('codex.wakeup.runtimeConfigTitle')}</h4>
                    <p>{t('codex.wakeup.runtimeConfigHint')}</p>
                  </div>
                  <div
                    className={`codex-wakeup-runtime-config-grid ${
                      showCodexCliInput !== showNodeInput ? 'single' : ''
                    }`}
                  >
                    {showCodexCliInput && (
                      <label className="codex-wakeup-runtime-config-field">
                        <span>{t('codex.wakeup.runtimeConfigCodexCliLabel')}</span>
                        <input
                          type="text"
                          className="wakeup-input"
                          value={runtimeConfigDraft.codexCliPath}
                          onChange={(event) =>
                            handleRuntimeConfigInputChange('codexCliPath', event.target.value)
                          }
                          placeholder={t('codex.wakeup.runtimeConfigCodexCliPlaceholder')}
                          disabled={runtimeGuideRefreshing}
                        />
                      </label>
                    )}
                    {showNodeInput && (
                      <label className="codex-wakeup-runtime-config-field">
                        <span>{t('codex.wakeup.runtimeConfigNodeLabel')}</span>
                        <input
                          type="text"
                          className="wakeup-input"
                          value={runtimeConfigDraft.nodePath}
                          onChange={(event) =>
                            handleRuntimeConfigInputChange('nodePath', event.target.value)
                          }
                          placeholder={t('codex.wakeup.runtimeConfigNodePlaceholder')}
                          disabled={runtimeGuideRefreshing}
                        />
                      </label>
                    )}
                  </div>
                  <p className="codex-wakeup-runtime-config-tip">
                    {t('codex.wakeup.runtimeConfigApplyHint')}
                  </p>
                  {runtimeGuideConfigError && (
                    <p className="codex-wakeup-runtime-config-error">{runtimeGuideConfigError}</p>
                  )}
                </section>
              )}
              {runtimeGuideNeedInstall && renderInstallCommands(runtime?.install_hints || [])}
              {runtimeGuideNeedInstall && (
                <p className="codex-wakeup-install-footnote">{t('codex.wakeup.installFootnote')}</p>
              )}
            </div>
            <div className="modal-footer">
              <button className="btn btn-secondary" onClick={closeRuntimeGuideModal} disabled={runtimeGuideRefreshing}>
                {t('common.close')}
              </button>
              <button className="btn btn-primary" onClick={() => void handleRefreshRuntimeGuide()} disabled={runtimeGuideRefreshing}>
                <RefreshCw size={16} className={runtimeGuideRefreshing ? 'loading-spinner' : ''} />
                {t('codex.wakeup.refreshRuntime')}
              </button>
            </div>
          </div>
        </div>
      )}

      {showPresetModal && (
        <div className="modal-overlay codex-wakeup-preset-overlay" onClick={closePresetModal}>
          <div className="modal modal-lg wakeup-modal codex-wakeup-modal" onClick={(event) => event.stopPropagation()}>
            <div className="modal-header">
              {presetModalSource !== 'page' && (
                <button
                  className="btn btn-secondary icon-only"
                  onClick={closePresetModal}
                  title={t('common.back', '返回')}
                  aria-label={t('common.back', '返回')}
                  disabled={saving}
                >
                  <ChevronLeft size={14} />
                </button>
              )}
              <h2>{t('codex.wakeup.presetManagerTitle')}</h2>
              <button className="modal-close" onClick={closePresetModal} disabled={saving}>
                <X />
              </button>
            </div>
            <div className="modal-body codex-wakeup-modal-body">
              <ModalErrorMessage message={presetModalError} scrollKey={presetModalErrorScrollKey} />
              <div className="wakeup-form-group">
                <div className="codex-wakeup-inline-header">
                  <label>{t('codex.wakeup.presetListLabel')}</label>
                  <button
                    type="button"
                    className="btn btn-secondary"
                    onClick={() => {
                      setPresetDraft(createEmptyPresetDraft());
                      setPresetModalError(null);
                    }}
                  >
                    <Plus size={14} /> {t('codex.wakeup.addPreset')}
                  </button>
                </div>
                {state.model_presets.length === 0 ? (
                  <p className="wakeup-hint">{t('codex.wakeup.presetEmpty')}</p>
                ) : (
                  <div className="wakeup-chip-grid">
                    {state.model_presets.map((preset) => (
                      <button
                        key={preset.id}
                        type="button"
                        className={`wakeup-chip ${presetDraft.id === preset.id ? 'selected' : ''}`}
                        onClick={() => {
                          setPresetDraft(buildPresetDraft(preset));
                          setPresetModalError(null);
                        }}
                      >
                        {preset.name}
                      </button>
                    ))}
                  </div>
                )}
              </div>

              <div className="wakeup-form-group">
                <label>{t('codex.wakeup.presetNameLabel')}</label>
                <input
                  className="wakeup-input"
                  value={presetDraft.name}
                  onChange={(event) => setPresetDraft((current) => ({ ...current, name: event.target.value }))}
                  placeholder={t('codex.wakeup.presetNamePlaceholder')}
                />
              </div>

              <div className="wakeup-form-group">
                <label>{t('codex.wakeup.presetModelLabel')}</label>
                <input
                  className="wakeup-input"
                  value={presetDraft.model}
                  onChange={(event) => setPresetDraft((current) => ({ ...current, model: event.target.value }))}
                  placeholder={t('codex.wakeup.presetModelPlaceholder')}
                />
              </div>

              <div className="wakeup-form-group">
                <label>{t('codex.wakeup.presetAllowedReasoningLabel')}</label>
                <div className="wakeup-chip-grid">
                  {REASONING_EFFORT_OPTIONS.map((effort) => {
                    const active = presetDraft.allowedReasoningEfforts.includes(effort);
                    return (
                      <button
                        key={effort}
                        type="button"
                        className={`wakeup-chip ${active ? 'selected' : ''}`}
                        onClick={() =>
                          setPresetDraft((current) => {
                            const nextAllowed = active
                              ? current.allowedReasoningEfforts.filter((item) => item !== effort)
                              : [...current.allowedReasoningEfforts, effort];
                            const nextDefault = nextAllowed.includes(current.defaultReasoningEffort as CodexWakeupReasoningEffort)
                              ? current.defaultReasoningEffort
                              : nextAllowed[0] ?? '';
                            return {
                              ...current,
                              allowedReasoningEfforts: nextAllowed,
                              defaultReasoningEffort: nextDefault,
                            };
                          })
                        }
                      >
                        {reasoningEffortLabel(effort, t)}
                      </button>
                    );
                  })}
                </div>
              </div>

              <div className="wakeup-form-group">
                <label>{t('codex.wakeup.presetDefaultReasoningLabel')}</label>
                <select
                  className="wakeup-input"
                  value={presetDraft.defaultReasoningEffort}
                  onChange={(event) =>
                    setPresetDraft((current) => ({
                      ...current,
                      defaultReasoningEffort: event.target.value as CodexWakeupReasoningEffort,
                    }))
                  }
                >
                  <option value="">{t('codex.wakeup.selectReasoningPlaceholder')}</option>
                  {presetDraft.allowedReasoningEfforts.map((effort) => (
                    <option key={effort} value={effort}>
                      {reasoningEffortLabel(effort, t)}
                    </option>
                  ))}
                </select>
              </div>
            </div>
            <div className="modal-footer">
              {presetDraft.id && (
                <button
                  className="btn btn-danger"
                  onClick={() => {
                    const preset = state.model_presets.find((item) => item.id === presetDraft.id);
                    if (preset) {
                      void handleDeletePreset(preset);
                    }
                  }}
                  disabled={saving}
                >
                  {t('common.delete')}
                </button>
              )}
              <button className="btn btn-secondary" onClick={closePresetModal} disabled={saving}>
                {t('common.close')}
              </button>
              <button className="btn btn-primary" onClick={() => void handleSavePreset()} disabled={saving}>
                {presetDraft.id ? t('common.save') : t('codex.wakeup.addPreset')}
              </button>
            </div>
          </div>
        </div>
      )}

      {showTaskModal && (
        <div className="modal-overlay" onClick={closeTaskModal}>
          <div className="modal modal-lg wakeup-modal codex-wakeup-modal" onClick={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <button className="btn btn-secondary icon-only" onClick={closeTaskModal} title={t('common.back', '返回')} aria-label={t('common.back', '返回')}><ChevronLeft size={14} /></button>
              <h2>{taskDraft.id ? t('codex.wakeup.editTaskTitle') : t('codex.wakeup.createTaskTitle')}</h2>
              <button className="modal-close" onClick={closeTaskModal}>
                <X />
              </button>
            </div>
            <div className="modal-body codex-wakeup-modal-body">
              <ModalErrorMessage message={taskModalError} scrollKey={taskModalErrorScrollKey} />
              <div className="wakeup-form-group">
                <label>{t('codex.wakeup.taskNameLabel')}</label>
                <input
                  className="wakeup-input"
                  value={taskDraft.name}
                  onChange={(event) => setTaskDraft((current) => ({ ...current, name: event.target.value }))}
                  placeholder={t('codex.wakeup.taskNamePlaceholder')}
                />
              </div>

              <div className="wakeup-form-group">
                <label>{t('common.status', '状态')}</label>
                <div className="wakeup-toggle-group">
                  <button
                    className={`btn btn-secondary ${taskDraft.enabled ? 'is-active' : ''}`}
                    onClick={() => setTaskDraft((current) => ({ ...current, enabled: true }))}
                  >
                    {t('common.enable')}
                  </button>
                  <button
                    className={`btn btn-secondary ${!taskDraft.enabled ? 'is-active' : ''}`}
                    onClick={() => setTaskDraft((current) => ({ ...current, enabled: false }))}
                  >
                    {t('common.disable')}
                  </button>
                </div>
              </div>

              <div className="wakeup-form-group">
                <label>{t('codex.wakeup.taskAccountsLabel')}</label>
                <p className="wakeup-hint">{t('codex.wakeup.taskAccountsHint')}</p>
                {renderAccountPickerFilters(
                  taskAccountFilters,
                  setTaskAccountFilters,
                  filteredTaskAccounts,
                  allFilteredTaskSelected,
                  () =>
                    setTaskDraft((current) => {
                      const visibleIds = filteredTaskAccounts.map((account) => account.id);
                      const visibleSet = new Set(visibleIds);
                      if (allFilteredTaskSelected) {
                        return {
                          ...current,
                          accountIds: current.accountIds.filter((item) => !visibleSet.has(item)),
                        };
                      }
                      return {
                        ...current,
                        accountIds: Array.from(new Set([...current.accountIds, ...visibleIds])),
                      };
                    }),
                )}
                {filteredTaskAccounts.length === 0 ? (
                  <div className="codex-wakeup-account-empty">
                    {t('codex.wakeup.accountFilterEmpty')}
                  </div>
                ) : (
                  <div className="wakeup-chip-list codex-wakeup-account-list">
                    {filteredTaskAccounts.map((account) => {
                    const checked = taskDraft.accountIds.includes(account.id);
                    return renderWakeupAccountOption(account, checked, () =>
                      setTaskDraft((current) => ({
                        ...current,
                        accountIds: checked
                          ? current.accountIds.filter((item) => item !== account.id)
                          : [...current.accountIds, account.id],
                      })),
                    );
                  })}
                  </div>
                )}
              </div>

              <div className="wakeup-form-group">
                <div className="codex-wakeup-inline-header">
                  <label>{t('codex.wakeup.taskModelLabel')}</label>
                  <button type="button" className="btn btn-secondary" onClick={() => openPresetModal('task')}>
                    {t('codex.wakeup.managePresets')}
                  </button>
                </div>
                <p className="wakeup-hint">{t('codex.wakeup.taskModelHint')}</p>
                <div className="codex-wakeup-dual-select">
                  <div className="codex-wakeup-dual-select-field">
                    <WakeupSingleSelectDropdown
                      value={taskDraft.modelPresetId}
                      options={modelPresetOptions}
                      placeholder={t('codex.wakeup.selectPresetPlaceholder')}
                      onSelect={handleSelectTaskPreset}
                    />
                  </div>
                  <div className="codex-wakeup-dual-select-field codex-wakeup-dual-select-field-compact">
                    <WakeupSingleSelectDropdown
                      value={taskDraft.modelReasoningEffort}
                      options={taskReasoningOptions}
                      placeholder={t('codex.wakeup.selectReasoningPlaceholder')}
                      onSelect={(value) => {
                        const nextReasoning = value as CodexWakeupReasoningEffort;
                        setTaskDraft((current) => ({
                          ...current,
                          modelReasoningEffort: nextReasoning,
                        }));
                        if (selectedTaskPreset) {
                          rememberModelSelection(
                            buildWakeupModelSelectionFromPreset(selectedTaskPreset, nextReasoning),
                          );
                        }
                      }}
                      disabled={taskReasoningOptions.length === 0}
                    />
                  </div>
                </div>
                {taskDraft.model && (
                  <p className="wakeup-hint">
                    {t('codex.wakeup.modelValuePreview', { model: taskDraft.model })}
                  </p>
                )}
                {taskReasoningOptions.length === 0 ? (
                  <p className="wakeup-hint">{t('codex.wakeup.reasoningEffortEmpty')}</p>
                ) : null}
              </div>

              <div className="wakeup-form-group">
                <label>{t('codex.wakeup.scheduleLabel')}</label>
                <div className="wakeup-segmented">
                  {(['daily', 'weekly', 'interval', 'quota_reset', 'startup'] as CodexWakeupScheduleKind[]).map((kind) => (
                    <button
                      type="button"
                      key={kind}
                      className={`wakeup-segment-btn ${taskDraft.scheduleKind === kind ? 'active' : ''}`}
                      onClick={() => setTaskDraft((current) => ({ ...current, scheduleKind: kind }))}
                    >
                      {kind === 'startup' ? t('wakeup.triggerSource.startup') : t(`codex.wakeup.schedule.${kind}`)}
                    </button>
                  ))}
                </div>
                {taskDraft.scheduleKind === 'quota_reset' && (
                  <>
                    <p className="codex-wakeup-quota-reset-tip">
                      <CircleAlert size={14} />
                      <span>{t('codex.wakeup.scheduleQuotaResetHint')}</span>
                    </p>
                    <div className="codex-wakeup-quota-reset-window-selector">
                      <label>{t('codex.wakeup.quotaResetWindowLabel')}</label>
                      <div className="wakeup-segmented codex-wakeup-quota-reset-window-buttons">
                        {(['either', 'primary_window', 'secondary_window'] as CodexWakeupQuotaResetWindow[]).map(
                          (windowType) => (
                            <button
                              type="button"
                              key={windowType}
                              className={`wakeup-segment-btn ${
                                taskDraft.quotaResetWindow === windowType ? 'active' : ''
                              }`}
                              onClick={() =>
                                setTaskDraft((current) => ({
                                  ...current,
                                  quotaResetWindow: windowType,
                                }))
                              }
                            >
                              {t(`codex.wakeup.quotaResetWindowOptions.${windowType}`)}
                            </button>
                          ),
                        )}
                      </div>
                      <p className="wakeup-hint">{t('codex.wakeup.quotaResetWindowHint')}</p>
                    </div>
                  </>
                )}
              </div>

              {taskDraft.scheduleKind === 'daily' && (
                <div className="wakeup-form-group">
                  <label>{t('codex.wakeup.dailyTimeLabel')}</label>
                  <div className="wakeup-chip-grid">
                    {QUICK_TIME_OPTIONS.map((time) => (
                      <button
                        key={time}
                        type="button"
                        className={`wakeup-chip ${taskDraft.dailyTime === time ? 'selected' : ''}`}
                        onClick={() => setTaskDraft((current) => ({ ...current, dailyTime: time }))}
                      >
                        {time}
                      </button>
                    ))}
                  </div>
                  <input
                    type="time"
                    className="wakeup-input wakeup-input-time"
                    value={taskDraft.dailyTime}
                    onChange={(event) => setTaskDraft((current) => ({ ...current, dailyTime: event.target.value }))}
                  />
                </div>
              )}

              {taskDraft.scheduleKind === 'startup' && (
                <div className="wakeup-form-group">
                  <label>{t('wakeup.triggerSource.startup')}</label>
                  <div className="wakeup-toggle-group">
                    <button
                      type="button"
                      className={`btn btn-secondary ${taskDraft.startupDelayMode === 'immediate' ? 'is-active' : ''}`}
                      onClick={() =>
                        setTaskDraft((current) => ({
                          ...current,
                          startupDelayMode: 'immediate',
                        }))
                      }
                    >
                      {t('settings.general.startupWakeupImmediate')}
                    </button>
                    <button
                      type="button"
                      className={`btn btn-secondary ${taskDraft.startupDelayMode === 'delayed' ? 'is-active' : ''}`}
                      onClick={() =>
                        setTaskDraft((current) => ({
                          ...current,
                          startupDelayMode: 'delayed',
                        }))
                      }
                    >
                      {t('settings.general.startupWakeupDelayed')}
                    </button>
                  </div>
                  {taskDraft.startupDelayMode === 'delayed' && (
                    <div className="wakeup-inline-row">
                      <input
                        type="number"
                        min={1}
                        max={MAX_STARTUP_DELAY_MINUTES}
                        className="wakeup-input wakeup-input-small"
                        value={taskDraft.startupDelayMinutes}
                        onChange={(event) =>
                          setTaskDraft((current) => ({
                            ...current,
                            startupDelayMinutes: event.target.value.replace(/[^\d]/g, ''),
                          }))
                        }
                      />
                      <span>{t('settings.general.minutes')}</span>
                    </div>
                  )}
                </div>
              )}

              {taskDraft.scheduleKind === 'weekly' && (
                <>
                  <div className="wakeup-form-group">
                    <label>{t('codex.wakeup.weeklyDaysLabel')}</label>
                    <div className="wakeup-chip-grid">
                      {WEEKDAY_OPTIONS.map((item) => {
                        const active = taskDraft.weeklyDays.includes(item.value);
                        return (
                          <button
                            type="button"
                            key={item.value}
                            className={`wakeup-chip ${active ? 'selected' : ''}`}
                            onClick={() =>
                              setTaskDraft((current) => ({
                                ...current,
                                weeklyDays: active
                                  ? current.weeklyDays.filter((value) => value !== item.value)
                                  : [...current.weeklyDays, item.value],
                              }))
                            }
                          >
                            {t(`codex.wakeup.weekdays.${item.value}`)}
                          </button>
                        );
                      })}
                    </div>
                  </div>
                  <div className="wakeup-form-group">
                    <label>{t('codex.wakeup.weeklyTimeLabel')}</label>
                    <div className="wakeup-chip-grid">
                      {QUICK_TIME_OPTIONS.map((time) => (
                        <button
                          key={time}
                          type="button"
                          className={`wakeup-chip ${taskDraft.weeklyTime === time ? 'selected' : ''}`}
                          onClick={() => setTaskDraft((current) => ({ ...current, weeklyTime: time }))}
                        >
                          {time}
                        </button>
                      ))}
                    </div>
                    <input
                      type="time"
                      className="wakeup-input wakeup-input-time"
                      value={taskDraft.weeklyTime}
                      onChange={(event) => setTaskDraft((current) => ({ ...current, weeklyTime: event.target.value }))}
                    />
                  </div>
                </>
              )}

              {taskDraft.scheduleKind === 'interval' && (
                <div className="wakeup-form-group">
                  <label>{t('codex.wakeup.intervalHoursLabel')}</label>
                  <input
                    type="number"
                    min={1}
                    max={24}
                    className="wakeup-input wakeup-input-small"
                    value={taskDraft.intervalHours}
                    onChange={(event) =>
                      setTaskDraft((current) => ({ ...current, intervalHours: event.target.value }))
                    }
                  />
                </div>
              )}

              <div className="wakeup-form-group">
                <label>{t('codex.wakeup.promptLabel')}</label>
                <textarea
                  className="token-input codex-wakeup-prompt-input"
                  value={taskDraft.prompt}
                  onChange={(event) => setTaskDraft((current) => ({ ...current, prompt: event.target.value }))}
                  placeholder={t('codex.wakeup.promptPlaceholder', { prompt: DEFAULT_PROMPT })}
                />
              </div>

              <div className="wakeup-form-group">
                <label>{t('wakeup.form.nextRuns', '接下来执行')}</label>
                <ul className="wakeup-preview-list">
                  {previewRuns.length === 0 && (
                    <li>
                      {taskDraft.scheduleKind === 'quota_reset'
                        ? t('codex.wakeup.nextRunsQuotaResetHint')
                        : taskDraft.scheduleKind === 'startup'
                          ? taskDraft.startupDelayMode === 'delayed'
                            ? `${t('wakeup.triggerSource.startup')} +${Math.min(
                                MAX_STARTUP_DELAY_MINUTES,
                                Math.max(1, Number(taskDraft.startupDelayMinutes) || 1),
                              )}${t('settings.general.minutes')}`
                            : t('settings.general.startupWakeupImmediate')
                          : t('wakeup.form.nextRunsEmpty', '暂无预览')}
                    </li>
                  )}
                  {previewRuns.map((date, index) => (
                    <li key={`${date.toISOString()}-${index}`}>
                      {index + 1}. {date.toLocaleString()}
                    </li>
                  ))}
                </ul>
              </div>
            </div>
            <div className="modal-footer">
              <button className="btn btn-secondary" onClick={closeTaskModal} disabled={saving}>
                {t('common.cancel')}
              </button>
              <button className="btn btn-primary" onClick={() => void handleSaveTask()} disabled={saving}>
                {saving ? t('common.saving', '保存中...') : t('common.save')}
              </button>
            </div>
          </div>
        </div>
      )}

      {showTestModal && (
        <div className="modal-overlay" onClick={closeTestModal}>
          <div className="modal modal-lg wakeup-modal wakeup-test-modal codex-wakeup-modal" onClick={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <button className="btn btn-secondary icon-only" onClick={closeTestModal} title={t('common.back', '返回')} aria-label={t('common.back', '返回')}><ChevronLeft size={14} /></button>
              <h2>{t('codex.wakeup.testTitle')}</h2>
              <button className="modal-close" onClick={closeTestModal}>
                <X />
              </button>
            </div>
            <div className="modal-body codex-wakeup-modal-body">
              <ModalErrorMessage message={testModalError} scrollKey={testModalErrorScrollKey} />
              <div className="wakeup-form-group">
                <label>{t('codex.wakeup.testAccountsLabel')}</label>
                {renderAccountPickerFilters(
                  testAccountFilters,
                  setTestAccountFilters,
                  filteredTestAccounts,
                  allFilteredTestSelected,
                  () =>
                    setTestAccountIds((current) => {
                      const visibleIds = filteredTestAccounts.map((account) => account.id);
                      const visibleSet = new Set(visibleIds);
                      if (allFilteredTestSelected) {
                        return current.filter((item) => !visibleSet.has(item));
                      }
                      return Array.from(new Set([...current, ...visibleIds]));
                    }),
                )}
                {filteredTestAccounts.length === 0 ? (
                  <div className="codex-wakeup-account-empty">
                    {t('codex.wakeup.accountFilterEmpty')}
                  </div>
                ) : (
                  <div className="wakeup-chip-list codex-wakeup-account-list">
                    {filteredTestAccounts.map((account) => {
                    const checked = testAccountIds.includes(account.id);
                    return renderWakeupAccountOption(account, checked, () =>
                      setTestAccountIds((current) =>
                        checked
                          ? current.filter((item) => item !== account.id)
                          : [...current, account.id],
                      ),
                    );
                  })}
                  </div>
                )}
              </div>
              <div className="wakeup-form-group">
                <div className="codex-wakeup-inline-header">
                  <label>{t('codex.wakeup.testModelLabel')}</label>
                  <button type="button" className="btn btn-secondary" onClick={() => openPresetModal('test')}>
                    {t('codex.wakeup.managePresets')}
                  </button>
                </div>
                <p className="wakeup-hint">{t('codex.wakeup.testModelHint')}</p>
                <div className="codex-wakeup-dual-select">
                  <div className="codex-wakeup-dual-select-field">
                    <WakeupSingleSelectDropdown
                      value={testModelPresetId}
                      options={modelPresetOptions}
                      placeholder={t('codex.wakeup.selectPresetPlaceholder')}
                      onSelect={handleSelectTestPreset}
                    />
                  </div>
                  <div className="codex-wakeup-dual-select-field codex-wakeup-dual-select-field-compact">
                    <WakeupSingleSelectDropdown
                      value={testModelReasoningEffort}
                      options={testReasoningOptions}
                      placeholder={t('codex.wakeup.selectReasoningPlaceholder')}
                      onSelect={(value) => {
                        const nextReasoning = value as CodexWakeupReasoningEffort;
                        setTestModelReasoningEffort(nextReasoning);
                        if (selectedTestPreset) {
                          rememberModelSelection(
                            buildWakeupModelSelectionFromPreset(selectedTestPreset, nextReasoning),
                          );
                        }
                      }}
                      disabled={testReasoningOptions.length === 0}
                    />
                  </div>
                </div>
                {testModel && (
                  <p className="wakeup-hint">{t('codex.wakeup.modelValuePreview', { model: testModel })}</p>
                )}
                {testReasoningOptions.length === 0 ? (
                  <p className="wakeup-hint">{t('codex.wakeup.reasoningEffortEmpty')}</p>
                ) : null}
              </div>
              <div className="wakeup-form-group">
                <label>{t('codex.wakeup.promptLabel')}</label>
                <textarea
                  className="token-input codex-wakeup-prompt-input"
                  value={testPrompt}
                  onChange={(event) => setTestPrompt(event.target.value)}
                  placeholder={t('codex.wakeup.promptPlaceholder', { prompt: DEFAULT_PROMPT })}
                />
              </div>
            </div>
            <div className="modal-footer">
              <button className="btn btn-secondary" onClick={closeTestModal} disabled={testing}>
                {t('common.cancel')}
              </button>
              <button className="btn btn-primary" onClick={() => void handleRunTest()} disabled={testing || !runtime?.available}>
                {testing ? <RefreshCw size={16} className="loading-spinner" /> : <Play size={16} />}
                {testing ? t('codex.wakeup.testing') : t('codex.wakeup.startTest')}
              </button>
            </div>
          </div>
        </div>
      )}

      {showHistoryModal && (
        <div className="modal-overlay" onClick={() => setShowHistoryModal(false)}>
          <div className="modal wakeup-modal wakeup-history-modal codex-wakeup-history-modal" onClick={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <button className="btn btn-secondary icon-only" onClick={() => setShowHistoryModal(false)} title={t('common.back', '返回')} aria-label={t('common.back', '返回')}><ChevronLeft size={14} /></button>
              <h2>{t('codex.wakeup.historyTitle')}</h2>
              <button className="modal-close" onClick={() => setShowHistoryModal(false)}>
                <X />
              </button>
            </div>
            <div className="modal-body">
              {historyBatches.length === 0 ? (
                <p className="wakeup-hint">{t('codex.wakeup.historyEmptyDesc')}</p>
              ) : (
                <ul className="codex-wakeup-history-run-list">
                  {historyBatches.map((batch) => {
                    const badgeClass =
                      batch.triggerType === 'scheduled' ||
                      batch.triggerType === 'quota_reset' ||
                      batch.triggerType === 'startup'
                        ? 'auto'
                        : 'manual';
                    return (
                      <li key={batch.runId} className="codex-wakeup-history-run-card">
                        <div className="codex-wakeup-history-run-head">
                          <div className="codex-wakeup-history-run-copy">
                            <h4>
                              {batch.taskName ||
                                (batch.triggerType === 'test'
                                  ? t('codex.wakeup.testTitle')
                                  : triggerLabel(batch.triggerType, t))}
                            </h4>
                            <div className="codex-wakeup-history-run-meta">
                              <span>{formatHistoryTimestamp(batch.timestamp)}</span>
                              {batch.durationMs !== undefined && <span>{formatDuration(batch.durationMs)}</span>}
                              <span>{t('accounts.groups.accountCount', { count: batch.total })}</span>
                            </div>
                          </div>
                          <div className="codex-wakeup-history-run-actions">
                            <span className={`wakeup-history-badge codex-wakeup-history-trigger-badge ${badgeClass}`}>
                              {triggerLabel(batch.triggerType, t)}
                            </span>
                            <button
                              className="btn btn-secondary codex-wakeup-history-detail-btn"
                              onClick={() => {
                                setShowHistoryModal(false);
                                setExecutionSessionFromHistory(true);
                                setExecutionSession(buildExecutionSessionFromHistory(batch));
                              }}
                            >
                              {t('common.detail')}
                            </button>
                          </div>
                        </div>

                        <div className="codex-wakeup-history-run-stats">
                          <span className="codex-wakeup-history-stat-chip is-total">
                            <span>{t('codex.wakeup.resultsTotal')}</span>
                            <strong>{batch.total}</strong>
                          </span>
                          <span className="codex-wakeup-history-stat-chip is-success">
                            <span>{t('codex.wakeup.resultsSuccess')}</span>
                            <strong>{batch.successCount}</strong>
                          </span>
                          <span className="codex-wakeup-history-stat-chip is-error">
                            <span>{t('codex.wakeup.resultsFailed')}</span>
                            <strong>{batch.failureCount}</strong>
                          </span>
                        </div>
                      </li>
                    );
                  })}
                </ul>
              )}
            </div>
            <div className="modal-footer">
              <button className="btn btn-secondary codex-wakeup-subtle-btn" onClick={() => setShowHistoryModal(false)}>
                {t('common.close')}
              </button>
              <button className="btn btn-secondary codex-wakeup-subtle-btn" onClick={() => void handleClearHistory()} disabled={historyBatches.length === 0}>
                {t('codex.wakeup.clearHistory')}
              </button>
            </div>
          </div>
        </div>
      )}

      {executionSession && (
        <div
          className="modal-overlay"
          onClick={() => {
            if (!executionSession.running) {
              setExecutionSession(null);
              setExecutionSessionFromHistory(false);
            }
          }}
        >
          <div
            className="modal codex-wakeup-modal codex-wakeup-results-modal"
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-header">
              {executionSessionFromHistory && !executionSession.running && (
                <button className="btn btn-secondary icon-only" onClick={() => {
                  setExecutionSession(null);
                  setExecutionSessionFromHistory(false);
                  setShowHistoryModal(true);
                }} title={t('common.back', '返回')} aria-label={t('common.back', '返回')}><ChevronLeft size={14} /></button>
              )}
              <h2>{t('codex.wakeup.resultsTitle')}</h2>
              <button
                className="modal-close"
                onClick={() => { setExecutionSession(null); setExecutionSessionFromHistory(false); }}
                disabled={executionSession.running}
              >
                <X />
              </button>
            </div>
            <div className="modal-body codex-wakeup-modal-body codex-wakeup-results-body">
              <ModalErrorMessage message={executionSession.errorText} />
              <section className="codex-wakeup-results-summary-bar">
                <div className="codex-wakeup-results-summary-copy">
                  <div className="codex-wakeup-results-summary-head">
                    <span className="codex-wakeup-results-kicker">
                      {triggerLabel(executionSession.triggerType, t)}
                    </span>
                    <h3>{executionSession.title}</h3>
                  </div>
                  <div className="codex-wakeup-results-summary-meta">
                    <span>
                      {executionSession.preview
                        ? t('codex.wakeup.executionStatusPending')
                        : executionSession.running
                        ? t('codex.wakeup.executionStatusRunning')
                        : formatHistoryTimestamp(executionSession.startedAt)}
                    </span>
                    {!executionSession.running &&
                      !executionSession.preview &&
                      executionDuration !== undefined && (
                      <span>{formatDuration(executionDuration)}</span>
                    )}
                    <span>{t('accounts.groups.accountCount', { count: executionSession.total })}</span>
                  </div>
                </div>
                <div className="codex-wakeup-results-summary-progress">
                  <strong>
                    {executionSession.completed}/{executionSession.total}
                  </strong>
                  <span>
                    {executionSession.preview
                      ? t('codex.wakeup.executionStatusPending')
                      : executionSession.running
                      ? t('codex.wakeup.executionStatusRunning')
                      : t('codex.wakeup.executionStatusCompleted')}
                  </span>
                </div>
              </section>

              <section className="codex-wakeup-results-progress-strip">
                <div className="codex-wakeup-results-progress-head">
                  <span>{t('codex.wakeup.resultsTitle')}</span>
                  <strong>
                    {executionSession.completed}/{executionSession.total}
                  </strong>
                </div>
                <div className="codex-wakeup-results-progress-track">
                  <div
                    className="codex-wakeup-results-progress-fill"
                    style={{
                      width: `${
                        executionSession.total > 0
                          ? (executionSession.completed / executionSession.total) * 100
                          : 0
                      }%`,
                    }}
                  />
                </div>
              </section>

              <div className="codex-wakeup-results-filter-bar">
                {executionFilterOptions.map((option) => (
                  <button
                    key={option.key}
                    type="button"
                    className={`codex-wakeup-results-filter-chip ${
                      executionFilter === option.key ? 'active' : ''
                    } tone-${option.tone}`}
                    onClick={() => setExecutionFilter(option.key)}
                  >
                    <span>{option.label}</span>
                    <strong>{option.count}</strong>
                  </button>
                ))}
              </div>

              <div className="codex-wakeup-results-runtime-meta">
                <span>{t('codex.wakeup.runtimeCardTitle')}</span>
                <strong className="codex-wakeup-runtime-path">
                  {executionSession.runtime?.binary_path || t('codex.wakeup.runtimeUnknownPath')}
                </strong>
                {(executionSession.runtime?.version ||
                  executionSession.runtime?.source ||
                  executionSession.runtime?.message) && (
                  <span>
                    {[
                      executionSession.runtime?.version,
                      executionSession.runtime?.source,
                      executionSession.runtime?.message,
                    ]
                      .filter(Boolean)
                      .join(' · ')}
                  </span>
                )}
              </div>

              {executionSession.runtime && !executionSession.runtime.available && (
                <div className="codex-wakeup-runtime-warning">
                  <div>
                    <strong>{t('codex.wakeup.installTitle')}</strong>
                    <span>
                      {executionSession.runtime.message || t('codex.wakeup.installSubtitle')}
                    </span>
                  </div>
                  <button className="btn btn-secondary" onClick={openRuntimeGuideModal}>
                    {t('codex.wakeup.installTitle')}
                  </button>
                </div>
              )}
              <div className="codex-wakeup-results-list">
                {filteredExecutionRecords.map((record) => {
                  const maskedEmail = maskAccountText(record.accountEmail);
                  return (
                    <article
                      key={record.id}
                      className={`codex-wakeup-execution-row is-${record.status}`}
                    >
                      <div className="codex-wakeup-execution-row-head">
                        <div>
                          <h4 className="codex-wakeup-execution-row-title">
                            {record.accountContextText
                              ? `${maskedEmail} · ${record.accountContextText}`
                              : maskedEmail}
                          </h4>
                          <span className="codex-wakeup-execution-row-subtitle">
                            {triggerLabel(record.triggerType, t)}
                          </span>
                        </div>
                        <span className={`codex-wakeup-execution-badge is-${record.status}`}>
                          {record.status === 'running' && <RefreshCw size={14} className="loading-spinner" />}
                          {executionStatusLabel(record.status, t)}
                        </span>
                      </div>
                      {(record.model || record.modelReasoningEffort) && (
                        <div className="codex-wakeup-execution-row-prompt">
                          {t('codex.wakeup.modelSummaryLabel', {
                            model: formatWakeupModelLabel(record.model, record.modelDisplayName) || t('codex.wakeup.modelDefault'),
                            reasoning: record.modelReasoningEffort
                              ? reasoningEffortLabel(record.modelReasoningEffort, t)
                              : t('codex.wakeup.modelDefault'),
                          })}
                        </div>
                      )}
                      {record.prompt && (
                        <div className="codex-wakeup-execution-row-prompt">
                          {t('codex.wakeup.promptLabel')}: {record.prompt}
                        </div>
                      )}
                      <p className="codex-wakeup-execution-row-message">
                        {record.status === 'pending'
                          ? t('codex.wakeup.executionPendingDesc')
                          : record.status === 'running'
                            ? t('codex.wakeup.executionRunningDesc')
                            : record.status === 'success'
                              ? record.reply || t('codex.wakeup.historyNoReply')
                              : record.error || t('codex.wakeup.historyUnknownError')}
                      </p>
                      <div className="codex-wakeup-execution-row-meta">
                        {record.timestamp && <span>{formatHistoryTimestamp(record.timestamp)}</span>}
                        {record.durationMs !== undefined && <span>{formatDuration(record.durationMs)}</span>}
                      </div>
                    </article>
                  );
                })}
                {filteredExecutionRecords.length === 0 && (
                  <p className="wakeup-hint">{t('common.none', '暂无')}</p>
                )}
              </div>
            </div>
            <div className="modal-footer">
              {executionSession.running && executionSession.triggerType === 'test' && (
                <button
                  className="btn btn-secondary"
                  onClick={cancelRunningTest}
                >
                  {t('common.cancel')}
                </button>
              )}
              <button
                className="btn btn-primary codex-wakeup-results-close-btn"
                onClick={() => { setExecutionSession(null); setExecutionSessionFromHistory(false); }}
                disabled={executionSession.running}
              >
                {t('common.close', '关闭')}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
