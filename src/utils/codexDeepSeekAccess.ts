import type { CodexAccount } from "../types/codex";
import {
  buildCodexProviderGatewayBindId,
  CODEX_PROVIDER_GATEWAY_BIND_PREFIX,
} from "../types/instance.ts";
import {
  DEEPSEEK_API_BASE_URL,
  DEEPSEEK_API_PROVIDER_ID,
  DEEPSEEK_CODEX_MODEL_CATALOG,
} from "./codexProviderPresets.ts";
import {
  isCodexApiKeyAccount,
  isCodexChatCompletionsApiKeyAccount,
  isCodexNewApiAccount,
} from "../types/codex.ts";

export const DEEPSEEK_ACCESS_MODE_GATEWAY = "gateway";
export const DEEPSEEK_ACCESS_MODE_DIRECT = "direct";
export const DEEPSEEK_ACCESS_MODE_CDP = "cdp";
export const DEEPSEEK_DEFAULT_STARTUP_MODEL = "deepseek-v4-flash";

export const DEEPSEEK_DIRECT_MODELS = [
  {
    id: "deepseek-v4-flash",
    label: "DeepSeek-V4-Flash",
  },
  {
    id: "deepseek-v4-pro",
    label: "DeepSeek-V4-Pro",
  },
] as const;

export type DeepSeekAccessMode = "gateway" | "direct" | "cdp";

export type DeepSeekStartTarget = Pick<
  CodexAccount,
  | "api_provider_id"
  | "api_base_url"
  | "api_wire_api"
  | "api_instance_access_mode"
  | "api_startup_model"
> & {
  id?: string;
};

export type DeepSeekStartChoice = {
  accessMode: DeepSeekAccessMode;
  modelId: string;
};

export function isCodexTokenPlanAccount(
  account: Pick<CodexAccount, "api_provider_id" | "api_base_url">,
): boolean {
  const providerId = (account.api_provider_id || "").trim().toLowerCase();
  if (
    providerId === "minimax" ||
    providerId === "minimax-cn" ||
    providerId === "minimax-portal" ||
    providerId === "minimax-portal-cn" ||
    providerId === "zhipu" ||
    providerId === "zai" ||
    providerId === "zai-coding-plan"
  ) {
    return true;
  }
  try {
    const host = new URL((account.api_base_url || "").trim()).hostname.toLowerCase();
    return [
      "api.minimaxi.com",
      "www.minimaxi.com",
      "api.minimax.io",
      "www.minimax.io",
      "open.bigmodel.cn",
      "bigmodel.cn",
      "api.z.ai",
      "z.ai",
    ].includes(host);
  } catch {
    return false;
  }
}

export function isDeepSeekAccount(
  account: Pick<CodexAccount, "api_provider_id" | "api_base_url">,
): boolean {
  const providerId = (account.api_provider_id || "").trim().toLowerCase();
  if (providerId === DEEPSEEK_API_PROVIDER_ID) {
    return true;
  }
  const baseUrl = (account.api_base_url || "").trim().toLowerCase();
  return (
    baseUrl.includes("api.deepseek.com") ||
    baseUrl === DEEPSEEK_API_BASE_URL.toLowerCase()
  );
}

export function isCodexApiKeyUsageQueryEligible(
  account: CodexAccount,
): boolean {
  const tokenPlan = isCodexTokenPlanAccount(account);
  return (
    isCodexApiKeyAccount(account) &&
    !isCodexNewApiAccount(account) &&
    (!isCodexChatCompletionsApiKeyAccount(account) ||
      isDeepSeekAccount(account) ||
      tokenPlan) &&
    Boolean(account.openai_api_key?.trim())
  );
}

export function shouldShowCodexApiKeyUsagePanel(
  account: CodexAccount,
  hideRelayQuota = false,
): boolean {
  if (!isCodexApiKeyAccount(account) || isCodexNewApiAccount(account)) {
    return false;
  }
  const deepseek = isDeepSeekAccount(account);
  const tokenPlan = isCodexTokenPlanAccount(account);
  if (isCodexChatCompletionsApiKeyAccount(account) && !deepseek && !tokenPlan) {
    return false;
  }
  return !hideRelayQuota || deepseek || tokenPlan;
}

export function isDeepSeekResponsesAccount(
  account: Pick<CodexAccount, "api_provider_id" | "api_base_url" | "api_wire_api">,
): boolean {
  if (!isDeepSeekAccount(account)) {
    return false;
  }
  const wire = (account.api_wire_api || "responses").trim().toLowerCase();
  return wire !== "chat_completions";
}

export function resolveDeepSeekAccessMode(
  account?: DeepSeekStartTarget | null,
): DeepSeekAccessMode {
  if (!account || !isDeepSeekResponsesAccount(account)) {
    return DEEPSEEK_ACCESS_MODE_GATEWAY;
  }
  const mode = (account.api_instance_access_mode || "").trim().toLowerCase();
  if (mode === DEEPSEEK_ACCESS_MODE_DIRECT) {
    return DEEPSEEK_ACCESS_MODE_DIRECT;
  }
  if (mode === DEEPSEEK_ACCESS_MODE_CDP) {
    return DEEPSEEK_ACCESS_MODE_CDP;
  }
  return DEEPSEEK_ACCESS_MODE_GATEWAY;
}

export function isDeepSeekDirectAccess(
  account: Pick<
    CodexAccount,
    "api_provider_id" | "api_base_url" | "api_wire_api" | "api_instance_access_mode"
  >,
): boolean {
  return resolveDeepSeekAccessMode(account) === DEEPSEEK_ACCESS_MODE_DIRECT;
}

export function isDeepSeekCdpAccess(
  account: Pick<
    CodexAccount,
    "api_provider_id" | "api_base_url" | "api_wire_api" | "api_instance_access_mode"
  >,
): boolean {
  return resolveDeepSeekAccessMode(account) === DEEPSEEK_ACCESS_MODE_CDP;
}

export function shouldUseDeepSeekProviderGateway(
  account: Pick<
    CodexAccount,
    "api_provider_id" | "api_base_url" | "api_wire_api" | "api_instance_access_mode"
  >,
): boolean {
  return (
    isDeepSeekAccount(account) &&
    !isDeepSeekDirectAccess(account) &&
    !isDeepSeekCdpAccess(account)
  );
}

export function resolveDeepSeekStartupModel(
  account?: Pick<CodexAccount, "api_startup_model"> | null,
): string {
  const model = (account?.api_startup_model || "").trim().toLowerCase();
  if ((DEEPSEEK_CODEX_MODEL_CATALOG as readonly string[]).includes(model)) {
    return model;
  }
  return DEEPSEEK_DEFAULT_STARTUP_MODEL;
}

export function resolveDeepSeekBindAccountId(
  account: Pick<
    CodexAccount,
    | "id"
    | "api_provider_id"
    | "api_base_url"
    | "api_wire_api"
    | "api_instance_access_mode"
  >,
): string {
  if (shouldUseDeepSeekProviderGateway(account)) {
    return buildCodexProviderGatewayBindId(account.id);
  }
  return account.id;
}

export function parseCodexBoundAccountId(
  bindAccountId?: string | null,
): string | null {
  const bindId = (bindAccountId || "").trim();
  if (!bindId) {
    return null;
  }
  if (bindId.startsWith(CODEX_PROVIDER_GATEWAY_BIND_PREFIX)) {
    return bindId.slice(CODEX_PROVIDER_GATEWAY_BIND_PREFIX.length) || null;
  }
  return bindId;
}
