import { useMemo } from 'react';
import { loadCodexWakeupHistory } from '../services/codexWakeupService';
import type { CodexWakeupHistoryItem } from '../types/codexWakeup';
import { useLocalHistory } from './useLocalHistory';

export function latestCodexWakeupByAccount(records: readonly CodexWakeupHistoryItem[]) {
  const results = new Map<string, CodexWakeupHistoryItem>();
  for (const item of records) {
    const previous = results.get(item.account_id);
    if (!previous || previous.timestamp < item.timestamp) results.set(item.account_id, item);
  }
  return results;
}

export function codexWakeupResultCode(item: CodexWakeupHistoryItem): string | undefined {
  if (item.http_status !== undefined) return `HTTP ${item.http_status}`;
  if (item.success && item.cli_path) return 'CLI 0';

  const error = item.error?.trim() ?? '';
  const http = /^API 直连唤醒失败\(([1-5][0-9]{2})\):/.exec(error);
  if (http) return `HTTP ${http[1]}`;
  const exit = /^Codex CLI 退出失败: exit (?:status|code): (0x[0-9a-f]+|-?\d+)$/i.exec(error);
  if (exit) return `CLI ${exit[1]}`;
  return /^CHATGPT_PLAN_[A-Z_]+$/.test(error) ? error : undefined;
}

export function codexWakeupResultLabel(item: CodexWakeupHistoryItem, translate: (key: string) => string) {
  const statusKey = item.error === 'Codex 唤醒测试已取消'
    ? 'common.cancelled'
    : item.success ? 'codex.wakeup.historySuccess' : 'codex.wakeup.historyFailed';
  const code = codexWakeupResultCode(item);
  return [translate('codex.wakeup.tab'), translate(statusKey), code].filter(Boolean).join(' · ');
}

export function useCodexAccountWakeupResults(enabled: boolean) {
  const { records } = useLocalHistory(
    loadCodexWakeupHistory, 'codex://wakeup-history-changed', undefined, enabled,
  );
  return useMemo(() => latestCodexWakeupByAccount(records), [records]);
}
