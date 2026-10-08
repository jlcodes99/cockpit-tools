export const APIKEY_FUN_REGISTER_URL = 'https://apikey.fan/register?aff=cockpit';
export const APIKEY_FUN_DOCS_URL = 'https://apikey.fan/docs';
export const APIKEY_FUN_GLOBAL_ENDPOINT = 'https://api.apikey.fan';
export const APIKEY_FUN_DIRECT_ENDPOINT = 'https://slb.apikey.fan';
export const APIKEY_FUN_SOURCE_TAG = 'apikey_fun';
const APIKEY_FUN_LEGACY_PROVIDER_BASE_URL = 'https://api.apikey.fun/v1';
export const APIKEY_FUN_DEFAULT_MODEL_CATALOG = [
  'gpt-6.1-sol',
  'gpt-5.5',
  'claude-sonnet-4-5',
  'claude-sonnet-4-5-thinking',
  'claude-opus-4-6-thinking',
  'gemini-3-pro-preview',
  'gemini-3-flash-preview',
  'gemini-3.5-flash',
] as const;
export const APIKEY_FUN_PROVIDER_BASE_URL = buildApiKeyFunProviderBaseUrl(
  APIKEY_FUN_GLOBAL_ENDPOINT,
);

export function buildApiKeyFunProviderBaseUrl(endpoint: string): string {
  return `${endpoint.trim().replace(/\/+$/, '')}/v1`;
}

export function normalizeApiKeyFunOfficialUrl(value?: string | null): string {
  const raw = value?.trim() ?? '';
  if (!raw) return '';
  try {
    const parsed = new URL(raw);
    const hostname = parsed.hostname.toLowerCase();
    if (
      parsed.protocol === 'https:' &&
      (hostname === 'apikey.fan' || hostname === 'apikey.fun') &&
      (parsed.pathname === '/' || parsed.pathname === '/register')
    ) {
      return APIKEY_FUN_REGISTER_URL;
    }
  } catch {
    return raw;
  }
  return raw;
}

export function isApiKeyFunProviderBaseUrl(value?: string | null): boolean {
  const raw = value?.trim() ?? '';
  if (!raw) return false;
  try {
    const parsed = new URL(raw);
    return [APIKEY_FUN_PROVIDER_BASE_URL, APIKEY_FUN_LEGACY_PROVIDER_BASE_URL].some(
      (candidate) => {
        const expected = new URL(candidate);
        return (
          parsed.protocol === expected.protocol &&
          parsed.hostname.toLowerCase() === expected.hostname.toLowerCase() &&
          parsed.pathname.replace(/\/+$/, '') === expected.pathname.replace(/\/+$/, '')
        );
      },
    );
  } catch {
    return false;
  }
}

export function normalizeApiKeyFunProviderBaseUrl(value?: string | null): string {
  const raw = value?.trim() ?? '';
  if (!raw) return raw;
  try {
    const parsed = new URL(raw);
    if (parsed.hostname.toLowerCase() === 'api.apikey.fun') {
      parsed.hostname = 'api.apikey.fan';
    }
    return `${parsed.protocol}//${parsed.host}${parsed.pathname.replace(/\/+$/, '')}`;
  } catch {
    return raw;
  }
}

export function resolveApiKeyFunWireApi(
  baseUrl?: string | null,
  wireApi?: 'responses' | 'chat_completions' | null,
): 'responses' | 'chat_completions' | null {
  return isApiKeyFunProviderBaseUrl(baseUrl) ? 'responses' : wireApi ?? null;
}
