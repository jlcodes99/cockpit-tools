import type { CodexAppSpeed } from "./codex.ts";

export type InstanceLaunchMode = "app" | "cli";

export const CODEX_API_SERVICE_BIND_ID = "__api_service__";
export const CODEX_PROVIDER_GATEWAY_BIND_PREFIX = "__provider_gateway__:";
export const CODEX_PROVIDER_GATEWAY_INTERNAL_NAMESPACE = "__provider_gateway__";

export function buildCodexProviderGatewayBindId(accountId: string): string {
  return `${CODEX_PROVIDER_GATEWAY_BIND_PREFIX}${accountId}`;
}

export type CodexLaunchCredentialType = "api" | "account";

export interface CodexLaunchCredentialChange {
  from: CodexLaunchCredentialType;
  to: CodexLaunchCredentialType;
}

export interface CodexInstanceApiRoute {
  id: string;
  namespace: string;
  providerAccountId: string;
  enabled: boolean;
  selectedModels?: string[];
  extraModels?: string[];
}

export interface CodexInstanceModelRouting {
  enabled: boolean;
  version: number;
  routes: CodexInstanceApiRoute[];
}

export interface InstanceProfile {
  id: string;
  name: string;
  userDataDir: string;
  workingDir?: string | null;
  extraArgs: string;
  bindAccountId?: string | null;
  modelRouting?: CodexInstanceModelRouting | null;
  launchMode?: InstanceLaunchMode;
  appSpeed?: CodexAppSpeed;
  createdAt: number;
  lastLaunchedAt?: number | null;
  lastPid?: number | null;
  running: boolean;
  initialized?: boolean;
  isDefault?: boolean;
  followLocalAccount?: boolean;
  autoSyncThreads?: boolean;
  codexLaunchCredentialChange?: CodexLaunchCredentialChange | null;
}

export type InstanceInitMode = "copy" | "empty" | "existingDir";

export interface InstanceDefaults {
  rootDir: string;
  defaultUserDataDir: string;
}
