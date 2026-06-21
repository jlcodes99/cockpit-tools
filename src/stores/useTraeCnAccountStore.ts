import {
  TraeAccount,
  getTraeAccountDisplayEmail,
  getTraePlanBadge,
  getTraeUsage,
} from '../types/trae';
import * as traeCnService from '../services/traeCnService';
import { getProviderCurrentAccountId } from '../services/providerCurrentAccountService';
import { createProviderAccountStore } from './createProviderAccountStore';

const TRAE_CN_ACCOUNTS_CACHE_KEY = 'agtools.trae_cn.accounts.cache';
const TRAE_CN_CURRENT_ACCOUNT_ID_KEY = 'agtools.trae_cn.current_account_id';

export const useTraeCnAccountStore = createProviderAccountStore<TraeAccount>(
  TRAE_CN_ACCOUNTS_CACHE_KEY,
  {
    listAccounts: traeCnService.listTraeCnAccounts,
    deleteAccount: traeCnService.deleteTraeCnAccount,
    deleteAccounts: traeCnService.deleteTraeCnAccounts,
    injectAccount: traeCnService.injectTraeCnAccount,
    refreshToken: traeCnService.refreshTraeCnToken,
    refreshAllTokens: traeCnService.refreshAllTraeCnTokens,
    importFromJson: traeCnService.importTraeCnFromJson,
    exportAccounts: traeCnService.exportTraeCnAccounts,
    updateAccountTags: traeCnService.updateTraeCnAccountTags,
  },
  {
    getDisplayEmail: getTraeAccountDisplayEmail,
    getPlanBadge: getTraePlanBadge,
    getUsage: (account) => {
      const usage = getTraeUsage(account);
      return {
        inlineSuggestionsUsedPercent: usage.usedPercent,
        chatMessagesUsedPercent: usage.usedPercent,
        allowanceResetAt: usage.resetAt,
      };
    },
  },
  {
    platformId: 'trae_cn',
    currentAccountIdKey: TRAE_CN_CURRENT_ACCOUNT_ID_KEY,
    resolveCurrentAccountId: () => getProviderCurrentAccountId('trae_cn'),
  },
);
