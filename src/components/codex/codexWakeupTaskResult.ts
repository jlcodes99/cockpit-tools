import type { TFunction } from 'i18next';
import type { CodexWakeupTask } from '../../types/codexWakeup';
import { codexBackgroundError } from '../../utils/codexBackgroundError';

function formatFailureReason(message: string, t: TFunction): string {
  switch (message) {
    case 'CODEX_CONFIRMATION_EXPIRED':
      return t('codex.wakeup.confirmationTimedOut');
    case 'CODEX_CONFIRMATION_DECLINED':
      return t('codex.wakeup.confirmationDeclined');
    case 'CODEX_CONFIRMATION_DECLINED_OR_EXPIRED':
      return t('codex.wakeup.confirmationDeclinedOrExpired');
    case 'CODEX_CONFIRMATION_CANCELLED':
    case 'Codex 唤醒测试已取消':
      return t('codex.wakeup.taskRunCancelled');
    case 'CODEX_CONFIRMATION_UI_UNAVAILABLE':
    case 'CODEX_CONFIRMATION_MODE_INVALID':
    case 'CODEX_CONFIRMATION_LOCK':
      return t('codex.wakeup.confirmationUnavailable');
    case 'CODEX_WAKEUP_RUN_INTERRUPTED':
      return t('codex.wakeup.taskRunInterrupted');
    case 'CODEX_WAKEUP_POOL_EMPTY':
      return t('codex.wakeup.taskAccountsApiPoolEmpty');
    default:
      return codexBackgroundError(message, t);
  }
}

export function getCodexWakeupTaskResult(
  task: CodexWakeupTask,
  t: TFunction,
): { summary: string; accountError?: string } {
  const message = task.last_message?.trim();
  const reason = message ? formatFailureReason(message, t) : undefined;
  // Older versions wrote a failed-account count of one for task-level errors.
  // A completed account batch always records a duration, including zero.
  if (task.last_status === 'error' && task.last_duration_ms == null) {
    return { summary: reason ?? t('codex.wakeup.historyFailed') };
  }

  const count = task.last_success_count ?? 0;
  const failed = task.last_failure_count ?? 0;
  if (failed > 0) {
    const summary = count > 0
      ? t('codex.wakeup.lastStatusMixedSummary', { success: count, failed })
      : t('codex.wakeup.lastStatusFailedSummary', { count: failed });
    return { summary, accountError: reason };
  }
  if (count > 0) return { summary: t('codex.wakeup.lastStatusSuccessSummary', { count }) };
  if (task.last_status === 'success') return { summary: t('common.success') };
  if (task.last_status === 'error') return { summary: reason ?? t('codex.wakeup.historyFailed') };
  return { summary: t('codex.wakeup.lastStatusIdle') };
}
