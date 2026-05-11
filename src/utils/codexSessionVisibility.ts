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
      "No writable provider differences were found; skipped {{count}} invalid or corrupted state_5.sqlite file(s). Codex must regenerate them before their SQLite records can be repaired.",
      { count: summary.skippedSqliteFileCount },
    );
  }

  return t(
    'codex.sessionManager.messages.repairVisibilitySkippedWithBase',
    "{{message}}; skipped {{count}} invalid or corrupted state_5.sqlite file(s). Codex must regenerate them before their SQLite records can be repaired.",
    { message: summary.message, count: summary.skippedSqliteFileCount },
  );
}
