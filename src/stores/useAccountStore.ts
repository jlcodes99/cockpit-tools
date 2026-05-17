import { create } from 'zustand';
import { persist, createJSONStorage } from 'zustand/middleware';
import { Account, QuotaData, RefreshStats, TokenData } from '../types/account';
import * as accountService from '../services/accountService';
import { emitAccountsChanged, emitCurrentAccountChanged } from '../utils/accountSyncEvents';

const ACCOUNTS_STORE_KEY = 'agtools.accounts.store.v1';
const LEGACY_ACCOUNTS_CACHE_KEY = 'agtools.accounts.cache';
const LEGACY_CURRENT_ACCOUNT_CACHE_KEY = 'agtools.accounts.current';

let accountStoreQuotaCleanupScheduled = false;
let accountStoreQuotaWarned = false;

function isQuotaExceededError(error: unknown): boolean {
  if (error instanceof DOMException) {
    return (
      error.name === 'QuotaExceededError' ||
      error.code === 22 ||
      error.code === 1014
    );
  }
  const message = String(error);
  return message.includes('QuotaExceededError') || message.includes('quota');
}

function scheduleAccountStoreQuotaRecovery(storageKey: string) {
  if (typeof window === 'undefined' || accountStoreQuotaCleanupScheduled) return;
  accountStoreQuotaCleanupScheduled = true;
  setTimeout(() => {
    try {
      localStorage.removeItem(storageKey);
      localStorage.removeItem(LEGACY_ACCOUNTS_CACHE_KEY);
      localStorage.removeItem(LEGACY_CURRENT_ACCOUNT_CACHE_KEY);
    } catch (error) {
      console.warn('[AccountStore] 清理超限缓存失败:', error);
    } finally {
      accountStoreQuotaCleanupScheduled = false;
    }
  }, 0);
}

const accountStoreStorage = createJSONStorage(() => ({
  getItem: (name: string) => {
    try {
      return localStorage.getItem(name);
    } catch (error) {
      console.warn(`[AccountStore] 读取持久化数据失败: ${name}`, error);
      return null;
    }
  },
  setItem: (name: string, value: string) => {
    try {
      localStorage.setItem(name, value);
    } catch (error) {
      if (isQuotaExceededError(error)) {
        if (!accountStoreQuotaWarned) {
          console.warn(
            '[AccountStore] 本地缓存空间不足，已自动清理账号缓存并回退为仅内存态。',
            error
          );
          accountStoreQuotaWarned = true;
        }
        scheduleAccountStoreQuotaRecovery(name);
        return;
      }
      console.warn(`[AccountStore] 写入持久化数据失败: ${name}`, error);
    }
  },
  removeItem: (name: string) => {
    try {
      localStorage.removeItem(name);
    } catch (error) {
      console.warn(`[AccountStore] 删除持久化数据失败: ${name}`, error);
    }
  },
}));

function toPersistedTokenSnapshot(token: TokenData): TokenData {
  return {
    access_token: '',
    refresh_token: '',
    expires_in: 0,
    expiry_timestamp: 0,
    token_type: token.token_type || 'Bearer',
    email: token.email,
    project_id: token.project_id,
    is_gcp_tos: token.is_gcp_tos,
    session_id: token.session_id,
  };
}

function toPersistedQuotaSnapshot(quota?: QuotaData): QuotaData | undefined {
  if (!quota) return undefined;
  return {
    models: [],
    last_updated: quota.last_updated ?? 0,
    is_forbidden: quota.is_forbidden,
    subscription_tier: quota.subscription_tier,
    tier_id: quota.tier_id,
  };
}

function toPersistedAccountSnapshot(account: Account): Account {
  return {
    ...account,
    token: toPersistedTokenSnapshot(account.token),
    quota: toPersistedQuotaSnapshot(account.quota),
  };
}

// 防抖状态（在 store 外部维护，避免触发 re-render）
let fetchAccountsPromise: Promise<void> | null = null;
let fetchAccountsLastTime = 0;
let fetchCurrentPromise: Promise<void> | null = null;
let fetchCurrentLastTime = 0;
let allowNextEmptyAccountList = false;
let allowNextEmptyCurrentAccount = false;
const DEBOUNCE_MS = 500;

interface AccountState {
    accounts: Account[];
    currentAccount: Account | null;
    loading: boolean;
    error: string | null;
    fetchAccounts: () => Promise<void>;
    fetchCurrentAccount: () => Promise<void>;
    addAccount: (email: string, refreshToken: string) => Promise<Account>;
    deleteAccount: (accountId: string) => Promise<void>;
    deleteAccounts: (accountIds: string[]) => Promise<void>;
    setCurrentAccount: (accountId: string) => Promise<void>;
    refreshQuota: (accountId: string) => Promise<void>;
    refreshAllQuotas: () => Promise<RefreshStats>;
    startOAuthLogin: () => Promise<Account>;
    reorderAccounts: (accountIds: string[]) => Promise<void>;
    switchAccount: (accountId: string) => Promise<Account>;
    syncCurrentFromClient: () => Promise<void>;
    updateAccountTags: (accountId: string, tags: string[]) => Promise<Account>;
}

export const useAccountStore = create<AccountState>()(
  persist(
    (set, get) => ({
      accounts: [],
      currentAccount: null,
      loading: false,
      error: null,

      fetchAccounts: async () => {
          const now = Date.now();
          
          // 如果正在请求中，且距离上次请求不足 DEBOUNCE_MS，复用现有 Promise
          if (fetchAccountsPromise && now - fetchAccountsLastTime < DEBOUNCE_MS) {
              return fetchAccountsPromise;
          }
          
          fetchAccountsLastTime = now;
          
          fetchAccountsPromise = (async () => {
              set({ loading: true, error: null });
              try {
                  const accounts = await accountService.listAccounts();
                  if (accounts.length === 0 && get().accounts.length > 0 && !allowNextEmptyAccountList) {
                      console.warn('[AccountStore] 忽略异常空账号列表，保留本地缓存账号');
                      set({ loading: false });
                      return;
                  }
                  allowNextEmptyAccountList = false;
                  set({ accounts, loading: false });
              } catch (e) {
                  set({ error: String(e), loading: false });
              } finally {
                  allowNextEmptyAccountList = false;
                  // 请求完成后延迟清除 Promise，允许短时间内的后续调用也复用结果
                  setTimeout(() => {
                      fetchAccountsPromise = null;
                  }, 100);
              }
          })();
          
          return fetchAccountsPromise;
      },

      fetchCurrentAccount: async () => {
          const now = Date.now();
          
          // 防抖：复用正在进行的请求
          if (fetchCurrentPromise && now - fetchCurrentLastTime < DEBOUNCE_MS) {
              return fetchCurrentPromise;
          }
          
          fetchCurrentLastTime = now;
          
          fetchCurrentPromise = (async () => {
              try {
                  const account = await accountService.getCurrentAccount();
                  if (
                      !account &&
                      get().currentAccount &&
                      get().accounts.length > 0 &&
                      !allowNextEmptyCurrentAccount
                  ) {
                      console.warn('[AccountStore] 忽略异常空当前账号，保留本地缓存当前账号');
                      return;
                  }
                  allowNextEmptyCurrentAccount = false;
                  set({ currentAccount: account });
              } catch (e) {
                  console.error('Failed to fetch current account:', e);
              } finally {
                  allowNextEmptyCurrentAccount = false;
                  setTimeout(() => {
                      fetchCurrentPromise = null;
                  }, 100);
              }
          })();
          
          return fetchCurrentPromise;
      },

    addAccount: async (email: string, refreshToken: string) => {
        const account = await accountService.addAccount(email, refreshToken);
        await get().fetchAccounts();
        await emitAccountsChanged({
            platformId: 'antigravity',
            reason: 'import',
        });
        return account;
    },

    deleteAccount: async (accountId: string) => {
        const previousCurrentAccountId = get().currentAccount?.id ?? null;
        allowNextEmptyAccountList = get().accounts.length <= 1;
        allowNextEmptyCurrentAccount = previousCurrentAccountId === accountId;
        try {
            await accountService.deleteAccount(accountId);
            await get().fetchAccounts();
            await get().fetchCurrentAccount();
        } finally {
            allowNextEmptyAccountList = false;
            allowNextEmptyCurrentAccount = false;
        }
        await emitAccountsChanged({
            platformId: 'antigravity',
            reason: 'delete',
        });
        const nextCurrentAccountId = get().currentAccount?.id ?? null;
        if (previousCurrentAccountId !== nextCurrentAccountId) {
            await emitCurrentAccountChanged({
                platformId: 'antigravity',
                accountId: nextCurrentAccountId,
                reason: 'delete',
            });
        }
    },

    deleteAccounts: async (accountIds: string[]) => {
        const previousCurrentAccountId = get().currentAccount?.id ?? null;
        const deleteIdSet = new Set(accountIds);
        allowNextEmptyAccountList = get().accounts.every((account) => deleteIdSet.has(account.id));
        allowNextEmptyCurrentAccount = previousCurrentAccountId
            ? deleteIdSet.has(previousCurrentAccountId)
            : false;
        try {
            await accountService.deleteAccounts(accountIds);
            await get().fetchAccounts();
            await get().fetchCurrentAccount();
        } finally {
            allowNextEmptyAccountList = false;
            allowNextEmptyCurrentAccount = false;
        }
        await emitAccountsChanged({
            platformId: 'antigravity',
            reason: 'delete',
        });
        const nextCurrentAccountId = get().currentAccount?.id ?? null;
        if (previousCurrentAccountId !== nextCurrentAccountId) {
            await emitCurrentAccountChanged({
                platformId: 'antigravity',
                accountId: nextCurrentAccountId,
                reason: 'delete',
            });
        }
    },

    setCurrentAccount: async (accountId: string) => {
        await accountService.setCurrentAccount(accountId);
        await get().fetchCurrentAccount();
        await emitCurrentAccountChanged({
            platformId: 'antigravity',
            accountId: get().currentAccount?.id ?? accountId,
            reason: 'switch',
        });
    },

    refreshQuota: async (accountId: string) => {
        try {
            const updatedAccount = await accountService.fetchAccountQuota(accountId);
            // 成功：后端已更新该账号并返回最新状态（包含 quota_error），局部更新该账号，保持滚动位置不变
            set((state) => ({
                accounts: state.accounts.map((acc) =>
                    acc.id === accountId ? updatedAccount : acc
                ),
            }));
            
            // 如果刷新的是当前账号，需要同时更新 currentAccount
            const { currentAccount } = get();
            if (currentAccount?.id === accountId) {
                set({ currentAccount: updatedAccount });
            }

            // 如果后端返回了配额错误信息，需要抛出异常让 UI 捕获并显示为失败（红叉）
            if (updatedAccount.quota_error) {
                throw new Error(updatedAccount.quota_error.message);
            }
            if (updatedAccount.quota?.is_forbidden) {
                throw new Error("403 Forbidden");
            }
        } catch (e) {
            // Token 级别失败（如 invalid_grant 会改变 disabled 状态）：全量刷新确保数据正确
            // 如果是我们自己 throw 的配额错误，因为状态已经局部更新，不再需要全量刷新
            const isQuotaError = e instanceof Error && (
                get().accounts.find(a => a.id === accountId)?.quota_error?.message === e.message ||
                e.message === "403 Forbidden"
            );
            if (!isQuotaError) {
                await get().fetchAccounts();
            }
            throw e;
        } finally {
            await get().fetchCurrentAccount();
        }
    },

    refreshAllQuotas: async () => {
        const stats = await accountService.refreshAllQuotas();
        await get().fetchAccounts();
        await get().fetchCurrentAccount();
        return stats;
    },

    startOAuthLogin: async () => {
        const account = await accountService.startOAuthLogin();
        await get().fetchAccounts();
        await emitAccountsChanged({
            platformId: 'antigravity',
            reason: 'oauth',
        });
        return account;
    },

    reorderAccounts: async (accountIds: string[]) => {
        await accountService.reorderAccounts(accountIds);
        await get().fetchAccounts();
    },

    switchAccount: async (accountId: string) => {
        const previousCurrentAccountId = get().currentAccount?.id ?? null;
        try {
            const account = await accountService.switchAccount(accountId);
            set({ currentAccount: account });
            await get().fetchAccounts();
            await emitCurrentAccountChanged({
                platformId: 'antigravity',
                accountId: account.id,
                reason: 'switch',
            });
            return account;
        } catch (error) {
            await get().fetchAccounts();
            await get().fetchCurrentAccount();
            const nextCurrentAccountId = get().currentAccount?.id ?? null;
            if (previousCurrentAccountId !== nextCurrentAccountId) {
                await emitCurrentAccountChanged({
                    platformId: 'antigravity',
                    accountId: nextCurrentAccountId,
                    reason: 'switch',
                });
            }
            throw error;
        }
    },

    syncCurrentFromClient: async () => {
        const previousCurrentAccountId = get().currentAccount?.id ?? null;
        try {
            const account = await accountService.getCurrentAccount();
            set({ currentAccount: account });
            const nextCurrentAccountId = account?.id ?? null;
            if (previousCurrentAccountId !== nextCurrentAccountId) {
                await emitCurrentAccountChanged({
                    platformId: 'antigravity',
                    accountId: nextCurrentAccountId,
                    reason: 'sync',
                });
            }
        } catch (e) {
            console.error('Failed to refresh current account:', e);
        }
    },

    updateAccountTags: async (accountId: string, tags: string[]) => {
        const account = await accountService.updateAccountTags(accountId, tags);
        await get().fetchAccounts();
        return account;
    },
  }),
  {
    name: ACCOUNTS_STORE_KEY,
    storage: accountStoreStorage,
    partialize: (state) => ({
      accounts: state.accounts.map(toPersistedAccountSnapshot),
      currentAccount: state.currentAccount
        ? toPersistedAccountSnapshot(state.currentAccount)
        : null,
    }),
    onRehydrateStorage: () => (state) => {
      // Migrate from old ACCOUNTS_CACHE_KEY if the new state is empty
      if (state && state.accounts.length === 0 && typeof window !== 'undefined') {
        setTimeout(() => {
          try {
            const oldAccountsRaw = localStorage.getItem(LEGACY_ACCOUNTS_CACHE_KEY);
            const oldCurrentRaw = localStorage.getItem(LEGACY_CURRENT_ACCOUNT_CACHE_KEY);
            let hasMigrated = false;
            
            if (oldAccountsRaw) {
              const oldAccounts = JSON.parse(oldAccountsRaw);
              if (Array.isArray(oldAccounts) && oldAccounts.length > 0) {
                useAccountStore.setState({ accounts: oldAccounts });
                hasMigrated = true;
              }
            }
            if (oldCurrentRaw) {
              const oldCurrent = JSON.parse(oldCurrentRaw);
              if (oldCurrent && oldCurrent.id) {
                useAccountStore.setState({ currentAccount: oldCurrent });
                hasMigrated = true;
              }
            }
            
            // Cleanup the old keys if we migrated successfully
            if (hasMigrated) {
              localStorage.removeItem(LEGACY_ACCOUNTS_CACHE_KEY);
              localStorage.removeItem(LEGACY_CURRENT_ACCOUNT_CACHE_KEY);
            }
          } catch (error) {
            // ignore migration errors
          }
        }, 0);
      }
    },
  }
));
