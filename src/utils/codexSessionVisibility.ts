import type { TFunction } from 'i18next';
import type { CodexSessionVisibilityRepairSummary } from '../types/codex';

export function formatCodexSessionVisibilityRepairMessage(
  summary: CodexSessionVisibilityRepairSummary,
  t: TFunction,
): string {
  if (summary.skippedSqliteFileCount <= 0) {
    return summary.message;
  }

  if (summary.mutatedInstanceCount === 0) {
    const skipped = t(
      'codex.sessionManager.messages.repairVisibilitySkippedOnly',
      '未发现需要写入的会话索引差异；已跳过 {{count}} 个无效或损坏的 SQLite 会话库，需由 Codex 重新生成后才能修复其中的记录',
      { count: summary.skippedSqliteFileCount },
    );
    return [skipped, summary.localProjectRecoveryDeferredMessage].filter(Boolean).join(' ');
  }

  return t(
    'codex.sessionManager.messages.repairVisibilitySkippedWithBase',
    '{{message}}；已跳过 {{count}} 个无效或损坏的 SQLite 会话库，需由 Codex 重新生成后才能修复其中的记录',
    { message: summary.message, count: summary.skippedSqliteFileCount },
  );
}
