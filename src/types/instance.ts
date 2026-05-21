import type { CodexAppSpeed } from "./codex";

export type InstanceLaunchMode = "app" | "cli";

export const CODEX_API_SERVICE_BIND_ID = "__api_service__";

export interface InstanceProfile {
  id: string;
  name: string;
  userDataDir: string;
  workingDir?: string | null;
  extraArgs: string;
  bindAccountId?: string | null;
  launchMode?: InstanceLaunchMode;
  appSpeed?: CodexAppSpeed;
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
