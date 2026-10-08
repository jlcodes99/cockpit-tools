import type { CodexLocalAccessRequestPayload } from '../types/codexLocalAccess';

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
