import {
  TraeAccount,
  getTraeAccountDisplayEmail,
  getTraePlanBadge,
  getTraeUsage,
} from '../types/trae';
import * as traeService from '../services/traeService';
import { createProviderAccountStore } from './createProviderAccountStore';

const TRAE_ACCOUNTS_CACHE_KEY = 'agtools.trae.accounts.cache';

export const useTraeAccountStore = createProviderAccountStore<TraeAccount>(
  TRAE_ACCOUNTS_CACHE_KEY,
  {
    listAccounts: traeService.listTraeAccounts,
    deleteAccount: traeService.deleteTraeAccount,
    deleteAccounts: traeService.deleteTraeAccounts,
    injectAccount: traeService.injectTraeAccount,
    refreshToken: traeService.refreshTraeToken,
    refreshAllTokens: traeService.refreshAllTraeTokens,
    importFromJson: traeService.importTraeFromJson,
    exportAccounts: traeService.exportTraeAccounts,
    updateAccountTags: traeService.updateTraeAccountTags,
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
    platformId: 'trae',
    resolveCurrentAccountId: traeService.getTraeCurrentAccountId,
  },
);
