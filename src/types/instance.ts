export type InstanceLaunchMode = "app" | "cli";

export interface InstanceProfile {
  id: string;
  name: string;
  userDataDir: string;
  workingDir?: string | null;
  extraArgs: string;
  bindAccountId?: string | null;
  launchMode?: InstanceLaunchMode;
  createdAt: number;
  lastLaunchedAt?: number | null;
  lastPid?: number | null;
  running: boolean;
  initialized?: boolean;
  isDefault?: boolean;
  followLocalAccount?: boolean;
}

export type InstanceInitMode = "copy" | "empty" | "existingDir";

export interface InstanceDefaults {
  rootDir: string;
  defaultUserDataDir: string;
}
