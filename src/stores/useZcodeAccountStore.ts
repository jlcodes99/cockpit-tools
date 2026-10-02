import {
  ZcodeAccount,
  getZcodeAccountDisplayName,
  getZcodePlanBadge,
  getZcodeUsage,
} from '../types/zcode';
import * as zcodeService from '../services/zcodeService';
import { createProviderAccountStore } from './createProviderAccountStore';

const ZCODE_ACCOUNTS_CACHE_KEY = 'agtools.zcode.accounts.cache';

export const useZcodeAccountStore = createProviderAccountStore<ZcodeAccount>(
  ZCODE_ACCOUNTS_CACHE_KEY,
  {
    listAccounts: zcodeService.listZcodeAccounts,
    deleteAccount: zcodeService.deleteZcodeAccount,
    deleteAccounts: zcodeService.deleteZcodeAccounts,
    injectAccount: zcodeService.injectZcodeAccount,
    refreshToken: zcodeService.refreshZcodeQuota,
    refreshAllTokens: zcodeService.refreshAllZcodeQuotas,
    importFromJson: zcodeService.importZcodeFromJson,
    exportAccounts: zcodeService.exportZcodeAccounts,
    updateAccountTags: zcodeService.updateZcodeAccountTags,
  },
  {
    getDisplayEmail: getZcodeAccountDisplayName,
    getPlanBadge: getZcodePlanBadge,
    getUsage: getZcodeUsage,
  },
  {
    platformId: 'zcode',
  },
);
