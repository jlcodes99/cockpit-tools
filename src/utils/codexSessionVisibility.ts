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
    return t(
      'codex.sessionManager.messages.repairVisibilitySkippedOnly',
      '未发现需要写入的 provider 差异；已跳过 {{count}} 个无效或损坏的 state_5.sqlite，需由 Codex 重新生成后才能修复其中的 SQLite 记录',
      { count: summary.skippedSqliteFileCount },
    );
  }

  return t(
    'codex.sessionManager.messages.repairVisibilitySkippedWithBase',
    '{{message}}；已跳过 {{count}} 个无效或损坏的 state_5.sqlite，需由 Codex 重新生成后才能修复其中的 SQLite 记录',
    { message: summary.message, count: summary.skippedSqliteFileCount },
  );
}
