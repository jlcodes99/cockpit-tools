import type { CodexApiProviderPreset } from "./codexProviderPresets";
import {
  findCodexApiProviderPresetById,
  resolveCodexApiProviderPresetId,
} from "./codexProviderPresets";

export type CodexProviderWireApi = "responses" | "chat_completions";
export type CodexProviderEnableMode = "direct" | "gateway";
export type CodexProviderEnableModePreference = "auto" | CodexProviderEnableMode;
export type CodexProviderAdapterProfile =
  | "openai_responses_native"
  | "openai_chat_completions_bridge";
export type CodexProviderGatewayStrategy =
  | "passthrough"
  | "responses_to_chat_completions";

export interface CodexProviderCapabilities {
  responses: boolean;
  chatCompletions: boolean;
  tools: boolean;
  reasoning: boolean;
  streamUsage: boolean;
  hotSwitch: boolean;
  requestLogs: boolean;
  failover: boolean;
}

export interface CodexProviderCapabilityProfile {
  wireApi: CodexProviderWireApi;
  adapterProfile: CodexProviderAdapterProfile;
  gatewayStrategy: CodexProviderGatewayStrategy;
  defaultEnableMode: CodexProviderEnableMode;
  requiresGateway: boolean;
  supportsDirect: boolean;
  capabilities: CodexProviderCapabilities;
}

const CHAT_COMPLETIONS_PRESET_IDS = new Set([
  "deepseek",
  "moonshot",
  "siliconflow",
  "siliconflow_en",
  "zhipu_glm",
  "zhipu_glm_en",
  "volcengine_agentplan",
  "byteplus",
  "doubaoseed",
  "qianfan_coding",
  "bailian",
  "stepfun",
  "stepfun_en",
  "modelscope",
  "longcat",
  "minimax",
  "minimax_en",
  "bailing",
  "xiaomi_mimo",
  "xiaomi_mimo_token_plan",
  "novita",
  "nvidia",
  "runapi",
  "relaxycode",
  "compshare_coding",
  "eflowcode",
  "pipellm",
  "therouter",
  "openrouter",
]);

export function resolveCodexProviderCapabilityProfile(input: {
  presetId?: string | null;
  baseUrl: string;
  wireApi?: CodexProviderWireApi | null;
}): CodexProviderCapabilityProfile {
  const presetId =
    input.presetId?.trim() || resolveCodexApiProviderPresetId(input.baseUrl);
  const inferredChatCompletions = CHAT_COMPLETIONS_PRESET_IDS.has(presetId);
  const wireApi =
    input.wireApi ?? (inferredChatCompletions ? "chat_completions" : "responses");

  if (wireApi === "chat_completions") {
    return {
      wireApi: "chat_completions",
      adapterProfile: "openai_chat_completions_bridge",
      gatewayStrategy: "responses_to_chat_completions",
      defaultEnableMode: "gateway",
      requiresGateway: true,
      supportsDirect: false,
      capabilities: {
        responses: false,
        chatCompletions: true,
        tools: true,
        reasoning: false,
        streamUsage: true,
        hotSwitch: true,
        requestLogs: true,
        failover: true,
      },
    };
  }

  return {
    wireApi: "responses",
    adapterProfile: "openai_responses_native",
    gatewayStrategy: "passthrough",
    defaultEnableMode: "direct",
    requiresGateway: false,
    supportsDirect: true,
    capabilities: {
      responses: true,
      chatCompletions: false,
      tools: true,
      reasoning: true,
      streamUsage: true,
      hotSwitch: false,
      requestLogs: false,
      failover: false,
    },
  };
}

export function resolveCodexProviderEnableMode(
  profile: CodexProviderCapabilityProfile,
  preference: CodexProviderEnableModePreference = "auto",
): CodexProviderEnableMode {
  if (preference === "gateway") return "gateway";
  if (preference === "direct" && profile.supportsDirect) return "direct";
  return profile.defaultEnableMode;
}

export function resolveCodexProviderPreset(input: {
  baseUrl: string;
}): CodexApiProviderPreset | null {
  return findCodexApiProviderPresetById(
    resolveCodexApiProviderPresetId(input.baseUrl),
  );
}
