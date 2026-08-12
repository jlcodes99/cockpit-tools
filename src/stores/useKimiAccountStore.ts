import { createProviderAccountStore } from './createProviderAccountStore';
import * as kimiService from '../services/kimiService';
import {
  getKimiAccountDisplayEmail,
  getKimiPlanBadge,
  getKimiUsage,
  type KimiAccount,
} from '../types/kimi';

export const useKimiAccountStore = createProviderAccountStore<KimiAccount>(
  'agtools.kimi.accounts.cache',
  {
    listAccounts: kimiService.listKimiAccounts,
    deleteAccount: kimiService.deleteKimiAccount,
    deleteAccounts: kimiService.deleteKimiAccounts,
    injectAccount: kimiService.switchKimiAccount,
    refreshToken: kimiService.refreshKimiAccount,
    refreshAllTokens: kimiService.refreshAllKimiAccounts,
    importFromJson: kimiService.importKimiFromJson,
    exportAccounts: kimiService.exportKimiAccounts,
    updateAccountTags: kimiService.updateKimiAccountTags,
  },
  {
    getDisplayEmail: getKimiAccountDisplayEmail,
    getPlanBadge: getKimiPlanBadge,
    getUsage: getKimiUsage,
  },
  {
    platformId: 'kimi',
    currentAccountIdKey: 'agtools.kimi.current_account_id',
    resolveCurrentAccountId: kimiService.getKimiCurrentAccountId,
    acceptEmptyCurrentAccountId: true,
    preserveSourceQuota: true,
  },
);
