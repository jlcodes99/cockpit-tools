import { invoke } from '@tauri-apps/api/core';
import { subscribeTauriEvent } from '../utils/tauriEventListener';
import { codexResetCreditDetails } from './codexResetCreditService';

export interface ResetRule {
  enabled: boolean;
  expiry_enabled: boolean;
  before_seconds: number;
  quota_enabled: boolean;
  remaining_percent: number;
}
export type ResetAccountRule = { mode: 'inherit' } | { mode: 'off' } | { mode: 'custom'; rule: ResetRule };
export interface ResetAutomationRecord {
  id: string; account_id: string; at: number; reason: string; status: 'pending' | 'success' | 'failed'; detail?: string;
}
export interface ResetAutomationState {
  revision: number; public_rule: ResetRule; accounts: Record<string, ResetAccountRule>; records: ResetAutomationRecord[];
}
export const defaultResetRule: ResetRule = {
  enabled: false, expiry_enabled: true, before_seconds: 3_600,
  quota_enabled: true, remaining_percent: 5,
};
let state: ResetAutomationState | null = null;
let pending: Promise<ResetAutomationState> | null = null;
let stop: (() => void) | undefined;
let generation = 0;
const subscribers = new Set<() => void>();
function publish(next: ResetAutomationState) {
  if (state && next.revision < state.revision) return state;
  for (const record of next.records) {
    if (record.status === 'success' && !state?.records.some(old => old.id === record.id)) {
      codexResetCreditDetails.invalidate(record.account_id);
    }
  }
  state = next; subscribers.forEach(fn => fn()); return next;
}
export function getResetAutomationSnapshot() { return state; }
export function resolveResetAutomation(snapshot: ResetAutomationState | null, accountId: string) {
  const account = snapshot?.accounts[accountId];
  const mode = account?.mode ?? 'inherit';
  const rule = account?.mode === 'custom' ? account.rule : snapshot?.public_rule;
  return { mode, rule, enabled: Boolean(snapshot && mode !== 'off' && rule?.enabled && (rule.expiry_enabled || rule.quota_enabled)) };
}
export function loadResetAutomation(force = false): Promise<ResetAutomationState> {
  if (pending) return pending;
  if (state && !force) return Promise.resolve(state);
  const version = generation;
  pending = invoke<ResetAutomationState>('get_codex_reset_automation').then(next => version === generation ? publish(next) : state ?? next).finally(() => {
    pending = null;
    if (version !== generation && subscribers.size) void loadResetAutomation(true).catch(error => console.warn('[Reset Automation]', error));
  });
  return pending;
}
export function subscribeResetAutomation(fn: () => void) {
  subscribers.add(fn);
  if (!stop) stop = subscribeTauriEvent('codex:reset-automation-updated', () => {
    generation++;
    void loadResetAutomation(true).catch(error => console.warn('[Reset Automation]', error));
  });
  return () => { subscribers.delete(fn); if (!subscribers.size) { stop?.(); stop = undefined; } };
}
export async function saveResetPublicRule(rule: ResetRule, expected: ResetRule, applyToAll = false, expectedAccounts?: Record<string, ResetAccountRule>) {
  generation++;
  return publish(await invoke<ResetAutomationState>('save_codex_reset_public_rule', { rule, expected, applyToAll, expectedAccounts }));
}
export async function saveResetAccountRule(accountId: string, rule: ResetAccountRule, expected: ResetAccountRule) {
  generation++;
  return publish(await invoke<ResetAutomationState>('save_codex_reset_account_rule', { accountId, rule, expected }));
}
export function validateResetRule(rule: ResetRule): 'invalidTime' | 'invalidPercent' | null {
  if (rule.expiry_enabled && (!Number.isSafeInteger(rule.before_seconds) || rule.before_seconds <= 0)) return 'invalidTime';
  if (!Number.isInteger(rule.remaining_percent) || rule.remaining_percent < 0 || rule.remaining_percent > 100) return 'invalidPercent';
  return null;
}

// An unfinished input in a disabled condition must not prevent turning it off.
// Retain its previously saved value, rather than serializing NaN as JSON null.
export function prepareResetRuleDraft(rule: ResetRule, previous: ResetRule): ResetRule {
  return {
    ...rule,
    before_seconds: (!rule.enabled || !rule.expiry_enabled) && (!Number.isSafeInteger(rule.before_seconds) || rule.before_seconds <= 0)
      ? previous.before_seconds : rule.before_seconds,
    remaining_percent: (!rule.enabled || !rule.quota_enabled) && (!Number.isInteger(rule.remaining_percent) || rule.remaining_percent < 0 || rule.remaining_percent > 100)
      ? previous.remaining_percent : rule.remaining_percent,
  };
}
