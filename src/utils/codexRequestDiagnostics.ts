import type { CodexLocalAccessRequestPayload } from '../types/codexLocalAccess';

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
  if (['standard', 'default'].includes(tier.toLowerCase())) return 'codex.speed.standard';
  return null;
}

const PHASES = new Set([
  'selection', 'upstream_connect', 'upstream_response', 'stream_read',
  'downstream_delivery', 'client_canceled', 'stream_incomplete', 'completed',
]);

export function formatRequestDiagnosticError(cause: unknown, translate: (key: string) => string): string {
  const message = String(cause).replace(/^Error:\s*/, '');
  return message.startsWith('codex.localAccess.requestDetail.') ? translate(message) : message;
}

export function requestDiagnosticPhaseKey(phase: string | null | undefined): string {
  return `codex.requestDiagnostics.phases.${phase && PHASES.has(phase) ? phase : 'unknown'}`;
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
