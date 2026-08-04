import type { CodexAccount } from '../types/codex';
import {
  isCodexAgentIdentityAccount,
  isCodexApiKeyAccount,
  isCodexPendingOAuthAccount,
  isCodexWebSessionAccount,
} from '../types/codex';
import type {
  CreateManagedCodexTaskInput,
  ManagedCodexTaskEvidence,
  ManagedCodexTaskStatus,
} from '../types/codexManagedTask';

export interface ManagedTaskDraft {
  objective: string;
  cwd: string;
  scopeKind: 'cockpit_pool' | 'selected';
  accountIds: string[];
  initialAccountId: string;
  model: string;
  reasoningEffort: string;
  maxSwitches: string;
}

export type ManagedTaskDraftIssue = 'cwd' | 'objective' | 'accounts' | 'max_switches';

export function emptyManagedTaskDraft(): ManagedTaskDraft {
  return {
    objective: '',
    cwd: '',
    scopeKind: 'cockpit_pool',
    accountIds: [],
    initialAccountId: '',
    model: '',
    reasoningEffort: '',
    maxSwitches: '',
  };
}

export function isEligibleManagedCodexAccount(account: CodexAccount): boolean {
  return !(
    isCodexApiKeyAccount(account) ||
    isCodexAgentIdentityAccount(account) ||
    isCodexWebSessionAccount(account) ||
    isCodexPendingOAuthAccount(account) ||
    account.requires_reauth
  );
}

export function validateManagedTaskDraft(draft: ManagedTaskDraft): ManagedTaskDraftIssue | null {
  if (!draft.cwd.trim()) return 'cwd';
  if (!draft.objective.trim()) return 'objective';
  if (draft.scopeKind === 'selected' && draft.accountIds.length === 0) return 'accounts';
  if (draft.maxSwitches.trim()) {
    const value = Number(draft.maxSwitches);
    if (!Number.isInteger(value) || value < 0) return 'max_switches';
  }
  return null;
}

export function buildManagedTaskInput(draft: ManagedTaskDraft): CreateManagedCodexTaskInput {
  const issue = validateManagedTaskDraft(draft);
  if (issue) throw new Error(`invalid managed task draft: ${issue}`);
  return {
    objective: draft.objective.trim(),
    cwd: draft.cwd.trim(),
    accountScope:
      draft.scopeKind === 'cockpit_pool'
        ? { kind: 'cockpit_pool' }
        : { kind: 'selected', accountIds: [...new Set(draft.accountIds)] },
    initialAccountId: draft.initialAccountId || undefined,
    model: draft.model.trim() || undefined,
    reasoningEffort: draft.reasoningEffort || undefined,
    maxSwitches: draft.maxSwitches.trim() ? Number(draft.maxSwitches) : undefined,
  };
}

export function mergeManagedTaskEvidence(
  current: ManagedCodexTaskEvidence[],
  additions: ManagedCodexTaskEvidence[],
): ManagedCodexTaskEvidence[] {
  const byId = new Map(current.map((item) => [item.id, item]));
  for (const item of additions) byId.set(item.id, item);
  return [...byId.values()].sort(
    (left, right) => left.observedAt - right.observedAt || left.id.localeCompare(right.id),
  );
}

export function canCancelManagedTask(status: ManagedCodexTaskStatus): boolean {
  return ['queued', 'preparing', 'running', 'draining', 'switching', 'resuming'].includes(status);
}
