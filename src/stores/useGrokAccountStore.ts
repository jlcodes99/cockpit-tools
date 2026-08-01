import { createProviderAccountStore } from './createProviderAccountStore';
import * as grokService from '../services/grokService';
import {
  getGrokAccountDisplayEmail,
  getGrokPlanBadge,
  getGrokUsage,
  type GrokAccount,
} from '../types/grok';

export const useGrokAccountStore = createProviderAccountStore<GrokAccount>(
  'agtools.grok.accounts.cache',
  {
    listAccounts: grokService.listGrokAccounts,
    deleteAccount: grokService.deleteGrokAccount,
    deleteAccounts: grokService.deleteGrokAccounts,
    injectAccount: grokService.switchGrokAccount,
    refreshToken: grokService.refreshGrokAccount,
    refreshAllTokens: grokService.refreshAllGrokAccounts,
    importFromJson: grokService.importGrokFromJson,
    exportAccounts: grokService.exportGrokAccounts,
    updateAccountTags: grokService.updateGrokAccountTags,
  },
  {
    getDisplayEmail: getGrokAccountDisplayEmail,
    getPlanBadge: getGrokPlanBadge,
    getUsage: getGrokUsage,
  },
  {
    platformId: 'grok',
    // 当前账号独立于是否同步官方登录；关闭同步时指向下次默认实例启动使用的账号。
    currentAccountIdKey: 'agtools.grok.current_account_id',
    resolveCurrentAccountId: grokService.getGrokCurrentAccountId,
    acceptEmptyCurrentAccountId: true,
    preserveSourceQuota: true,
  },
);

// 开关「切号同步官方登录」变化后立即按官方或独立目录模式重算当前账号。
if (typeof window !== 'undefined') {
  window.addEventListener('config-updated', () => {
    void useGrokAccountStore.getState().fetchCurrentAccountId();
  });
}
