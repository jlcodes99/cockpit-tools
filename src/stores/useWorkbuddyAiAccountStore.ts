import * as service from '../services/workbuddyAiService';
import { getProviderCurrentAccountId } from '../services/providerCurrentAccountService';
import {
  type WorkbuddyAccount,
  getWorkbuddyAccountDisplayEmail,
  getWorkbuddyAiPlanBadge,
  getWorkbuddyUsage,
} from '../types/workbuddy';
import { createProviderAccountStore } from './createProviderAccountStore';

export const useWorkbuddyAiAccountStore = createProviderAccountStore<WorkbuddyAccount>(
  'agtools.workbuddy_ai.accounts.cache',
  { ...service, injectAccount: service.injectToVSCode },
  {
    getDisplayEmail: getWorkbuddyAccountDisplayEmail,
    getPlanBadge: getWorkbuddyAiPlanBadge,
    getUsage: getWorkbuddyUsage,
  },
  {
    platformId: 'workbuddy_ai',
    acceptEmptyCurrentAccountId: true,
    currentAccountIdKey: 'agtools.workbuddy_ai.current_account_id',
    resolveCurrentAccountId: () => getProviderCurrentAccountId('workbuddy_ai'),
  },
);
