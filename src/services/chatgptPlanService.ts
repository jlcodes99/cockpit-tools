import { invoke } from '@tauri-apps/api/core';
import type { CodexLocalAccessState } from '../types/codexLocalAccess';

/** Public summaries deliberately contain no access or refresh tokens. */
export interface ChatGptPlanAuthorization {
  id: string;
  label: string;
  email?: string | null;
  subject: string;
  client_id?: string;
  source_account_id?: string | null;
  source_binding_valid?: boolean;
  source_binding_error?: string | null;
  connected: boolean;
  needs_reauth: boolean;
  verified_at?: number | null;
  verified_model?: string | null;
  last_checked_at?: number | null;
  check_error?: string | null;
  last_service_mode?: 'codex' | 'official' | null;
  expires_at?: number | null;
  created_at: number;
  updated_at: number;
}
export interface ChatGptPlanAccount extends ChatGptPlanAuthorization { in_service: boolean }

/** Use the same persisted pool membership as the rest of the API Service page. */
export function withChatGptPlanMembership(
  account: ChatGptPlanAuthorization | ChatGptPlanAccount,
  memberAccountIds?: readonly string[],
  previous?: ChatGptPlanAccount,
  officialAccountBindings?: Readonly<Record<string, string>>,
): ChatGptPlanAccount {
  const inService = memberAccountIds
    ? officialAccountBindings
      ? Boolean(account.source_account_id && memberAccountIds.includes(account.source_account_id) && officialAccountBindings[account.source_account_id] === account.id)
      : memberAccountIds.includes(`chatgpt_plan_${account.id}`) || Boolean(account.source_account_id && memberAccountIds.includes(account.source_account_id) && 'in_service' in account && account.in_service)
    : 'in_service' in account && typeof account.in_service === 'boolean'
      ? account.in_service
      : previous?.id === account.id ? previous.in_service : false;
  return { ...account, in_service: inService };
}

export interface ChatGptPlanModel { id: string; display_name?: string }
export interface ChatGptPlanLogin { login_id: string; authorize_url: string; expires_at: number }
export interface ChatGptPlanLoginStatus {
  status: 'pending' | 'exchanging' | 'complete' | 'error' | 'cancelled' | 'expired';
  account?: ChatGptPlanAuthorization | null;
  error?: string | null;
}

let accountsRead: Promise<ChatGptPlanAccount[]> | null = null;
export function listChatGptPlanAccounts(): Promise<ChatGptPlanAccount[]> {
  if (accountsRead) return accountsRead;
  const request = new Promise<ChatGptPlanAccount[]>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('CHATGPT_PLAN_TIMEOUT')), 15_000);
    invoke<ChatGptPlanAccount[]>('chatgpt_plan_list_accounts').then(
      (accounts) => { clearTimeout(timer); resolve(accounts); },
      (failure) => { clearTimeout(timer); reject(failure); },
    );
  });
  accountsRead = request;
  const clear = () => { if (accountsRead === request) accountsRead = null; };
  void request.then(clear, clear);
  return request;
}
export const unlinkChatGptPlanAccount = async (registrationId: string) =>
  (await invoke<ChatGptPlanReleaseResult>('chatgpt_plan_unbind_account', { registrationId })).authorization;
export const bindChatGptPlanAccount = (registrationId: string, sourceAccountId: string) =>
  invoke<ChatGptPlanAuthorization>('chatgpt_plan_bind_account', { registrationId, sourceAccountId });
interface ServiceModeFlight { identity: string; request: Promise<CodexLocalAccessState>; wait?: Promise<CodexLocalAccessState> }
export interface ChatGptPlanServiceModeOptions { intent: 'join' | 'switch' }
/** Membership is always the source account; optional read models confirm its current channel. */
export function isChatGptPlanServiceModeApplied(state: CodexLocalAccessState, sourceId: string, mode: 'codex' | 'official', registrationId?: string): boolean {
  if (!state.collection?.accountIds.includes(sourceId) || state.collection.pausedAccountIds?.includes(sourceId)) return false;
  if (mode === 'official' && !registrationId) return false;
  const bindings = state.collection.officialAccountBindings;
  if (bindings !== undefined && (mode === 'official' ? bindings[sourceId] !== registrationId : Boolean(bindings[sourceId]))) return false;
  const member = state.serviceMembers?.find((item) => item.sourceAccountId === sourceId);
  return !state.serviceMembers || Boolean(member && member.accountId === sourceId && member.mode === mode && !member.paused);
}
const serviceModeRequests = new Map<string, ServiceModeFlight>();
function waitForServiceMode(flight: ServiceModeFlight): Promise<CodexLocalAccessState> {
  if (flight.wait) return flight.wait;
  const wait = new Promise<CodexLocalAccessState>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('CHATGPT_PLAN_TIMEOUT')), 90_000);
    flight.request.then((state) => { clearTimeout(timer); resolve(state); }, (error) => { clearTimeout(timer); reject(error); });
  });
  flight.wait = wait;
  const releaseWait = () => { if (flight.wait === wait) flight.wait = undefined; };
  void wait.then(releaseWait, releaseWait);
  return wait;
}
/** A UI timeout never releases the underlying mutation's single-flight lock. */
export function setChatGptPlanServiceMode(sourceAccountId: string, mode: 'codex' | 'official', registrationId?: string, modelOrOptions?: string | ChatGptPlanServiceModeOptions): Promise<CodexLocalAccessState> {
  const model = typeof modelOrOptions === 'string' ? modelOrOptions : undefined;
  const intent = typeof modelOrOptions === 'object' ? modelOrOptions.intent : 'join';
  const identity = JSON.stringify([mode, registrationId ?? null, model ?? null, intent]);
  const pending = serviceModeRequests.get(sourceAccountId);
  if (pending) return pending.identity === identity ? waitForServiceMode(pending) : Promise.reject(new Error('CHATGPT_PLAN_BUSY'));
  const request = invoke<CodexLocalAccessState>('chatgpt_plan_set_service_mode', { sourceAccountId, mode, registrationId: registrationId ?? null, intent, ...(model === undefined ? {} : { model }) });
  const flight: ServiceModeFlight = { identity, request };
  serviceModeRequests.set(sourceAccountId, flight);
  const release = () => { if (serviceModeRequests.get(sourceAccountId)?.request === request) serviceModeRequests.delete(sourceAccountId); };
  void request.then(() => {
    release();
    // A backend transaction may finish after every UI waiter timed out.
    if (typeof window !== 'undefined') {
      window.dispatchEvent(new Event('chatgpt-plan-accounts-changed'));
      window.dispatchEvent(new Event('codex-local-access-state-updated'));
    }
  }, release);
  return waitForServiceMode(flight);
}

export const checkChatGptPlanAuthorization = (registrationId: string) =>
  invoke<ChatGptPlanAuthorization>('chatgpt_plan_check_authorization', { registrationId });

/** Required scopes are validated by the backend during OAuth, bind and join. */
export function isChatGptPlanAuthorizationUsable(account?: ChatGptPlanAuthorization | null): boolean {
  return Boolean(account?.connected && !account.needs_reauth && account.source_account_id
    && account.source_binding_valid === true && !account.source_binding_error);
}

export const startChatGptPlanLogin = (registrationId?: string) =>
  invoke<ChatGptPlanLogin>('chatgpt_plan_start_login', { registrationId: registrationId ?? null });
export const pollChatGptPlanLogin = (loginId: string) =>
  invoke<ChatGptPlanLoginStatus>('chatgpt_plan_poll_login', { loginId });
export const submitChatGptPlanCallbackUrl = (loginId: string, callbackUrl: string) =>
  invoke<void>('chatgpt_plan_submit_callback_url', { loginId, callbackUrl });
export const openChatGptPlanIncognitoWindow = (loginId: string) =>
  invoke<void>('chatgpt_plan_open_incognito_window', { loginId });
export const cancelChatGptPlanLogin = (loginId: string) =>
  invoke<void>('chatgpt_plan_cancel_login', { loginId });
export const listChatGptPlanModels = (registrationId: string) =>
  invoke<ChatGptPlanModel[]>('chatgpt_plan_list_models', { registrationId });
export const verifyChatGptPlan = (registrationId: string, model: string) =>
  invoke<ChatGptPlanAuthorization>('chatgpt_plan_verify', { registrationId, model });
// Joining never performs a generated model test. Keep the optional legacy argument compatible.
export const joinChatGptPlan = (registrationId: string, model?: string) =>
  invoke<void>('chatgpt_plan_join', { registrationId, ...(model === undefined ? {} : { model }) });
export const removeChatGptPlanFromService = (registrationId: string) =>
  invoke<void>('chatgpt_plan_remove_from_service', { registrationId });
export const disconnectChatGptPlan = (registrationId: string) =>
  invoke<ChatGptPlanReleaseResult>('chatgpt_plan_disconnect', { registrationId });
export type ChatGptPlanReleaseDisposition = 'local_gateway' | 'pause';
export interface ChatGptPlanReleaseResult {
  authorization: ChatGptPlanAccount;
  state: CodexLocalAccessState;
  remote_revoked: boolean;
}
/** One atomic backend transition; never implement release as separate UI mutations. */
export const releaseChatGptPlanAuthorization = (registrationId: string, disposition: ChatGptPlanReleaseDisposition) =>
  invoke<ChatGptPlanReleaseResult>('chatgpt_plan_release_authorization', { registrationId, disposition });

export function chatGptPlanStatusKey(account: ChatGptPlanAccount): string {
  if (!account.connected) return 'chatgptPlan.status.disconnected';
  if (account.needs_reauth) return 'chatgptPlan.status.reauthorize';
  if (account.source_account_id && account.source_binding_error) return chatGptPlanErrorKey(account.source_binding_error);
  if (account.source_account_id && account.source_binding_valid !== true) return 'chatgptPlan.officialFlow.bindingChecking';
  if (account.in_service) return 'chatgptPlan.status.joined';
  return 'chatgptPlan.status.ready';
}

const errors: Record<string, string> = {
  MODE_PREFERENCE_SAVE_FAILED: 'modePreferenceSaveFailed',
  SERVICE_MODE_INVALID: 'serviceModeInvalid', SOURCE_INVALID: 'sourceUnsupported',
  SERVICE_ACCOUNT_INELIGIBLE: 'serviceAccountIneligible', SERVICE_CHANGED: 'serviceChanged', SERVICE_MEMBER_REMOVED: 'serviceMemberRemoved', MODEL_REQUIRED: 'modelRequired',
  NETWORK: 'network', TIMEOUT: 'timeout', STORAGE: 'storage', STORAGE_FAILED: 'storage',
  BUSY: 'busy', STORAGE_BUSY: 'busy', INVALID_ID: 'missing',
  NOT_FOUND: 'missing', DISCONNECTED: 'reauthorize', REAUTH_REQUIRED: 'reauthorize',
  PERMISSION_REQUIRED: 'permission', AUTH_DENIED: 'denied', INVALID_CALLBACK: 'callback',
  INVALID_ID_TOKEN: 'identity', IDENTITY_MISMATCH: 'mismatch', REGISTRATION_CHANGED: 'mismatch',
  LOGIN_EXPIRED: 'expired', LOGIN_CANCELLED: 'cancelled', NO_MODELS: 'noModels',
  MODEL_UNAVAILABLE: 'model', VERIFY_FAILED: 'verify', STREAM_INCOMPLETE: 'verify',
  USAGE_LIMIT: 'usage', USAGE_UNAVAILABLE: 'usageUnavailable', NATIVE_REQUIRED: 'native',
  OPEN_BROWSER: 'browser',
  BINDING_REQUIRED: 'bindingRequired', BINDING_CONFLICT: 'bindingConflict',
  BINDING_MISMATCH: 'bindingRequired', ACCOUNT_NOT_FOUND: 'missing',
  SOURCE_NOT_FOUND: 'sourceMissing', SOURCE_UNSUPPORTED: 'sourceUnsupported',
  SOURCE_IDENTITY_UNKNOWN: 'sourceIdentityUnknown', SOURCE_CHANGED: 'sourceChanged',
  SOURCE_EMAIL_MISMATCH: 'sourceEmailMismatch',
  CONFIGURATION: 'configuration', NOT_VERIFIED: 'notVerified',
  NOT_ELIGIBLE: 'notEligible', UNSUPPORTED_CAPABILITY: 'unsupportedCapability',
};

/** Only translated, allowlisted errors reach UI; upstream text may contain credentials. */
export function chatGptPlanErrorKey(error: unknown): string {
  const code = error instanceof Error ? error.message : String(error);
  const reason = code.startsWith('CHATGPT_PLAN_') ? errors[code.slice('CHATGPT_PLAN_'.length)] : undefined;
  return `chatgptPlan.error.${reason ?? 'generic'}`;
}

export function chatGptPlanAuthorizationUrl(raw: string): string {
  const url = new URL(raw);
  if (url.protocol !== 'https:' || url.hostname !== 'auth.openai.com' || url.username || url.password) {
    throw new Error('CHATGPT_PLAN_INVALID_CALLBACK');
  }
  return url.toString();
}
