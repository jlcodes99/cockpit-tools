import type { TFunction } from 'i18next';
import type { CodexSessionVisibilityRepairSummary } from '../types/codex';

const RESTART_SUFFIXES = [
  '。运行中的实例可能需要重启后显示；请手动彻底退出Codex进程后再启动',
  '。请手动彻底退出Codex进程后再启动',
  '；稍等 10 秒钟后，你必须手动彻底退出Codex进程后再启动',
];

function stripRestartSuffix(message: string): string {
  for (const suffix of RESTART_SUFFIXES) {
    if (message.endsWith(suffix)) {
      return message.slice(0, -suffix.length);
    }
  }
  return message;
}

export function formatCodexSessionVisibilityRepairMessage(
  summary: CodexSessionVisibilityRepairSummary,
  t: TFunction,
): string {
  const restartInstruction = t(
    'codex.sessionManager.messages.repairVisibilityRestartInstruction',
    '稍等 10 秒钟后，你必须手动彻底退出Codex进程后再启动',
  );
  const baseMessage = stripRestartSuffix(summary.message);

  if (summary.skippedSqliteFileCount <= 0) {
    return `${baseMessage}；${restartInstruction}`;
  }

  if (summary.mutatedInstanceCount === 0) {
    return t(
      'codex.sessionManager.messages.repairVisibilitySkippedOnly',
      '未发现需要写入的 provider 差异；已跳过 {{count}} 个无效或损坏的 state_5.sqlite，需由 Codex 重新生成后才能修复其中的 SQLite 记录；{{restartInstruction}}',
      { count: summary.skippedSqliteFileCount, restartInstruction },
    );
  }

  return t(
    'codex.sessionManager.messages.repairVisibilitySkippedWithBase',
    '{{message}}；已跳过 {{count}} 个无效或损坏的 state_5.sqlite，需由 Codex 重新生成后才能修复其中的 SQLite 记录；{{restartInstruction}}',
    {
      message: baseMessage,
      count: summary.skippedSqliteFileCount,
      restartInstruction,
    },
  );
}
