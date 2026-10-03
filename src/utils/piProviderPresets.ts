import { CLAUDE_API_PROVIDER_PRESETS } from "./claudeProviderPresets";
import { CODEX_API_PROVIDER_PRESETS } from "./codexProviderPresets";

/** pi `api` protocol values offered for third-party gateways. */
export type PiGatewayApi =
  | "openai-completions"
  | "openai-responses"
  | "anthropic-messages";

export const PI_GATEWAY_APIS: readonly PiGatewayApi[] = [
  "openai-completions",
  "openai-responses",
  "anthropic-messages",
];

export interface PiGatewayPreset {
  id: string;
  name: string;
  /** Endpoints per protocol; a preset only offers the protocols it lists. */
  endpoints: Partial<Record<PiGatewayApi, string[]>>;
  modelCatalog?: string[];
  website?: string;
  apiKeyUrl?: string;
  isPartner?: boolean;
}

export const PI_GATEWAY_CUSTOM_ID = "custom";

/** pi built-in providers usable with a plain API key (no models.json entry). */
export const PI_BUILTIN_PROVIDERS = [
  "anthropic",
  "openai",
  "google",
  "openrouter",
  "xai",
  "groq",
  "mistral",
  "deepseek",
  "cerebras",
  "zai",
  "moonshotai",
  "minimax",
  "huggingface",
  "fireworks",
  "together",
] as const;

const isUsableUrl = (url: string) =>
  /^https?:\/\//i.test(url.trim()) && !url.includes("${") && !url.includes("YOUR_");

function nameKey(name: string): string {
  return name.toLowerCase().replace(/[^a-z0-9]/g, "");
}

function buildPresets(): PiGatewayPreset[] {
  const byKey = new Map<string, PiGatewayPreset>();
  const order: string[] = [];
  const upsert = (
    key: string,
    base: Omit<PiGatewayPreset, "endpoints">,
    api: PiGatewayApi,
    urls: string[],
    models?: string[],
  ) => {
    const usable = urls.filter(isUsableUrl);
    if (usable.length === 0) return;
    let preset = byKey.get(key);
    if (!preset) {
      preset = { ...base, endpoints: {} };
      byKey.set(key, preset);
      order.push(key);
    }
    preset.endpoints[api] = usable;
    if (!preset.modelCatalog?.length && models?.length) {
      preset.modelCatalog = [...models];
    }
    preset.isPartner ||= base.isPartner;
  };

  for (const preset of CLAUDE_API_PROVIDER_PRESETS) {
    if (preset.isOfficial || preset.extraEnv?.CLAUDE_CODE_USE_BEDROCK) continue;
    upsert(
      nameKey(preset.name),
      {
        id: preset.id,
        name: preset.name,
        website: preset.website,
        apiKeyUrl: preset.apiKeyUrl,
        isPartner: preset.isPartner,
      },
      "anthropic-messages",
      preset.baseUrls,
      preset.modelCatalog,
    );
  }
  for (const preset of CODEX_API_PROVIDER_PRESETS) {
    if (preset.isOfficial || preset.isService) continue;
    const key = byKey.has(nameKey(preset.name))
      ? nameKey(preset.name)
      : [...byKey.entries()].find(([, item]) => item.id === preset.id)?.[0] ??
        nameKey(preset.name);
    upsert(
      key,
      {
        id: preset.id,
        name: preset.name,
        website: preset.website,
        apiKeyUrl: preset.apiKeyUrl,
        isPartner: preset.isPartner,
      },
      "openai-completions",
      preset.baseUrls,
      preset.modelCatalog,
    );
  }
  return order.map((key) => byKey.get(key)!);
}

export const PI_GATEWAY_PRESETS: readonly PiGatewayPreset[] = buildPresets();

export function findPiGatewayPreset(id: string): PiGatewayPreset | null {
  return PI_GATEWAY_PRESETS.find((preset) => preset.id === id) ?? null;
}

export function getPresetApis(preset: PiGatewayPreset | null): PiGatewayApi[] {
  if (!preset) return [...PI_GATEWAY_APIS];
  return PI_GATEWAY_APIS.filter((api) => preset.endpoints[api]?.length);
}

/**
 * pi provider id for a gateway. The `gw-` prefix keeps it from overriding a
 * pi built-in provider (e.g. `deepseek`) that has its own model catalog.
 */
export function toGatewayProviderId(presetId: string): string {
  const slug = presetId
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return `gw-${slug || "custom"}`;
}
