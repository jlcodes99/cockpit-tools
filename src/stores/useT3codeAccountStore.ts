import { createProviderAccountStore } from './createProviderAccountStore';
import * as t3codeService from '../services/t3codeService';
import {
  getT3codeAccountDisplayEmail,
  getT3codeUsage,
  type T3CodeAccount,
} from '../types/t3code';

export const useT3codeAccountStore = createProviderAccountStore<T3CodeAccount>(
  'agtools.t3code.accounts.cache',
  {
    listAccounts: t3codeService.listT3codeAccounts,
    deleteAccount: t3codeService.deleteT3codeAccount,
    deleteAccounts: t3codeService.deleteT3codeAccounts,
    injectAccount: t3codeService.injectT3codeAccount,
    refreshToken: t3codeService.refreshT3codeAccount,
    refreshAllTokens: t3codeService.refreshAllT3codeAccounts,
    importFromJson: t3codeService.importT3codeFromJson,
    exportAccounts: t3codeService.exportT3codeAccounts,
    updateAccountTags: t3codeService.updateT3codeAccountTags,
  },
  {
    getDisplayEmail: getT3codeAccountDisplayEmail,
    getPlanBadge: (account) => account.plan_type ?? '',
    getUsage: getT3codeUsage,
  },
  {
    platformId: 't3code',
    currentAccountIdKey: 'agtools.t3code.current_account_id',
    acceptEmptyCurrentAccountId: true,
    resolveCurrentAccountId: t3codeService.getT3codeCurrentAccountId,
  },
);
