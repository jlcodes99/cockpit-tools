export const CHATGPT_PLAN_OPEN_REQUEST = 'chatgpt-plan-open-request';
export interface ChatgptPlanAuthorizationContext { email?: string }
let pending: ChatgptPlanAuthorizationContext | null = null;

export function hasPendingChatgptPlanAuthorization(): boolean { return pending !== null; }

export function takePendingChatgptPlanAuthorization(): ChatgptPlanAuthorizationContext | null {
  const request = pending;
  pending = null;
  return request;
}

/** Navigate first; the account panel consumes context after it mounts. */
export function requestChatgptPlanAuthorization(email?: string): void {
  pending = { email: email?.trim() || undefined };
  window.dispatchEvent(new CustomEvent('app-request-navigate', { detail: 'codex-api-service' }));
  window.dispatchEvent(new CustomEvent(CHATGPT_PLAN_OPEN_REQUEST));
}
