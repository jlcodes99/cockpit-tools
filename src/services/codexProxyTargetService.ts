import { useCodexAccountStore } from '../stores/useCodexAccountStore';
import { updateCodexAccountEgressProxy } from './codexService';
import { invoke } from '@tauri-apps/api/core';
import type { CodexAccount } from '../types/codex';
import { isCodexApiKeyAccount } from '../types/codex';
import { singleFlightRead } from '../utils/codexProxyPreview';
import { codexProxyStatusEvents } from '../utils/codexProxyStatusEvents';

export const CODEX_API_SERVICE_PROXY_TARGET = '__api_service__';
export function isCodexProxyTarget(account: Pick<CodexAccount, 'id' | 'auth_mode'>): boolean {
  return account.id === CODEX_API_SERVICE_PROXY_TARGET || isCodexApiKeyAccount(account as CodexAccount);
}
export type CodexProxyTargetAccount = CodexAccount & { proxyTargetLoading?: boolean; proxyTargetError?: boolean };
const snapshots = new Map<string, CodexProxyTargetAccount>();
const revisions = new Map<string, number>();
const listeners = new Set<() => void>();
export function subscribeCodexProxyTargets(listener: () => void) { listeners.add(listener); return () => { listeners.delete(listener); }; }
export function peekCodexProxyTarget(id: string) { return snapshots.get(id); }
function publish() { for (const listener of listeners) listener(); }
export function acceptCodexProxyTarget(account: CodexAccount) {
  revisions.set(account.id, (revisions.get(account.id) ?? 0) + 1);
  snapshots.set(account.id, { ...account, proxyTargetLoading: false, proxyTargetError: false });
  publish();
  codexProxyStatusEvents.publish({ accountId: account.id, status: null });
}
const read = singleFlightRead((targetId: string) => invoke<CodexAccount>('codex_proxy_target_get', { targetId }));
export async function getCodexProxyTarget(targetId: string): Promise<CodexAccount> {
  const revision = revisions.get(targetId) ?? 0;
  try {
    const account = await read(targetId);
    if ((revisions.get(targetId) ?? 0) === revision) { snapshots.set(targetId, { ...account, proxyTargetLoading: false, proxyTargetError: false }); publish(); }
    return snapshots.get(targetId) ?? account;
  } catch (error) {
    if ((revisions.get(targetId) ?? 0) === revision) {
      const previous = snapshots.get(targetId);
      snapshots.set(targetId, { ...(previous ?? apiServiceProxyAccount('')), id: targetId, proxyTargetLoading: false, proxyTargetError: true });
      publish();
    }
    throw error;
  }
}
export function apiServiceProxyAccount(name: string): CodexAccount {
  return { id: CODEX_API_SERVICE_PROXY_TARGET, email: '', account_name: name, auth_mode: 'apikey', created_at: 0, last_used: 0 } as CodexAccount;
}

/** Ordinary OAuth snapshots remain real accounts; card-only projections never overwrite them. */
export function applyCodexProxyAccountSnapshot(account: CodexAccount) {
  if (isCodexProxyTarget(account)) acceptCodexProxyTarget(account);
  else useCodexAccountStore.getState().applyAccountSnapshot(account);
}
export async function updateCodexProxyAccount(accountId: string, disabled = false): Promise<CodexAccount> {
  if (accountId === CODEX_API_SERVICE_PROXY_TARGET || useCodexAccountStore.getState().accounts.some((account) => account.id === accountId && isCodexProxyTarget(account))) {
    const account = await updateCodexAccountEgressProxy(accountId, null, disabled);
    acceptCodexProxyTarget(account);
    return account;
  }
  return useCodexAccountStore.getState().updateAccountEgressProxy(accountId, null, disabled);
}

/** Resource removal may unbind a target; reread every displayed projection before returning. */
export async function reloadCodexProxyTargets(): Promise<void> {
  const results = await Promise.allSettled([...snapshots.keys()].map(getCodexProxyTarget));
  const failed = results.find((result) => result.status === 'rejected');
  if (failed?.status === 'rejected') throw failed.reason;
}
