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
import { open as openFileDialog } from '@tauri-apps/plugin-dialog';
import {
  Check,
  ChevronDown,
  ChevronLeft,
  CircleAlert,
  FolderOpen,
  Pencil,
  Play,
  Plus,
  Power,
  RefreshCw,
  Search,
  Terminal,
  Trash2,
  X,
} from 'lucide-react';
import * as kimiWakeupService from '../../services/kimiWakeupService';
import { useKimiWakeupStore } from '../../stores/useKimiWakeupStore';
import type { KimiAccount } from '../../types/kimi';
import {
  getKimiAccountDisplayEmail,
  getKimiPlanBadge,
  getKimiQuotaClass,
  getKimiQuotaSummaryItems,
} from '../../types/kimi';
import {
  DEFAULT_KIMI_WAKEUP_MODEL,
  DEFAULT_KIMI_WAKEUP_PROMPT,
  normalizeKimiModelId,
  type KimiWakeupQuotaResetWindow,
  type KimiWakeupScheduleKind,
  type KimiWakeupTask,
} from '../../types/kimiWakeup';
import { ModalErrorMessage, useModalErrorState } from '../ModalErrorMessage';
import {
  BUILTIN_PRESETS,
  MAX_STARTUP_DELAY_MINUTES,
  QUICK_TIME_OPTIONS,
  WEEKDAY_OPTIONS,
  buildTaskDraft,
  calculatePreviewRuns,
  createEmptyTaskDraft,
  formatDateTime,
  formatDuration,
  formatSelectionPreview,
  formatTaskLastResult,
  groupHistoryByRun,
  loadCustomPresets,
  loadRememberedModel,
  rememberModel,
  saveCustomPresets,
  scheduleSummary,
  triggerLabel,
  type KimiModelPreset,
  type TaskDraft,
} from './kimiWakeupUiUtils';

interface KimiWakeupContentProps {
  accounts: KimiAccount[];
  onRefreshAccounts: () => Promise<void>;
}

interface WakeupSingleSelectOption {
  value: string;
  label: string;
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
  const [panelPosition, setPanelPosition] = useState<{
    top: number;
    left: number;
    width: number;
  } | null>(null);
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
    if (disabled) setOpen(false);
  }, [disabled]);

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
              selectedOption
                ? 'codex-wakeup-single-select-text'
                : 'codex-wakeup-single-select-placeholder'
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

export function KimiWakeupContent({
  accounts,
  onRefreshAccounts,
}: KimiWakeupContentProps) {
  const { t } = useTranslation();
  const {
    state,
    history,
    runtime,
    loading,
    error,
    fetchOverview,
    setEnabled,
    upsertTask,
    deleteTask,
    toggleTask,
    clearHistory,
  } = useKimiWakeupStore();

  const [customPresets, setCustomPresets] = useState<KimiModelPreset[]>(() => loadCustomPresets());
  const allPresets = useMemo(
    () => [...BUILTIN_PRESETS, ...customPresets],
    [customPresets],
  );
  const presetMap = useMemo(() => {
    const map = new Map<string, KimiModelPreset>();
    for (const p of allPresets) map.set(p.id, p);
    return map;
  }, [allPresets]);

  const [taskDraft, setTaskDraft] = useState<TaskDraft>(() => createEmptyTaskDraft());
  const [showTaskModal, setShowTaskModal] = useState(false);
  const [showHistoryModal, setShowHistoryModal] = useState(false);
  const [showTestModal, setShowTestModal] = useState(false);
  const [showPresetModal, setShowPresetModal] = useState(false);
  const [showCliModal, setShowCliModal] = useState(false);
  const [presetDraft, setPresetDraft] = useState({ id: '', name: '', model: '' });
  const [testAccountIds, setTestAccountIds] = useState<string[]>([]);
  const [testModelPresetId, setTestModelPresetId] = useState(BUILTIN_PRESETS[0].id);
  const [testPrompt, setTestPrompt] = useState(DEFAULT_KIMI_WAKEUP_PROMPT);
  const [accountQuery, setAccountQuery] = useState('');
  const [testAccountQuery, setTestAccountQuery] = useState('');
  const [cliPath, setCliPath] = useState('');
  const [cliBusy, setCliBusy] = useState(false);
  const [cliDetecting, setCliDetecting] = useState(false);
  const [cliModalSuccess, setCliModalSuccess] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [runningTaskId, setRunningTaskId] = useState<string | null>(null);
  const [notice, setNotice] = useState<{ tone: 'success' | 'error'; text: string } | null>(null);
  const {
    message: taskModalError,
    scrollKey: taskModalErrorScrollKey,
    report: reportTaskModalError,
    clear: clearTaskModalError,
  } = useModalErrorState();
  const {
    message: testModalError,
    scrollKey: testModalErrorScrollKey,
    report: reportTestModalError,
    clear: clearTestModalError,
  } = useModalErrorState();
  const {
    message: presetModalError,
    scrollKey: presetModalErrorScrollKey,
    report: reportPresetModalError,
    clear: clearPresetModalError,
  } = useModalErrorState();
  const {
    message: cliModalError,
    scrollKey: cliModalErrorScrollKey,
    report: reportCliModalError,
    clear: clearCliModalError,
  } = useModalErrorState();
  const [taskModalSuccess, setTaskModalSuccess] = useState<string | null>(null);
  const [presetModalSuccess, setPresetModalSuccess] = useState<string | null>(null);

  useEffect(() => {
    void fetchOverview();
  }, [fetchOverview]);

  useEffect(() => {
    if (runtime?.configured_path) setCliPath(runtime.configured_path);
    else if (runtime?.binary_path) setCliPath(runtime.binary_path);
  }, [runtime?.configured_path, runtime?.binary_path]);

  const accountMap = useMemo(() => {
    const map = new Map<string, KimiAccount>();
    for (const a of accounts) map.set(a.id, a);
    return map;
  }, [accounts]);

  const sortedTasks = useMemo(
    () =>
      [...state.tasks].sort((a, b) => {
        if (a.enabled !== b.enabled) return a.enabled ? -1 : 1;
        return (b.updated_at || 0) - (a.updated_at || 0);
      }),
    [state.tasks],
  );

  const historyBatches = useMemo(() => groupHistoryByRun(history), [history]);

  const modelPresetOptions = useMemo<WakeupSingleSelectOption[]>(
    () => allPresets.map((p) => ({ value: p.id, label: p.name })),
    [allPresets],
  );

  const previewRuns = useMemo(() => calculatePreviewRuns(taskDraft), [taskDraft]);

  const filterAccounts = useCallback(
    (query: string) => {
      const q = query.trim().toLowerCase();
      if (!q) return accounts;
      return accounts.filter((a) => {
        const email = getKimiAccountDisplayEmail(a).toLowerCase();
        const tags = (a.tags || []).join(' ').toLowerCase();
        return email.includes(q) || tags.includes(q) || a.id.toLowerCase().includes(q);
      });
    },
    [accounts],
  );

  const filteredTaskAccounts = useMemo(
    () => filterAccounts(accountQuery),
    [accountQuery, filterAccounts],
  );
  const filteredTestAccounts = useMemo(
    () => filterAccounts(testAccountQuery),
    [testAccountQuery, filterAccounts],
  );

  const selectPresetOnDraft = useCallback(
    (presetId: string, setDraft: Dispatch<SetStateAction<TaskDraft>>) => {
      const preset = presetMap.get(presetId);
      if (!preset) return;
      setDraft((current) => ({
        ...current,
        modelPresetId: preset.id,
        model: preset.model,
        modelDisplayName: preset.name,
      }));
      rememberModel(preset.id, preset.model);
    },
    [presetMap],
  );

  const openNewTaskModal = () => {
    const remembered = loadRememberedModel();
    const preset = presetMap.get(remembered.modelPresetId) || BUILTIN_PRESETS[0];
    const draft = createEmptyTaskDraft(preset);
    setTaskDraft(draft);
    setAccountQuery('');
    clearTaskModalError();
    setTaskModalSuccess(null);
    setShowTaskModal(true);
  };

  const openEditTaskModal = (task: KimiWakeupTask) => {
    setTaskDraft(buildTaskDraft(task, allPresets));
    setAccountQuery('');
    clearTaskModalError();
    setTaskModalSuccess(null);
    setShowTaskModal(true);
  };

  const closeTaskModal = () => {
    if (busy) return;
    clearTaskModalError();
    setTaskModalSuccess(null);
    setShowTaskModal(false);
  };

  const handleSaveTask = async () => {
    clearTaskModalError();
    setTaskModalSuccess(null);
    if (!taskDraft.name.trim()) {
      reportTaskModalError(t('kimi.wakeup.nameRequired', '请填写任务名称'));
      return;
    }
    if (taskDraft.accountIds.length === 0) {
      reportTaskModalError(t('kimi.wakeup.accountsRequired', '请至少选择一个账号'));
      return;
    }
    if (!taskDraft.model.trim()) {
      reportTaskModalError(t('codex.wakeup.modelRequired', '请选择模型'));
      return;
    }
    if (taskDraft.scheduleKind === 'weekly' && taskDraft.weeklyDays.length === 0) {
      reportTaskModalError(t('codex.wakeup.weeklyDaysRequired', '请至少选择一个星期'));
      return;
    }

    const startupDelay =
      taskDraft.scheduleKind === 'startup' && taskDraft.startupDelayMode === 'delayed'
        ? Math.min(
            MAX_STARTUP_DELAY_MINUTES,
            Math.max(1, Number(taskDraft.startupDelayMinutes) || 1),
          )
        : 0;

    const now = Math.floor(Date.now() / 1000);
    const task: KimiWakeupTask = {
      id: taskDraft.id || `kimi-task-${Date.now()}`,
      name: taskDraft.name.trim(),
      enabled: taskDraft.enabled,
      account_ids: taskDraft.accountIds,
      prompt: taskDraft.prompt || DEFAULT_KIMI_WAKEUP_PROMPT,
      model: normalizeKimiModelId(taskDraft.model),
      schedule: {
        kind: taskDraft.scheduleKind,
        daily_time: taskDraft.scheduleKind === 'daily' ? taskDraft.dailyTime : undefined,
        weekly_days: taskDraft.scheduleKind === 'weekly' ? taskDraft.weeklyDays : [],
        weekly_time: taskDraft.scheduleKind === 'weekly' ? taskDraft.weeklyTime : undefined,
        interval_hours:
          taskDraft.scheduleKind === 'interval'
            ? Math.max(1, Number(taskDraft.intervalHours) || 6)
            : undefined,
        quota_reset_window:
          taskDraft.scheduleKind === 'quota_reset' ? taskDraft.quotaResetWindow : undefined,
        startup_delay_minutes:
          taskDraft.scheduleKind === 'startup' ? startupDelay : undefined,
      },
      created_at: taskDraft.id
        ? state.tasks.find((x) => x.id === taskDraft.id)?.created_at || now
        : now,
      updated_at: now,
      last_run_at: taskDraft.id
        ? state.tasks.find((x) => x.id === taskDraft.id)?.last_run_at
        : undefined,
      last_status: taskDraft.id
        ? state.tasks.find((x) => x.id === taskDraft.id)?.last_status
        : undefined,
      last_message: taskDraft.id
        ? state.tasks.find((x) => x.id === taskDraft.id)?.last_message
        : undefined,
      last_success_count: taskDraft.id
        ? state.tasks.find((x) => x.id === taskDraft.id)?.last_success_count
        : undefined,
      last_failure_count: taskDraft.id
        ? state.tasks.find((x) => x.id === taskDraft.id)?.last_failure_count
        : undefined,
      last_duration_ms: taskDraft.id
        ? state.tasks.find((x) => x.id === taskDraft.id)?.last_duration_ms
        : undefined,
    };

    setBusy(true);
    try {
      await upsertTask(task);
      rememberModel(taskDraft.modelPresetId, taskDraft.model);
      setTaskModalSuccess(t('kimi.wakeup.saved', '任务已保存'));
      setNotice({ tone: 'success', text: t('kimi.wakeup.saved', '任务已保存') });
      // Keep modal open briefly so the user sees in-modal feedback, then close.
      window.setTimeout(() => {
        setShowTaskModal(false);
        setTaskModalSuccess(null);
        clearTaskModalError();
      }, 450);
    } catch (e) {
      reportTaskModalError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const handleRunTask = async (task: KimiWakeupTask) => {
    setRunningTaskId(task.id);
    setNotice(null);
    try {
      const result = await kimiWakeupService.runKimiWakeupTask(task.id);
      const firstError = result.records.find((r) => !r.success)?.error;
      const firstReply = result.records.find((r) => r.success && r.reply)?.reply;
      const detail =
        firstError ||
        (firstReply ? firstReply.slice(0, 160) : '') ||
        (!result.runtime.available
          ? result.runtime.message || 'Kimi CLI 不可用'
          : '');
      setNotice({
        tone: result.failure_count > 0 || result.success_count === 0 ? 'error' : 'success',
        text: [
          t('kimi.wakeup.runDone', '完成：成功 {{ok}} / 失败 {{fail}}', {
            ok: result.success_count,
            fail: result.failure_count,
          }),
          detail ? ` — ${detail}` : '',
        ].join(''),
      });
      await fetchOverview();
      await onRefreshAccounts();
    } catch (e) {
      setNotice({ tone: 'error', text: String(e) });
    } finally {
      setRunningTaskId(null);
    }
  };

  const handleToggleTask = async (task: KimiWakeupTask) => {
    try {
      await toggleTask(task.id, !task.enabled);
    } catch (e) {
      setNotice({ tone: 'error', text: String(e) });
    }
  };

  const handleDeleteTask = async (task: KimiWakeupTask) => {
    try {
      await deleteTask(task.id);
      setNotice({ tone: 'success', text: t('common.deleted', '已删除') });
    } catch (e) {
      setNotice({ tone: 'error', text: String(e) });
    }
  };

  const openTestModal = () => {
    const remembered = loadRememberedModel();
    setTestModelPresetId(remembered.modelPresetId || BUILTIN_PRESETS[0].id);
    setTestAccountIds(accounts[0] ? [accounts[0].id] : []);
    setTestPrompt(DEFAULT_KIMI_WAKEUP_PROMPT);
    setTestAccountQuery('');
    clearTestModalError();
    setShowTestModal(true);
  };

  const handleTestRun = async () => {
    clearTestModalError();
    if (testAccountIds.length === 0) {
      reportTestModalError(t('kimi.wakeup.accountsRequired', '请至少选择一个账号'));
      return;
    }
    const preset =
      presetMap.get(testModelPresetId) ||
      BUILTIN_PRESETS.find((p) => p.id === DEFAULT_KIMI_WAKEUP_MODEL) ||
      BUILTIN_PRESETS[0];
    const modelId = normalizeKimiModelId(preset.model);
    setBusy(true);
    setNotice(null);
    try {
      const result = await kimiWakeupService.testKimiWakeup(
        testAccountIds,
        testPrompt || DEFAULT_KIMI_WAKEUP_PROMPT,
        modelId,
      );
      rememberModel(preset.id, modelId);
      const firstError = result.records.find((r) => !r.success)?.error;
      const firstReply = result.records.find((r) => r.success && r.reply)?.reply;
      const detail =
        firstError ||
        (firstReply ? firstReply.slice(0, 160) : '') ||
        (!result.runtime.available
          ? result.runtime.message || 'Kimi CLI 不可用'
          : '');
      const summary = [
        t('kimi.wakeup.testDone', '测试完成：成功 {{ok}} / 失败 {{fail}}', {
          ok: result.success_count,
          fail: result.failure_count,
        }),
        detail ? ` — ${detail}` : '',
      ].join('');
      if (result.failure_count > 0 || result.success_count === 0) {
        reportTestModalError(summary);
      } else {
        setShowTestModal(false);
        clearTestModalError();
        setNotice({ tone: 'success', text: summary });
      }
      await fetchOverview();
      await onRefreshAccounts();
    } catch (e) {
      reportTestModalError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const openCliModal = useCallback(() => {
    clearCliModalError();
    setCliModalSuccess(null);
    if (runtime?.configured_path) setCliPath(runtime.configured_path);
    else if (runtime?.binary_path) setCliPath(runtime.binary_path);
    setShowCliModal(true);
  }, [clearCliModalError, runtime?.binary_path, runtime?.configured_path]);

  const closeCliModal = useCallback(() => {
    if (cliBusy || cliDetecting) return;
    clearCliModalError();
    setCliModalSuccess(null);
    setShowCliModal(false);
  }, [clearCliModalError, cliBusy, cliDetecting]);

  const saveCliPath = useCallback(async () => {
    clearCliModalError();
    setCliModalSuccess(null);
    setCliBusy(true);
    try {
      await kimiWakeupService.updateKimiWakeupRuntimeConfig({
        kimi_cli_path: cliPath.trim() || null,
      });
      await fetchOverview();
      const status = await kimiWakeupService.getKimiWakeupCliStatus();
      const savedMsg = t('kimi.wakeup.cliPathSaved', 'CLI 路径已保存');
      if (!status.available) {
        reportCliModalError(
          status.message ||
            t('kimi.wakeup.cliMissing', '未检测到 kimi CLI'),
        );
        setNotice({ tone: 'error', text: status.message || savedMsg });
        return;
      }
      setCliModalSuccess(
        t('kimi.wakeup.cliOk', '已检测 {{path}}', {
          path: status.binary_path || cliPath.trim() || '--',
        }),
      );
      setNotice({ tone: 'success', text: savedMsg });
      window.setTimeout(() => {
        setShowCliModal(false);
        setCliModalSuccess(null);
        clearCliModalError();
      }, 450);
    } catch (e) {
      reportCliModalError(String(e));
    } finally {
      setCliBusy(false);
    }
  }, [clearCliModalError, cliPath, fetchOverview, reportCliModalError, t]);

  const handleBrowseCliPath = useCallback(async () => {
    clearCliModalError();
    setCliModalSuccess(null);
    try {
      const selected = await openFileDialog({
        multiple: false,
        directory: false,
        title: t('kimi.wakeup.cliBrowseTitle', '选择 Kimi CLI 可执行文件'),
        filters: [
          { name: 'Executable', extensions: ['exe', 'cmd', 'bat', 'ps1'] },
          { name: 'All', extensions: ['*'] },
        ],
      });
      const path = Array.isArray(selected) ? selected[0] : selected;
      if (!path) return;
      setCliPath(path);
      setCliModalSuccess(t('kimi.wakeup.cliBrowsePicked', '已选择文件，请点保存'));
    } catch (e) {
      reportCliModalError(String(e));
    }
  }, [clearCliModalError, reportCliModalError, t]);

  const handleDetectCliPath = useCallback(async () => {
    clearCliModalError();
    setCliModalSuccess(null);
    setCliDetecting(true);
    try {
      const status = await kimiWakeupService.detectKimiWakeupCli();
      if (!status.available || !status.binary_path) {
        reportCliModalError(
          status.message ||
            t(
              'kimi.wakeup.cliDetectFailed',
              '未在本机 PATH / 常见安装目录找到 kimi CLI，请手动选择可执行文件',
            ),
        );
        return;
      }
      setCliPath(status.binary_path);
      // Persist immediately so user doesn't need a second click after a successful detect.
      await kimiWakeupService.updateKimiWakeupRuntimeConfig({
        kimi_cli_path: status.binary_path,
      });
      await fetchOverview();
      setCliModalSuccess(
        t('kimi.wakeup.cliDetectOk', '已自动检测并保存：{{path}}', {
          path: status.binary_path,
        }),
      );
      setNotice({
        tone: 'success',
        text: t('kimi.wakeup.cliDetectOk', '已自动检测并保存：{{path}}', {
          path: status.binary_path,
        }),
      });
    } catch (e) {
      reportCliModalError(String(e));
    } finally {
      setCliDetecting(false);
    }
  }, [clearCliModalError, fetchOverview, reportCliModalError, t]);

  const handleSavePreset = () => {
    clearPresetModalError();
    setPresetModalSuccess(null);
    const name = presetDraft.name.trim();
    const model = presetDraft.model.trim();
    if (!name || !model) {
      reportPresetModalError(t('codex.wakeup.presetFieldsRequired', '请填写预设名称和模型 ID'));
      return;
    }
    if (/gpt|claude|codex|o3|o4/i.test(model + name)) {
      reportPresetModalError(
        t(
          'kimi.wakeup.invalidModelId',
          '请填写 Kimi 模型别名（如 kimi-code/kimi-for-coding），不要用 GPT/Codex 模型',
        ),
      );
      return;
    }
    const modelId = normalizeKimiModelId(model);
    if (presetDraft.id && BUILTIN_PRESETS.some((p) => p.id === presetDraft.id)) {
      reportPresetModalError(t('kimi.wakeup.builtinPresetReadonly', '内置预设不可修改，请新增'));
      return;
    }
    let next: KimiModelPreset[];
    if (presetDraft.id && customPresets.some((p) => p.id === presetDraft.id)) {
      next = customPresets.map((p) =>
        p.id === presetDraft.id ? { ...p, name, model: modelId, id: modelId } : p,
      );
    } else {
      next = [
        ...customPresets,
        { id: modelId, name, model: modelId },
      ];
    }
    setCustomPresets(next);
    saveCustomPresets(next);
    setPresetDraft({ id: '', name: '', model: '' });
    const msg = t('codex.wakeup.presetSaved', '预设已保存');
    setPresetModalSuccess(msg);
    setNotice({ tone: 'success', text: msg });
  };

  const handleDeletePreset = (preset: KimiModelPreset) => {
    if (BUILTIN_PRESETS.some((p) => p.id === preset.id)) return;
    const next = customPresets.filter((p) => p.id !== preset.id);
    setCustomPresets(next);
    saveCustomPresets(next);
    if (presetDraft.id === preset.id) {
      setPresetDraft({ id: '', name: '', model: '' });
    }
  };

  const renderAccountChip = (
    account: KimiAccount,
    checked: boolean,
    onToggle: () => void,
  ) => {
    const items = getKimiQuotaSummaryItems(account);
    const primary = items[0];
    const secondary = items[1];
    const remaining = (item: (typeof items)[0] | undefined) => {
      if (!item) return '--';
      const used = Math.round(item.percentage);
      return `${Math.max(0, Math.min(100, 100 - used))}%`;
    };
    const plan = getKimiPlanBadge(account) || t('common.none', '暂无');
    return (
      <button
        key={account.id}
        type="button"
        className={`wakeup-chip codex-wakeup-account-chip ${checked ? 'selected' : ''}`}
        onClick={onToggle}
        title={getKimiAccountDisplayEmail(account)}
      >
        <div className="codex-wakeup-account-chip-head">
          <span className="codex-wakeup-account-chip-email">
            {getKimiAccountDisplayEmail(account)}
          </span>
          <span className="tier-badge pro">{plan}</span>
        </div>
        <div className="codex-wakeup-account-chip-meta">
          <div className="codex-wakeup-account-chip-quotas">
            <span className="codex-wakeup-account-chip-quota codex-wakeup-account-chip-quota-primary">
              <span className="codex-wakeup-account-chip-quota-dot" />
              <span
                className={`codex-wakeup-account-chip-quota-value ${getKimiQuotaClass(primary?.percentage)}`}
              >
                {remaining(primary)}
              </span>
            </span>
            <span className="codex-wakeup-account-chip-quota codex-wakeup-account-chip-quota-secondary">
              <span className="codex-wakeup-account-chip-quota-dot" />
              <span
                className={`codex-wakeup-account-chip-quota-value ${getKimiQuotaClass(secondary?.percentage)}`}
              >
                {remaining(secondary)}
              </span>
            </span>
          </div>
        </div>
      </button>
    );
  };

  const renderAccountPicker = (
    filtered: KimiAccount[],
    selectedIds: string[],
    setSelected: (ids: string[]) => void,
    query: string,
    setQuery: (q: string) => void,
  ) => {
    const allSelected =
      filtered.length > 0 && filtered.every((a) => selectedIds.includes(a.id));
    return (
      <>
        <p className="wakeup-hint">{t('codex.wakeup.taskAccountsHint', '可多选账号，执行时按顺序依次唤醒。')}</p>
        <div className="codex-wakeup-account-filter-toolbar">
          <label className="codex-wakeup-account-search">
            <Search size={16} className="codex-wakeup-account-search-icon" />
            <input
              className="wakeup-input"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={t('codex.wakeup.accountSearchPlaceholder', '搜索账号…')}
            />
          </label>
          <div className="codex-wakeup-account-filter-actions">
            <button
              type="button"
              className="btn btn-secondary"
              onClick={() => {
                const visible = new Set(filtered.map((a) => a.id));
                if (allSelected) {
                  setSelected(selectedIds.filter((id) => !visible.has(id)));
                } else {
                  setSelected(Array.from(new Set([...selectedIds, ...visible])));
                }
              }}
            >
              {allSelected
                ? t('common.deselectAll', '取消全选')
                : t('common.selectAll', '全选')}
            </button>
          </div>
        </div>
        {filtered.length === 0 ? (
          <div className="codex-wakeup-account-empty">
            {t('codex.wakeup.accountFilterEmpty', '没有匹配的账号')}
          </div>
        ) : (
          <div className="wakeup-chip-list codex-wakeup-account-list">
            {filtered.map((account) => {
              const checked = selectedIds.includes(account.id);
              return renderAccountChip(account, checked, () => {
                setSelected(
                  checked
                    ? selectedIds.filter((id) => id !== account.id)
                    : [...selectedIds, account.id],
                );
              });
            })}
          </div>
        )}
      </>
    );
  };

  const cliMissing = runtime != null && !runtime.available;

  return (
    <div className="wakeup-page codex-wakeup-content">
      {(notice || error) && (
        <div
          className={`action-message ${notice?.tone === 'error' || error ? 'error' : 'success'}`}
        >
          <span className="action-message-text">{error || notice?.text}</span>
          <button
            className="action-message-close"
            onClick={() => setNotice(null)}
            aria-label={t('common.close', '关闭')}
          >
            <X size={14} />
          </button>
        </div>
      )}

      {cliMissing && (
        <div className="action-message error" style={{ marginBottom: 8 }}>
          <CircleAlert size={14} style={{ flexShrink: 0 }} />
          <span className="action-message-text">
            {runtime?.message || t('kimi.wakeup.cliMissing', '未检测到 kimi CLI')}
          </span>
          <button
            type="button"
            className="btn btn-secondary btn-sm"
            onClick={openCliModal}
          >
            {t('kimi.wakeup.cliSettings', 'CLI 设置')}
          </button>
        </div>
      )}

      <div className="toolbar wakeup-toolbar">
        <div className="toolbar-left">
          <div className={`wakeup-global-toggle ${state.enabled ? 'is-on' : 'is-off'}`}>
            <span className="toggle-label">{t('codex.wakeup.tab', '唤醒任务')}</span>
            <span className={`pill ${state.enabled ? 'pill-success' : 'pill-secondary'}`}>
              {state.enabled
                ? t('codex.wakeup.taskEnabled', '已启用')
                : t('codex.wakeup.taskPaused', '已暂停')}
            </span>
            <label
              className="wakeup-switch"
              onClick={(event) => {
                event.preventDefault();
                void setEnabled(!state.enabled);
              }}
            >
              <input type="checkbox" checked={state.enabled} readOnly />
              <span className="wakeup-slider" />
            </label>
          </div>
        </div>
        <div className="toolbar-right">
          <button
            type="button"
            className={`btn btn-secondary kimi-cli-status-btn${
              runtime?.available ? ' is-ready' : ' is-missing'
            }`}
            onClick={openCliModal}
            title={
              runtime?.binary_path ||
              runtime?.message ||
              t('kimi.wakeup.cliSettings', 'CLI 设置')
            }
          >
            <Terminal size={14} />
            <span>
              {runtime?.available
                ? runtime.version ||
                  t('kimi.wakeup.cliReadyShort', '已检测')
                : t('kimi.wakeup.cliMissingShort', '未检测')}
            </span>
          </button>
          <button
            className="btn btn-primary"
            onClick={openNewTaskModal}
            disabled={accounts.length === 0}
          >
            <Plus size={16} /> {t('codex.wakeup.addTask', '添加任务')}
          </button>
          <button className="btn btn-secondary" onClick={() => setShowPresetModal(true)}>
            {t('codex.wakeup.managePresets', '模型预设')}
          </button>
          <button
            className="btn btn-secondary"
            onClick={openTestModal}
            disabled={accounts.length === 0}
          >
            {t('codex.wakeup.testNow', '测试唤醒')}
          </button>
          <button className="btn btn-secondary" onClick={() => setShowHistoryModal(true)}>
            {historyBatches.length > 0
              ? `${t('codex.wakeup.historyTitle', '触发历史')} (${historyBatches.length})`
              : t('codex.wakeup.historyTitle', '触发历史')}
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
          <h3>{t('codex.wakeup.emptyTitle', '还没有唤醒任务')}</h3>
          <p>
            {t('codex.wakeup.emptyDesc', '先创建一个任务，之后就能按时间自动执行。')}
          </p>
          <button
            className="btn btn-primary"
            onClick={openNewTaskModal}
            disabled={accounts.length === 0}
          >
            <Plus size={18} /> {t('codex.wakeup.addTask', '添加任务')}
          </button>
        </div>
      ) : (
        <div className="wakeup-task-grid">
          {sortedTasks.map((task) => {
            const accountLabels = task.account_ids.map((id) => {
              const acc = accountMap.get(id);
              return acc ? getKimiAccountDisplayEmail(acc) : id;
            });
            const modelId = normalizeKimiModelId(task.model);
            const modelLabel =
              allPresets.find((p) => p.model === modelId || p.id === modelId)?.name ||
              modelId;
            return (
              <div
                key={task.id}
                className={`wakeup-task-card ${task.enabled ? 'is-enabled' : 'is-disabled'}`}
              >
                <div className="wakeup-task-header">
                  <div className="wakeup-task-title">
                    <span>{task.name}</span>
                    <span
                      className={`pill ${task.enabled ? 'pill-success' : 'pill-secondary'}`}
                    >
                      {task.enabled
                        ? t('codex.wakeup.taskEnabled', '已启用')
                        : t('codex.wakeup.taskPaused', '已暂停')}
                    </span>
                  </div>
                  <div className="wakeup-task-actions">
                    <button
                      className="btn btn-secondary icon-only"
                      onClick={() => void handleRunTask(task)}
                      disabled={runningTaskId === task.id}
                      title={t('codex.wakeup.testNow', '测试唤醒')}
                    >
                      {runningTaskId === task.id ? (
                        <RefreshCw size={14} className="loading-spinner" />
                      ) : (
                        <Play size={14} />
                      )}
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
                      title={
                        task.enabled
                          ? t('codex.wakeup.pauseOne', '暂停')
                          : t('codex.wakeup.resumeOne', '启用')
                      }
                    >
                      <Power size={14} />
                    </button>
                    <button
                      className="btn btn-danger icon-only"
                      onClick={() => void handleDeleteTask(task)}
                      title={t('common.delete', '删除')}
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
                    <span>
                      {t('codex.wakeup.taskAccountsLabel', '选择账号')}:{' '}
                      {formatSelectionPreview(accountLabels)}
                    </span>
                  </div>
                  <div className="wakeup-task-meta wakeup-task-meta-prompt">
                    <span>
                      {t('kimi.wakeup.modelLine', '模型：{{model}}', {
                        model: modelLabel,
                      })}
                    </span>
                  </div>
                  {task.prompt && (
                    <div className="wakeup-task-meta wakeup-task-meta-prompt">
                      <span>
                        {t('codex.wakeup.promptLabel', '唤醒提示词')}: {task.prompt}
                      </span>
                    </div>
                  )}
                </div>

                <div className="wakeup-task-section wakeup-task-section-muted">
                  <div className="wakeup-task-meta wakeup-task-meta-status">
                    <span>
                      {t('codex.wakeup.lastStatusLabel', '最近结果')}:{' '}
                      {formatTaskLastResult(task, t)}
                    </span>
                    <span>
                      {t('codex.wakeup.lastDurationLabel', '最近耗时')}:{' '}
                      {formatDuration(task.last_duration_ms)}
                    </span>
                  </div>
                  <div className="wakeup-task-meta wakeup-task-meta-timeline">
                    <span>
                      {t('codex.wakeup.lastRunLabel', {
                        time: formatDateTime(task.last_run_at),
                        defaultValue: `上次执行 ${formatDateTime(task.last_run_at)}`,
                      })}
                    </span>
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      )}

      {/* ===== Task create/edit modal (Codex-shaped) ===== */}
      {showTaskModal && (
        <div className="modal-overlay">
          <div
            className="modal modal-lg wakeup-modal codex-wakeup-modal"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="modal-header">
              <button
                className="btn btn-secondary icon-only"
                onClick={closeTaskModal}
                title={t('common.back', '返回')}
                aria-label={t('common.back', '返回')}
              >
                <ChevronLeft size={14} />
              </button>
              <h2>
                {taskDraft.id
                  ? t('codex.wakeup.editTaskTitle', '编辑唤醒任务')
                  : t('codex.wakeup.createTaskTitle', '添加唤醒任务')}
              </h2>
              <button className="modal-close" onClick={closeTaskModal}>
                <X />
              </button>
            </div>
            <div className="modal-body codex-wakeup-modal-body">
              <ModalErrorMessage
                message={taskModalError}
                scrollKey={taskModalErrorScrollKey}
              />
              {taskModalSuccess && (
                <div
                  className="action-message success"
                  style={{ marginBottom: 12 }}
                  role="status"
                >
                  <span className="action-message-text">{taskModalSuccess}</span>
                </div>
              )}
              <div className="wakeup-form-group">
                <label>{t('codex.wakeup.taskNameLabel', '任务名称')}</label>
                <input
                  className="wakeup-input"
                  value={taskDraft.name}
                  onChange={(e) =>
                    setTaskDraft((c) => ({ ...c, name: e.target.value }))
                  }
                  placeholder={t('codex.wakeup.taskNamePlaceholder', '例如：早间唤醒')}
                />
              </div>

              <div className="wakeup-form-group">
                <label>{t('common.status', '状态')}</label>
                <div className="wakeup-toggle-group">
                  <button
                    type="button"
                    className={`btn btn-secondary ${taskDraft.enabled ? 'is-active' : ''}`}
                    onClick={() => setTaskDraft((c) => ({ ...c, enabled: true }))}
                  >
                    {t('common.enable', '启用')}
                  </button>
                  <button
                    type="button"
                    className={`btn btn-secondary ${!taskDraft.enabled ? 'is-active' : ''}`}
                    onClick={() => setTaskDraft((c) => ({ ...c, enabled: false }))}
                  >
                    {t('common.disable', '禁用')}
                  </button>
                </div>
              </div>

              <div className="wakeup-form-group">
                <label>{t('codex.wakeup.taskAccountsLabel', '选择账号')}</label>
                {renderAccountPicker(
                  filteredTaskAccounts,
                  taskDraft.accountIds,
                  (ids) => setTaskDraft((c) => ({ ...c, accountIds: ids })),
                  accountQuery,
                  setAccountQuery,
                )}
              </div>

              <div className="wakeup-form-group">
                <div className="codex-wakeup-inline-header">
                  <label>{t('codex.wakeup.taskModelLabel', '模型')}</label>
                  <button
                    type="button"
                    className="btn btn-secondary"
                    onClick={() => setShowPresetModal(true)}
                  >
                    {t('codex.wakeup.managePresets', '模型预设')}
                  </button>
                </div>
                <p className="wakeup-hint">
                  {t(
                    'codex.wakeup.taskModelHint',
                    '选择预设模型。可在「模型预设」中添加自定义模型 ID。',
                  )}
                </p>
                <div className="codex-wakeup-dual-select">
                  <div className="codex-wakeup-dual-select-field">
                    <WakeupSingleSelectDropdown
                      value={taskDraft.modelPresetId}
                      options={modelPresetOptions}
                      placeholder={t('codex.wakeup.selectPresetPlaceholder', '选择模型预设')}
                      onSelect={(value) => selectPresetOnDraft(value, setTaskDraft)}
                    />
                  </div>
                </div>
                {taskDraft.model && (
                  <p className="wakeup-hint">
                    {t('codex.wakeup.modelValuePreview', {
                      model: taskDraft.model,
                      defaultValue: `模型 ID：${taskDraft.model}`,
                    })}
                  </p>
                )}
              </div>

              <div className="wakeup-form-group">
                <label>{t('codex.wakeup.scheduleLabel', '触发方式')}</label>
                <div className="wakeup-segmented">
                  {(
                    [
                      'daily',
                      'weekly',
                      'interval',
                      'quota_reset',
                      'startup',
                    ] as KimiWakeupScheduleKind[]
                  ).map((kind) => (
                    <button
                      type="button"
                      key={kind}
                      className={`wakeup-segment-btn ${taskDraft.scheduleKind === kind ? 'active' : ''}`}
                      onClick={() =>
                        setTaskDraft((c) => ({ ...c, scheduleKind: kind }))
                      }
                    >
                      {kind === 'startup'
                        ? t('wakeup.triggerSource.startup', '启动时')
                        : t(`codex.wakeup.schedule.${kind}`, kind)}
                    </button>
                  ))}
                </div>
                {taskDraft.scheduleKind === 'quota_reset' && (
                  <>
                    <p className="codex-wakeup-quota-reset-tip">
                      <CircleAlert size={14} />
                      <span>
                        {t(
                          'codex.wakeup.scheduleQuotaResetHint',
                          '额度重置后自动触发，依赖账号额度刷新。',
                        )}
                      </span>
                    </p>
                    <div className="codex-wakeup-quota-reset-window-selector">
                      <label>
                        {t('codex.wakeup.quotaResetWindowLabel', '监听窗口')}
                      </label>
                      <div className="wakeup-segmented codex-wakeup-quota-reset-window-buttons">
                        {(
                          [
                            'either',
                            'primary_window',
                            'secondary_window',
                          ] as KimiWakeupQuotaResetWindow[]
                        ).map((windowType) => (
                          <button
                            type="button"
                            key={windowType}
                            className={`wakeup-segment-btn ${
                              taskDraft.quotaResetWindow === windowType ? 'active' : ''
                            }`}
                            onClick={() =>
                              setTaskDraft((c) => ({
                                ...c,
                                quotaResetWindow: windowType,
                              }))
                            }
                          >
                            {t(
                              `codex.wakeup.quotaResetWindowOptions.${windowType}`,
                              windowType,
                            )}
                          </button>
                        ))}
                      </div>
                      <p className="wakeup-hint">
                        {t(
                          'codex.wakeup.quotaResetWindowHint',
                          'primary ≈ 周额度，secondary ≈ 5 小时窗口。',
                        )}
                      </p>
                    </div>
                  </>
                )}
              </div>

              {taskDraft.scheduleKind === 'daily' && (
                <div className="wakeup-form-group">
                  <label>{t('codex.wakeup.dailyTimeLabel', '每天时间')}</label>
                  <div className="wakeup-chip-grid">
                    {QUICK_TIME_OPTIONS.map((time) => (
                      <button
                        key={time}
                        type="button"
                        className={`wakeup-chip ${taskDraft.dailyTime === time ? 'selected' : ''}`}
                        onClick={() =>
                          setTaskDraft((c) => ({ ...c, dailyTime: time }))
                        }
                      >
                        {time}
                      </button>
                    ))}
                  </div>
                  <input
                    type="time"
                    className="wakeup-input wakeup-input-time"
                    value={taskDraft.dailyTime}
                    onChange={(e) =>
                      setTaskDraft((c) => ({ ...c, dailyTime: e.target.value }))
                    }
                  />
                </div>
              )}

              {taskDraft.scheduleKind === 'startup' && (
                <div className="wakeup-form-group">
                  <label>{t('wakeup.triggerSource.startup', '启动时')}</label>
                  <div className="wakeup-toggle-group">
                    <button
                      type="button"
                      className={`btn btn-secondary ${
                        taskDraft.startupDelayMode === 'immediate' ? 'is-active' : ''
                      }`}
                      onClick={() =>
                        setTaskDraft((c) => ({
                          ...c,
                          startupDelayMode: 'immediate',
                        }))
                      }
                    >
                      {t('settings.general.startupWakeupImmediate', '立即执行')}
                    </button>
                    <button
                      type="button"
                      className={`btn btn-secondary ${
                        taskDraft.startupDelayMode === 'delayed' ? 'is-active' : ''
                      }`}
                      onClick={() =>
                        setTaskDraft((c) => ({
                          ...c,
                          startupDelayMode: 'delayed',
                        }))
                      }
                    >
                      {t('settings.general.startupWakeupDelayed', '延迟执行')}
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
                        onChange={(e) =>
                          setTaskDraft((c) => ({
                            ...c,
                            startupDelayMinutes: e.target.value.replace(/[^\d]/g, ''),
                          }))
                        }
                      />
                      <span>{t('settings.general.minutes', '分钟')}</span>
                    </div>
                  )}
                </div>
              )}

              {taskDraft.scheduleKind === 'weekly' && (
                <>
                  <div className="wakeup-form-group">
                    <label>{t('codex.wakeup.weeklyDaysLabel', '星期')}</label>
                    <div className="wakeup-chip-grid">
                      {WEEKDAY_OPTIONS.map((item) => {
                        const active = taskDraft.weeklyDays.includes(item.value);
                        return (
                          <button
                            type="button"
                            key={item.value}
                            className={`wakeup-chip ${active ? 'selected' : ''}`}
                            onClick={() =>
                              setTaskDraft((c) => ({
                                ...c,
                                weeklyDays: active
                                  ? c.weeklyDays.filter((v) => v !== item.value)
                                  : [...c.weeklyDays, item.value],
                              }))
                            }
                          >
                            {t(`codex.wakeup.weekdays.${item.value}`, String(item.value))}
                          </button>
                        );
                      })}
                    </div>
                  </div>
                  <div className="wakeup-form-group">
                    <label>{t('codex.wakeup.weeklyTimeLabel', '时间')}</label>
                    <div className="wakeup-chip-grid">
                      {QUICK_TIME_OPTIONS.map((time) => (
                        <button
                          key={time}
                          type="button"
                          className={`wakeup-chip ${
                            taskDraft.weeklyTime === time ? 'selected' : ''
                          }`}
                          onClick={() =>
                            setTaskDraft((c) => ({ ...c, weeklyTime: time }))
                          }
                        >
                          {time}
                        </button>
                      ))}
                    </div>
                    <input
                      type="time"
                      className="wakeup-input wakeup-input-time"
                      value={taskDraft.weeklyTime}
                      onChange={(e) =>
                        setTaskDraft((c) => ({
                          ...c,
                          weeklyTime: e.target.value,
                        }))
                      }
                    />
                  </div>
                </>
              )}

              {taskDraft.scheduleKind === 'interval' && (
                <div className="wakeup-form-group">
                  <label>{t('codex.wakeup.intervalHoursLabel', '间隔（小时）')}</label>
                  <input
                    type="number"
                    min={1}
                    max={24}
                    className="wakeup-input wakeup-input-small"
                    value={taskDraft.intervalHours}
                    onChange={(e) =>
                      setTaskDraft((c) => ({
                        ...c,
                        intervalHours: e.target.value,
                      }))
                    }
                  />
                </div>
              )}

              <div className="wakeup-form-group">
                <label>{t('codex.wakeup.promptLabel', '唤醒提示词')}</label>
                <textarea
                  className="token-input codex-wakeup-prompt-input"
                  value={taskDraft.prompt}
                  onChange={(e) =>
                    setTaskDraft((c) => ({ ...c, prompt: e.target.value }))
                  }
                  placeholder={t('codex.wakeup.promptPlaceholder', {
                    prompt: DEFAULT_KIMI_WAKEUP_PROMPT,
                    defaultValue: `默认：${DEFAULT_KIMI_WAKEUP_PROMPT}`,
                  })}
                />
              </div>

              <div className="wakeup-form-group">
                <label>{t('wakeup.form.nextRuns', '接下来执行')}</label>
                <ul className="wakeup-preview-list">
                  {previewRuns.length === 0 && (
                    <li>
                      {taskDraft.scheduleKind === 'quota_reset'
                        ? t(
                            'codex.wakeup.nextRunsQuotaResetHint',
                            '额度重置触发无固定时间预览',
                          )
                        : taskDraft.scheduleKind === 'startup'
                          ? taskDraft.startupDelayMode === 'delayed'
                            ? `${t('wakeup.triggerSource.startup', '启动时')} +${Math.min(
                                MAX_STARTUP_DELAY_MINUTES,
                                Math.max(1, Number(taskDraft.startupDelayMinutes) || 1),
                              )}${t('settings.general.minutes', '分钟')}`
                            : t(
                                'settings.general.startupWakeupImmediate',
                                '启动时立即执行',
                              )
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
              <button
                className="btn btn-secondary"
                onClick={closeTaskModal}
                disabled={busy}
              >
                {t('common.cancel', '取消')}
              </button>
              <button
                className="btn btn-primary"
                onClick={() => void handleSaveTask()}
                disabled={busy}
              >
                {busy ? t('common.saving', '保存中...') : t('common.save', '保存')}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* ===== Model preset manager ===== */}
      {showPresetModal && (
        <div className="modal-overlay codex-wakeup-preset-overlay">
          <div
            className="modal modal-lg wakeup-modal codex-wakeup-modal"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="modal-header">
              <h2>{t('codex.wakeup.presetManagerTitle', '模型预设')}</h2>
              <button
                className="modal-close"
                onClick={() => setShowPresetModal(false)}
              >
                <X />
              </button>
            </div>
            <div className="modal-body codex-wakeup-modal-body">
              <ModalErrorMessage
                message={presetModalError}
                scrollKey={presetModalErrorScrollKey}
              />
              {presetModalSuccess && (
                <div
                  className="action-message success"
                  style={{ marginBottom: 12 }}
                  role="status"
                >
                  <Check size={14} style={{ flexShrink: 0 }} />
                  <span className="action-message-text">{presetModalSuccess}</span>
                </div>
              )}
              <div className="wakeup-form-group">
                <div className="codex-wakeup-inline-header">
                  <label>{t('codex.wakeup.presetListLabel', '预设列表')}</label>
                  <button
                    type="button"
                    className="btn btn-secondary"
                    onClick={() => {
                      clearPresetModalError();
                      setPresetModalSuccess(null);
                      setPresetDraft({ id: '', name: '', model: '' });
                    }}
                  >
                    <Plus size={14} /> {t('codex.wakeup.addPreset', '新增预设')}
                  </button>
                </div>
                <div className="wakeup-chip-grid">
                  {allPresets.map((preset) => (
                    <button
                      key={preset.id}
                      type="button"
                      className={`wakeup-chip ${presetDraft.id === preset.id ? 'selected' : ''}`}
                      onClick={() =>
                        setPresetDraft({
                          id: preset.id,
                          name: preset.name,
                          model: preset.model,
                        })
                      }
                    >
                      {preset.name}
                    </button>
                  ))}
                </div>
              </div>
              <div className="wakeup-form-group">
                <label>{t('codex.wakeup.presetNameLabel', '预设名称')}</label>
                <input
                  className="wakeup-input"
                  value={presetDraft.name}
                  onChange={(e) =>
                    setPresetDraft((c) => ({ ...c, name: e.target.value }))
                  }
                  placeholder={t('codex.wakeup.presetNamePlaceholder', '例如：Coding')}
                  disabled={BUILTIN_PRESETS.some((p) => p.id === presetDraft.id)}
                />
              </div>
              <div className="wakeup-form-group">
                <label>{t('codex.wakeup.presetModelLabel', '模型 ID')}</label>
                <input
                  className="wakeup-input"
                  value={presetDraft.model}
                  onChange={(e) =>
                    setPresetDraft((c) => ({ ...c, model: e.target.value }))
                  }
                  placeholder={t(
                    'codex.wakeup.presetModelPlaceholder',
                    DEFAULT_KIMI_WAKEUP_MODEL,
                  )}
                  disabled={BUILTIN_PRESETS.some((p) => p.id === presetDraft.id)}
                />
              </div>
            </div>
            <div className="modal-footer">
              {presetDraft.id &&
                !BUILTIN_PRESETS.some((p) => p.id === presetDraft.id) && (
                  <button
                    className="btn btn-danger"
                    onClick={() => {
                      const p = customPresets.find((x) => x.id === presetDraft.id);
                      if (p) handleDeletePreset(p);
                    }}
                  >
                    {t('common.delete', '删除')}
                  </button>
                )}
              <button
                className="btn btn-secondary"
                onClick={() => setShowPresetModal(false)}
              >
                {t('common.close', '关闭')}
              </button>
              {!BUILTIN_PRESETS.some((p) => p.id === presetDraft.id) && (
                <button className="btn btn-primary" onClick={handleSavePreset}>
                  {presetDraft.id
                    ? t('common.save', '保存')
                    : t('codex.wakeup.addPreset', '新增预设')}
                </button>
              )}
            </div>
          </div>
        </div>
      )}

      {/* ===== Test modal ===== */}
      {showTestModal && (
        <div className="modal-overlay">
          <div
            className="modal modal-lg wakeup-modal wakeup-test-modal codex-wakeup-modal"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="modal-header">
              <button
                className="btn btn-secondary icon-only"
                onClick={() => setShowTestModal(false)}
                title={t('common.back', '返回')}
              >
                <ChevronLeft size={14} />
              </button>
              <h2>{t('codex.wakeup.testTitle', '测试唤醒')}</h2>
              <button
                className="modal-close"
                onClick={() => setShowTestModal(false)}
              >
                <X />
              </button>
            </div>
            <div className="modal-body codex-wakeup-modal-body">
              <ModalErrorMessage
                message={testModalError}
                scrollKey={testModalErrorScrollKey}
              />
              <div className="wakeup-form-group">
                <label>{t('codex.wakeup.taskAccountsLabel', '选择账号')}</label>
                {renderAccountPicker(
                  filteredTestAccounts,
                  testAccountIds,
                  setTestAccountIds,
                  testAccountQuery,
                  setTestAccountQuery,
                )}
              </div>
              <div className="wakeup-form-group">
                <label>{t('codex.wakeup.taskModelLabel', '模型')}</label>
                <WakeupSingleSelectDropdown
                  value={testModelPresetId}
                  options={modelPresetOptions}
                  placeholder={t('codex.wakeup.selectPresetPlaceholder', '选择模型预设')}
                  onSelect={setTestModelPresetId}
                />
              </div>
              <div className="wakeup-form-group">
                <label>{t('codex.wakeup.promptLabel', '唤醒提示词')}</label>
                <textarea
                  className="token-input codex-wakeup-prompt-input"
                  value={testPrompt}
                  onChange={(e) => setTestPrompt(e.target.value)}
                  placeholder={DEFAULT_KIMI_WAKEUP_PROMPT}
                />
              </div>
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={() => setShowTestModal(false)}
              >
                {t('common.cancel', '取消')}
              </button>
              <button
                className="btn btn-primary"
                disabled={busy || testAccountIds.length === 0}
                onClick={() => void handleTestRun()}
              >
                <Play size={14} /> {t('codex.wakeup.testNow', '测试唤醒')}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* ===== History modal ===== */}
      {showHistoryModal && (
        <div className="modal-overlay">
          <div
            className="modal wakeup-modal wakeup-history-modal codex-wakeup-history-modal"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="modal-header">
              <button
                className="btn btn-secondary icon-only"
                onClick={() => setShowHistoryModal(false)}
                title={t('common.back', '返回')}
              >
                <ChevronLeft size={14} />
              </button>
              <h2>{t('codex.wakeup.historyTitle', '触发历史')}</h2>
              <button
                className="modal-close"
                onClick={() => setShowHistoryModal(false)}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              {historyBatches.length === 0 ? (
                <p className="wakeup-hint">
                  {t('codex.wakeup.historyEmptyDesc', '还没有触发记录。')}
                </p>
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
                      <li
                        key={batch.runId}
                        className="codex-wakeup-history-run-card"
                      >
                        <div className="codex-wakeup-history-run-head">
                          <div className="codex-wakeup-history-run-copy">
                            <h4>
                              {batch.taskName ||
                                (batch.triggerType === 'test'
                                  ? t('codex.wakeup.testTitle', '测试唤醒')
                                  : triggerLabel(batch.triggerType, t))}
                            </h4>
                            <div className="codex-wakeup-history-run-meta">
                              <span>{formatDateTime(batch.timestamp)}</span>
                              {batch.durationMs != null && (
                                <span>{formatDuration(batch.durationMs)}</span>
                              )}
                              <span>
                                {t('accounts.groups.accountCount', {
                                  count: batch.records.length,
                                  defaultValue: `${batch.records.length} 个账号`,
                                })}
                              </span>
                            </div>
                          </div>
                          <div className="codex-wakeup-history-run-actions">
                            <span
                              className={`wakeup-history-badge codex-wakeup-history-trigger-badge ${badgeClass}`}
                            >
                              {triggerLabel(batch.triggerType, t)}
                            </span>
                          </div>
                        </div>
                        <div className="codex-wakeup-history-run-stats">
                          <span className="codex-wakeup-history-stat-chip is-total">
                            <span>{t('codex.wakeup.resultsTotal', '合计')}</span>
                            <strong>{batch.records.length}</strong>
                          </span>
                          <span className="codex-wakeup-history-stat-chip is-success">
                            <span>{t('codex.wakeup.resultsSuccess', '成功')}</span>
                            <strong>{batch.successCount}</strong>
                          </span>
                          <span className="codex-wakeup-history-stat-chip is-error">
                            <span>{t('codex.wakeup.resultsFailed', '失败')}</span>
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
              <button
                className="btn btn-secondary codex-wakeup-subtle-btn"
                onClick={() => setShowHistoryModal(false)}
              >
                {t('common.close', '关闭')}
              </button>
              <button
                className="btn btn-secondary codex-wakeup-subtle-btn"
                onClick={() => void clearHistory()}
                disabled={historyBatches.length === 0}
              >
                {t('codex.wakeup.clearHistory', '清空历史')}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* CLI path — select / auto-detect / open folder + in-modal feedback */}
      {showCliModal && (
        <div className="modal-overlay">
          <div
            className="modal wakeup-modal codex-wakeup-modal kimi-cli-settings-modal"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="modal-header">
              <button
                className="btn btn-secondary icon-only"
                onClick={closeCliModal}
                title={t('common.back', '返回')}
                aria-label={t('common.back', '返回')}
                disabled={cliBusy || cliDetecting}
              >
                <ChevronLeft size={14} />
              </button>
              <h2>{t('kimi.wakeup.cliSettings', 'Kimi CLI 设置')}</h2>
              <button
                className="modal-close"
                onClick={closeCliModal}
                disabled={cliBusy || cliDetecting}
              >
                <X />
              </button>
            </div>
            <div className="modal-body">
              <ModalErrorMessage
                message={cliModalError}
                scrollKey={cliModalErrorScrollKey}
              />
              {cliModalSuccess && (
                <div
                  className="action-message success"
                  style={{ marginBottom: 12 }}
                  role="status"
                >
                  <Check size={14} style={{ flexShrink: 0 }} />
                  <span className="action-message-text">{cliModalSuccess}</span>
                </div>
              )}

              <div
                className={`kimi-cli-runtime-status${
                  runtime?.available ? ' is-ready' : ' is-missing'
                }`}
              >
                {runtime?.available ? (
                  <Check size={16} />
                ) : (
                  <CircleAlert size={16} />
                )}
                <div className="kimi-cli-runtime-status-copy">
                  <strong>
                    {runtime?.available
                      ? t('kimi.wakeup.cliReadyShort', '已检测')
                      : t('kimi.wakeup.cliMissingShort', '未检测')}
                  </strong>
                  <span>
                    {runtime?.available
                      ? runtime.binary_path ||
                        runtime.version ||
                        t('kimi.wakeup.cliOk', '已检测 {{path}}', {
                          path: '--',
                        })
                      : runtime?.message ||
                        t('kimi.wakeup.cliMissing', '未检测到 kimi CLI')}
                  </span>
                  {runtime?.version ? (
                    <span className="kimi-cli-runtime-version">
                      {runtime.version}
                    </span>
                  ) : null}
                </div>
                <button
                  type="button"
                  className="btn btn-secondary icon-only"
                  onClick={() => void fetchOverview()}
                  disabled={cliBusy || cliDetecting}
                  title={t('common.refresh', '刷新')}
                  aria-label={t('common.refresh', '刷新')}
                >
                  <RefreshCw size={14} />
                </button>
              </div>

              <p className="wakeup-hint">
                {t(
                  'kimi.wakeup.cliHint',
                  '可留空以从 PATH 与常见安装目录自动查找；也可手动选择 kimi / kimi-code 可执行文件。',
                )}
              </p>

              <label className="wakeup-form-group kimi-cli-path-field">
                <span>{t('kimi.wakeup.cliPath', 'Kimi CLI 路径')}</span>
                <div className="kimi-cli-path-row">
                  <input
                    className="wakeup-input"
                    value={cliPath}
                    placeholder={t(
                      'kimi.wakeup.cliPathPlaceholder',
                      '例如 C:\\…\\kimi.exe 或留空自动检测',
                    )}
                    onChange={(e) => {
                      setCliPath(e.target.value);
                      clearCliModalError();
                      setCliModalSuccess(null);
                    }}
                    disabled={cliBusy || cliDetecting}
                    autoFocus
                  />
                  <div className="kimi-cli-path-actions">
                    <button
                      type="button"
                      className="btn btn-secondary"
                      onClick={() => void handleBrowseCliPath()}
                      disabled={cliBusy || cliDetecting}
                    >
                      <FolderOpen size={14} />
                      {t('kimi.wakeup.cliBrowse', '选择')}
                    </button>
                    <button
                      type="button"
                      className="btn btn-secondary"
                      onClick={() => void handleDetectCliPath()}
                      disabled={cliBusy || cliDetecting}
                    >
                      <RefreshCw
                        size={14}
                        className={cliDetecting ? 'loading-spinner' : undefined}
                      />
                      {cliDetecting
                        ? t('kimi.wakeup.cliDetecting', '检测中…')
                        : t('kimi.wakeup.cliDetect', '自动检测')}
                    </button>
                  </div>
                </div>
              </label>
            </div>
            <div className="modal-footer">
              <button
                className="btn btn-secondary"
                onClick={closeCliModal}
                disabled={cliBusy || cliDetecting}
              >
                {t('common.close', '关闭')}
              </button>
              <button
                className="btn btn-primary"
                disabled={cliBusy || cliDetecting}
                onClick={() => void saveCliPath()}
              >
                {cliBusy
                  ? t('common.loading', '加载中...')
                  : t('common.save', '保存')}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
