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
  // Diagnostic only. Storage-contract support is reported by supported/reason.
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
  // Opaque backend approval token. Never derive it from the version or plan.
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
  // Retained for IPC compatibility; an empty diagnostic version is valid.
  desktopVersion: string;
}

export interface ClaudeHandoffPair {
  sourceAccountId: string;
  targetAccountId: string;
}

export interface ClaudeHandoffApplyInput extends ClaudeHandoffPair {
  fingerprint: string;
  // Echo the preview diagnostic for older backends; it does not grant eligibility.
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
function unwrapClaudeHandoffError(error: unknown): unknown {
  let current = error;
  // Bound both wrapper depth and JSON input; malformed metadata fails closed.
  for (let depth = 0; depth < 8; depth += 1) {
    if (typeof current === 'string') {
      if (current.length > 16_384) return null;
      const message = current.trim().replace(/^Error:\s*/, '');
      if (!message.startsWith('{')) return message;
      try { current = JSON.parse(message); } catch { return null; }
      continue;
    }
    if (!current || typeof current !== 'object' || Array.isArray(current)) return null;
    const value = current as { code?: unknown; message?: unknown };
    if (typeof value.code === 'string' && /^[A-Z][A-Z0-9_]+$/.test(value.code)) return current;
    if (typeof value.message !== 'string') return current;
    current = value.message;
  }
  return null;
}

export function getClaudeHandoffErrorCode(error: unknown): string | null {
  const value = unwrapClaudeHandoffError(error);
  if (typeof value === 'string') return value.match(/^\[?([A-Z][A-Z0-9_]+)(?:\]|:|\s|$)/)?.[1] ?? null;
  if (value && typeof value === 'object') {
    const code = (value as { code?: unknown }).code;
    if (typeof code === 'string' && /^[A-Z][A-Z0-9_]+$/.test(code)) {
      return code;
    }
  }
  return null;
}

export function getClaudeHandoffRunAction(state: string): 'rollback' | 'recover' | null {
  if (state === 'rolled_back') return null;
  return state === 'applied' ? 'rollback' : 'recover';
}

export interface ClaudeHandoffProcessDiagnostic {
  processId: number;
  processRole: 'desktop-main' | 'claude-cli' | 'desktop-helper' | 'desktop-updater';
  processName: string;
}

// Only consume the backend's bounded process metadata, never message/args/path.
export function getClaudeHandoffProcessDiagnostic(error: unknown): ClaudeHandoffProcessDiagnostic | null {
  const unwrapped = unwrapClaudeHandoffError(error);
  if (!unwrapped || typeof unwrapped !== 'object') return null;
  const value = unwrapped as Record<string, unknown>;
  if (value.code !== 'CLAUDE_WRITER_RUNNING' && value.code !== 'DESKTOP_UPDATE_TIMEOUT'
    && value.code !== 'DESKTOP_CONTRACT_CHANGED') return null;
  if (typeof value.processId !== 'number' || !Number.isInteger(value.processId)
    || value.processId < 1 || value.processId > 0xffffffff) return null;
  if (typeof value.processRole !== 'string'
    || !['desktop-main', 'claude-cli', 'desktop-helper', 'desktop-updater'].includes(value.processRole)) return null;
  if (typeof value.processName !== 'string' || !/^[\p{L}\p{N}._()+ -]{1,128}$/u.test(value.processName)
    || value.processName !== value.processName.trim() || /^[-. ]+$/.test(value.processName)
    || value.processName.startsWith('-') || /\s-\S/.test(value.processName)) return null;
  return { processId: value.processId, processRole: value.processRole as ClaudeHandoffProcessDiagnostic['processRole'], processName: value.processName };
}

export function canRollbackClaudeHandoff(status: ClaudeHandoffStatus | null): boolean {
  // These contract errors come after the backend's default-profile checks.
  // Recovery validates the saved preimages independently of the current format.
  return Boolean(status && (status.supported
    || status.reason === 'DESKTOP_CONTRACT_UNAVAILABLE'
    || status.reason === 'DESKTOP_CONTRACT_UNSUPPORTED'
    || status.reason === 'DESKTOP_CONTRACT_CHANGED'
    || status.reason === 'DESKTOP_VERSION_REQUIRES_REVIEW'));
}

export function canApplyClaudeHandoffPreview(preview: ClaudeHandoffPreview | null): boolean {
  return Boolean(preview?.fingerprint
    && preview.missing === 0 && preview.stale === 0 && preview.replacedBranches === 0
    && (preview.created + preview.updated > 0 || preview.baselineChanged === true));
}

export function canPreviewClaudeHandoff(
  status: ClaudeHandoffStatus | null,
  pair: ClaudeHandoffPair,
): boolean {
  if (!status?.supported || !pair.sourceAccountId || !pair.targetAccountId) return false;
  if (pair.sourceAccountId === pair.targetAccountId) return false;
  return [pair.sourceAccountId, pair.targetAccountId].every((id) =>
    status.accounts.some((account) => account.id === id && account.eligible),
  );
}
