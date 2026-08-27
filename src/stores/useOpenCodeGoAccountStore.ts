import { create } from 'zustand';
import { openCodeGoService } from '../services/openCodeGoService.ts';
import type {
  OpenCodeGoConnection,
  OpenCodeGoQuotaSnapshot,
} from '../types/openCodeGo.ts';

/**
 * Transport seam for the OpenCode Go Tauri commands
 * (`list_opencode_go_connections`, `query_opencode_go_quota`,
 * `query_all_opencode_go_quotas`). Injected so the store stays testable and
 * the page component never imports `@tauri-apps/api` directly.
 */
export interface OpenCodeGoAccountClient {
  listAccounts: () => Promise<OpenCodeGoConnection[]>;
  refreshQuota: (accountId: string) => Promise<{
    connection: OpenCodeGoConnection;
    quota: OpenCodeGoQuotaSnapshot;
  }>;
  refreshAllQuotas: () => Promise<{ connections: OpenCodeGoConnection[] }>;
}

export interface OpenCodeGoAccountState {
  accounts: OpenCodeGoConnection[];
  loading: boolean;
  refreshingId: string | null;
  error: string | null;
  fetchAccounts: () => Promise<void>;
  refreshQuota: (accountId: string) => Promise<OpenCodeGoQuotaSnapshot>;
  refreshAllQuotas: () => Promise<number>;
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error ?? 'unknown');
}

export function createOpenCodeGoAccountStore(client: OpenCodeGoAccountClient) {
  return create<OpenCodeGoAccountState>((set) => ({
    accounts: [],
    loading: false,
    refreshingId: null,
    error: null,

    fetchAccounts: async () => {
      set({ loading: true, error: null });
      try {
        const accounts = await client.listAccounts();
        set({ accounts, loading: false });
      } catch (error) {
        set({ loading: false, error: errorText(error) });
      }
    },

    /**
     * Refreshes one connection's quota. On provider failure the cached
     * connection list is reconciled from the backend before the error is
     * rethrown, so persisted quota-error state still reaches the UI.
     */
    refreshQuota: async (accountId: string) => {
      set({ refreshingId: accountId, error: null });
      try {
        const { connection, quota } = await client.refreshQuota(accountId);
        set((state) => ({
          accounts: state.accounts.map((item) =>
            item.id === connection.id ? connection : item,
          ),
          refreshingId: null,
        }));
        return quota;
      } catch (error) {
        try {
          const accounts = await client.listAccounts();
          set({ accounts });
        } catch {
          // keep the current view if even reconciliation fails
        }
        set({ refreshingId: null, error: errorText(error) });
        throw error;
      }
    },

    refreshAllQuotas: async () => {
      set({ loading: true, error: null });
      try {
        // The backend persists per-connection quota errors inside this call;
        // the returned summaries are the authoritative full snapshot.
        const { connections } = await client.refreshAllQuotas();
        set({ accounts: connections, loading: false });
        return connections.filter((connection) => connection.quota != null).length;
      } catch (error) {
        set({ loading: false, error: errorText(error) });
        throw error;
      }
    },
  }));
}

export const useOpenCodeGoAccountStore = createOpenCodeGoAccountStore({
  listAccounts: () => openCodeGoService.listConnections(),
  refreshQuota: (accountId) => openCodeGoService.queryQuota(accountId),
  refreshAllQuotas: async () => ({
    connections: await openCodeGoService.queryAllQuotas(),
  }),
});
