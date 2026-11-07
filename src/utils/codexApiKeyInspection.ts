export interface CodexApiKeyInspectionRequest {
  apiKeyId: string;
  requestKind: string;
}

const inspectionEvent = 'codex-api-key-inspection-requested';
let pendingRequest: CodexApiKeyInspectionRequest | null = null;

/** Keep context until the existing API service page mounts or receives the request. */
export function requestCodexApiKeyInspection(request: CodexApiKeyInspectionRequest): void {
  pendingRequest = { ...request };
  window.dispatchEvent(new CustomEvent('app-request-navigate', { detail: 'codex-api-service' }));
  window.dispatchEvent(new Event(inspectionEvent));
}

export function consumeCodexApiKeyInspectionRequest(): CodexApiKeyInspectionRequest | null {
  const request = pendingRequest;
  pendingRequest = null;
  return request;
}

/** Install before consuming, so both an already mounted page and a new page work without timers. */
export function subscribeCodexApiKeyInspectionRequests(
  listener: (request: CodexApiKeyInspectionRequest) => void,
): () => void {
  const handleRequest = () => {
    const request = consumeCodexApiKeyInspectionRequest();
    if (request) listener(request);
  };
  window.addEventListener(inspectionEvent, handleRequest);
  handleRequest();
  return () => window.removeEventListener(inspectionEvent, handleRequest);
}
