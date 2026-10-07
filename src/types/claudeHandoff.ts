export interface ClaudeHandoffAccount {
  id: string;
  label: string;
  identity?: ClaudeHandoffIdentity | null;
  eligible: boolean;
  reason: string | null;
}

export interface ClaudeHandoffIdentity {
  account: string;
  org: string;
}

export interface ClaudeHandoffRunSummary {
  id: string;
  state: string;
  createdAt: number;
  created: number;
  updated: number;
  skippedMissing?: number;
  skippedStale?: number;
  replacedBranches?: number;
  source: ClaudeHandoffIdentity;
  target: ClaudeHandoffIdentity;
  backupDir: string;
  lastError?: string | null;
}

export interface ClaudeHandoffStatus {
  currentAccountId?: string | null;
  currentIdentity?: ClaudeHandoffIdentity | null;
  usesSavedAccountIndex?: boolean;
  supported: boolean;
  reason: string | null;
  desktopVersion: string | null;
  accounts: ClaudeHandoffAccount[];
  runs: ClaudeHandoffRunSummary[];
}

export interface ClaudeHandoffIssue {
  sessionId: string;
  title?: string | null;
  reason: string;
}

export interface ClaudeHandoffPreview {
  fingerprint: string;
  created: number;
  updated: number;
  unchanged: number;
  missing: number;
  stale: number;
  replacedBranches: number;
  baselineChanged: boolean;
  issues: ClaudeHandoffIssue[];
  warnings: ClaudeHandoffIssue[];
  quotaPausesCleared: number;
  preservedBranches?: number;
  desktopVersion: string;
}

export interface ClaudeHandoffPair {
  sourceAccountId: string;
  targetAccountId: string;
}

export interface ClaudeHandoffApplyInput extends ClaudeHandoffPair {
  fingerprint: string;
  desktopVersion: string;
}

export interface ClaudeHandoffMutationResult {
  run: ClaudeHandoffRunSummary;
  reopened: boolean;
  warning: string | null;
  accountSwitched?: boolean;
}

// IPC errors may be structured, JSON-encoded, or CODE: detail strings. Never
// render the raw detail: it can contain local paths and private account IDs.
export function getClaudeHandoffErrorCode(error: unknown): string | null {
  if (error && typeof error === 'object') {
    const value = error as { code?: unknown; message?: unknown };
    if (typeof value.code === 'string' && /^[A-Z][A-Z0-9_]+$/.test(value.code)) {
      return value.code;
    }
    return typeof value.message === 'string' ? getClaudeHandoffErrorCode(value.message) : null;
  }
  if (typeof error !== 'string') return null;
  const message = error.trim().replace(/^Error:\s*/, '');
  if (message.startsWith('{')) {
    try {
      return getClaudeHandoffErrorCode(JSON.parse(message));
    } catch {
      return null;
    }
  }
  return message.match(/^\[?([A-Z][A-Z0-9_]+)(?:\]|:|\s|$)/)?.[1] ?? null;
}

export function getClaudeHandoffRunAction(state: string): 'rollback' | 'recover' | null {
  if (state === 'rolled_back') return null;
  return state === 'applied' ? 'rollback' : 'recover';
}

export function canRollbackClaudeHandoff(status: ClaudeHandoffStatus | null): boolean {
  return Boolean(status && (status.supported || status.reason === 'DESKTOP_VERSION_REQUIRES_REVIEW'));
}

export function canApplyClaudeHandoffPreview(preview: ClaudeHandoffPreview | null): boolean {
  return Boolean(preview?.fingerprint && preview.desktopVersion
    && preview.missing === 0 && preview.stale === 0 && preview.replacedBranches === 0
    && (preview.created + preview.updated > 0 || preview.baselineChanged === true));
}

export function canPreviewClaudeHandoff(
  status: ClaudeHandoffStatus | null,
  pair: ClaudeHandoffPair,
): boolean {
  if (!status?.supported || !status.desktopVersion || !pair.sourceAccountId || !pair.targetAccountId) return false;
  if (pair.sourceAccountId === pair.targetAccountId) return false;
  return [pair.sourceAccountId, pair.targetAccountId].every((id) =>
    status.accounts.some((account) => account.id === id && account.eligible),
  );
}
