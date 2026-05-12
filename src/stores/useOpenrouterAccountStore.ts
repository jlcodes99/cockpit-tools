import {
  OpenRouterAccount,
  getOpenRouterAccountDisplayEmail,
  getOpenRouterPlanBadge,
  getOpenRouterUsage,
} from '../types/openrouter';
import * as openrouterService from '../services/openrouterService';
import { getProviderCurrentAccountId } from '../services/providerCurrentAccountService';
import { createProviderAccountStore } from './createProviderAccountStore';

const OPENROUTER_ACCOUNTS_CACHE_KEY = 'agtools.openrouter.accounts.cache';
const OPENROUTER_CURRENT_ACCOUNT_ID_KEY = 'agtools.openrouter.current_account_id';

export const useOpenrouterAccountStore = createProviderAccountStore<OpenRouterAccount>(
  OPENROUTER_ACCOUNTS_CACHE_KEY,
  {
    listAccounts: openrouterService.listOpenRouterAccounts,
    deleteAccount: openrouterService.deleteOpenRouterAccount,
    deleteAccounts: openrouterService.deleteOpenRouterAccounts,
    injectAccount: openrouterService.injectOpenRouterAccount,
    refreshToken: openrouterService.refreshOpenRouterAccount,
    refreshAllTokens: openrouterService.refreshAllOpenRouterTokens,
    importFromJson: openrouterService.importOpenRouterFromJson,
    exportAccounts: openrouterService.exportOpenRouterAccounts,
    updateAccountTags: openrouterService.updateOpenRouterAccountTags,
  },
  {
    getDisplayEmail: getOpenRouterAccountDisplayEmail,
    getPlanBadge: getOpenRouterPlanBadge,
    getUsage: getOpenRouterUsage,
  },
  {
    platformId: 'openrouter',
    currentAccountIdKey: OPENROUTER_CURRENT_ACCOUNT_ID_KEY,
    resolveCurrentAccountId: () => getProviderCurrentAccountId('openrouter'),
  },
);
