import { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Activity,
  AlertTriangle,
  CheckCircle2,
  ChevronRight,
  CircleStop,
  Clock3,
  FolderOpen,
  ListRestart,
  LoaderCircle,
  Play,
  RefreshCw,
  RotateCcw,
  ShieldCheck,
  TerminalSquare,
  Users,
  XCircle,
} from 'lucide-react';
import { confirm as confirmDialog, open as openDialog } from '@tauri-apps/plugin-dialog';
import type { CodexAccount } from '../../types/codex';
import * as managedTaskService from '../../services/codexManagedTaskService';
import type {
  CreateManagedCodexTaskInput,
  ManagedCodexTaskEvidence,
  ManagedCodexTaskRuntimeStatus,
  ManagedCodexTaskSnapshot,
  ManagedCodexTaskStatus,
} from '../../types/codexManagedTask';
import {
  buildManagedTaskInput,
  canCancelManagedTask,
  emptyManagedTaskDraft,
  isEligibleManagedCodexAccount,
  mergeManagedTaskEvidence,
  validateManagedTaskDraft,
  type ManagedTaskDraft,
} from '../../utils/codexManagedTask';
import './CodexManagedTasks.css';

interface CodexManagedTasksProps {
  accounts: CodexAccount[];
}

function accountLabel(account: CodexAccount): string {
  return account.account_name?.trim() || account.email?.trim() || account.id;
}

function maskAccountId(accountId?: string): string {
  if (!accountId) return '—';
  if (accountId.length <= 8) return '***';
  return `${accountId.slice(0, 4)}…${accountId.slice(-4)}`;
}

function formatDateTime(value?: number): string {
  if (!value) return '—';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return '—';
  return new Intl.DateTimeFormat(undefined, {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  }).format(date);
}

function formatDuration(start?: number, end?: number): string {
  if (!start) return '—';
  const seconds = Math.max(0, Math.floor(((end ?? Date.now()) - start) / 1_000));
  const hours = Math.floor(seconds / 3_600);
  const minutes = Math.floor((seconds % 3_600) / 60);
  const rest = seconds % 60;
  if (hours > 0) return `${hours}h ${minutes}m ${rest}s`;
  if (minutes > 0) return `${minutes}m ${rest}s`;
  return `${rest}s`;
}

function statusIcon(status: ManagedCodexTaskStatus) {
  if (status === 'completed') return <CheckCircle2 size={16} />;
  if (status === 'failed' || status === 'cancelled') return <XCircle size={16} />;
  if (status === 'needs_attention') return <AlertTriangle size={16} />;
  if (status === 'queued') return <Clock3 size={16} />;
  return <LoaderCircle size={16} className="managed-task-spin" />;
}

export function CodexManagedTasks({ accounts }: CodexManagedTasksProps) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState<ManagedTaskDraft>(emptyManagedTaskDraft);
  const [tasks, setTasks] = useState<ManagedCodexTaskSnapshot[]>([]);
  const [runtime, setRuntime] = useState<ManagedCodexTaskRuntimeStatus | null>(null);
  const [selectedTaskId, setSelectedTaskId] = useState<string | null>(null);
  const [evidence, setEvidence] = useState<ManagedCodexTaskEvidence[]>([]);
  const [loading, setLoading] = useState(true);
  const [creating, setCreating] = useState(false);
  const [actionTaskId, setActionTaskId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [, setClockTick] = useState(0);

  const eligibleAccounts = useMemo(
    () => accounts.filter(isEligibleManagedCodexAccount),
    [accounts],
  );
  const scopedAccounts = useMemo(
    () =>
      draft.scopeKind === 'selected'
        ? eligibleAccounts.filter((account) => draft.accountIds.includes(account.id))
        : eligibleAccounts,
    [draft.accountIds, draft.scopeKind, eligibleAccounts],
  );
  const selectedTask = useMemo(
    () => tasks.find((task) => task.id === selectedTaskId) ?? null,
    [selectedTaskId, tasks],
  );

  const loadOverview = useCallback(async (showLoading = false) => {
    if (showLoading) setLoading(true);
    try {
      const [nextTasks, nextRuntime] = await Promise.all([
        managedTaskService.listManagedCodexTasks(),
        managedTaskService.getManagedCodexTaskRuntimeStatus(),
      ]);
      setTasks(nextTasks);
      setRuntime(nextRuntime);
      setError(null);
    } catch (loadError) {
      setError(String(loadError));
    } finally {
      if (showLoading) setLoading(false);
    }
  }, []);

  const loadEvidence = useCallback(async (taskId: string) => {
    try {
      const page = await managedTaskService.listManagedCodexTaskEvidence(taskId, undefined, 200);
      setEvidence(mergeManagedTaskEvidence([], page.items));
    } catch (loadError) {
      setError(String(loadError));
    }
  }, []);

  useEffect(() => {
    void loadOverview(true);
    const refreshTimer = window.setInterval(() => void loadOverview(false), 3_000);
    const clockTimer = window.setInterval(() => setClockTick((value) => value + 1), 1_000);
    let unlistenUpdated: (() => void) | undefined;
    let unlistenEvidence: (() => void) | undefined;
    void managedTaskService.listenManagedCodexTaskUpdated((updated) => {
      setTasks((current) => {
        const exists = current.some((task) => task.id === updated.id);
        const next = exists
          ? current.map((task) => (task.id === updated.id ? updated : task))
          : [updated, ...current];
        return next.sort((left, right) => right.updatedAt - left.updatedAt);
      });
    }).then((unlisten) => {
      unlistenUpdated = unlisten;
    });
    void managedTaskService.listenManagedCodexTaskEvidence((payload) => {
      if (payload.taskId === selectedTaskId) {
        setEvidence((current) => mergeManagedTaskEvidence(current, [payload.evidence]));
      }
    }).then((unlisten) => {
      unlistenEvidence = unlisten;
    });
    return () => {
      window.clearInterval(refreshTimer);
      window.clearInterval(clockTimer);
      unlistenUpdated?.();
      unlistenEvidence?.();
    };
  }, [loadOverview, selectedTaskId]);

  useEffect(() => {
    if (selectedTaskId) void loadEvidence(selectedTaskId);
    else setEvidence([]);
  }, [loadEvidence, selectedTaskId]);

  useEffect(() => {
    if (
      draft.initialAccountId &&
      !scopedAccounts.some((account) => account.id === draft.initialAccountId)
    ) {
      setDraft((current) => ({ ...current, initialAccountId: '' }));
    }
  }, [draft.initialAccountId, scopedAccounts]);

  const chooseWorkingDirectory = async () => {
    const selected = await openDialog({ directory: true, multiple: false });
    if (typeof selected === 'string') {
      setDraft((current) => ({ ...current, cwd: selected }));
    }
  };

  const toggleAccount = (accountId: string) => {
    setDraft((current) => ({
      ...current,
      accountIds: current.accountIds.includes(accountId)
        ? current.accountIds.filter((id) => id !== accountId)
        : [...current.accountIds, accountId],
    }));
  };

  const draftError = useMemo(() => {
    const issue = validateManagedTaskDraft(draft);
    if (issue === 'cwd') return t('managedCodexTasks.validationCwd', 'Choose a working directory.');
    if (issue === 'objective') return t('managedCodexTasks.validationObjective', 'Enter a task objective.');
    if (issue === 'accounts') return t('managedCodexTasks.validationAccounts', 'Select at least one eligible OAuth account.');
    if (issue === 'max_switches') return t('managedCodexTasks.validationSwitches', 'Switch limit must be a non-negative integer.');
    return null;
  }, [draft, t]);

  const createTask = async () => {
    if (draftError || !runtime?.cli.available) return;
    const input: CreateManagedCodexTaskInput = buildManagedTaskInput(draft);
    setCreating(true);
    setError(null);
    try {
      const created = await managedTaskService.createManagedCodexTask(input);
      setTasks((current) => [created, ...current]);
      setSelectedTaskId(created.id);
      setDraft((current) => ({ ...emptyManagedTaskDraft(), cwd: current.cwd }));
      await loadOverview(false);
    } catch (createError) {
      setError(String(createError));
    } finally {
      setCreating(false);
    }
  };

  const cancelTask = async (task: ManagedCodexTaskSnapshot) => {
    const confirmed = await confirmDialog(
      t('managedCodexTasks.cancelConfirm', 'Cancel this managed task and terminate its Codex process tree?'),
      { kind: 'warning' },
    );
    if (!confirmed) return;
    setActionTaskId(task.id);
    try {
      await managedTaskService.cancelManagedCodexTask(task.id);
      await loadOverview(false);
    } catch (actionError) {
      setError(String(actionError));
    } finally {
      setActionTaskId(null);
    }
  };

  const resumeTask = async (
    task: ManagedCodexTaskSnapshot,
    mode: 'same_account' | 'next_eligible',
  ) => {
    if (task.processId) {
      const confirmed = await confirmDialog(
        t(
          'managedCodexTasks.orphanResumeConfirm',
          'Terminate the matching orphan Codex process tree, wait for exit, and then resume this task?',
        ),
        { kind: 'warning' },
      );
      if (!confirmed) return;
    }
    setActionTaskId(task.id);
    try {
      await managedTaskService.resumeManagedCodexTask(task.id, mode);
      await loadOverview(false);
    } catch (actionError) {
      setError(String(actionError));
    } finally {
      setActionTaskId(null);
    }
  };

  const statusLabel = (status: ManagedCodexTaskStatus) =>
    t(`managedCodexTasks.status.${status}`, status.replace('_', ' '));

  return (
    <main className="managed-codex-tasks" aria-label={t('managedCodexTasks.title', 'Managed tasks')}>
      <section className="managed-task-hero">
        <div>
          <div className="managed-task-eyebrow">
            <ShieldCheck size={16} />
            {t('managedCodexTasks.optIn', 'Explicit opt-in supervision')}
          </div>
          <h1>{t('managedCodexTasks.title', 'Managed Codex tasks')}</h1>
          <p>
            {t(
              'managedCodexTasks.subtitle',
              'Cockpit owns one codex exec process at a time, waits for an authoritative quota terminal state, then resumes the same thread with the next eligible account.',
            )}
          </p>
        </div>
        <button className="managed-task-secondary-button" onClick={() => void loadOverview(true)}>
          <RefreshCw size={15} />
          {t('common.refresh', 'Refresh')}
        </button>
      </section>

      {runtime && (
        <section className={`managed-task-runtime ${runtime.cli.available ? 'available' : 'unavailable'}`}>
          <TerminalSquare size={20} />
          <div>
            <strong>
              {runtime.cli.available
                ? t('managedCodexTasks.cliReady', 'Codex CLI ready')
                : t('managedCodexTasks.cliUnavailable', 'Codex CLI unavailable')}
            </strong>
            <span>
              {runtime.cli.available
                ? `${runtime.cli.version ?? 'version unknown'} · ${runtime.cli.binaryPath ?? 'path unknown'}`
                : runtime.cli.message ?? t('managedCodexTasks.cliHint', 'Configure an executable official Codex CLI path.')}
            </span>
          </div>
          <div className="managed-task-runtime-counts">
            <span>{t('managedCodexTasks.queue', 'Queue')}: {runtime.queueLength}</span>
            <span>{t('managedCodexTasks.active', 'Active')}: {runtime.activeTaskId ? maskAccountId(runtime.activeTaskId) : '—'}</span>
          </div>
        </section>
      )}

      {error && (
        <div className="managed-task-error" role="alert">
          <AlertTriangle size={16} />
          <span>{error}</span>
          <button onClick={() => setError(null)} aria-label={t('common.close', 'Close')}>×</button>
        </div>
      )}

      <section className="managed-task-create-card">
        <div className="managed-task-section-heading">
          <div>
            <h2>{t('managedCodexTasks.createTitle', 'Create a managed task')}</h2>
            <p>{t('managedCodexTasks.createHint', 'Tasks enter the persistent FIFO queue immediately.')}</p>
          </div>
          <Activity size={20} />
        </div>

        <div className="managed-task-form-grid">
          <label className="managed-task-field managed-task-field-wide">
            <span>{t('managedCodexTasks.cwd', 'Working directory')}</span>
            <div className="managed-task-input-action">
              <input
                value={draft.cwd}
                onChange={(event) => setDraft((current) => ({ ...current, cwd: event.target.value }))}
                placeholder="C:\\workspace\\project"
              />
              <button type="button" onClick={() => void chooseWorkingDirectory()} aria-label={t('managedCodexTasks.chooseFolder', 'Choose folder')}>
                <FolderOpen size={16} />
              </button>
            </div>
          </label>

          <label className="managed-task-field managed-task-field-wide">
            <span>{t('managedCodexTasks.objective', 'Objective')}</span>
            <textarea
              value={draft.objective}
              onChange={(event) => setDraft((current) => ({ ...current, objective: event.target.value }))}
              placeholder={t('managedCodexTasks.objectivePlaceholder', 'Describe the exact outcome Codex should complete…')}
              rows={4}
            />
          </label>

          <fieldset className="managed-task-field managed-task-field-wide managed-task-scope">
            <legend>{t('managedCodexTasks.accountScope', 'Account scope')}</legend>
            <label>
              <input
                type="radio"
                checked={draft.scopeKind === 'cockpit_pool'}
                onChange={() => setDraft((current) => ({ ...current, scopeKind: 'cockpit_pool' }))}
              />
              <Users size={16} />
              <span>
                <strong>{t('managedCodexTasks.wholePool', 'Entire Cockpit pool')}</strong>
                <small>{t('managedCodexTasks.wholePoolHint', 'Re-read the live pool and routing policy at every switch.')}</small>
              </span>
            </label>
            <label>
              <input
                type="radio"
                checked={draft.scopeKind === 'selected'}
                onChange={() => setDraft((current) => ({ ...current, scopeKind: 'selected' }))}
              />
              <ShieldCheck size={16} />
              <span>
                <strong>{t('managedCodexTasks.selectedAccounts', 'Selected OAuth accounts')}</strong>
                <small>{t('managedCodexTasks.selectedAccountsHint', 'Keep a fixed allowlist for this task.')}</small>
              </span>
            </label>
          </fieldset>

          {draft.scopeKind === 'selected' && (
            <div className="managed-task-account-picker managed-task-field-wide">
              {eligibleAccounts.length === 0 ? (
                <p>{t('managedCodexTasks.noEligibleAccounts', 'No injectable OAuth accounts are available.')}</p>
              ) : eligibleAccounts.map((account) => (
                <label key={account.id}>
                  <input
                    type="checkbox"
                    checked={draft.accountIds.includes(account.id)}
                    onChange={() => toggleAccount(account.id)}
                  />
                  <span>{accountLabel(account)}</span>
                  <code>{maskAccountId(account.id)}</code>
                </label>
              ))}
            </div>
          )}

          <label className="managed-task-field">
            <span>{t('managedCodexTasks.initialAccount', 'Initial account (optional)')}</span>
            <select
              value={draft.initialAccountId}
              onChange={(event) => setDraft((current) => ({ ...current, initialAccountId: event.target.value }))}
            >
              <option value="">{t('managedCodexTasks.usePolicy', 'Use pool policy')}</option>
              {scopedAccounts.map((account) => (
                <option key={account.id} value={account.id}>{accountLabel(account)}</option>
              ))}
            </select>
          </label>

          <label className="managed-task-field">
            <span>{t('managedCodexTasks.model', 'Model (optional)')}</span>
            <input
              value={draft.model}
              onChange={(event) => setDraft((current) => ({ ...current, model: event.target.value }))}
              placeholder="gpt-5.6-sol"
            />
          </label>

          <label className="managed-task-field">
            <span>{t('managedCodexTasks.reasoning', 'Reasoning effort')}</span>
            <select
              value={draft.reasoningEffort}
              onChange={(event) => setDraft((current) => ({ ...current, reasoningEffort: event.target.value }))}
            >
              <option value="">{t('managedCodexTasks.defaultValue', 'Default')}</option>
              {['low', 'medium', 'high', 'xhigh'].map((value) => (
                <option key={value} value={value}>{value}</option>
              ))}
            </select>
          </label>

          <label className="managed-task-field">
            <span>{t('managedCodexTasks.maxSwitches', 'Maximum switches (optional)')}</span>
            <input
              type="number"
              min={0}
              step={1}
              value={draft.maxSwitches}
              onChange={(event) => setDraft((current) => ({ ...current, maxSwitches: event.target.value }))}
              placeholder={t('managedCodexTasks.untilPoolExhausted', 'Until pool exhausted')}
            />
          </label>
        </div>

        <div className="managed-task-create-actions">
          <span className={draftError ? 'invalid' : ''}>{draftError ?? t('managedCodexTasks.privacyHint', 'Prompts and transcripts are not stored in the supervisor database.')}</span>
          <button
            className="managed-task-primary-button"
            disabled={Boolean(draftError) || !runtime?.cli.available || creating}
            onClick={() => void createTask()}
          >
            {creating ? <LoaderCircle size={16} className="managed-task-spin" /> : <Play size={16} />}
            {t('managedCodexTasks.enqueue', 'Create and enqueue')}
          </button>
        </div>
      </section>

      <section className="managed-task-workspace">
        <div className="managed-task-list-panel">
          <div className="managed-task-section-heading compact">
            <div>
              <h2>{t('managedCodexTasks.taskList', 'Task queue and history')}</h2>
              <p>{tasks.length} {t('managedCodexTasks.tasks', 'tasks')}</p>
            </div>
            <ListRestart size={20} />
          </div>
          {loading ? (
            <div className="managed-task-empty"><LoaderCircle className="managed-task-spin" /></div>
          ) : tasks.length === 0 ? (
            <div className="managed-task-empty">
              <Activity size={28} />
              <strong>{t('managedCodexTasks.emptyTitle', 'No managed tasks yet')}</strong>
              <span>{t('managedCodexTasks.emptyHint', 'Create one above to start the opt-in supervisor.')}</span>
            </div>
          ) : (
            <div className="managed-task-list">
              {tasks.map((task) => (
                <article
                  key={task.id}
                  className={`managed-task-row ${selectedTaskId === task.id ? 'selected' : ''}`}
                  onClick={() => setSelectedTaskId(task.id)}
                >
                  <div className={`managed-task-status-icon status-${task.status}`}>
                    {statusIcon(task.status)}
                  </div>
                  <div className="managed-task-row-main">
                    <div className="managed-task-row-title">
                      <strong>{task.config.objective}</strong>
                      <span className={`managed-task-status status-${task.status}`}>{statusLabel(task.status)}</span>
                    </div>
                    <div className="managed-task-row-meta">
                      {task.queuePosition && <span>#{task.queuePosition} {t('managedCodexTasks.inQueue', 'in queue')}</span>}
                      <span>{maskAccountId(task.activeAccountId)}</span>
                      <span>{task.switchCount} {t('managedCodexTasks.switches', 'switches')}</span>
                      <span>{formatDuration(task.startedAt, task.completedAt)}</span>
                    </div>
                    <code className="managed-task-cwd">{task.config.cwd}</code>
                  </div>
                  {canCancelManagedTask(task.status) && (
                    <button
                      className="managed-task-icon-button danger"
                      disabled={actionTaskId === task.id}
                      onClick={(event) => {
                        event.stopPropagation();
                        void cancelTask(task);
                      }}
                      aria-label={t('managedCodexTasks.cancel', 'Cancel')}
                    >
                      <CircleStop size={16} />
                    </button>
                  )}
                  <ChevronRight size={17} />
                </article>
              ))}
            </div>
          )}
        </div>

        <aside className="managed-task-detail-panel">
          {!selectedTask ? (
            <div className="managed-task-empty">
              <Activity size={28} />
              <span>{t('managedCodexTasks.selectTask', 'Select a task to inspect normalized evidence.')}</span>
            </div>
          ) : (
            <>
              <div className="managed-task-detail-header">
                <div>
                  <span className={`managed-task-status status-${selectedTask.status}`}>{statusLabel(selectedTask.status)}</span>
                  <h2>{selectedTask.config.objective}</h2>
                </div>
                {selectedTask.status === 'needs_attention' && (
                  <div className="managed-task-resume-actions">
                    {selectedTask.threadId && selectedTask.activeAccountId && (
                      <button
                        disabled={actionTaskId === selectedTask.id}
                        onClick={() => void resumeTask(selectedTask, 'same_account')}
                      >
                        <RotateCcw size={15} />
                        {t('managedCodexTasks.resumeSame', 'Resume current account')}
                      </button>
                    )}
                    <button
                      disabled={actionTaskId === selectedTask.id}
                      onClick={() => void resumeTask(selectedTask, 'next_eligible')}
                    >
                      <RefreshCw size={15} />
                      {selectedTask.threadId
                        ? t('managedCodexTasks.resumeNext', 'Resume next eligible')
                        : t('managedCodexTasks.retrySelection', 'Retry account selection')}
                    </button>
                  </div>
                )}
              </div>

              {(selectedTask.needsAttentionReason || selectedTask.lastError) && (
                <div className="managed-task-attention">
                  <AlertTriangle size={16} />
                  <span>{selectedTask.needsAttentionReason ?? selectedTask.lastError}</span>
                </div>
              )}

              <dl className="managed-task-facts">
                <div><dt>{t('managedCodexTasks.threadId', 'Thread ID')}</dt><dd><code>{selectedTask.threadId ?? '—'}</code></dd></div>
                <div><dt>{t('managedCodexTasks.account', 'Account')}</dt><dd>{maskAccountId(selectedTask.activeAccountId)}</dd></div>
                <div><dt>{t('managedCodexTasks.switches', 'Switches')}</dt><dd>{selectedTask.switchCount}</dd></div>
                <div><dt>{t('managedCodexTasks.lastActivity', 'Last activity')}</dt><dd>{formatDateTime(selectedTask.lastActivityAt)}</dd></div>
                <div><dt>{t('managedCodexTasks.started', 'Started')}</dt><dd>{formatDateTime(selectedTask.startedAt)}</dd></div>
                <div><dt>{t('managedCodexTasks.runtime', 'Runtime')}</dt><dd>{formatDuration(selectedTask.startedAt, selectedTask.completedAt)}</dd></div>
              </dl>

              <div className="managed-task-timeline-heading">
                <h3>{t('managedCodexTasks.evidenceTimeline', 'Normalized evidence timeline')}</h3>
                <button onClick={() => void loadEvidence(selectedTask.id)} aria-label={t('common.refresh', 'Refresh')}>
                  <RefreshCw size={14} />
                </button>
              </div>
              <div className="managed-task-timeline">
                {evidence.length === 0 ? (
                  <div className="managed-task-empty small">{t('managedCodexTasks.noEvidence', 'No persisted evidence yet.')}</div>
                ) : evidence.map((item) => (
                  <div key={item.id} className={`managed-task-evidence evidence-${item.kind}`}>
                    <div className="managed-task-evidence-dot" />
                    <div>
                      <div className="managed-task-evidence-title">
                        <strong>{item.rawEventType ?? item.kind}</strong>
                        <span>{item.source} · {item.confidence}</span>
                        {item.terminal && <em>{t('managedCodexTasks.terminal', 'terminal')}</em>}
                      </div>
                      {item.message && <p>{item.message}</p>}
                      <time>{formatDateTime(item.observedAt)}</time>
                    </div>
                  </div>
                ))}
              </div>
            </>
          )}
        </aside>
      </section>
    </main>
  );
}
