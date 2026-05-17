import { invoke } from "@tauri-apps/api/core";
import { createPlatformInstanceService } from "./platform/createPlatformInstanceService";
import type {
  CodexSessionVisibilityRepairSummary,
  CodexInstanceThreadSyncSummary,
  CodexInstanceTargetThreadSyncSummary,
  CodexSessionRecord,
  CodexSessionTokenStats,
  CodexSessionTrashSummary,
  CodexTrashedSessionRecord,
  CodexSessionRestoreSummary,
  CodexQuickConfig,
  CodexAppSpeed,
} from "../types/codex";
import type { InstanceLaunchMode, InstanceProfile } from "../types/instance";

const service = createPlatformInstanceService("codex");

export const getInstanceDefaults = service.getInstanceDefaults;
export const listInstances = service.listInstances;
export const deleteInstance = service.deleteInstance;
export const startInstance = service.startInstance;
export const stopInstance = service.stopInstance;
export const closeAllInstances = service.closeAllInstances;
export const openInstanceWindow = service.openInstanceWindow;

export async function createInstance(payload: {
  name: string;
  userDataDir: string;
  workingDir?: string | null;
  extraArgs?: string;
  bindAccountId?: string | null;
  launchMode?: InstanceLaunchMode;
  appSpeed?: CodexAppSpeed;
  copySourceInstanceId: string;
  initMode?: "copy" | "empty" | "existingDir";
}): Promise<InstanceProfile> {
  return await invoke("codex_create_instance", {
    name: payload.name,
    userDataDir: payload.userDataDir,
    workingDir: payload.workingDir ?? null,
    extraArgs: payload.extraArgs ?? "",
    bindAccountId: payload.bindAccountId ?? null,
    launchMode: payload.launchMode ?? "app",
    appSpeed: payload.appSpeed ?? "standard",
    copySourceInstanceId: payload.copySourceInstanceId,
    initMode: payload.initMode ?? "copy",
  });
}

export async function updateInstance(payload: {
  instanceId: string;
  name?: string;
  workingDir?: string | null;
  extraArgs?: string;
  bindAccountId?: string | null;
  followLocalAccount?: boolean;
  launchMode?: InstanceLaunchMode;
  appSpeed?: CodexAppSpeed;
}): Promise<InstanceProfile> {
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
  if (payload.launchMode !== undefined) {
    body.launchMode = payload.launchMode;
  }
  if (payload.appSpeed !== undefined) {
    body.appSpeed = payload.appSpeed;
  }
  return await invoke("codex_update_instance", body);
}

export async function getCodexInstanceQuickConfig(
  instanceId: string,
): Promise<CodexQuickConfig> {
  return await invoke("codex_get_instance_quick_config", {
    instanceId,
  });
}

export async function saveCodexInstanceQuickConfig(
  instanceId: string,
  modelContextWindow?: number,
  autoCompactTokenLimit?: number,
): Promise<CodexQuickConfig> {
  return await invoke("codex_save_instance_quick_config", {
    instanceId,
    modelContextWindow: modelContextWindow ?? null,
    autoCompactTokenLimit: autoCompactTokenLimit ?? null,
  });
}

export async function openCodexInstanceConfigToml(
  instanceId: string,
): Promise<void> {
  return await invoke("codex_open_instance_config_toml", {
    instanceId,
  });
}

export interface CodexInstanceLaunchInfo {
  instanceId: string;
  userDataDir: string;
  launchCommand: string;
}

export async function getCodexInstanceLaunchCommand(
  instanceId: string,
): Promise<CodexInstanceLaunchInfo> {
  return await invoke("codex_get_instance_launch_command", { instanceId });
}

export async function executeCodexInstanceLaunchCommand(
  instanceId: string,
  terminal?: string,
): Promise<string> {
  return await invoke("codex_execute_instance_launch_command", {
    instanceId,
    terminal: terminal ?? null,
  });
}

export async function syncThreadsAcrossInstances(): Promise<CodexInstanceThreadSyncSummary> {
  return await invoke("codex_sync_threads_across_instances");
}

export async function syncSessionsToInstance(
  sessionIds: string[],
  targetInstanceId: string,
): Promise<CodexInstanceTargetThreadSyncSummary> {
  return await invoke("codex_sync_sessions_to_instance", {
    sessionIds,
    targetInstanceId,
  });
}

export async function repairSessionVisibilityAcrossInstances(): Promise<CodexSessionVisibilityRepairSummary> {
  return await invoke("codex_repair_session_visibility_across_instances");
}

export async function listSessionsAcrossInstances(): Promise<
  CodexSessionRecord[]
> {
  return await invoke("codex_list_sessions_across_instances");
}

export async function getSessionTokenStatsAcrossInstances(
  sessionIds: string[],
): Promise<CodexSessionTokenStats[]> {
  return await invoke("codex_get_session_token_stats_across_instances", {
    sessionIds,
  });
}

export async function moveSessionsToTrashAcrossInstances(
  sessionIds: string[],
): Promise<CodexSessionTrashSummary> {
  return await invoke("codex_move_sessions_to_trash_across_instances", {
    sessionIds,
  });
}

export async function listTrashedSessionsAcrossInstances(): Promise<
  CodexTrashedSessionRecord[]
> {
  return await invoke("codex_list_trashed_sessions_across_instances");
}

export async function restoreSessionsFromTrashAcrossInstances(
  sessionIds: string[],
): Promise<CodexSessionRestoreSummary> {
  return await invoke("codex_restore_sessions_from_trash_across_instances", {
    sessionIds,
  });
}
