import {
  QoderAccount,
  getQoderAccountDisplayEmail,
  getQoderPlanBadge,
  getQoderUsage,
} from '../types/qoder';
import * as qoderService from '../services/qoderService';
import { getProviderCurrentAccountId } from '../services/providerCurrentAccountService';
import { createProviderAccountStore } from './createProviderAccountStore';

const QODER_ACCOUNTS_CACHE_KEY = 'agtools.qoder.accounts.cache';
const QODER_CURRENT_ACCOUNT_ID_KEY = 'agtools.qoder.current_account_id';

export const useQoderAccountStore = createProviderAccountStore<QoderAccount>(
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
    platformId: 'qoder',
    acceptEmptyCurrentAccountId: true,
    currentAccountIdKey: QODER_CURRENT_ACCOUNT_ID_KEY,
    resolveCurrentAccountId: () => getProviderCurrentAccountId('qoder'),
  },
);
