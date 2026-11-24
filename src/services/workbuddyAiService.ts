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

export interface WorkbuddyAiCheckinStatus {
  todayCheckedIn: boolean;
  active: boolean;
  streakDays: number;
  dailyCredit: number;
  todayCredit?: number | null;
  checkinDates?: string[] | null;
  todayDate?: string | null;
  todayScore: number;
  monthTotalDays: number;
  pendingConfirmation?: boolean;
  message?: string | null;
}

export interface WorkbuddyAiCheckinResult {
  success: boolean;
  alreadyClaimed: boolean;
  inactive?: boolean;
  pendingConfirmation?: boolean;
  resultCode?: string | null;
  activityDate?: string | null;
  warning?: string | null;
  message?: string | null;
  credit?: number | null;
  streakDays?: number | null;
  scoreBefore?: number | null;
  scoreAfter?: number | null;
  modelId?: string | null;
}

export const getCheckinStatusWorkbuddyAi = (accountId: string) =>
  invoke<WorkbuddyAiCheckinStatus>('get_checkin_status_workbuddy_ai', { accountId });

export const checkinWorkbuddyAi = (accountId: string) =>
  invoke<WorkbuddyAiCheckinResult>('checkin_workbuddy_ai', { accountId });

export function localizeWorkbuddyAiCheckinError(
  raw: unknown,
  translate: (key: string, defaultValue: string) => string = (key, defaultValue) =>
    i18n.t(key, { defaultValue }),
): string {
  const text = (raw instanceof Error ? raw.message : String(raw ?? '')).trim();
  if (!text) {
    return '';
  }
  const code = text.split(/[:\s]/, 1)[0];
  if (code === 'WORKBUDDY_AI_NO_FREE_MODEL') {
    return translate('workbuddyAi.checkin.errorNoFreeModel', '当前无可用 x0.00 免费模型，已中止');
  }
  if (
    code === 'WORKBUDDY_AI_TOKEN_EXPIRED' ||
    code === 'WORKBUDDY_AI_MISSING_SESSION' ||
    code === 'WORKBUDDY_AI_MISSING_ACCOUNT_ID' ||
    code === 'WORKBUDDY_AI_REFRESH_REJECTED' ||
    text.includes('WORKBUDDY_AI_REFRESH_REJECTED')
  ) {
    return translate('workbuddyAi.checkin.errorTokenExpired', '登录已过期，请重新登录');
  }
  if (
    code === 'WORKBUDDY_AI_PENDING_CONFIRMATION' ||
    text.includes('WORKBUDDY_AI_PENDING_CONFIRMATION')
  ) {
    return translate(
      'workbuddyAi.checkin.pendingConfirmation',
      '已提交活跃对话与事件，热力墙尚未同步，请稍后刷新状态确认（不会重复发送对话）。',
    );
  }
  if (code === 'WORKBUDDY_AI_ENTERPRISE_INACTIVE') {
    return translate('workbuddyAi.checkin.inactiveHint', '每日活跃活动未开启或不适用当前账号');
  }
  return text;
}
