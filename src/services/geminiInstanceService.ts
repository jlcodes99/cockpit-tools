import { invoke } from "@tauri-apps/api/core";
import { createPlatformInstanceService } from "./platform/createPlatformInstanceService";

const service = createPlatformInstanceService("gemini");

export const getInstanceDefaults = service.getInstanceDefaults;
export const listInstances = service.listInstances;
export const createInstance = service.createInstance;
export const updateInstance = service.updateInstance;
export const deleteInstance = service.deleteInstance;
export const startInstance = service.startInstance;
export const stopInstance = service.stopInstance;
export const closeAllInstances = service.closeAllInstances;
export const openInstanceWindow = service.openInstanceWindow;

export interface GeminiInstanceLaunchInfo {
  instanceId: string;
  userDataDir: string;
  launchCommand: string;
}

export async function getGeminiInstanceLaunchCommand(
  instanceId: string,
): Promise<GeminiInstanceLaunchInfo> {
  return await invoke("gemini_get_instance_launch_command", { instanceId });
}

export async function executeGeminiInstanceLaunchCommand(
  instanceId: string,
  terminal?: string,
): Promise<string> {
  return await invoke("gemini_execute_instance_launch_command", {
    instanceId,
    terminal: terminal ?? null,
  });
}
