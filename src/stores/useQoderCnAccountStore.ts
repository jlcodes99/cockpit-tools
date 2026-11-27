import {
  QoderAccount,
  getQoderAccountDisplayEmail,
  getQoderPlanBadge,
  getQoderUsage,
} from '../types/qoder';
import * as qoderService from '../services/qoderCnService';
import { getProviderCurrentAccountId } from '../services/providerCurrentAccountService';
import { createProviderAccountStore } from './createProviderAccountStore';

const QODER_ACCOUNTS_CACHE_KEY = 'agtools.qoder-cn.accounts.cache';
const QODER_CURRENT_ACCOUNT_ID_KEY = 'agtools.qoder-cn.current_account_id';

export const useQoderCnAccountStore = createProviderAccountStore<QoderAccount>(
  QODER_ACCOUNTS_CACHE_KEY,
  {
    listAccounts: qoderService.listQoderAccounts,
    deleteAccount: qoderService.deleteQoderAccount,
    deleteAccounts: qoderService.deleteQoderAccounts,
    injectAccount: qoderService.injectQoderAccount,
    refreshToken: qoderService.refreshQoderToken,
    refreshAllTokens: qoderService.refreshAllQoderTokens,
    importFromJson: qoderService.importQoderFromJson,
    exportAccounts: qoderService.exportQoderAccounts,
    updateAccountTags: qoderService.updateQoderAccountTags,
  },
  {
    getDisplayEmail: getQoderAccountDisplayEmail,
    getPlanBadge: getQoderPlanBadge,
    getUsage: getQoderUsage,
  },
  {
    platformId: 'qoder_cn',
    acceptEmptyCurrentAccountId: true,
    currentAccountIdKey: QODER_CURRENT_ACCOUNT_ID_KEY,
    resolveCurrentAccountId: () => getProviderCurrentAccountId('qoder_cn'),
  },
);
