import { invoke } from "@tauri-apps/api/core";
import type {
  CodebuddyLocalAccessCollection,
  CodebuddyLocalAccessLogPage,
  CodebuddyLocalAccessState,
  CodebuddyLocalAccessStats,
} from "../types/codebuddyLocalAccess";

export async function getCodebuddyLocalAccessState(): Promise<CodebuddyLocalAccessState> {
  return await invoke("codebuddy_local_access_get_state");
}

export async function saveCodebuddyLocalAccessCollection(
  collection: CodebuddyLocalAccessCollection,
): Promise<CodebuddyLocalAccessState> {
  return await invoke("codebuddy_local_access_save_collection", { collection });
}

export async function setCodebuddyLocalAccessEnabled(
  enabled: boolean,
): Promise<CodebuddyLocalAccessState> {
  return await invoke("codebuddy_local_access_set_enabled", { enabled });
}

export async function startCodebuddyLocalAccess(): Promise<CodebuddyLocalAccessState> {
  return await invoke("codebuddy_local_access_start");
}

export async function stopCodebuddyLocalAccess(): Promise<CodebuddyLocalAccessState> {
  return await invoke("codebuddy_local_access_stop");
}

export async function testCodebuddyLocalAccess(): Promise<string> {
  return await invoke("codebuddy_local_access_test");
}

export async function getCodebuddyLocalAccessStats(): Promise<CodebuddyLocalAccessStats> {
  return await invoke("codebuddy_local_access_get_stats");
}

export async function clearCodebuddyLocalAccessStats(): Promise<CodebuddyLocalAccessStats> {
  return await invoke("codebuddy_local_access_clear_stats");
}

export async function getCodebuddyLocalAccessLogs(
  page: number,
  pageSize: number,
  modelFilter?: string,
  apiKeyFilter?: string,
  successFilter?: boolean,
): Promise<CodebuddyLocalAccessLogPage> {
  return await invoke("codebuddy_local_access_get_logs", {
    page,
    pageSize,
    modelFilter: modelFilter ?? null,
    apiKeyFilter: apiKeyFilter ?? null,
    successFilter: successFilter ?? null,
  });
}

export async function createCodebuddyLocalAccessApiKey(
  name: string,
  accountIds?: string[] | null,
): Promise<CodebuddyLocalAccessState> {
  return await invoke("codebuddy_local_access_create_api_key", {
    name,
    accountIds: accountIds ?? null,
  });
}

export async function updateCodebuddyLocalAccessApiKey(
  id: string,
  options: {
    name?: string;
    enabled?: boolean;
    accountIds?: string[] | null;
  },
): Promise<CodebuddyLocalAccessState> {
  return await invoke("codebuddy_local_access_update_api_key", {
    id,
    name: options.name ?? null,
    enabled: options.enabled ?? null,
    accountIds: options.accountIds ?? null,
  });
}

export async function rotateCodebuddyLocalAccessApiKey(
  id: string,
): Promise<CodebuddyLocalAccessState> {
  return await invoke("codebuddy_local_access_rotate_api_key", { id });
}

export async function deleteCodebuddyLocalAccessApiKey(
  id: string,
): Promise<CodebuddyLocalAccessState> {
  return await invoke("codebuddy_local_access_delete_api_key", { id });
}

export interface CodebuddyLocalAccessChatMessage {
  role: string;
  content: string;
}

export async function chatTestCodebuddyLocalAccess(
  model: string,
  messages: CodebuddyLocalAccessChatMessage[],
): Promise<Record<string, unknown>> {
  return await invoke("codebuddy_local_access_chat_test", { model, messages });
}

/**
 * 强制回收占用指定端口的进程。
 *
 * 用于 sidecar 异常退出导致端口残留时的清理。返回被成功回收的进程数量。
 */
export async function killCodebuddyLocalAccessPort(port: number): Promise<number> {
  return await invoke("codebuddy_local_access_kill_port", { port });
}
