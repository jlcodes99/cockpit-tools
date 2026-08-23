export type ManagedCodexTaskStatus =
  | 'queued'
  | 'preparing'
  | 'running'
  | 'draining'
  | 'switching'
  | 'resuming'
  | 'completed'
  | 'failed'
  | 'cancelled'
  | 'needs_attention';

export type ManagedCodexAccountScope =
  | { kind: 'cockpit_pool' }
  | { kind: 'selected'; accountIds: string[] };

export interface CreateManagedCodexTaskInput {
  objective: string;
  cwd: string;
  accountScope: ManagedCodexAccountScope;
  initialAccountId?: string;
  model?: string;
  reasoningEffort?: string;
  maxSwitches?: number;
}

export type ManagedCodexFailureClass =
  | 'quota_exhausted'
  | 'authentication'
  | 'network'
  | 'context_window'
  | 'model_capacity'
  | 'user_interrupted'
  | 'other';

export interface ManagedCodexTaskSnapshot {
  id: string;
  config: CreateManagedCodexTaskInput;
  status: ManagedCodexTaskStatus;
  queuePosition?: number;
  activeAccountId?: string;
  pendingAccountId?: string;
  attemptedAccountIds: string[];
  threadId?: string;
  activeTurnId?: string;
  switchCount: number;
  createdAt: number;
  startedAt?: number;
  updatedAt: number;
  lastActivityAt?: number;
  completedAt?: number;
  lastFailureClass?: ManagedCodexFailureClass;
  lastError?: string;
  needsAttentionReason?: string;
  runGeneration: number;
  processId?: number;
  processStartedAt?: number;
  executablePath?: string;
  lastEventSeq: number;
  recoveryAttempts: number;
}

export type ManagedCodexEvidenceSource =
  | 'app_server'
  | 'exec_json'
  | 'rollout_jsonl'
  | 'proxy';

export type ManagedCodexEvidenceKind =
  | 'activity'
  | 'turn_started'
  | 'quota_warning'
  | 'turn_completed'
  | 'turn_failed'
  | 'turn_interrupted'
  | 'unknown';

export interface ManagedCodexTaskEvidence {
  id: string;
  observedAt: number;
  source: ManagedCodexEvidenceSource;
  kind: ManagedCodexEvidenceKind;
  confidence: 'informational' | 'suspected' | 'confirmed';
  terminal: boolean;
  threadId?: string;
  turnId?: string;
  rawEventType?: string;
  errorCode?: string;
  failureClass?: ManagedCodexFailureClass;
  message?: string;
}

export interface ManagedCodexEvidenceCursor {
  observedAt: number;
  id: string;
}

export interface ManagedCodexTaskEvidencePage {
  items: ManagedCodexTaskEvidence[];
  nextCursor?: ManagedCodexEvidenceCursor;
}

export interface ManagedCodexCliInstallHint {
  label: string;
  command: string;
}

export interface ManagedCodexCliStatus {
  available: boolean;
  binaryPath?: string;
  configuredCodexCliPath?: string;
  configuredNodePath?: string;
  version?: string;
  source?: string;
  message?: string;
  requiredRuntimePaths: string[];
  checkedAt: number;
  installHints: ManagedCodexCliInstallHint[];
}

export interface ManagedCodexTaskRuntimeStatus {
  cli: ManagedCodexCliStatus;
  activeTaskId?: string;
  queueLength: number;
}

export type ManagedCodexTaskResumeMode = 'same_account' | 'next_eligible';

export interface ManagedCodexEvidenceEventPayload {
  taskId: string;
  evidence: ManagedCodexTaskEvidence;
}
