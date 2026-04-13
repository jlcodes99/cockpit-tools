import { invoke } from "@tauri-apps/api/core";
import { createPlatformInstanceService } from "./platform/createPlatformInstanceService";
import type {
  CodexSessionVisibilityRepairSummary,
  CodexInstanceThreadSyncSummary,
  CodexSessionRecord,
  CodexSessionTrashSummary,
  CodexTrashedSessionRecord,
  CodexSessionRestoreSummary,
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
  return await invoke("codex_update_instance", body);
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

export async function repairSessionVisibilityAcrossInstances(): Promise<CodexSessionVisibilityRepairSummary> {
  return await invoke("codex_repair_session_visibility_across_instances");
}

export async function listSessionsAcrossInstances(): Promise<
  CodexSessionRecord[]
> {
  return await invoke("codex_list_sessions_across_instances");
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
