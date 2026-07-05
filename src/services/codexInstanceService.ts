import { invoke } from "@tauri-apps/api/core";
import { createPlatformInstanceService } from "./platform/createPlatformInstanceService";
import type {
  CodexSessionVisibilityRepairInstanceList,
  CodexSessionVisibilityRepairProviderList,
  CodexSessionVisibilityRepairRequestOptions,
  CodexSessionVisibilityRepairSummary,
  CodexInstanceThreadSyncSummary,
  CodexInstanceTargetThreadSyncSummary,
  CodexSessionRecord,
  CodexSessionSearchOptions,
  CodexSessionTokenStats,
  CodexSessionTrashSummary,
  CodexTrashedSessionRecord,
  CodexSessionRestoreSummary,
  CodexSessionTrashDeleteSummary,
  CodexSessionExportPreview,
  CodexSessionExportSummary,
  CodexSessionImportPreview,
  CodexSessionImportSummary,
  CodexQuickConfig,
  CodexAppSpeed,
} from "../types/codex";
import type { InstanceLaunchMode, InstanceProfile } from "../types/instance";

const service = createPlatformInstanceService("codex");

export const getInstanceDefaults = service.getInstanceDefaults;
export const listInstances = service.listInstances;
export const deleteInstance = service.deleteInstance;
export async function startInstance(instanceId: string): Promise<InstanceProfile> {
  const startedAt = performance.now();
  console.info("[Codex Start][Service] invoke codex_start_instance started", {
    instanceId,
  });
  try {
    return await service.startInstance(instanceId);
  } finally {
    console.info("[Codex Start][Service] invoke codex_start_instance finished", {
      instanceId,
      elapsedMs: Math.round(performance.now() - startedAt),
    });
  }
}
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
  autoSyncThreads?: boolean;
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
  if (payload.autoSyncThreads !== undefined) {
    body.autoSyncThreads = payload.autoSyncThreads;
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

export async function repairSessionVisibilityAcrossInstances(
  runId?: string,
  options?: CodexSessionVisibilityRepairRequestOptions,
): Promise<CodexSessionVisibilityRepairSummary> {
  return await invoke("codex_repair_session_visibility_across_instances", {
    mode: options?.mode ?? "quick",
    runId: runId ?? null,
    dryRun: options?.dryRun ?? false,
    targetProvider: options?.targetProvider ?? null,
    targetInstanceId: options?.targetInstanceId ?? null,
    repairInstanceIds: options?.repairInstanceIds ?? null,
    sessionIds: options?.sessionIds ?? null,
  });
}

export async function listSessionVisibilityRepairInstances(): Promise<
  CodexSessionVisibilityRepairInstanceList
> {
  return await invoke("codex_list_session_visibility_repair_instances");
}

export async function listSessionVisibilityRepairProviders(): Promise<
  CodexSessionVisibilityRepairProviderList
> {
  return await invoke("codex_list_session_visibility_repair_providers");
}

export async function listSessionsAcrossInstances(
  options: CodexSessionSearchOptions = {},
): Promise<
  CodexSessionRecord[]
> {
  return await invoke("codex_list_sessions_across_instances", {
    titleQuery: options.titleQuery?.trim() || null,
    contentQuery: options.contentQuery?.trim() || null,
  });
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

export async function deleteTrashedSessionsAcrossInstances(
  sessionIds: string[],
): Promise<CodexSessionTrashDeleteSummary> {
  return await invoke("codex_delete_trashed_sessions_across_instances", {
    sessionIds,
  });
}

export async function emptySessionTrashAcrossInstances(): Promise<CodexSessionTrashDeleteSummary> {
  return await invoke("codex_empty_session_trash_across_instances");
}

export async function previewSessionExport(
  sessionIds: string[],
): Promise<CodexSessionExportPreview> {
  return await invoke("codex_preview_session_export", {
    sessionIds,
  });
}

export async function exportSessions(
  sessionIds: string[],
  exportPath: string,
  transferId?: string | null,
): Promise<CodexSessionExportSummary> {
  return await invoke("codex_export_sessions", {
    sessionIds,
    exportPath,
    transferId: transferId ?? null,
  });
}

export async function previewSessionImport(
  importFilePath: string,
  targetInstanceId?: string | null,
): Promise<CodexSessionImportPreview> {
  return await invoke("codex_preview_session_import", {
    importFilePath,
    targetInstanceId: targetInstanceId ?? null,
  });
}

export async function importSessions(
  importFilePath: string,
  targetInstanceId: string,
  sessionIds: string[],
  transferId?: string | null,
): Promise<CodexSessionImportSummary> {
  return await invoke("codex_import_sessions", {
    importFilePath,
    targetInstanceId,
    sessionIds,
    transferId: transferId ?? null,
  });
}

export async function openSessionLocation(sessionId: string): Promise<void> {
  return await invoke("codex_open_session_location", {
    sessionId,
  });
}
