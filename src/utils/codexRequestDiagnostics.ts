import type { CodexAuthorizationFallbackTrace, CodexLocalAccessRequestDetail, CodexLocalAccessRequestPayload, CodexServiceTierAdaptation } from '../types/codexLocalAccess';

/** Only explicit gateway evidence establishes an adaptation, not response tier or route. */
export function recordedServiceTierAdaptation(value?: CodexServiceTierAdaptation | null): CodexServiceTierAdaptation | null {
  return value?.reason === 'official_default_only' && value.sentTier === 'default'
    && Boolean(value.requestedTier?.trim()) ? value : null;
}

/** Display recorded provenance only, never guess a fallback from the final local route. */
export function authorizationFallbackSummaryKey(trace?: CodexAuthorizationFallbackTrace | null): string | null {
  if (!trace || trace.fromRoute !== 'official' || trace.toRoute !== 'local_gateway'
    || !trace.fromAccountId || trace.fromAccountId !== trace.toAccountId) return null;
  return 'codex.requestDiagnostics.authorizationFallback.summary';
}

export function authorizationFallbackReasonKey(reason: string): string {
  const reasons = new Map([
    ['eligibility_rejected', 'notEligible'], ['subscription_sharing_user_not_eligible', 'notEligible'], ['not_eligible', 'notEligible'],
    ['reauthorize', 'authorization'], ['authorization', 'authorization'], ['authorization_failed', 'authorization'],
    ['capability_unsupported', 'unsupported'], ['unsupported_capability', 'unsupported'], ['unsupported', 'unsupported'],
    ['official_throttled', 'limited'], ['rate_limit', 'limited'], ['quota', 'limited'],
    ['cooldown', 'paused'], ['circuit_open', 'paused'],
    ['transport', 'unavailable'], ['upstream_error', 'unavailable'], ['unavailable', 'unavailable'],
  ]);
  return `codex.requestDiagnostics.authorizationFallback.reasons.${reasons.get(reason) || 'unavailable'}`;
}

/** Persisted endpoint IDs describe the route used then, independently of today's account binding. */
export function requestAuthorizationRouteKey(accountId?: string | null, authorizationRoute?: 'official' | 'local_gateway' | null): string {
  if (authorizationRoute === 'official') return 'codex.requestDiagnostics.routeOfficial';
  if (authorizationRoute === 'local_gateway') return 'codex.requestDiagnostics.routeLocal';
  const id = accountId?.trim();
  if (!id) return 'common.unknown';
  return id.startsWith('chatgpt_plan_')
    ? 'codex.requestDiagnostics.routeOfficial' : 'codex.requestDiagnostics.routeLocal';
}

/** Legacy serviceTier mixes request and response values; never infer its source. */
export function requestServiceTierPresentation(event: {
  requestedServiceTier?: string | null; responseServiceTier?: string | null; serviceTier?: string | null;
}) {
  const requested = event.requestedServiceTier?.trim() || '';
  const response = event.responseServiceTier?.trim() || '';
  const legacy = !requested && !response ? event.serviceTier?.trim() || '' : '';
  return { requested, response, legacy };
}

export function requestedServiceTierLabelKey(tier: string): string | null {
  if (tier.toLowerCase() === 'auto') return 'codex.contextOverride.automatic';
  if (['priority', 'fast'].includes(tier.toLowerCase())) return 'codex.speed.fast';
  if (tier.toLowerCase() === 'ultrafast') return 'codex.speed.ultrafast';
  if (['standard', 'default'].includes(tier.toLowerCase())) return 'codex.speed.standard';
  return null;
}

const PHASES = new Set([
  'selection', 'upstream_connect', 'upstream_response', 'stream_read',
  'downstream_delivery', 'client_canceled', 'stream_incomplete', 'completed',
  'routing', 'upstream',
]);
const PHASE_ALIASES = new Map(Object.entries({
  dial: 'upstream_connect', stream_idle: 'stream_read', total_timeout: 'stream_read',
  downstream: 'downstream_delivery', client_cancelled: 'client_canceled',
  identity: 'selection', transport_fallback: 'upstream_connect',
}));

export function formatRequestDiagnosticError(cause: unknown, translate: (key: string) => string): string {
  const message = String(cause).replace(/^Error:\s*/, '');
  return message.startsWith('codex.localAccess.requestDetail.') ? translate(message) : message;
}

export function requestDiagnosticPhaseKey(phase: string | null | undefined): string {
  const normalized = phase && (PHASES.has(phase) ? phase : PHASE_ALIASES.get(phase));
  return `codex.requestDiagnostics.phases.${normalized || 'unknown'}`;
}

/** Classify recorded stages, never an error code shared by local and upstream errors. */
export function requestDiagnosticErrorSource(detail: CodexLocalAccessRequestDetail | null): string {
  const phase = detail?.failurePhase;
  if (phase === 'routing' || phase === 'selection' || phase === 'identity') return 'gateway';
  // The native engine's generic "upstream" also includes validation after route
  // selection. Only the explicit response stage is evidence of an upstream reply.
  if (phase === 'upstream_response') return 'upstream';
  if (phase === 'client_canceled' || phase === 'client_cancelled' || phase === 'downstream' || phase === 'downstream_delivery') return 'client';
  if (phase && ['dial', 'upstream_connect', 'stream_read', 'stream_idle', 'total_timeout', 'stream_incomplete'].includes(phase)) return 'transport';
  return 'unknown';
}

export function formatRequestPayloadBody(payload: Pick<CodexLocalAccessRequestPayload, 'body' | 'truncated'>): string {
  if (payload.truncated) return payload.body;
  try { return JSON.stringify(JSON.parse(payload.body), null, 2); }
  catch { return payload.body; }
}

export function hasRequestFirstResponse(value: number | null | undefined): value is number {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0;
}

export type RequestTimeoutKind = 'open' | 'idle' | 'canceled' | 'gateway';
export type RequestTimeoutField = 'sidecarStreamOpenTimeoutMs' | 'sidecarStreamIdleTimeoutMs'
  | 'sidecarImageStreamOpenTimeoutMs' | 'sidecarImageStreamIdleTimeoutMs';

/** Read the threshold recorded by the gateway, never today's (possibly edited) setting. */
export function requestTimeoutDiagnostic(event: {
  success?: boolean; httpStatus?: number | null; errorCategory?: string | null;
  errorMessage?: string | null; failurePhase?: string | null; requestKind?: string;
}): { kind: RequestTimeoutKind; seconds?: number; field?: RequestTimeoutField } | null {
  if (event.success) return null;
  const message = (event.errorMessage ?? '').toLowerCase();
  if (event.httpStatus === 499 || event.failurePhase === 'client_canceled' || event.errorCategory === 'client_canceled'
    || /downstream_client_closed|stream_client_gone|client canceled|client disconnected/.test(message)) {
    return { kind: 'canceled' };
  }
  const match = message.match(/upstream timed out in (stream_open(?: attempt=\d+\/\d+)?|stream_idle) after ([\d.a-zµμ]+)/);
  const kind = match ? (match[1].startsWith('stream_open') ? 'open' : 'idle')
    : event.errorCategory === 'upstream_first_byte_timeout' ? 'open' : null;
  if (kind) {
    const duration = match?.[2];
    const parts = duration ? [...duration.matchAll(/(\d+(?:\.\d+)?)(ms|us|µs|μs|ns|h|m|s)/g)] : [];
    const units: Record<string, number> = { h: 3600, m: 60, s: 1, ms: .001, us: .000001, 'µs': .000001, 'μs': .000001, ns: .000000001 };
    const seconds = parts.length && parts.map((part) => part[0]).join('') === duration
      ? parts.reduce((sum, part) => sum + Number(part[1]) * units[part[2]], 0) : undefined;
    const image = event.requestKind === 'image_generation' || event.requestKind === 'image_edit';
    const field = kind === 'open'
      ? image ? 'sidecarImageStreamOpenTimeoutMs' : 'sidecarStreamOpenTimeoutMs'
      : image ? 'sidecarImageStreamIdleTimeoutMs' : 'sidecarStreamIdleTimeoutMs';
    return { kind, seconds, field };
  }
  return event.httpStatus === 504 ? { kind: 'gateway' } : null;
}
