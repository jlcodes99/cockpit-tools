import { invoke } from "@tauri-apps/api/core";
import {
  InstanceDefaults,
  InstanceInitMode,
  InstanceLaunchMode,
  InstanceProfile,
} from "../../types/instance";

type PlatformInstanceCommandPrefix =
  | ""
  | "codex"
  | "github_copilot"
  | "windsurf"
  | "kiro"
  | "cursor"
  | "gemini"
  | "codebuddy"
  | "codebuddy_cn"
  | "qoder"
  | "trae"
  | "workbuddy";

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

export type PlatformInstanceService = {
  getInstanceDefaults: () => Promise<InstanceDefaults>;
  listInstances: () => Promise<InstanceProfile[]>;
  createInstance: (payload: InstancePayload) => Promise<InstanceProfile>;
  updateInstance: (payload: UpdateInstancePayload) => Promise<InstanceProfile>;
  deleteInstance: (instanceId: string) => Promise<void>;
  startInstance: (instanceId: string) => Promise<InstanceProfile>;
  stopInstance: (instanceId: string) => Promise<InstanceProfile>;
  closeAllInstances: () => Promise<void>;
  openInstanceWindow: (instanceId: string) => Promise<void>;
};

const commandFor = (prefix: PlatformInstanceCommandPrefix, command: string) =>
  prefix ? `${prefix}_${command}` : command;

export function createPlatformInstanceService(
  prefix: PlatformInstanceCommandPrefix,
): PlatformInstanceService {
  return {
    getInstanceDefaults: async () => {
      return await invoke(commandFor(prefix, "get_instance_defaults"));
    },

    listInstances: async () => {
      return await invoke(commandFor(prefix, "list_instances"));
    },

    createInstance: async (payload) => {
      return await invoke(commandFor(prefix, "create_instance"), {
        name: payload.name,
        userDataDir: payload.userDataDir,
        workingDir: payload.workingDir ?? null,
        extraArgs: payload.extraArgs ?? "",
        bindAccountId: payload.bindAccountId ?? null,
        copySourceInstanceId: payload.copySourceInstanceId,
        initMode: payload.initMode ?? "copy",
      });
    },

    updateInstance: async (payload) => {
      const body: Record<string, unknown> = {
        instanceId: payload.instanceId,
      };
      if (payload.name !== undefined) {
        body.name = payload.name;
      }
      if (payload.workingDir !== undefined) {
        body.workingDir = payload.workingDir;
      }
      if (payload.extraArgs !== undefined) {
        body.extraArgs = payload.extraArgs;
      }
      if (payload.bindAccountId !== undefined) {
        body.bindAccountId = payload.bindAccountId;
      }
      if (payload.followLocalAccount !== undefined) {
        body.followLocalAccount = payload.followLocalAccount;
      }
      return await invoke(commandFor(prefix, "update_instance"), body);
    },

    deleteInstance: async (instanceId) => {
      return await invoke(commandFor(prefix, "delete_instance"), {
        instanceId,
      });
    },

    startInstance: async (instanceId) => {
      return await invoke(commandFor(prefix, "start_instance"), { instanceId });
    },

    stopInstance: async (instanceId) => {
      return await invoke(commandFor(prefix, "stop_instance"), { instanceId });
    },

    closeAllInstances: async () => {
      return await invoke(commandFor(prefix, "close_all_instances"));
    },

    openInstanceWindow: async (instanceId) => {
      return await invoke(commandFor(prefix, "open_instance_window"), {
        instanceId,
      });
    },
  };
}
