import { create } from "zustand";
import {
  InstanceDefaults,
  InstanceInitMode,
  InstanceLaunchMode,
  InstanceProfile,
} from "../types/instance";
import type { CodexAppSpeed } from "../types/codex";

export type InstanceStoreState = {
  instances: InstanceProfile[];
  defaults: InstanceDefaults | null;
  loading: boolean;
  error: string | null;
  fetchInstances: () => Promise<void>;
  refreshInstances: () => Promise<InstanceProfile[]>;
  fetchDefaults: () => Promise<void>;
  createInstance: (payload: {
    name: string;
    userDataDir: string;
    workingDir?: string | null;
    extraArgs?: string;
    bindAccountId?: string | null;
    launchMode?: InstanceLaunchMode;
    appSpeed?: CodexAppSpeed;
    copySourceInstanceId: string;
    initMode?: InstanceInitMode;
  }) => Promise<InstanceProfile>;
  updateInstance: (payload: {
    instanceId: string;
    name?: string;
    workingDir?: string | null;
    extraArgs?: string;
    bindAccountId?: string | null;
    followLocalAccount?: boolean;
    launchMode?: InstanceLaunchMode;
    appSpeed?: CodexAppSpeed;
    autoSyncThreads?: boolean;
  }) => Promise<InstanceProfile>;
  deleteInstance: (instanceId: string) => Promise<void>;
  startInstance: (instanceId: string) => Promise<InstanceProfile>;
  stopInstance: (instanceId: string) => Promise<InstanceProfile>;
  closeAllInstances: () => Promise<void>;
  openInstanceWindow: (instanceId: string) => Promise<void>;
};

type InstanceService = {
  getInstanceDefaults: () => Promise<InstanceDefaults>;
  listInstances: () => Promise<InstanceProfile[]>;
  createInstance: (payload: {
    name: string;
    userDataDir: string;
    workingDir?: string | null;
    extraArgs?: string;
    bindAccountId?: string | null;
    launchMode?: InstanceLaunchMode;
    appSpeed?: CodexAppSpeed;
    copySourceInstanceId: string;
    initMode?: InstanceInitMode;
  }) => Promise<InstanceProfile>;
  updateInstance: (payload: {
    instanceId: string;
    name?: string;
    workingDir?: string | null;
    extraArgs?: string;
    bindAccountId?: string | null;
    followLocalAccount?: boolean;
    launchMode?: InstanceLaunchMode;
    appSpeed?: CodexAppSpeed;
    autoSyncThreads?: boolean;
  }) => Promise<InstanceProfile>;
  deleteInstance: (instanceId: string) => Promise<void>;
  startInstance: (instanceId: string) => Promise<InstanceProfile>;
  stopInstance: (instanceId: string) => Promise<InstanceProfile>;
  closeAllInstances: () => Promise<void>;
  openInstanceWindow: (instanceId: string) => Promise<void>;
};

export function createInstanceStore(
  service: InstanceService,
  cacheKey: string,
) {
  const loadCachedInstances = () => {
    try {
      const raw = localStorage.getItem(cacheKey);
      if (!raw) return { instances: [], loaded: false };
      const parsed = JSON.parse(raw);
      return Array.isArray(parsed)
        ? { instances: parsed as InstanceProfile[], loaded: true }
        : { instances: [], loaded: false };
    } catch {
      return { instances: [], loaded: false };
    }
  };

  const persistInstancesCache = (instances: InstanceProfile[]) => {
    try {
      localStorage.setItem(cacheKey, JSON.stringify(instances));
    } catch {
      // ignore cache write failures
    }
  };

  const cachedInstances = loadCachedInstances();
  let hasLoadedInstances = cachedInstances.loaded;

  return create<InstanceStoreState>((set, get) => ({
    instances: cachedInstances.instances,
    defaults: null,
    loading: false,
    error: null,

    fetchInstances: async () => {
      const showInitialLoading =
        !hasLoadedInstances && get().instances.length === 0;
      set(
        showInitialLoading
          ? { loading: true, error: null }
          : { error: null },
      );
      try {
        const instances = await service.listInstances();
        hasLoadedInstances = true;
        set({ instances, loading: false });
        persistInstancesCache(instances);
      } catch (e) {
        set({ error: String(e), loading: false });
      }
    },

    refreshInstances: async () => {
      set({ error: null });
      try {
        const instances = await service.listInstances();
        hasLoadedInstances = true;
        set({ instances });
        persistInstancesCache(instances);
        return instances;
      } catch (e) {
        set({ error: String(e) });
        return get().instances;
      }
    },

    fetchDefaults: async () => {
      try {
        const defaults = await service.getInstanceDefaults();
        set({ defaults });
      } catch (e) {
        set({ error: String(e) });
      }
    },

    createInstance: async (payload) => {
      const instance = await service.createInstance(payload);
      await get().fetchInstances();
      return instance;
    },

    updateInstance: async (payload) => {
      const instance = await service.updateInstance(payload);
      await get().fetchInstances();
      return instance;
    },

    deleteInstance: async (instanceId) => {
      await service.deleteInstance(instanceId);
      await get().fetchInstances();
    },

    startInstance: async (instanceId) => {
      const flowStartedAt = performance.now();
      console.info("[Instance Start][Store] startInstance started", { instanceId });
      const instance = await service.startInstance(instanceId);
      console.info("[Instance Start][Store] service.startInstance finished", {
        instanceId,
        elapsedMs: Math.round(performance.now() - flowStartedAt),
      });
      await get().fetchInstances();
      console.info("[Instance Start][Store] fetchInstances after start finished", {
        instanceId,
        elapsedMs: Math.round(performance.now() - flowStartedAt),
      });
      return instance;
    },

    stopInstance: async (instanceId) => {
      const instance = await service.stopInstance(instanceId);
      await get().fetchInstances();
      return instance;
    },

    closeAllInstances: async () => {
      await service.closeAllInstances();
      await get().fetchInstances();
    },

    openInstanceWindow: async (instanceId) => {
      await service.openInstanceWindow(instanceId);
    },
  }));
}
