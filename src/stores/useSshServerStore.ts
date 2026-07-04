import { create } from 'zustand';
import * as sshServerService from '../services/sshServerService';
import type { SshCodexSyncResult, SshServer, SshServerDraft } from '../types/sshServer';

interface SshServerState {
  servers: SshServer[];
  selectedServerId: string | null;
  loading: boolean;
  error: string | null;
  lastSyncResult: SshCodexSyncResult | null;
  fetchServers: () => Promise<void>;
  upsertServer: (draft: SshServerDraft) => Promise<void>;
  deleteServer: (serverId: string) => Promise<void>;
  selectServer: (serverId: string | null) => Promise<void>;
  testConnection: (serverId: string) => Promise<string>;
  syncNow: (serverId?: string | null) => Promise<SshCodexSyncResult>;
  applySyncResult: (result: SshCodexSyncResult) => void;
}

function selectedIdFromList(selectedServerId: string | null, servers: SshServer[]) {
  return selectedServerId && servers.some((server) => server.id === selectedServerId)
    ? selectedServerId
    : null;
}

export const useSshServerStore = create<SshServerState>((set, get) => ({
  servers: [],
  selectedServerId: null,
  loading: false,
  error: null,
  lastSyncResult: null,

  fetchServers: async () => {
    set({ loading: true, error: null });
    try {
      const list = await sshServerService.listSshServers();
      set({
        servers: list.servers,
        selectedServerId: selectedIdFromList(list.selected_server_id, list.servers),
        loading: false,
      });
    } catch (error) {
      set({ error: String(error), loading: false });
    }
  },

  upsertServer: async (draft) => {
    const list = await sshServerService.upsertSshServer(draft);
    set({
      servers: list.servers,
      selectedServerId: selectedIdFromList(list.selected_server_id, list.servers),
      error: null,
    });
  },

  deleteServer: async (serverId) => {
    const list = await sshServerService.deleteSshServer(serverId);
    set({
      servers: list.servers,
      selectedServerId: selectedIdFromList(list.selected_server_id, list.servers),
      error: null,
    });
  },

  selectServer: async (serverId) => {
    const list = await sshServerService.selectSshServer(serverId);
    set({
      servers: list.servers,
      selectedServerId: selectedIdFromList(list.selected_server_id, list.servers),
      error: null,
    });
  },

  testConnection: async (serverId) => sshServerService.testSshServerConnection(serverId),

  syncNow: async (serverId) => {
    const result = await sshServerService.syncCurrentCodexAccountToSshServer(
      serverId ?? get().selectedServerId,
    );
    get().applySyncResult(result);
    void get().fetchServers();
    return result;
  },

  applySyncResult: (result) => {
    set((state) => ({
      lastSyncResult: result,
      servers: state.servers.map((server) =>
        server.id === result.server_id
          ? {
              ...server,
              last_sync: {
                account_id: result.account_id,
                account_email: result.account_email,
                token_generation: result.token_generation,
                bundle_hash: result.bundle_hash,
                synced_at: result.synced_at,
                verified: result.verified,
                error: result.error,
              },
            }
          : server,
      ),
    }));
  },
}));
