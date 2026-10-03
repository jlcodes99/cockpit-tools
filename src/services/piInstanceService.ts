import { invoke } from '@tauri-apps/api/core';
import { createPlatformInstanceService } from './platform/createPlatformInstanceService';

const service = createPlatformInstanceService('pi');

export const getInstanceDefaults = service.getInstanceDefaults;
export const listInstances = service.listInstances;
export const createInstance = service.createInstance;
export const updateInstance = service.updateInstance;
export const deleteInstance = service.deleteInstance;
export const startInstance = service.startInstance;
export const stopInstance = service.stopInstance;
export const closeAllInstances = service.closeAllInstances;
export const openInstanceWindow = service.openInstanceWindow;

export interface PiInstanceLaunchInfo {
  instanceId: string;
  userDataDir: string;
  launchCommand: string;
  warning?: string | null;
}

/** Only show the install guide when the CLI is missing / path invalid. */
export function isPiCliMissingError(error: unknown): boolean {
  const text = String(error ?? '').toLowerCase();
  return (
    text.includes('未检测到 pi cli') ||
    text.includes('pi cli 路径不存在') ||
    text.includes('请先通过官方安装脚本安装')
  );
}

export interface PiCliStatus {
  available: boolean;
  binaryPath?: string | null;
  configuredPath?: string | null;
  version?: string | null;
  source?: string | null;
  message?: string | null;
  checkedAt: number;
}

export async function getPiCliStatus(): Promise<PiCliStatus> {
  return await invoke('pi_get_cli_status');
}

export async function updatePiCliRuntimeConfig(
  piCliPath?: string | null,
): Promise<PiCliStatus> {
  return await invoke('pi_update_cli_runtime_config', {
    piCliPath: piCliPath?.trim() || null,
  });
}

export async function executePiCliInstallCommand(terminal?: string): Promise<void> {
  await invoke('pi_execute_cli_install_command', { terminal: terminal ?? null });
}

/** Opens a terminal running `pi` so the user can run `/login`. */
export async function executePiLoginCommand(terminal?: string): Promise<void> {
  await invoke('pi_execute_login_command', { terminal: terminal ?? null });
}

export async function getPiInstanceLaunchCommand(
  instanceId: string,
  options?: {
    workingDir?: string | null;
    applyWorkingDirOverride?: boolean;
    /** Use this account's isolated PI_CODING_AGENT_DIR. */
    accountId?: string | null;
  },
): Promise<PiInstanceLaunchInfo> {
  return await invoke('pi_get_instance_launch_command', {
    instanceId,
    workingDir: options?.workingDir?.trim() || null,
    applyWorkingDirOverride: options?.applyWorkingDirOverride ?? false,
    accountId: options?.accountId?.trim() || null,
  });
}

export async function executePiInstanceLaunchCommand(
  instanceId: string,
  terminal?: string,
  options?: {
    workingDir?: string | null;
    applyWorkingDirOverride?: boolean;
    accountId?: string | null;
  },
): Promise<string> {
  return await invoke('pi_execute_instance_launch_command', {
    instanceId,
    terminal: terminal ?? null,
    workingDir: options?.workingDir?.trim() || null,
    applyWorkingDirOverride: options?.applyWorkingDirOverride ?? false,
    accountId: options?.accountId?.trim() || null,
  });
}
