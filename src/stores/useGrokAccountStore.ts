import {
  GrokAccount,
  getGrokAccountDisplayEmail,
  getGrokPlanBadge,
  getGrokUsage,
} from '../types/grok';
import * as grokService from '../services/grokService';
import { getProviderCurrentAccountId } from '../services/providerCurrentAccountService';
import { createProviderAccountStore } from './createProviderAccountStore';

const GROK_ACCOUNTS_CACHE_KEY = 'agtools.grok.accounts.cache';
const GROK_CURRENT_ACCOUNT_ID_KEY = 'agtools.grok.current_account_id';

export const useGrokAccountStore = createProviderAccountStore<GrokAccount>(
  GROK_ACCOUNTS_CACHE_KEY,
  {
    listAccounts: grokService.listGrokAccounts,
    deleteAccount: grokService.deleteGrokAccount,
    deleteAccounts: grokService.deleteGrokAccounts,
    injectAccount: async (accountId: string) => {
      await grokService.switchGrokAccount(accountId);
    },
    refreshToken: grokService.refreshGrokToken,
    refreshAllTokens: grokService.refreshAllGrokTokens,
    importFromJson: grokService.importGrokFromJson,
    exportAccounts: grokService.exportGrokAccounts,
    updateAccountTags: grokService.updateGrokAccountTags,
  },
  {
    getDisplayEmail: getGrokAccountDisplayEmail,
    getPlanBadge: getGrokPlanBadge,
    getUsage: (account) => {
      const usage = getGrokUsage(account);
      return {
        inlineSuggestionsUsedPercent: usage.inlineSuggestionsUsedPercent,
        chatMessagesUsedPercent: usage.chatMessagesUsedPercent,
        allowanceResetAt: usage.allowanceResetAt,
      };
    },
  },
  {
    platformId: 'grok',
    currentAccountIdKey: GROK_CURRENT_ACCOUNT_ID_KEY,
    resolveCurrentAccountId: () => getProviderCurrentAccountId('grok'),
  },
);
