import type { CodexProxyRuntimeStatus } from '../services/codexAccountProxyService';

export interface CodexProxyStatusUpdate {
  accountId: string;
  status: CodexProxyRuntimeStatus | null;
}

/** A manual measurement updates all open views without starting periodic probes. */
export function createCodexProxyStatusEvents() {
  const revisions = new Map<string, number>();
  const listeners = new Set<(update: CodexProxyStatusUpdate) => void>();
  return {
    revision(accountId: string) { return revisions.get(accountId) ?? 0; },
    subscribe(listener: (update: CodexProxyStatusUpdate) => void) {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
    publish(update: CodexProxyStatusUpdate) {
      revisions.set(update.accountId, (revisions.get(update.accountId) ?? 0) + 1);
      for (const listener of listeners) listener(update);
    },
  };
}

export const codexProxyStatusEvents = createCodexProxyStatusEvents();
