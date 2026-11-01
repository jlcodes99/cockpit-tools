import { invoke } from '@tauri-apps/api/core';
import { withClaudeProfileErrors } from './claudeProfileErrors';
import { createPlatformInstanceService } from './platform/createPlatformInstanceService';

const service = createPlatformInstanceService('claude');

export const getInstanceDefaults = service.getInstanceDefaults;
export const listInstances = service.listInstances;
export const createInstance = (...args: Parameters<typeof service.createInstance>) =>
  withClaudeProfileErrors(() => service.createInstance(...args));
export const updateInstance = (...args: Parameters<typeof service.updateInstance>) =>
  withClaudeProfileErrors(() => service.updateInstance(...args));
export const deleteInstance = service.deleteInstance;
export const startInstance = (...args: Parameters<typeof service.startInstance>) =>
  withClaudeProfileErrors(() => service.startInstance(...args));
export const stopInstance = service.stopInstance;
export const closeAllInstances = service.closeAllInstances;
export const openInstanceWindow = service.openInstanceWindow;

export interface ClaudeInstanceLaunchInfo {
  instanceId: string;
  userDataDir: string;
  launchCommand: string;
}

export async function getClaudeInstanceLaunchCommand(
  instanceId: string,
): Promise<ClaudeInstanceLaunchInfo> {
  return await invoke('claude_get_instance_launch_command', { instanceId });
}

export async function executeClaudeInstanceLaunchCommand(
  instanceId: string,
  terminal?: string,
): Promise<string> {
  return await invoke('claude_execute_instance_launch_command', {
    instanceId,
    terminal: terminal ?? null,
  });
}
