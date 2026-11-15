import { invoke } from '@tauri-apps/api/core';
import i18n from '../i18n';
import type { WorkbuddyAccount } from '../types/workbuddy';
import type { WorkbuddyOAuthLoginStartResponse } from './workbuddyService';

export const listAccounts = () => invoke<WorkbuddyAccount[]>('list_workbuddy_ai_accounts');
export const deleteAccounts = (accountIds: string[]) =>
  invoke<void>('delete_workbuddy_ai_accounts', { accountIds });
export const deleteAccount = (accountId: string) => deleteAccounts([accountId]);
export const importFromJson = (jsonContent: string) =>
  invoke<WorkbuddyAccount[]>('import_workbuddy_ai_from_json', { jsonContent });
export const importFromLocal = () => invoke<WorkbuddyAccount[]>('import_workbuddy_ai_from_local');
export const exportAccounts = (accountIds: string[]) =>
  invoke<string>('export_workbuddy_ai_accounts', { accountIds });
function oauthError(error: unknown): never {
  const code = String(error);
  if (code === 'WORKBUDDY_AI_NETWORK_TIMEOUT' || code === 'WORKBUDDY_AI_LOGIN_TIMEOUT') {
    throw new Error(i18n.t('common.networkErrors.directTimeout', { target: 'WorkBuddy AI' }));
  }
  if (code === 'WORKBUDDY_AI_NETWORK_CONNECTION') {
    throw new Error(i18n.t('common.networkErrors.directConnection', { target: 'WorkBuddy AI' }));
  }
  throw error;
}

export const startLogin = () =>
  invoke<WorkbuddyOAuthLoginStartResponse>('workbuddy_ai_oauth_login_start').catch(oauthError);
export const completeLogin = (loginId: string) =>
  invoke<WorkbuddyAccount>('workbuddy_ai_oauth_login_complete', { loginId }).catch(oauthError);
export const cancelLogin = (loginId?: string) =>
  invoke<void>('workbuddy_ai_oauth_login_cancel', { loginId: loginId ?? null });
export const addWithToken = (accessToken: string) =>
  invoke<WorkbuddyAccount>('add_workbuddy_ai_account_with_token', { accessToken });
export const refreshToken = (accountId: string) =>
  invoke<WorkbuddyAccount>('refresh_workbuddy_ai_token', { accountId });
export const refreshAllTokens = () => invoke<number>('refresh_all_workbuddy_ai_tokens');
export const updateAccountTags = (accountId: string, tags: string[]) =>
  invoke<WorkbuddyAccount>('update_workbuddy_ai_account_tags', { accountId, tags });
export const injectToVSCode = (accountId: string) =>
  invoke<void>('switch_workbuddy_ai_account', { accountId });
