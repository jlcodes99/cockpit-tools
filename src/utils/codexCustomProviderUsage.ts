export interface CustomProviderUsage {
  endpoint: string;
  headers?: Record<string, string>;
  mappings: Record<string, string>;
  unit?: string;
}

export function normalizeCustomProviderUsage(value: unknown): CustomProviderUsage | undefined {
  if (typeof value !== 'object' || value === null) return undefined;
  const { endpoint, headers, mappings, unit } = value as Partial<CustomProviderUsage>;
  const stringMap = (record: unknown): record is Record<string, string> =>
    typeof record === 'object' && record !== null && !Array.isArray(record) &&
    Object.entries(record).every(([key, text]) => key.trim() && typeof text === 'string' && text.trim());
  if (typeof endpoint !== 'string' || !endpoint.trim() ||
      !stringMap(mappings) || !Object.keys(mappings).length ||
      (headers !== undefined && !stringMap(headers)) ||
      (unit !== undefined && typeof unit !== 'string')) return undefined;
  return {
    endpoint: endpoint.trim(),
    mappings: { ...mappings },
    ...(headers && { headers: { ...headers } }),
    ...(unit !== undefined && { unit: unit.trim() }),
  };
}

export function parseCustomProviderUsage(text: string, baseUrl: string): CustomProviderUsage | undefined {
  if (!text.trim()) return undefined;
  const config = normalizeCustomProviderUsage(JSON.parse(text));
  if (!config) throw new Error('PROVIDER_CUSTOM_USAGE_SETTINGS_INVALID');
  const url = new URL(config.endpoint, baseUrl);
  if (url.origin !== new URL(baseUrl).origin || url.username || url.password) {
    throw new Error('PROVIDER_CUSTOM_USAGE_ENDPOINT_MUST_BE_SAME_ORIGIN');
  }
  return config;
}

export function customProviderUsageBinding(provider?: {
  id: string;
  updatedAt: number;
  customUsage?: CustomProviderUsage;
} | null): string | undefined {
  return provider?.customUsage ? `custom:${provider.id}:${provider.updatedAt}` : undefined;
}

export function currentCustomProviderUsageState<T extends { sourceBinding?: string }>(
  provider: Parameters<typeof customProviderUsageBinding>[0],
  state: T | undefined,
): T | undefined {
  const binding = customProviderUsageBinding(provider);
  if (binding || state?.sourceBinding?.startsWith('custom:')) {
    return state?.sourceBinding === binding ? state : undefined;
  }
  return state;
}

export function customProviderUsageForRequest(provider?: {
  baseUrl: string;
  customUsage?: CustomProviderUsage;
} | null): CustomProviderUsage | undefined {
  if (!provider?.customUsage) return undefined;
  return {
    ...provider.customUsage,
    endpoint: new URL(provider.customUsage.endpoint, provider.baseUrl).href,
  };
}
