import {
  getMinimaxCodeAccountDisplayEmail,
  getMinimaxCodePlanBadge,
  getMinimaxCodeUsage,
} from '../types/minimaxCode';
import type { MinimaxCodeAccount } from '../types/minimaxCode';
import * as minimaxCodeService from '../services/minimaxCodeService';
import { getProviderCurrentAccountId } from '../services/providerCurrentAccountService';
import { createProviderAccountStore } from './createProviderAccountStore';

const CACHE_KEY = 'agtools.minimaxcode.accounts.cache';
const CURRENT_ID_KEY = 'agtools.minimaxcode.current_account_id';

export const useMinimaxCodeAccountStore = createProviderAccountStore<MinimaxCodeAccount>(
  CACHE_KEY,
  {
    listAccounts: minimaxCodeService.listMinimaxCodeAccounts,
    deleteAccount: minimaxCodeService.deleteMinimaxCodeAccount,
    deleteAccounts: minimaxCodeService.deleteMinimaxCodeAccounts,
    injectAccount: minimaxCodeService.injectMinimaxCodeToClient,
    refreshToken: minimaxCodeService.refreshMinimaxCodeToken,
    refreshAllTokens: minimaxCodeService.refreshAllMinimaxCodeTokens,
    importFromJson: minimaxCodeService.importMinimaxCodeFromJson,
    exportAccounts: minimaxCodeService.exportMinimaxCodeAccounts,
    updateAccountTags: minimaxCodeService.updateMinimaxCodeAccountTags,
  },
  {
    getDisplayEmail: getMinimaxCodeAccountDisplayEmail,
    getPlanBadge: getMinimaxCodePlanBadge,
    getUsage: getMinimaxCodeUsage,
  },
  {
    platformId: 'minimaxcode',
    currentAccountIdKey: CURRENT_ID_KEY,
    resolveCurrentAccountId: () => getProviderCurrentAccountId('minimaxcode'),
  },
);
