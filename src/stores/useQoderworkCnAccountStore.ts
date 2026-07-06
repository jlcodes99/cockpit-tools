import {
  QoderworkCnAccount,
  getQoderworkCnAccountDisplayEmail,
  getQoderworkCnPlanBadge,
  getQoderworkCnUsage,
} from '../types/qoderworkCn';
import * as qoderworkCnService from '../services/qoderworkCnService';
import { getProviderCurrentAccountId } from '../services/providerCurrentAccountService';
import { createProviderAccountStore } from './createProviderAccountStore';

const QODERWORK_CN_ACCOUNTS_CACHE_KEY = 'agtools.qoderwork_cn.accounts.cache';
const QODERWORK_CN_CURRENT_ACCOUNT_ID_KEY = 'agtools.qoderwork_cn.current_account_id';

export const useQoderworkCnAccountStore = createProviderAccountStore<QoderworkCnAccount>(
  QODERWORK_CN_ACCOUNTS_CACHE_KEY,
  {
    listAccounts: qoderworkCnService.listQoderworkCnAccounts,
    deleteAccount: qoderworkCnService.deleteQoderworkCnAccount,
    deleteAccounts: qoderworkCnService.deleteQoderworkCnAccounts,
    injectAccount: qoderworkCnService.switchQoderworkCnAccount,
    refreshToken: qoderworkCnService.refreshQoderworkCnToken,
    refreshAllTokens: qoderworkCnService.refreshAllQoderworkCnTokens,
    importFromJson: qoderworkCnService.importQoderworkCnFromJson,
    exportAccounts: qoderworkCnService.exportQoderworkCnAccounts,
    updateAccountTags: qoderworkCnService.updateQoderworkCnAccountTags,
  },
  {
    getDisplayEmail: getQoderworkCnAccountDisplayEmail,
    getPlanBadge: getQoderworkCnPlanBadge,
    getUsage: (account: QoderworkCnAccount) => {
      const usage = getQoderworkCnUsage(account);
      return {
        inlineSuggestionsUsedPercent: usage.usagePercent,
        chatMessagesUsedPercent: null,
        remainingCompletions: usage.creditsRemaining,
        totalCompletions: usage.creditsTotal,
      };
    },
  },
  {
    platformId: 'qoderwork_cn',
    currentAccountIdKey: QODERWORK_CN_CURRENT_ACCOUNT_ID_KEY,
    resolveCurrentAccountId: () => getProviderCurrentAccountId('qoderwork_cn'),
  },
);
