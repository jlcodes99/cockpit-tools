import { create } from 'zustand';
import {
  CodexAccount,
  CodexApiProviderMode,
  CodexAppSpeed,
  CodexQuota,
  hasCodexAccountStructure,
  hasCodexAccountName,
  isCodexTeamLikePlan,
} from '../types/codex';
import * as codexService from '../services/codexService';
import { emitAccountsChanged, emitCurrentAccountChanged } from '../utils/accountSyncEvents';

const CODEX_ACCOUNTS_CACHE_KEY = 'agtools.codex.accounts.cache';
const CODEX_CURRENT_ACCOUNT_CACHE_KEY = 'agtools.codex.accounts.current';
const CODEX_PROFILE_SYNC_IN_FLIGHT = new Set<string>();
const CODEX_PROFILE_SYNC_LAST_ATTEMPT = new Map<string, number>();
const CODEX_PROFILE_SYNC_RETRY_INTERVAL_MS = 5 * 60 * 1000;
let allowNextEmptyCodexAccountList = false;
let allowNextEmptyCodexCurrentAccount = false;

const loadCachedCodexAccounts = () => {
  try {
    const raw = localStorage.getItem(CODEX_ACCOUNTS_CACHE_KEY);
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? parsed : [];
  } catch {
    return [];
  }
};

const loadCachedCodexCurrentAccount = () => {
  try {
    const raw = localStorage.getItem(CODEX_CURRENT_ACCOUNT_CACHE_KEY);
    if (!raw) return null;
    return JSON.parse(raw) as CodexAccount;
  } catch {
    return null;
  }
};

const persistCodexAccountsCache = (accounts: CodexAccount[]) => {
  try {
    localStorage.setItem(CODEX_ACCOUNTS_CACHE_KEY, JSON.stringify(accounts));
  } catch {
    // ignore cache write failures
  }
};

const persistCodexCurrentAccountCache = (account: CodexAccount | null) => {
  try {
    if (!account) {
      localStorage.removeItem(CODEX_CURRENT_ACCOUNT_CACHE_KEY);
      return;
    }
    localStorage.setItem(CODEX_CURRENT_ACCOUNT_CACHE_KEY, JSON.stringify(account));
  } catch {
    // ignore cache write failures
  }
};

const shouldHydrateCodexProfile = (account: CodexAccount): boolean =>
  !hasCodexAccountStructure(account) ||
  (isCodexTeamLikePlan(account.plan_type) && !hasCodexAccountName(account));

interface CodexAccountState {
  accounts: CodexAccount[];
  currentAccount: CodexAccount | null;
  loading: boolean;
  error: string | null;
  
  // Actions
  fetchAccounts: () => Promise<void>;
  fetchCurrentAccount: () => Promise<void>;
  switchAccount: (accountId: string) => Promise<CodexAccount>;
  deleteAccount: (accountId: string) => Promise<void>;
  deleteAccounts: (accountIds: string[]) => Promise<void>;
  refreshQuota: (accountId: string) => Promise<CodexQuota>;
  refreshAllQuotas: () => Promise<number>;
  hydrateAccountProfilesIfNeeded: (accountIds?: string[]) => Promise<void>;
  importFromLocal: () => Promise<CodexAccount>;
  importFromJson: (jsonContent: string) => Promise<CodexAccount[]>;
  updateAccountName: (accountId: string, name: string) => Promise<CodexAccount>;
  updateApiKeyCredentials: (
    accountId: string,
    apiKey: string,
    apiBaseUrl?: string,
    apiProviderMode?: CodexApiProviderMode,
    apiProviderId?: string,
    apiProviderName?: string,
  ) => Promise<CodexAccount>;
  updateApiKeyBoundOAuthAccount: (
    accountId: string,
    boundOauthAccountId: string | null,
  ) => Promise<CodexAccount>;
  updateAccountTags: (accountId: string, tags: string[]) => Promise<CodexAccount>;
  updateAccountNote: (accountId: string, note: string) => Promise<CodexAccount>;
  updateAccountAppSpeed: (accountId: string, speed: CodexAppSpeed) => Promise<CodexAccount>;
}

export const useCodexAccountStore = create<CodexAccountState>((set, get) => ({
  accounts: loadCachedCodexAccounts(),
  currentAccount: loadCachedCodexCurrentAccount(),
  loading: false,
  error: null,
  
  fetchAccounts: async () => {
    set({ loading: true, error: null });
    try {
      const accounts = await codexService.listCodexAccounts();
      if (
        accounts.length === 0 &&
        get().accounts.length > 0 &&
        !allowNextEmptyCodexAccountList
      ) {
        console.warn('[CodexAccountStore] 忽略异常空账号列表，保留本地缓存账号');
        set({ loading: false });
        return;
      }
      allowNextEmptyCodexAccountList = false;
      set({ accounts, loading: false });
      persistCodexAccountsCache(accounts);
      void get().hydrateAccountProfilesIfNeeded(accounts.map((account) => account.id));
    } catch (e) {
      set({ error: String(e), loading: false });
    }
  },
  
  fetchCurrentAccount: async () => {
    try {
      const currentAccount = await codexService.getCurrentCodexAccount();
      if (
        !currentAccount &&
        get().currentAccount &&
        get().accounts.length > 0 &&
        !allowNextEmptyCodexCurrentAccount
      ) {
        console.warn('[CodexAccountStore] 忽略异常空当前账号，保留本地缓存当前账号');
        return;
      }
      allowNextEmptyCodexCurrentAccount = false;
      set({ currentAccount });
      persistCodexCurrentAccountCache(currentAccount);
    } catch (e) {
      console.error('获取当前 Codex 账号失败:', e);
    } finally {
      allowNextEmptyCodexCurrentAccount = false;
    }
  },
  
  switchAccount: async (accountId: string) => {
    const account = await codexService.switchCodexAccount(accountId);
    set({ currentAccount: account });
    await get().fetchAccounts();
    await emitCurrentAccountChanged({
      platformId: 'codex',
      accountId: account.id,
      reason: 'switch',
    });
    return account;
  },
  
  deleteAccount: async (accountId: string) => {
    const previousCurrentAccountId = get().currentAccount?.id ?? null;
    allowNextEmptyCodexAccountList = get().accounts.length <= 1;
    allowNextEmptyCodexCurrentAccount = previousCurrentAccountId === accountId;
    try {
      await codexService.deleteCodexAccount(accountId);
      await get().fetchAccounts();
      await get().fetchCurrentAccount();
    } finally {
      allowNextEmptyCodexAccountList = false;
      allowNextEmptyCodexCurrentAccount = false;
    }
    await emitAccountsChanged({
      platformId: 'codex',
      reason: 'delete',
    });
    const nextCurrentAccountId = get().currentAccount?.id ?? null;
    if (previousCurrentAccountId !== nextCurrentAccountId) {
      await emitCurrentAccountChanged({
        platformId: 'codex',
        accountId: nextCurrentAccountId,
        reason: 'delete',
      });
    }
  },
  
  deleteAccounts: async (accountIds: string[]) => {
    const previousCurrentAccountId = get().currentAccount?.id ?? null;
    const deleteIdSet = new Set(accountIds);
    allowNextEmptyCodexAccountList = get().accounts.every((account) =>
      deleteIdSet.has(account.id),
    );
    allowNextEmptyCodexCurrentAccount = previousCurrentAccountId
      ? deleteIdSet.has(previousCurrentAccountId)
      : false;
    try {
      await codexService.deleteCodexAccounts(accountIds);
      await get().fetchAccounts();
      await get().fetchCurrentAccount();
    } finally {
      allowNextEmptyCodexAccountList = false;
      allowNextEmptyCodexCurrentAccount = false;
    }
    await emitAccountsChanged({
      platformId: 'codex',
      reason: 'delete',
    });
    const nextCurrentAccountId = get().currentAccount?.id ?? null;
    if (previousCurrentAccountId !== nextCurrentAccountId) {
      await emitCurrentAccountChanged({
        platformId: 'codex',
        accountId: nextCurrentAccountId,
        reason: 'delete',
      });
    }
  },
  
  refreshQuota: async (accountId: string) => {
    const quota = await codexService.refreshCodexQuota(accountId);
    await get().fetchAccounts();
    await get().fetchCurrentAccount();
    return quota;
  },
  
  refreshAllQuotas: async () => {
    const successCount = await codexService.refreshAllCodexQuotas();
    await get().fetchAccounts();
    await get().fetchCurrentAccount();
    return successCount;
  },

  hydrateAccountProfilesIfNeeded: async (accountIds?: string[]) => {
    const now = Date.now();
    const scope = accountIds ? new Set(accountIds) : null;
    const candidates = get().accounts.filter(
      (account) =>
        (!scope || scope.has(account.id)) &&
        shouldHydrateCodexProfile(account) &&
        !CODEX_PROFILE_SYNC_IN_FLIGHT.has(account.id) &&
        now - (CODEX_PROFILE_SYNC_LAST_ATTEMPT.get(account.id) ?? 0) >=
          CODEX_PROFILE_SYNC_RETRY_INTERVAL_MS,
    );

    for (const account of candidates) {
      CODEX_PROFILE_SYNC_IN_FLIGHT.add(account.id);
      CODEX_PROFILE_SYNC_LAST_ATTEMPT.set(account.id, now);
      try {
        const updatedAccount = await codexService.refreshCodexAccountProfile(account.id);
        set((state) => {
          const nextAccounts = state.accounts.map((item) =>
            item.id === updatedAccount.id ? { ...item, ...updatedAccount } : item,
          );
          const nextCurrentAccount =
            state.currentAccount?.id === updatedAccount.id
              ? { ...state.currentAccount, ...updatedAccount }
              : state.currentAccount;

          persistCodexAccountsCache(nextAccounts);
          persistCodexCurrentAccountCache(nextCurrentAccount);

          return {
            accounts: nextAccounts,
            currentAccount: nextCurrentAccount,
          };
        });
      } catch (e) {
        console.warn('刷新 Codex 账号资料失败:', account.id, e);
      } finally {
        CODEX_PROFILE_SYNC_IN_FLIGHT.delete(account.id);
      }
    }
  },
  
  importFromLocal: async () => {
    const account = await codexService.importCodexFromLocal();
    await get().fetchAccounts();
    await emitAccountsChanged({
      platformId: 'codex',
      reason: 'import',
    });
    return account;
  },
  
  importFromJson: async (jsonContent: string) => {
    const accounts = await codexService.importCodexFromJson(jsonContent);
    await get().fetchAccounts();
    await emitAccountsChanged({
      platformId: 'codex',
      reason: 'import',
    });
    return accounts;
  },

  updateAccountName: async (accountId: string, name: string) => {
    const account = await codexService.updateCodexAccountName(accountId, name);
    await get().fetchAccounts();
    await get().fetchCurrentAccount();
    return account;
  },

  updateApiKeyCredentials: async (
    accountId: string,
    apiKey: string,
    apiBaseUrl?: string,
    apiProviderMode?: CodexApiProviderMode,
    apiProviderId?: string,
    apiProviderName?: string,
  ) => {
    const account = await codexService.updateCodexApiKeyCredentials(
      accountId,
      apiKey,
      apiBaseUrl,
      apiProviderMode,
      apiProviderId,
      apiProviderName,
    );
    await get().fetchAccounts();
    await get().fetchCurrentAccount();
    return account;
  },

  updateApiKeyBoundOAuthAccount: async (
    accountId: string,
    boundOauthAccountId: string | null,
  ) => {
    const account = await codexService.updateCodexApiKeyBoundOAuthAccount(
      accountId,
      boundOauthAccountId,
    );
    await get().fetchAccounts();
    await get().fetchCurrentAccount();
    return account;
  },

  updateAccountTags: async (accountId: string, tags: string[]) => {
    const account = await codexService.updateCodexAccountTags(accountId, tags);
    await get().fetchAccounts();
    return account;
  },

  updateAccountNote: async (accountId: string, note: string) => {
    const account = await codexService.updateCodexAccountNote(accountId, note);
    await get().fetchAccounts();
    await get().fetchCurrentAccount();
    return account;
  },

  updateAccountAppSpeed: async (accountId: string, speed: CodexAppSpeed) => {
    const account = await codexService.updateCodexAccountAppSpeed(accountId, speed);
    await get().fetchAccounts();
    await get().fetchCurrentAccount();
    return account;
  },
}));
