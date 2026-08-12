import type { TFunction } from 'i18next';
import {
  DEFAULT_KIMI_WAKEUP_MODEL,
  DEFAULT_KIMI_WAKEUP_PROMPT,
  KIMI_BUILTIN_MODELS,
  normalizeKimiModelId,
  type KimiWakeupHistoryItem,
  type KimiWakeupQuotaResetWindow,
  type KimiWakeupScheduleKind,
  type KimiWakeupTask,
} from '../../types/kimiWakeup';

export interface KimiModelPreset {
  id: string;
  name: string;
  model: string;
}

export interface TaskDraft {
  id?: string;
  name: string;
  enabled: boolean;
  accountIds: string[];
  prompt: string;
  modelPresetId: string;
  model: string;
  modelDisplayName: string;
  scheduleKind: KimiWakeupScheduleKind;
  dailyTime: string;
  weeklyDays: number[];
  weeklyTime: string;
  intervalHours: string;
  quotaResetWindow: KimiWakeupQuotaResetWindow;
  startupDelayMode: 'immediate' | 'delayed';
  startupDelayMinutes: string;
}

export const WEEKDAY_OPTIONS = [
  { value: 1 },
  { value: 2 },
  { value: 3 },
  { value: 4 },
  { value: 5 },
  { value: 6 },
  { value: 0 },
];
export const QUICK_TIME_OPTIONS = ['07:00', '08:00', '09:00', '10:00', '14:00', '18:00', '22:00'];
export const MAX_STARTUP_DELAY_MINUTES = 1440;
export const PRESET_STORAGE_KEY = 'agtools.kimi.wakeup.model_presets';
export const MODEL_MEMORY_KEY = 'agtools.kimi.wakeup.model_selection';

export const BUILTIN_PRESETS: KimiModelPreset[] = KIMI_BUILTIN_MODELS.map((m) => ({
  id: m.id,
  name: m.displayName,
  model: m.id, // full alias for `kimi -m`
}));

export function loadCustomPresets(): KimiModelPreset[] {
  try {
    const raw = localStorage.getItem(PRESET_STORAGE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw) as KimiModelPreset[];
    if (!Array.isArray(parsed)) return [];
    return parsed
      .filter(
        (p) =>
          p &&
          typeof p.id === 'string' &&
          typeof p.model === 'string' &&
          typeof p.name === 'string',
      )
      .map((p) => ({
        ...p,
        model: normalizeKimiModelId(p.model),
        id: normalizeKimiModelId(p.id.includes('/') ? p.id : p.model),
      }))
      // Drop any accidental Codex / GPT leftovers from earlier experiments.
      .filter((p) => !/gpt|claude|codex|o3|o4/i.test(p.model + p.name));
  } catch {
    return [];
  }
}

export function saveCustomPresets(presets: KimiModelPreset[]) {
  try {
    localStorage.setItem(PRESET_STORAGE_KEY, JSON.stringify(presets));
  } catch {
    // ignore
  }
}

export function loadRememberedModel(): { modelPresetId: string; model: string } {
  const fallback = BUILTIN_PRESETS.find((p) => p.id === DEFAULT_KIMI_WAKEUP_MODEL) || BUILTIN_PRESETS[0];
  try {
    const raw = localStorage.getItem(MODEL_MEMORY_KEY);
    if (!raw) return { modelPresetId: fallback.id, model: fallback.model };
    const parsed = JSON.parse(raw) as { modelPresetId?: string; model?: string };
    const model = normalizeKimiModelId(parsed.model || parsed.modelPresetId || fallback.model);
    if (/gpt|claude|codex|o3|o4/i.test(model)) {
      return { modelPresetId: fallback.id, model: fallback.model };
    }
    return {
      modelPresetId: normalizeKimiModelId(parsed.modelPresetId || model),
      model,
    };
  } catch {
    return { modelPresetId: fallback.id, model: fallback.model };
  }
}

export function rememberModel(modelPresetId: string, model: string) {
  try {
    localStorage.setItem(MODEL_MEMORY_KEY, JSON.stringify({ modelPresetId, model }));
  } catch {
    // ignore
  }
}

export function formatDateTime(value?: number) {
  if (!value) return '—';
  const ms = value > 1e12 ? value : value * 1000;
  return new Date(ms).toLocaleString();
}

export function formatDuration(value?: number) {
  if (value == null) return '—';
  if (value < 1000) return `${value}ms`;
  return `${(value / 1000).toFixed(1)}s`;
}

export function formatSelectionPreview(labels: string[], max = 2): string {
  if (labels.length === 0) return '—';
  if (labels.length <= max) return labels.join(' / ');
  return `${labels.slice(0, max).join(' / ')} +${labels.length - max}`;
}

export function scheduleSummary(
  task: KimiWakeupTask,
  t: TFunction,
): string {
  const schedule = task.schedule;
  if (schedule.kind === 'startup') {
    const delay = schedule.startup_delay_minutes ?? 0;
    if (delay <= 0) return t('settings.general.startupWakeupImmediate', '启动时立即执行');
    return `${t('wakeup.triggerSource.startup', '启动时')} +${delay}${t('settings.general.minutes', '分钟')}`;
  }
  if (schedule.kind === 'daily') {
    return t('codex.wakeup.scheduleDailySummary', {
      time: schedule.daily_time || '09:00',
      defaultValue: `每天 ${schedule.daily_time || '09:00'}`,
    });
  }
  if (schedule.kind === 'weekly') {
    const days = (schedule.weekly_days || [])
      .map((day) => t(`codex.wakeup.weekdays.${day}`, String(day)))
      .join(' / ');
    return t('codex.wakeup.scheduleWeeklySummary', {
      days: days || t('codex.wakeup.weekdaysFallback', '选定日'),
      time: schedule.weekly_time || '10:00',
      defaultValue: `每周 ${days || '—'} ${schedule.weekly_time || '10:00'}`,
    });
  }
  if (schedule.kind === 'quota_reset') {
    const windowKey = schedule.quota_reset_window || 'either';
    const windowLabel = t(`codex.wakeup.quotaResetWindowOptions.${windowKey}`, windowKey);
    return t('codex.wakeup.scheduleQuotaResetSummary', {
      window: windowLabel,
      defaultValue: `额度重置后自动触发（${windowLabel}）`,
    });
  }
  return t('codex.wakeup.scheduleIntervalSummary', {
    hours: schedule.interval_hours ?? 6,
    defaultValue: `每 ${schedule.interval_hours ?? 6} 小时`,
  });
}

export function formatTaskLastResult(
  task: KimiWakeupTask,
  t: TFunction,
): string {
  const successCount = task.last_success_count ?? 0;
  const failureCount = task.last_failure_count ?? 0;
  if (successCount > 0 || failureCount > 0) {
    if (failureCount === 0) {
      return t('codex.wakeup.lastStatusSuccessSummary', {
        count: successCount,
        defaultValue: `成功 ${successCount} 个账号`,
      });
    }
    if (successCount === 0) {
      return t('codex.wakeup.lastStatusFailedSummary', {
        count: failureCount,
        defaultValue: `失败 ${failureCount} 个账号`,
      });
    }
    return t('codex.wakeup.lastStatusMixedSummary', {
      success: successCount,
      failed: failureCount,
      defaultValue: `成功 ${successCount} / 失败 ${failureCount}`,
    });
  }
  if (task.last_status === 'success') return t('common.success', '成功');
  if (task.last_status === 'error') return t('codex.wakeup.historyFailed', '失败');
  return task.last_message || t('codex.wakeup.lastStatusIdle', '尚未执行');
}

export function triggerLabel(triggerType: string, t: TFunction) {
  if (triggerType === 'scheduled') return t('codex.wakeup.triggerScheduled', '定时');
  if (triggerType === 'quota_reset') return t('codex.wakeup.triggerQuotaReset', '额度重置');
  if (triggerType === 'startup') return t('wakeup.triggerSource.startup', '启动时');
  if (triggerType === 'manual_task') return t('codex.wakeup.triggerManualTask', '手动任务');
  return t('codex.wakeup.triggerTest', '测试');
}

export function groupHistoryByRun(history: KimiWakeupHistoryItem[]) {
  const map = new Map<
    string,
    {
      runId: string;
      timestamp: number;
      triggerType: string;
      taskName?: string;
      records: KimiWakeupHistoryItem[];
      successCount: number;
      failureCount: number;
      durationMs?: number;
    }
  >();
  for (const item of history) {
    const key = item.run_id || item.id;
    let batch = map.get(key);
    if (!batch) {
      batch = {
        runId: key,
        timestamp: item.timestamp,
        triggerType: item.trigger_type,
        taskName: item.task_name,
        records: [],
        successCount: 0,
        failureCount: 0,
      };
      map.set(key, batch);
    }
    batch.records.push(item);
    if (item.success) batch.successCount += 1;
    else batch.failureCount += 1;
    if (item.duration_ms != null) {
      batch.durationMs = (batch.durationMs || 0) + item.duration_ms;
    }
    if (item.timestamp > batch.timestamp) batch.timestamp = item.timestamp;
    if (!batch.taskName && item.task_name) batch.taskName = item.task_name;
  }
  return Array.from(map.values()).sort((a, b) => b.timestamp - a.timestamp);
}

export function createEmptyTaskDraft(preset?: KimiModelPreset | null): TaskDraft {
  const remembered = loadRememberedModel();
  const usePreset = preset || BUILTIN_PRESETS.find((p) => p.id === remembered.modelPresetId) || BUILTIN_PRESETS[0];
  return {
    name: '',
    enabled: true,
    accountIds: [],
    prompt: DEFAULT_KIMI_WAKEUP_PROMPT,
    modelPresetId: usePreset.id,
    model: usePreset.model,
    modelDisplayName: usePreset.name,
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

export function buildTaskDraft(task: KimiWakeupTask, presets: KimiModelPreset[]): TaskDraft {
  const modelId = normalizeKimiModelId(task.model);
  const matched =
    presets.find((p) => p.model === modelId || p.id === modelId) ||
    null;
  const delay = task.schedule.startup_delay_minutes ?? 0;
  return {
    id: task.id,
    name: task.name,
    enabled: task.enabled,
    accountIds: [...task.account_ids],
    prompt: task.prompt || DEFAULT_KIMI_WAKEUP_PROMPT,
    modelPresetId: matched?.id || modelId,
    model: modelId,
    modelDisplayName: matched?.name || modelId,
    scheduleKind: task.schedule.kind,
    dailyTime: task.schedule.daily_time || '09:00',
    weeklyDays: task.schedule.weekly_days?.length ? [...task.schedule.weekly_days] : [1, 2, 3, 4, 5],
    weeklyTime: task.schedule.weekly_time || '10:00',
    intervalHours: String(task.schedule.interval_hours ?? 6),
    quotaResetWindow: task.schedule.quota_reset_window || 'either',
    startupDelayMode: delay > 0 ? 'delayed' : 'immediate',
    startupDelayMinutes: String(delay > 0 ? delay : 1),
  };
}

export function calculatePreviewRuns(draft: TaskDraft, count = 5): Date[] {
  const now = new Date();
  const results: Date[] = [];
  if (draft.scheduleKind === 'daily') {
    const [hh, mm] = (draft.dailyTime || '09:00').split(':').map((x) => Number(x) || 0);
    let cursor = new Date(now);
    cursor.setSeconds(0, 0);
    cursor.setHours(hh, mm, 0, 0);
    if (cursor <= now) cursor = new Date(cursor.getTime() + 24 * 3600 * 1000);
    for (let i = 0; i < count; i++) {
      results.push(new Date(cursor));
      cursor = new Date(cursor.getTime() + 24 * 3600 * 1000);
    }
    return results;
  }
  if (draft.scheduleKind === 'weekly') {
    const days = draft.weeklyDays.length ? draft.weeklyDays : [1];
    const [hh, mm] = (draft.weeklyTime || '10:00').split(':').map((x) => Number(x) || 0);
    let cursor = new Date(now);
    cursor.setSeconds(0, 0);
    for (let guard = 0; results.length < count && guard < 60; guard++) {
      cursor = new Date(cursor.getTime() + (guard === 0 ? 0 : 24 * 3600 * 1000));
      if (guard === 0) {
        // start from today
      } else {
        // already advanced
      }
      const day = cursor.getDay();
      if (!days.includes(day)) continue;
      const candidate = new Date(cursor);
      candidate.setHours(hh, mm, 0, 0);
      if (candidate <= now) continue;
      if (results.some((d) => d.getTime() === candidate.getTime())) continue;
      results.push(candidate);
    }
    return results;
  }
  if (draft.scheduleKind === 'interval') {
    const hours = Math.max(1, Number(draft.intervalHours) || 6);
    let cursor = new Date(now.getTime() + hours * 3600 * 1000);
    for (let i = 0; i < count; i++) {
      results.push(new Date(cursor));
      cursor = new Date(cursor.getTime() + hours * 3600 * 1000);
    }
    return results;
  }
  return [];
}

