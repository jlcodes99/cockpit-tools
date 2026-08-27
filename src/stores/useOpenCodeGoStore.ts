import { create } from 'zustand';
import { openCodeGoService } from '../services/openCodeGoService';
import type { OpenCodeGoConnection } from '../types/openCodeGo';
import { OPENCODE_GO_CONNECTION_LIMIT } from '../utils/openCodeGoConnections';

interface OpenCodeGoState {
  connections: OpenCodeGoConnection[];
  loaded: boolean;
  loading: boolean;
  error: string | null;
  fetchConnections(): Promise<void>;
  createConnection(name: string, apiKey: string, provider?: 'go' | 'zen'): Promise<void>;
  updateConnection(
    connectionId: string,
    patch: { name?: string; apiKey?: string },
  ): Promise<void>;
  setConnectionEnabled(connectionId: string, enabled: boolean): Promise<void>;
  deleteConnection(connectionId: string): Promise<void>;
  testConnection(connectionId: string): Promise<void>;
  refreshConnection(connectionId: string): Promise<void>;
  refreshAll(): Promise<void>;
}

function replaceConnection(
  connections: OpenCodeGoConnection[],
  updated: OpenCodeGoConnection,
): OpenCodeGoConnection[] {
  return connections.map((connection) =>
    connection.id === updated.id ? updated : connection,
  );
}

export const useOpenCodeGoStore = create<OpenCodeGoState>((set, get) => ({
  connections: [],
  loaded: false,
  loading: false,
  error: null,

  async fetchConnections() {
    set({ loading: true, error: null });
    try {
      const connections = await openCodeGoService.listConnections();
      if (connections.length > OPENCODE_GO_CONNECTION_LIMIT) {
        throw new Error('OPENCODE_GO_CONNECTION_LIMIT_EXCEEDED');
      }
      set({ connections, loaded: true, loading: false });
    } catch (error) {
      set({ error: String(error), loaded: true, loading: false });
    }
  },

  async createConnection(name, apiKey, provider = 'go') {
    if (get().connections.length >= OPENCODE_GO_CONNECTION_LIMIT) {
      throw new Error('OPENCODE_GO_CONNECTION_LIMIT_REACHED');
    }
    const connection = await openCodeGoService.createConnection({ name, apiKey, provider });
    set((state) => ({
      connections: [...state.connections, connection],
      error: null,
    }));
  },

  async updateConnection(connectionId, patch) {
    const connection = await openCodeGoService.updateConnection(connectionId, patch);
    set((state) => ({
      connections: replaceConnection(state.connections, connection),
      error: null,
    }));
  },

  async setConnectionEnabled(connectionId, enabled) {
    const connection = await openCodeGoService.setConnectionEnabled(connectionId, enabled);
    set((state) => ({
      connections: replaceConnection(state.connections, connection),
      error: null,
    }));
  },

  async deleteConnection(connectionId) {
    await openCodeGoService.deleteConnection(connectionId);
    set((state) => ({
      connections: state.connections.filter(
        (connection) => connection.id !== connectionId,
      ),
      error: null,
    }));
  },

  async testConnection(connectionId) {
    await openCodeGoService.testConnection(connectionId);
    set({ error: null });
  },

  async refreshConnection(connectionId) {
    const result = await openCodeGoService.queryQuota(connectionId);
    set((state) => ({
      connections: replaceConnection(state.connections, result.connection),
      error: null,
    }));
  },

  async refreshAll() {
    set({ loading: true, error: null });
    try {
      const connections = await openCodeGoService.queryAllQuotas();
      set({ connections, loaded: true, loading: false });
    } catch (error) {
      set({ error: String(error), loading: false });
    }
  },
}));
