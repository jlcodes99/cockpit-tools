import {
  OpenCodeAccount,
  getOpenCodeAccountDisplayEmail,
  getOpenCodePlanBadge,
  getOpenCodeGoUsage,
} from '../types/opencode';
import * as opencodeService from '../services/opencodeService';
import { getProviderCurrentAccountId } from '../services/providerCurrentAccountService';
import { createProviderAccountStore } from './createProviderAccountStore';

const OPENCODE_ACCOUNTS_CACHE_KEY = 'agtools.opencode.accounts.cache';
const OPENCODE_CURRENT_ACCOUNT_ID_KEY = 'agtools.opencode.current_account_id';

export const useOpencodeAccountStore = createProviderAccountStore<OpenCodeAccount>(
  OPENCODE_ACCOUNTS_CACHE_KEY,
  {
    listAccounts: opencodeService.listOpenCodeAccounts,
    deleteAccount: opencodeService.deleteOpenCodeAccount,
    deleteAccounts: opencodeService.deleteOpenCodeAccounts,
    injectAccount: opencodeService.injectOpenCodeAccount,
    refreshToken: opencodeService.refreshOpenCodeAccount,
    refreshAllTokens: opencodeService.refreshAllOpenCodeTokens,
    importFromJson: opencodeService.importOpenCodeFromJson,
    exportAccounts: opencodeService.exportOpenCodeAccounts,
    updateAccountTags: opencodeService.updateOpenCodeAccountTags,
  },
  {
    getDisplayEmail: getOpenCodeAccountDisplayEmail,
    getPlanBadge: getOpenCodePlanBadge,
    getUsage: (account) => {
      if (account.tier === 'go') {
        const usage = getOpenCodeGoUsage(account);
        return {
          inlineSuggestionsUsedPercent: usage.totalPercentUsed,
          chatMessagesUsedPercent: usage.totalPercentUsed,
        };
      }
      return {
        inlineSuggestionsUsedPercent: null,
        chatMessagesUsedPercent: null,
      };
    },
  },
  {
    platformId: 'opencode',
    currentAccountIdKey: OPENCODE_CURRENT_ACCOUNT_ID_KEY,
    resolveCurrentAccountId: () => getProviderCurrentAccountId('opencode'),
  },
);
