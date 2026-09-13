import {
  WorkbuddyAccount,
  getWorkbuddyAccountDisplayEmail,
  getWorkbuddyPlanBadge,
  getWorkbuddyUsage,
} from '../types/workbuddy';
import * as workbuddyAiService from '../services/workbuddyAiService';
import { getProviderCurrentAccountId } from '../services/providerCurrentAccountService';
import { createProviderAccountStore } from './createProviderAccountStore';

const WORKBUDDY_AI_ACCOUNTS_CACHE_KEY = 'agtools.workbuddy_ai.accounts.cache';
const WORKBUDDY_AI_CURRENT_ACCOUNT_ID_KEY = 'agtools.workbuddy_ai.current_account_id';

export const useWorkbuddyAiAccountStore = createProviderAccountStore<WorkbuddyAccount>(
  WORKBUDDY_AI_ACCOUNTS_CACHE_KEY,
  {
    listAccounts: workbuddyAiService.listWorkbuddyAiAccounts,
    deleteAccount: workbuddyAiService.deleteWorkbuddyAiAccount,
    deleteAccounts: workbuddyAiService.deleteWorkbuddyAiAccounts,
    injectAccount: workbuddyAiService.injectWorkbuddyAiToVSCode,
    refreshToken: workbuddyAiService.refreshWorkbuddyAiToken,
    refreshAllTokens: workbuddyAiService.refreshAllWorkbuddyAiTokens,
    importFromJson: workbuddyAiService.importWorkbuddyAiFromJson,
    exportAccounts: workbuddyAiService.exportWorkbuddyAiAccounts,
    updateAccountTags: workbuddyAiService.updateWorkbuddyAiAccountTags,
  },
  {
    getDisplayEmail: getWorkbuddyAccountDisplayEmail,
    getPlanBadge: getWorkbuddyPlanBadge,
    getUsage: getWorkbuddyUsage,
  },
  {
    platformId: 'workbuddy_ai',
    currentAccountIdKey: WORKBUDDY_AI_CURRENT_ACCOUNT_ID_KEY,
    resolveCurrentAccountId: () => getProviderCurrentAccountId('workbuddy_ai'),
  },
);
