import { withCliLaunchPath } from '../utils/cliLaunchPath';
import { invoke } from '@tauri-apps/api/core';

export type CodebuddyCliRegion = 'international' | 'china';

export interface CodebuddyCliAccountRef {
  region: CodebuddyCliRegion;
  accountId: string;
}

export interface CodebuddyCliInstance {
  id: string;
  name: string;
  account: CodebuddyCliAccountRef;
  workingDir: string;
}

export type CodebuddyCliInstanceDraft = Omit<CodebuddyCliInstance, 'id'> & { id?: string };

export type CodebuddyCliLaunchTarget =
  | { kind: 'instance'; id: string; workingDir?: string }
  | { kind: 'account'; account: CodebuddyCliAccountRef; workingDir?: string; rememberWorkingDir?: boolean };

export const listCodebuddyCliInstances = () =>
  invoke<CodebuddyCliInstance[]>('codebuddy_cli_list_instances');

export const saveCodebuddyCliInstance = (draft: CodebuddyCliInstanceDraft) =>
  invoke<CodebuddyCliInstance[]>('codebuddy_cli_save_instance', { draft });

export const removeCodebuddyCliInstance = (id: string) =>
  invoke<CodebuddyCliInstance[]>('codebuddy_cli_remove_instance', { id });

export const launchCodebuddyCli = (target: CodebuddyCliLaunchTarget, terminal?: string, customCommand?: string) =>
  withCliLaunchPath('codebuddy_cli', () => invoke<void>('codebuddy_cli_launch', { target, terminal, customCommand }),
    (error) => error === 'cliMissing');

export const getCodebuddyCliLaunchCommand = (target: CodebuddyCliLaunchTarget, customCommand?: string) =>
  withCliLaunchPath('codebuddy_cli', () => invoke<string>('codebuddy_cli_launch_command', { target, customCommand }),
    (error) => error === 'cliMissing');

export const isCodebuddyCliAvailable = () =>
  invoke<boolean>('codebuddy_cli_available');

export const getCodebuddyCliWorkingDir = (account: CodebuddyCliAccountRef) =>
  invoke<string | null>('codebuddy_cli_working_dir', { account });
