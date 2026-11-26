import { invoke } from '@tauri-apps/api/core';

export type CodebuddyCliRegion = 'international' | 'china';

export interface CodebuddyCliAccountRef {
  region: CodebuddyCliRegion;
  accountId: string;
}

export type CodebuddyCliLaunchTarget = {
  kind: 'account';
  account: CodebuddyCliAccountRef;
  workingDir?: string;
  rememberWorkingDir?: boolean;
};

export const launchCodebuddyCli = (target: CodebuddyCliLaunchTarget, terminal?: string, customCommand?: string) =>
  invoke<void>('codebuddy_cli_launch', { target, terminal, customCommand });

export const getCodebuddyCliLaunchCommand = (target: CodebuddyCliLaunchTarget, customCommand?: string) =>
  invoke<string>('codebuddy_cli_launch_command', { target, customCommand });

export const getCodebuddyCliWorkingDir = (account: CodebuddyCliAccountRef) =>
  invoke<string | null>('codebuddy_cli_working_dir', { account });
