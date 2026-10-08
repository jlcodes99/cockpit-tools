import type { CodexSwitchAuthFailure } from './codexSwitchAuthFailure';

/** Lifecycle failures are retryable setup errors, not requests to reauthorize. */
export function codexLaunchErrorKey(error: unknown): string | null {
  const code = String(error ?? '').replace(/^Error:\s*/, '');
  if (code === 'CODEX_SWITCH_BINDING_FAILED') return 'codex.launchErrors.bindingFailed';
  if (code === 'CODEX_PROCESS_SCAN_FAILED') return 'codex.launchErrors.processScanFailed';
  if (code === 'PROXY_ENTRY_PORT_UNAVAILABLE' || code === 'PROXY_RUNTIME_NOT_READY') {
    return 'codex.launchErrors.proxyEntryFailed';
  }
  return null;
}

export type CodexLaunchOperation = 'instance-launch' | 'switch-and-start';

export type CodexLaunchStepId =
  | 'checkInstance'
  | 'checkAccount'
  | 'checkOccupancy'
  | 'stopPrevious'
  | 'prepareCredentials'
  | 'writeProfile'
  | 'startClient';

export type CodexLaunchStepStatus =
  | 'pending'
  | 'running'
  | 'completed'
  | 'warning'
  | 'skipped'
  | 'error';

export interface CodexMappedLaunchProgress {
  type?: 'start' | 'complete' | 'cancelled' | 'error';
  instanceId: string;
  instanceName: string;
  isDefault: true;
  accountId: string;
  operation: 'switch-and-start';
  progress?: number;
  step?: CodexLaunchStepId;
  stepStatus?: CodexLaunchStepStatus;
  details: Record<string, unknown>;
  error?: string;
  authFailure?: CodexSwitchAuthFailure | null;
  canRetry?: boolean;
  source: 'switch-service';
}

const SWITCH_STEP_MAP: Record<string, CodexLaunchStepId> = {
  credentials: 'checkAccount',
  accessToken: 'checkAccount',
  refreshTokens: 'prepareCredentials',
  stopRuntime: 'stopPrevious',
  writeCredentials: 'writeProfile',
  syncSettings: 'writeProfile',
  startClient: 'startClient',
};

function normalizedSwitchDetails(
  step: string | undefined,
  details: Record<string, unknown>,
  accountId: string,
): Record<string, unknown> {
  const normalized: Record<string, unknown> = { ...details, accountId };
  if (step === 'accessToken') {
    normalized.accessTokenExpiresAt = details.expiresAt;
    normalized.accessTokenRefreshDue = details.refreshDue;
    delete normalized.expiresAt;
    delete normalized.refreshDue;
  } else if (step === 'refreshTokens') {
    normalized.refreshRequired = details.required;
    delete normalized.required;
  }
  return normalized;
}

export function mapCodexSwitchProgressToLaunch(
  payload: Record<string, unknown>,
): CodexMappedLaunchProgress | null {
  const accountId = typeof payload.accountId === 'string' ? payload.accountId : '';
  if (!accountId) return null;
  const switchStep = typeof payload.step === 'string' ? payload.step : undefined;
  const details =
    payload.details && typeof payload.details === 'object'
      ? (payload.details as Record<string, unknown>)
      : {};
  return {
    type:
      payload.type === 'start' || payload.type === 'complete' || payload.type === 'cancelled' || payload.type === 'error'
        ? payload.type
        : undefined,
    instanceId: '__default__',
    instanceName: '',
    isDefault: true,
    accountId,
    operation: 'switch-and-start',
    progress: typeof payload.progress === 'number' ? payload.progress : undefined,
    step: switchStep ? SWITCH_STEP_MAP[switchStep] : undefined,
    stepStatus:
      typeof payload.stepStatus === 'string'
        ? (payload.stepStatus as CodexLaunchStepStatus)
        : undefined,
    details: normalizedSwitchDetails(switchStep, details, accountId),
    error: typeof payload.error === 'string' ? payload.error : undefined,
    authFailure: (payload.authFailure as CodexSwitchAuthFailure | null | undefined) ?? null,
    canRetry: payload.canRetry === true,
    source: 'switch-service',
  };
}
