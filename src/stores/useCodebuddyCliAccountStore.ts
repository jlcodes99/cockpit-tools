import {
  WorkbuddyAccount,
  getWorkbuddyAccountDisplayEmail,
  getWorkbuddyPlanBadge,
  getWorkbuddyUsage,
} from '../types/workbuddy';
import * as workbuddyService from '../services/workbuddyService';
import { injectWorkbuddyToCodebuddyCli } from '../services/codebuddyCliService';
import { getProviderCurrentAccountId } from '../services/providerCurrentAccountService';
import { createProviderAccountStore } from './createProviderAccountStore';

const CODEBUDDY_CLI_ACCOUNTS_CACHE_KEY = 'agtools.codebuddy_cli.accounts.cache';
const CODEBUDDY_CLI_CURRENT_ACCOUNT_ID_KEY = 'agtools.codebuddy_cli.current_account_id';

/**
 * CodeBuddy CLI 平台 store：账号数据复用 WorkBuddy 账号库，
 * 仅切号命令与平台标识不同（写入 CLI 官方认证文件）。
 */
export const useCodebuddyCliAccountStore = createProviderAccountStore<WorkbuddyAccount>(
  CODEBUDDY_CLI_ACCOUNTS_CACHE_KEY,
  {
    listAccounts: workbuddyService.listWorkbuddyAccounts,
    deleteAccount: workbuddyService.deleteWorkbuddyAccount,
    deleteAccounts: workbuddyService.deleteWorkbuddyAccounts,
    injectAccount: injectWorkbuddyToCodebuddyCli,
    refreshToken: workbuddyService.refreshWorkbuddyToken,
    refreshAllTokens: workbuddyService.refreshAllWorkbuddyTokens,
    importFromJson: workbuddyService.importWorkbuddyFromJson,
    exportAccounts: workbuddyService.exportWorkbuddyAccounts,
    updateAccountTags: workbuddyService.updateWorkbuddyAccountTags,
  },
  {
    getDisplayEmail: getWorkbuddyAccountDisplayEmail,
    getPlanBadge: getWorkbuddyPlanBadge,
    getUsage: getWorkbuddyUsage,
  },
  {
    platformId: 'codebuddy_cli',
    currentAccountIdKey: CODEBUDDY_CLI_CURRENT_ACCOUNT_ID_KEY,
    resolveCurrentAccountId: () => getProviderCurrentAccountId('codebuddy_cli'),
  },
);
