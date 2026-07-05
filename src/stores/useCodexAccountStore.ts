import { create } from 'zustand';
import {
  CodexAccount,
  CodexAccountNoteUpdate,
  CodexApiProviderMode,
  CodexAppSpeed,
  CodexProviderWireApi,
  CodexQuota,
  hasCodexAccountStructure,
  hasCodexAccountName,
  isCodexTeamLikePlan,
  isCodexPendingOAuthAccount,
} from '../types/codex';
import * as codexService from '../services/codexService';
import { emitAccountsChanged, emitCurrentAccountChanged } from '../utils/accountSyncEvents';

const APP_PROFILE = (import.meta.env.VITE_COCKPIT_TOOLS_PROFILE || '').trim();
const STORAGE_PROFILE_SUFFIX =
  APP_PROFILE && APP_PROFILE !== 'prod' ? `.${APP_PROFILE}` : '';
const SHOULD_PRESERVE_CACHE_ON_EMPTY_LIST = !STORAGE_PROFILE_SUFFIX;
const CODEX_ACCOUNTS_CACHE_KEY = `agtools.codex.accounts.cache${STORAGE_PROFILE_SUFFIX}`;
const CODEX_CURRENT_ACCOUNT_CACHE_KEY = `agtools.codex.accounts.current${STORAGE_PROFILE_SUFFIX}`;
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

const CODEX_STALE_ACCOUNT_ERROR = 'CODEX_STALE_ACCOUNT';

const mergeCodexAccountIntoList = (
  accounts: CodexAccount[],
  account: CodexAccount,
): CodexAccount[] => {
  const index = accounts.findIndex((item) => item.id === account.id);
  if (index < 0) {
    return [account, ...accounts];
  }
  const next = [...accounts];
  next[index] = account;
  return next;
};

type FetchCodexAccountsOptions = {
  allowEmpty?: boolean;
};

type FetchCodexCurrentAccountOptions = {
  allowEmpty?: boolean;
};

interface CodexAccountState {
  accounts: CodexAccount[];
  currentAccount: CodexAccount | null;
  loading: boolean;
  error: string | null;
  
  // Actions
  fetchAccounts: (options?: FetchCodexAccountsOptions) => Promise<void>;
  fetchCurrentAccount: (options?: FetchCodexCurrentAccountOptions) => Promise<void>;
  switchAccount: (accountId: string) => Promise<CodexAccount>;
  deleteAccount: (accountId: string) => Promise<void>;
  deleteAccounts: (accountIds: string[]) => Promise<void>;
  refreshQuota: (accountId: string) => Promise<CodexQuota>;
  refreshSubscriptionInfo: (accountId: string) => Promise<CodexAccount>;
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
    apiModelCatalog?: string[],
    apiSupportsVision?: boolean,
    apiModelVisionSupport?: Record<string, boolean>,
    apiVisionRoutingModel?: string,
    apiWireApi?: CodexProviderWireApi,
  ) => Promise<CodexAccount>;
  updateApiKeyBoundOAuthAccount: (
    accountId: string,
    boundOauthAccountId: string | null,
    boundOauthUseLocalGateway?: boolean,
  ) => Promise<CodexAccount>;
  updateAccountTags: (accountId: string, tags: string[]) => Promise<CodexAccount>;
  updateAccountNote: (accountId: string, update: string | CodexAccountNoteUpdate) => Promise<CodexAccount>;
  updateAccountAppSpeed: (accountId: string, speed: CodexAppSpeed) => Promise<CodexAccount>;
}

export const useCodexAccountStore = create<CodexAccountState>((set, get) => ({
  accounts: loadCachedCodexAccounts(),
  currentAccount: loadCachedCodexCurrentAccount(),
  loading: false,
  error: null,
  
  fetchAccounts: async (options?: FetchCodexAccountsOptions) => {
    if (options?.allowEmpty) {
      allowNextEmptyCodexAccountList = true;
    }
    set({ loading: true, error: null });
    try {
      const accounts = await codexService.listCodexAccounts();
      if (
        SHOULD_PRESERVE_CACHE_ON_EMPTY_LIST &&
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
  
  fetchCurrentAccount: async (options?: FetchCodexCurrentAccountOptions) => {
    if (options?.allowEmpty) {
      allowNextEmptyCodexCurrentAccount = true;
    }
    try {
      const currentAccount = await codexService.getCurrentCodexAccount();
      if (
        SHOULD_PRESERVE_CACHE_ON_EMPTY_LIST &&
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
    const flowStartedAt = performance.now();
    console.info('[Codex Switch][Store] switchAccount started', {
      accountId,
    });
    const accounts = await codexService.listCodexAccounts();
    console.info('[Codex Switch][Store] listCodexAccounts finished', {
      accountId,
      elapsedMs: Math.round(performance.now() - flowStartedAt),
    });
    allowNextEmptyCodexAccountList = false;
    set({ accounts, loading: false, error: null });
    persistCodexAccountsCache(accounts);

    const targetExists = accounts.some((account) => account.id === accountId);
    if (!targetExists) {
      const currentAccount = await codexService.getCurrentCodexAccount();
      allowNextEmptyCodexCurrentAccount = false;
      set({ currentAccount });
      persistCodexCurrentAccountCache(currentAccount);
      throw new Error(CODEX_STALE_ACCOUNT_ERROR);
    }

    const account = await codexService.switchCodexAccount(accountId);
    console.info('[Codex Switch][Store] switchCodexAccount finished', {
      accountId,
      elapsedMs: Math.round(performance.now() - flowStartedAt),
    });
    set((state) => {
      const nextAccounts = mergeCodexAccountIntoList(state.accounts, account);
      persistCodexAccountsCache(nextAccounts);
      persistCodexCurrentAccountCache(account);
      return {
        accounts: nextAccounts,
        currentAccount: account,
        loading: false,
        error: null,
      };
    });
    void get()
      .fetchAccounts()
      .then(() => {
        console.info('[Codex Switch][Store] background fetchAccounts after switch finished', {
          accountId,
          elapsedMs: Math.round(performance.now() - flowStartedAt),
        });
      });
    await emitCurrentAccountChanged({
      platformId: 'codex',
      accountId: account.id,
      reason: 'switch',
    });
    console.info('[Codex Switch][Store] switchAccount finished', {
      accountId,
      elapsedMs: Math.round(performance.now() - flowStartedAt),
    });
    return account;
  },
  
  deleteAccount: async (accountId: string) => {
    const previousCurrentAccountId = get().currentAccount?.id ?? null;
    await codexService.deleteCodexAccount(accountId);
    set((state) => {
      const nextAccounts = state.accounts.filter((account) => account.id !== accountId);
      const nextCurrentAccount =
        state.currentAccount?.id === accountId ? null : state.currentAccount;
      persistCodexAccountsCache(nextAccounts);
      persistCodexCurrentAccountCache(nextCurrentAccount);
      return {
        accounts: nextAccounts,
        currentAccount: nextCurrentAccount,
        loading: false,
        error: null,
      };
    });
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
    await codexService.deleteCodexAccounts(accountIds);
    set((state) => {
      const nextAccounts = state.accounts.filter((account) => !deleteIdSet.has(account.id));
      const nextCurrentAccount =
        state.currentAccount && deleteIdSet.has(state.currentAccount.id)
          ? null
          : state.currentAccount;
      persistCodexAccountsCache(nextAccounts);
      persistCodexCurrentAccountCache(nextCurrentAccount);
      return {
        accounts: nextAccounts,
        currentAccount: nextCurrentAccount,
        loading: false,
        error: null,
      };
    });
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
    const account = get().accounts.find((item) => item.id === accountId);
    if (account && isCodexPendingOAuthAccount(account)) {
      throw new Error('CODEX_PENDING_OAUTH_ACCOUNT');
    }
    try {
      return await codexService.refreshCodexQuota(accountId);
    } finally {
      await get().fetchAccounts();
      await get().fetchCurrentAccount();
    }
  },

  refreshSubscriptionInfo: async (accountId: string) => {
    const account = await codexService.refreshCodexSubscriptionInfo(accountId);
    await get().fetchAccounts();
    await get().fetchCurrentAccount();
    return account;
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
    apiModelCatalog?: string[],
    apiSupportsVision?: boolean,
    apiModelVisionSupport?: Record<string, boolean>,
    apiVisionRoutingModel?: string,
    apiWireApi?: CodexProviderWireApi,
  ) => {
    const account = await codexService.updateCodexApiKeyCredentials(
      accountId,
      apiKey,
      apiBaseUrl,
      apiProviderMode,
      apiProviderId,
      apiProviderName,
      apiModelCatalog,
      apiSupportsVision,
      apiModelVisionSupport,
      apiVisionRoutingModel,
      apiWireApi,
    );
    await get().fetchAccounts();
    await get().fetchCurrentAccount();
    return account;
  },

  updateApiKeyBoundOAuthAccount: async (
    accountId: string,
    boundOauthAccountId: string | null,
    boundOauthUseLocalGateway = false,
  ) => {
    const account = await codexService.updateCodexApiKeyBoundOAuthAccount(
      accountId,
      boundOauthAccountId,
      boundOauthUseLocalGateway,
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

  updateAccountNote: async (accountId: string, update: string | CodexAccountNoteUpdate) => {
    const account = await codexService.updateCodexAccountNote(accountId, update);
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
