import { invoke } from '@tauri-apps/api/core';
import type { PlatformInstanceService } from './platform/createPlatformInstanceService';
import type { InstanceInitMode, InstanceLaunchMode, InstanceProfile } from '../types/instance';
import type { TraePlatformId } from './traeService';

type InstancePayload = {
  name: string;
  userDataDir: string;
  workingDir?: string | null;
  extraArgs?: string;
  bindAccountId?: string | null;
  launchMode?: InstanceLaunchMode;
  copySourceInstanceId: string;
  initMode?: InstanceInitMode;
};

type UpdateInstancePayload = {
  instanceId: string;
  name?: string;
  workingDir?: string | null;
  extraArgs?: string;
  bindAccountId?: string | null;
  followLocalAccount?: boolean;
  launchMode?: InstanceLaunchMode;
};

export function createTraeInstanceService(
  platformId: TraePlatformId = 'trae',
): PlatformInstanceService {
  return {
    getInstanceDefaults: async () =>
      await invoke('trae_get_instance_defaults', { platformId }),

    listInstances: async () =>
      await invoke('trae_list_instances', { platformId }),

    createInstance: async (payload: InstancePayload) =>
      await invoke('trae_create_instance', {
        platformId,
        name: payload.name,
        userDataDir: payload.userDataDir,
        workingDir: payload.workingDir ?? null,
        extraArgs: payload.extraArgs ?? '',
        bindAccountId: payload.bindAccountId ?? null,
        launchMode: payload.launchMode ?? null,
        copySourceInstanceId: payload.copySourceInstanceId,
        initMode: payload.initMode ?? 'copy',
      }),

    updateInstance: async (payload: UpdateInstancePayload): Promise<InstanceProfile> => {
      const body: Record<string, unknown> = {
        platformId,
        instanceId: payload.instanceId,
      };
      if (payload.name !== undefined) body.name = payload.name;
      if (payload.workingDir !== undefined) body.workingDir = payload.workingDir;
      if (payload.extraArgs !== undefined) body.extraArgs = payload.extraArgs;
      if (payload.bindAccountId !== undefined) body.bindAccountId = payload.bindAccountId;
      if (payload.followLocalAccount !== undefined) {
        body.followLocalAccount = payload.followLocalAccount;
      }
      if (payload.launchMode !== undefined) body.launchMode = payload.launchMode;
      return await invoke('trae_update_instance', body);
    },

    deleteInstance: async (instanceId: string) =>
      await invoke('trae_delete_instance', { platformId, instanceId }),

    startInstance: async (instanceId: string) =>
      await invoke('trae_start_instance', { platformId, instanceId }),

    stopInstance: async (instanceId: string) =>
      await invoke('trae_stop_instance', { platformId, instanceId }),

    closeAllInstances: async () =>
      await invoke('trae_close_all_instances', { platformId }),

    openInstanceWindow: async (instanceId: string) =>
      await invoke('trae_open_instance_window', { platformId, instanceId }),
  };
}

const service = createTraeInstanceService('trae');

export const getInstanceDefaults = service.getInstanceDefaults;
export const listInstances = service.listInstances;
export const createInstance = service.createInstance;
export const updateInstance = service.updateInstance;
export const deleteInstance = service.deleteInstance;
export const startInstance = service.startInstance;
export const stopInstance = service.stopInstance;
export const closeAllInstances = service.closeAllInstances;
export const openInstanceWindow = service.openInstanceWindow;
