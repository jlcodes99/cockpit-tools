import { createProviderAccountStore } from './createProviderAccountStore';
import * as piService from '../services/piService';
import {
  getPiAccountDisplayEmail,
  getPiPlanBadge,
  getPiUsage,
  type PiAccount,
} from '../types/pi';

export const usePiAccountStore = createProviderAccountStore<PiAccount>(
  'agtools.pi.accounts.cache',
  {
    listAccounts: piService.listPiAccounts,
    deleteAccount: piService.deletePiAccount,
    deleteAccounts: piService.deletePiAccounts,
    injectAccount: piService.switchPiAccount,
    refreshToken: piService.refreshPiAccount,
    refreshAllTokens: piService.refreshAllPiAccounts,
    importFromJson: piService.importPiFromJson,
    exportAccounts: piService.exportPiAccounts,
    updateAccountTags: piService.updatePiAccountTags,
  },
  {
    getDisplayEmail: getPiAccountDisplayEmail,
    getPlanBadge: getPiPlanBadge,
    getUsage: getPiUsage,
  },
  {
    platformId: 'pi',
    // Backend returns the current account only when sync-on-switch is enabled.
    currentAccountIdKey: 'agtools.pi.current_account_id',
    resolveCurrentAccountId: piService.getPiCurrentAccountId,
    acceptEmptyCurrentAccountId: true,
    preserveSourceQuota: true,
  },
);

if (typeof window !== 'undefined') {
  window.addEventListener('config-updated', () => {
    void usePiAccountStore.getState().fetchCurrentAccountId();
  });
}
