export const CODEX_REQUEST_LOGS_NAVIGATION = 'codex-request-logs-navigation';
export interface CodexRequestLogsNavigation { sessionId: string }
let pending: CodexRequestLogsNavigation | null = null;

export function parseCodexRequestLogsNavigation(target: string): CodexRequestLogsNavigation | null {
  const [page, query] = target.split('?');
  if (page !== 'codex-api-service' || !query) return null;
  const params = new URLSearchParams(query);
  if (params.get('view') !== 'logs') return null;
  return { sessionId: params.get('sessionId')?.trim() ?? '' };
}

// Retain the request until the lazy page mounts; an empty ID explicitly clears old filters.
export function requestCodexRequestLogsNavigation(target: string): boolean {
  const navigation = parseCodexRequestLogsNavigation(target);
  if (!navigation) return false;
  pending = navigation;
  window.dispatchEvent(new Event(CODEX_REQUEST_LOGS_NAVIGATION));
  return true;
}

export function takeCodexRequestLogsNavigation(): CodexRequestLogsNavigation | null {
  const request = pending;
  pending = null;
  return request;
}
