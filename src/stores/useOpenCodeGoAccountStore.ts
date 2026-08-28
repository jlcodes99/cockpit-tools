import { create } from 'zustand';
import { openCodeGoService, toOpenCodeGoCommandError } from '../services/openCodeGoService.ts';
import type {
  OpenCodeGoConnection,
  OpenCodeGoQuotaSnapshot,
} from '../types/openCodeGo.ts';

/**
 * Canonical in-memory owner for OpenCode Go connections. The dedicated page,
 * dashboard, and floating card all subscribe here so CRUD mutations are
 * immediately coherent across every surface.
 */
export interface OpenCodeGoAccountClient {
  listAccounts: () => Promise<OpenCodeGoConnection[]>;
  createConnection: (input: { name: string; apiKey: string; email?: string; provider?: 'go' | 'zen' }) => Promise<OpenCodeGoConnection>;
  updateConnection: (accountId: string, patch: { name?: string; apiKey?: string; email?: string }) => Promise<OpenCodeGoConnection>;
  setConnectionEnabled: (accountId: string, enabled: boolean) => Promise<OpenCodeGoConnection>;
  deleteConnection: (accountId: string) => Promise<void>;
  testConnection: (accountId: string) => Promise<void>;
  refreshQuota: (accountId: string) => Promise<{
    connection: OpenCodeGoConnection;
    quota: OpenCodeGoQuotaSnapshot;
  }>;
  refreshAllQuotas: () => Promise<{ connections: OpenCodeGoConnection[] }>;
}

export interface OpenCodeGoAccountState {
  accounts: OpenCodeGoConnection[];
  loaded: boolean;
  loading: boolean;
  refreshingId: string | null;
  error: string | null;
  fetchAccounts: () => Promise<void>;
  createConnection: (name: string, apiKey: string, email?: string, provider?: 'go' | 'zen') => Promise<OpenCodeGoConnection>;
  updateConnection: (accountId: string, patch: { name?: string; apiKey?: string; email?: string }) => Promise<OpenCodeGoConnection>;
  setConnectionEnabled: (accountId: string, enabled: boolean) => Promise<OpenCodeGoConnection>;
  deleteConnection: (accountId: string) => Promise<void>;
  testConnection: (accountId: string) => Promise<void>;
  refreshQuota: (accountId: string) => Promise<OpenCodeGoQuotaSnapshot>;
  refreshAllQuotas: () => Promise<number>;
}

function safeError(error: unknown): string {
  return toOpenCodeGoCommandError(error);
}

function replaceConnection(
  accounts: OpenCodeGoConnection[],
  connection: OpenCodeGoConnection,
): OpenCodeGoConnection[] {
  return accounts.map((item) => item.id === connection.id ? connection : item);
}

export function createOpenCodeGoAccountStore(client: OpenCodeGoAccountClient) {
  return create<OpenCodeGoAccountState>((set) => ({
    accounts: [],
    loaded: false,
    loading: false,
    refreshingId: null,
    error: null,

    fetchAccounts: async () => {
      set({ loading: true, error: null });
      try {
        const accounts = await client.listAccounts();
        set({ accounts, loaded: true, loading: false });
      } catch (error) {
        set({ loaded: true, loading: false, error: safeError(error) });
      }
    },

    createConnection: async (name, apiKey, email, provider = 'go') => {
      const connection = await client.createConnection({ name, apiKey, email, provider });
      set((state) => ({ accounts: [...state.accounts, connection], loaded: true, error: null }));
      return connection;
    },

    updateConnection: async (accountId, patch) => {
      const connection = await client.updateConnection(accountId, patch);
      set((state) => ({ accounts: replaceConnection(state.accounts, connection), error: null }));
      return connection;
    },

    setConnectionEnabled: async (accountId, enabled) => {
      const connection = await client.setConnectionEnabled(accountId, enabled);
      set((state) => ({ accounts: replaceConnection(state.accounts, connection), error: null }));
      return connection;
    },

    deleteConnection: async (accountId) => {
      await client.deleteConnection(accountId);
      set((state) => ({
        accounts: state.accounts.filter((item) => item.id !== accountId),
        error: null,
      }));
    },

    testConnection: async (accountId) => {
      await client.testConnection(accountId);
      set({ error: null });
    },

    refreshQuota: async (accountId) => {
      set({ refreshingId: accountId, error: null });
      try {
        const { connection, quota } = await client.refreshQuota(accountId);
        set((state) => ({
          accounts: replaceConnection(state.accounts, connection),
          refreshingId: null,
        }));
        return quota;
      } catch (error) {
        try {
          const accounts = await client.listAccounts();
          set({ accounts });
        } catch {
          // Preserve the known public snapshot when reconciliation also fails.
        }
        set({ refreshingId: null, error: safeError(error) });
        throw error;
      }
    },

    refreshAllQuotas: async () => {
      set({ loading: true, error: null });
      try {
        const { connections } = await client.refreshAllQuotas();
        set({ accounts: connections, loaded: true, loading: false });
        return connections.filter((connection) => connection.quota != null).length;
      } catch (error) {
        set({ loading: false, error: safeError(error) });
        throw error;
      }
    },
  }));
}

export const useOpenCodeGoAccountStore = createOpenCodeGoAccountStore({
  listAccounts: () => openCodeGoService.listConnections(),
  createConnection: (input) => openCodeGoService.createConnection(input),
  updateConnection: (accountId, patch) => openCodeGoService.updateConnection(accountId, patch),
  setConnectionEnabled: (accountId, enabled) => openCodeGoService.setConnectionEnabled(accountId, enabled),
  deleteConnection: (accountId) => openCodeGoService.deleteConnection(accountId),
  testConnection: (accountId) => openCodeGoService.testConnection(accountId),
  refreshQuota: (accountId) => openCodeGoService.queryQuota(accountId),
  refreshAllQuotas: async () => ({ connections: await openCodeGoService.queryAllQuotas() }),
});
