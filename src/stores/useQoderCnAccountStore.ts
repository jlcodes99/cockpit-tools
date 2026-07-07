import {
  QoderCnAccount,
  getQoderCnAccountDisplayEmail,
  getQoderCnPlanBadge,
  getQoderCnUsage,
} from '../types/qoderCn';
import * as qoderCnService from '../services/qoderCnService';
import { getProviderCurrentAccountId } from '../services/providerCurrentAccountService';
import { createProviderAccountStore } from './createProviderAccountStore';

const QODER_CN_ACCOUNTS_CACHE_KEY = 'agtools.qoder_cn.accounts.cache';
const QODER_CN_CURRENT_ACCOUNT_ID_KEY = 'agtools.qoder_cn.current_account_id';

export const useQoderCnAccountStore = createProviderAccountStore<QoderCnAccount>(
  QODER_CN_ACCOUNTS_CACHE_KEY,
  {
    listAccounts: qoderCnService.listQoderCnAccounts,
    deleteAccount: qoderCnService.deleteQoderCnAccount,
    deleteAccounts: qoderCnService.deleteQoderCnAccounts,
    injectAccount: qoderCnService.switchQoderCnAccount,
    refreshToken: qoderCnService.refreshQoderCnToken,
    refreshAllTokens: qoderCnService.refreshAllQoderCnTokens,
    importFromJson: qoderCnService.importQoderCnFromJson,
    exportAccounts: qoderCnService.exportQoderCnAccounts,
    updateAccountTags: qoderCnService.updateQoderCnAccountTags,
  },
  {
    getDisplayEmail: getQoderCnAccountDisplayEmail,
    getPlanBadge: getQoderCnPlanBadge,
    getUsage: (account: QoderCnAccount) => {
      const usage = getQoderCnUsage(account);
      return {
        inlineSuggestionsUsedPercent: usage.usagePercent,
        chatMessagesUsedPercent: null,
        remainingCompletions: usage.creditsRemaining,
        totalCompletions: usage.creditsTotal,
      };
    },
  },
  {
    platformId: 'qoder_cn',
    currentAccountIdKey: QODER_CN_CURRENT_ACCOUNT_ID_KEY,
    resolveCurrentAccountId: () => getProviderCurrentAccountId('qoder_cn'),
  },
);
