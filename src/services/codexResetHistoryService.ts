import { invoke } from '@tauri-apps/api/core';
import type { ResetRule } from './codexResetAutomationService';

export type ResetHistorySource = 'official' | 'local';
export interface ResetObservation {
  available_count: number | null;
  available_card_ids: string[];
  hourly: { remaining: number } | null;
  weekly: { remaining: number } | null;
}

interface LocalResetRecord {
  request_id: string;
  at: number;
  reason: string;
  outcome: string;
  detail?: string | null;
  context?: { rule: ResetRule; before: ResetObservation };
  after_observation?: ResetObservation;
  receipt?: { credit_id: string | null; windows_reset: number | null };
}
export interface ResetHistoryEntry {
  id: string;
  at: number | null;
  kind: string;
  reason?: string;
  detail?: string | null;
  requestId?: string;
  context?: LocalResetRecord['context'];
  after?: ResetObservation;
  receipt?: LocalResetRecord['receipt'];
}
export interface ResetHistoryPage {
  entries: ResetHistoryEntry[];
  nextCursor: string | number | null;
}

export async function loadCodexResetHistory(
  accountId: string, source: ResetHistorySource, cursor: string | number | null = null,
): Promise<ResetHistoryPage> {
  if (source === 'official') {
    const page = await invoke<{
      events: { id: string; kind: string; occurred_at: string }[];
      next_cursor: string | null;
    }>('get_codex_reset_credit_history', { accountId, cursor });
    return {
      entries: page.events.map(event => ({
        id: event.id, kind: event.kind,
        at: Number.isFinite(Date.parse(event.occurred_at)) ? Date.parse(event.occurred_at) : null,
      })),
      nextCursor: page.next_cursor || null,
    };
  }
  const page = await invoke<{
    records: LocalResetRecord[];
    next_cursor: number | null;
  }>('get_codex_reset_usage_history', { accountId, cursor });
  return {
    entries: page.records.map(record => ({
      id: record.request_id, at: record.at * 1000, kind: record.outcome,
      reason: record.reason, detail: record.detail, requestId: record.request_id,
      context: record.context, after: record.after_observation, receipt: record.receipt,
    })),
    nextCursor: page.next_cursor,
  };
}
