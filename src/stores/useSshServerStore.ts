import { create } from 'zustand';
import * as sshServerService from '../services/sshServerService';
import type { SshCodexSyncResult, SshServer, SshServerDraft, SshServerList } from '../types/sshServer';

interface SshServerState {
  servers: SshServer[];
  selectedServerIds: string[];
  loading: boolean;
  error: string | null;
  lastSyncResult: SshCodexSyncResult | null;
  fetchServers: () => Promise<void>;
  upsertServer: (draft: SshServerDraft) => Promise<void>;
  deleteServer: (serverId: string) => Promise<void>;
  selectServers: (serverIds: string[]) => Promise<void>;
  setServerSelected: (serverId: string, selected: boolean) => Promise<void>;
  testConnection: (serverId: string) => Promise<string>;
  syncNow: (serverId: string) => Promise<SshCodexSyncResult>;
  applySyncResult: (result: SshCodexSyncResult) => void;
}

function stateFromList(list: SshServerList) {
  return {
    servers: list.servers,
    selectedServerIds: [...new Set(list.selected_server_ids)]
      .filter(id => list.servers.some(server => server.id === id)),
    error: null,
  };
}

export const useSshServerStore = create<SshServerState>((set, get) => {
  let revision = 0;
  let fetchSequence = 0;
  let mutations = Promise.resolve();
  const syncRevisions = new Map<string, number>();
  const mutate = (operation: () => Promise<SshServerList>): Promise<void> => {
    revision++;
    const result = mutations.then(async () => {
      const started = revision;
      const list = await operation();
      revision++;
      set(state => {
        const next = stateFromList(list);
        const current = new Map(state.servers.map(server => [server.id, server]));
        return { ...next, servers: next.servers.map(server =>
          (syncRevisions.get(server.id) ?? 0) > started
            ? { ...server, last_sync: current.get(server.id)?.last_sync ?? server.last_sync }
            : server) };
      });
      for (const id of syncRevisions.keys()) {
        if (!list.servers.some(server => server.id === id)) syncRevisions.delete(id);
      }
    });
    mutations = result.catch(() => {});
    return result;
  };
  return {
  servers: [],
  selectedServerIds: [],
  loading: false,
  error: null,
  lastSyncResult: null,

  fetchServers: async () => {
    const started = revision;
    const sequence = ++fetchSequence;
    set({ loading: true, error: null });
    try {
      const list = await sshServerService.listSshServers();
      if (sequence === fetchSequence) {
        set({ ...(started === revision ? stateFromList(list) : {}), loading: false });
      }
    } catch (error) {
      if (sequence === fetchSequence) set({ error: String(error), loading: false });
    }
  },

  upsertServer: (draft) => mutate(() => sshServerService.upsertSshServer(draft)),
  deleteServer: (serverId) => mutate(() => sshServerService.deleteSshServer(serverId)),
  selectServers: (serverIds) => mutate(() => sshServerService.selectSshServers(serverIds)),
  setServerSelected: (serverId, selected) => mutate(() => sshServerService.setSshServerSelected(serverId, selected)),

  testConnection: async (serverId) => sshServerService.testSshServerConnection(serverId),

  syncNow: async (serverId) => {
    const result = await sshServerService.syncCurrentCodexAccountToSshServer(
      serverId,
    );
    get().applySyncResult(result);
    return result;
  },

  applySyncResult: (result) => {
    revision++;
    syncRevisions.set(result.server_id, revision);
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
};
});
