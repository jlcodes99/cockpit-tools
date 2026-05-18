import { invoke } from "@tauri-apps/api/core";

export type CodexProxyServiceType = "openai" | "claude" | "gemini" | "responses";

export interface CodexProxyGatewayConfig {
  gatewayBaseUrl: string;
  proxyAccessKey: string;
  adminAccessKey: string;
  binaryPath: string;
  autoStart: boolean;
}

export interface CodexProxyGatewayHealth {
  running: boolean;
  gatewayBaseUrl: string;
  status?: number | null;
  message: string;
}

export interface CodexProxyModelMapping {
  source: string;
  target: string;
}

export interface CodexProxyResponsesChannelInput {
  gatewayBaseUrl: string;
  proxyAccessKey: string;
  adminAccessKey: string;
  channelIndex?: number | null;
  name: string;
  serviceType: CodexProxyServiceType;
  upstreamBaseUrl: string;
  upstreamApiKey: string;
  modelMapping: CodexProxyModelMapping[];
  insecureSkipVerify: boolean;
  lowQuality: boolean;
  autoBlacklistBalance: boolean;
  normalizeMetadataUserId: boolean;
  normalizeNonstandardChatRoles: boolean;
  codexToolCompat: boolean;
}

export interface CodexProxyUpsertChannelResult {
  created: boolean;
  channelIndex?: number | null;
  codexBaseUrl: string;
  proxyAccessKey: string;
  routePrefix: string;
}

export interface CodexProxyResponsesChannel {
  index: number;
  name: string;
  routePrefix: string;
  serviceType: CodexProxyServiceType;
  upstreamBaseUrl: string;
  upstreamApiKey: string;
  modelMapping: CodexProxyModelMapping[];
  insecureSkipVerify: boolean;
  lowQuality: boolean;
  autoBlacklistBalance: boolean;
  normalizeMetadataUserId: boolean;
  normalizeNonstandardChatRoles: boolean;
  codexToolCompat: boolean;
  status: string;
}

export async function loadCodexProxyGatewayConfig(): Promise<CodexProxyGatewayConfig> {
  return await invoke("codex_proxy_load_config");
}

export async function saveCodexProxyGatewayConfig(
  config: CodexProxyGatewayConfig,
): Promise<CodexProxyGatewayConfig> {
  return await invoke("codex_proxy_save_config", { config });
}

export async function checkCodexProxyGatewayHealth(
  gatewayBaseUrl?: string,
): Promise<CodexProxyGatewayHealth> {
  return await invoke("codex_proxy_health_check", {
    gatewayBaseUrl: gatewayBaseUrl ?? null,
  });
}

export async function startCodexProxyGateway(): Promise<CodexProxyGatewayHealth> {
  return await invoke("codex_proxy_start");
}

export async function upsertCodexProxyResponsesChannel(
  input: CodexProxyResponsesChannelInput,
): Promise<CodexProxyUpsertChannelResult> {
  return await invoke("codex_proxy_upsert_responses_channel", { input });
}

export async function listCodexProxyResponsesChannels(input?: {
  gatewayBaseUrl?: string;
  proxyAccessKey?: string;
  adminAccessKey?: string;
}): Promise<CodexProxyResponsesChannel[]> {
  return await invoke("codex_proxy_list_responses_channels", {
    gatewayBaseUrl: input?.gatewayBaseUrl ?? null,
    proxyAccessKey: input?.proxyAccessKey ?? null,
    adminAccessKey: input?.adminAccessKey ?? null,
  });
}
