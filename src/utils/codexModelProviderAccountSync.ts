import type {
  CodexAccount,
  CodexApiProviderMode,
  CodexProviderWireApi,
} from "../types/codex";
import type { CodexModelProvider } from "../services/codexModelProviderService";
import {
  CODEX_API_PROVIDER_CUSTOM_ID,
  resolveCodexApiProviderPresetId,
} from "./codexProviderPresets";
import { resolveCodexProviderCapabilityProfile } from "./codexProviderGateway";
import { resolveCodexModelProviderAccountName } from "./codexModelProviderAccountName";

export interface CodexModelProviderReference {
  id: string;
  baseUrl: string;
}

export interface CodexModelProviderAccountSnapshot {
  apiBaseUrl: string;
  apiProviderMode: CodexApiProviderMode;
  apiProviderId: string;
  apiProviderName: string;
  apiModelCatalog?: string[];
  apiModelContextWindows?: Record<string, number>;
  apiWireApi: CodexProviderWireApi;
  apiSupportsWebsockets: boolean;
  apiSupportsVision: boolean;
  apiModelVisionSupport: Record<string, boolean>;
  apiVisionRoutingModel?: string;
  accountName: string;
}

export function mergeCodexModelProviderCredentialInput(
  provider: CodexModelProvider | null,
  fallback: {
    providerId?: string | null;
    previousProviderId?: string | null;
    providerName?: string | null;
    apiBaseUrl: string;
    apiKey: string;
    apiKeyName?: string | null;
    sourceTag?: string | null;
    modelCatalog?: string[];
    modelContextWindows?: Record<string, number>;
    supportsVision?: boolean;
    modelCapabilities?: Record<string, { supportsVision?: boolean }>;
    visionRoutingModel?: string | null;
    website?: string | null;
    apiKeyUrl?: string | null;
    wireApi?: CodexProviderWireApi | null;
    supportsWebsockets?: boolean;
    integrationType?: "sub2api" | "new_api" | null;
  },
) {
  return {
    ...fallback,
    providerId: provider?.id ?? fallback.providerId,
    providerName: provider?.name ?? fallback.providerName,
    apiBaseUrl: provider?.baseUrl ?? fallback.apiBaseUrl,
    apiKeyName: provider?.apiKeys.find(
      (item) => item.apiKey.trim() === fallback.apiKey.trim(),
    )?.name ?? fallback.apiKeyName,
    sourceTag: provider?.sourceTag ?? fallback.sourceTag,
    modelCatalog: fallback.modelCatalog ?? provider?.modelCatalog,
    modelContextWindows:
      fallback.modelContextWindows ?? provider?.modelContextWindows,
    supportsVision: provider?.supportsVision ?? fallback.supportsVision,
    modelCapabilities: provider?.modelCapabilities ?? fallback.modelCapabilities,
    visionRoutingModel:
      provider?.visionRoutingModel ?? fallback.visionRoutingModel,
    website: provider?.website ?? fallback.website,
    apiKeyUrl: provider?.apiKeyUrl ?? fallback.apiKeyUrl,
    wireApi:
      fallback.wireApi ??
      provider?.wireApi ??
      resolveCodexProviderCapabilityProfile({
        baseUrl: provider?.baseUrl ?? fallback.apiBaseUrl,
      }).wireApi,
    supportsWebsockets:
      provider?.supportsWebsockets ?? fallback.supportsWebsockets,
    integrationType: provider?.integrationType ?? fallback.integrationType,
  };
}

function normalizeBaseUrl(value?: string | null): string | null {
  const trimmed = value?.trim();
  if (!trimmed) return null;
  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") return null;
    return `${parsed.origin}${parsed.pathname}`.replace(/\/+$/, "").toLowerCase();
  } catch {
    return null;
  }
}

export function findCodexAccountsReferencingModelProvider(
  provider: CodexModelProviderReference,
  accounts: CodexAccount[],
): string[] {
  const providerId = provider.id.trim();
  const providerBaseUrl = normalizeBaseUrl(provider.baseUrl);

  return accounts
    .filter((account) => {
      if ((account.auth_mode ?? "").toLowerCase() !== "apikey") return false;
      if (!account.openai_api_key?.trim()) return false;

      const matchesId =
        providerId.length > 0 && account.api_provider_id?.trim() === providerId;
      const accountBaseUrl = normalizeBaseUrl(account.api_base_url);
      const matchesBaseUrl =
        providerBaseUrl !== null && accountBaseUrl === providerBaseUrl;
      return matchesId || matchesBaseUrl;
    })
    .map((account) => account.id);
}

export function buildCodexModelProviderAccountSnapshot(
  provider: CodexModelProvider,
  apiKeyName?: string | null,
): CodexModelProviderAccountSnapshot {
  const presetId = resolveCodexApiProviderPresetId(provider.baseUrl);
  const isOpenAI = presetId === "openai_official";
  const wireApi = resolveCodexProviderCapabilityProfile({
    presetId,
    baseUrl: provider.baseUrl,
    wireApi: provider.wireApi,
  }).wireApi;

  return {
    apiBaseUrl: provider.baseUrl,
    apiProviderMode: "custom",
    apiProviderId:
      presetId === CODEX_API_PROVIDER_CUSTOM_ID ? provider.id : presetId,
    apiProviderName: provider.name,
    apiModelCatalog: provider.modelCatalog,
    apiModelContextWindows: provider.modelContextWindows,
    apiWireApi: wireApi,
    apiSupportsWebsockets:
      !isOpenAI && wireApi === "responses" && provider.supportsWebsockets === true,
    apiSupportsVision: provider.supportsVision === true,
    apiModelVisionSupport: Object.fromEntries(
      Object.entries(provider.modelCapabilities ?? {}).map(
        ([model, capability]) => [model, capability.supportsVision === true],
      ),
    ),
    apiVisionRoutingModel: provider.visionRoutingModel,
    accountName: resolveCodexModelProviderAccountName(provider.name, apiKeyName),
  };
}
