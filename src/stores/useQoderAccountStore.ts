import {
  QoderAccount,
  QoderChannel,
  getQoderAccountDisplayEmail,
  getQoderPlanBadge,
  getQoderUsage,
} from '../types/qoder';
import * as qoderService from '../services/qoderService';
import { getProviderCurrentAccountId } from '../services/providerCurrentAccountService';
import { createProviderAccountStore } from './createProviderAccountStore';

function createChannelStore(channel: QoderChannel) {
  const cacheKey = `agtools.${channel}.accounts.cache`;
  const currentKey = `agtools.${channel}.current_account_id`;

  return createProviderAccountStore<QoderAccount>(
    cacheKey,
    {
      listAccounts: () => qoderService.listQoderChannelAccounts(channel),
      deleteAccount: (accountId: string) => qoderService.deleteQoderChannelAccount(channel, accountId),
      deleteAccounts: (accountIds: string[]) => qoderService.deleteQoderChannelAccounts(channel, accountIds),
      injectAccount: (accountId: string) => qoderService.injectQoderChannelAccount(channel, accountId),
      refreshToken: (accountId: string) => qoderService.refreshQoderToken(accountId, channel),
      refreshAllTokens: qoderService.refreshAllQoderTokens,
      importFromJson: (jsonContent: string) => qoderService.importQoderChannelFromJson(channel, jsonContent),
      exportAccounts: (accountIds: string[]) => qoderService.exportQoderChannelAccounts(channel, accountIds),
      updateAccountTags: (accountId: string, tags: string[]) =>
        qoderService.updateQoderChannelAccountTags(channel, accountId, tags),
    },
    {
      getDisplayEmail: getQoderAccountDisplayEmail,
      getPlanBadge: getQoderPlanBadge,
      getUsage: getQoderUsage,
    },
    {
      platformId: channel,
      currentAccountIdKey: currentKey,
      resolveCurrentAccountId: () => getProviderCurrentAccountId(channel),
    },
  );
}

export const qoderChannelStores: Record<QoderChannel, ReturnType<typeof createChannelStore>> = {
  qoder: createChannelStore('qoder'),
  qoder_app: createChannelStore('qoder_app'),
  qoder_cn_ide: createChannelStore('qoder_cn_ide'),
  qoder_cn_app: createChannelStore('qoder_cn_app'),
};

export const useQoderAccountStore = qoderChannelStores.qoder;
export const useQoderAppAccountStore = qoderChannelStores.qoder_app;
export const useQoderCnIdeAccountStore = qoderChannelStores.qoder_cn_ide;
export const useQoderCnAppAccountStore = qoderChannelStores.qoder_cn_app;

export function useQoderChannelAccountStore(channel: QoderChannel = 'qoder') {
  const store = qoderChannelStores[channel] || qoderChannelStores.qoder;
  return store();
}
