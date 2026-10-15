
type ImportRouting = { apiBaseUrl?: string | null; importUrl?: string | null };

function readString(item: Record<string, unknown>, keys: string[]): string | undefined {
  return keys.map((key) => item[key]).find(
    (value): value is string => typeof value === 'string' && value.trim().length > 0,
  )?.trim();
}

/** Never derive a credential destination from an import/download URL. */
export function applyExplicitApiBaseUrlToExternalImportItems(
  items: unknown[],
  request: ImportRouting,
  invalidUrlMessage = 'PROVIDER_BASE_URL_INVALID',
): unknown[] {
  const raw = request.apiBaseUrl?.trim();
  if (!raw) return items;
  let parsed: URL;
  try { parsed = new URL(raw); } catch { throw new Error(invalidUrlMessage); }
  if (!['http:', 'https:'].includes(parsed.protocol) || parsed.username || parsed.password) {
    throw new Error(invalidUrlMessage);
  }
  const baseUrl = raw.replace(/\/+$/, '');
  // Preserve the host's Cockpit Api import classification. The download URL may
  // identify the bundle format, but it must never determine a credential endpoint.
  let cockpitBundle = false;
  try {
    const path = new URL(request.importUrl || '').pathname.toLowerCase();
    cockpitBundle = ['/api/cockpit-tools/import/', '/user/api/toolsimport/'].some(marker => path.includes(marker));
  } catch { /* A missing download URL is normal for pasted bundles. */ }
  return items.map((item) => {
    if (!item || typeof item !== 'object' || Array.isArray(item)) return item;
    const value = item as Record<string, unknown>;
    const mode = readString(value, ['auth_mode', 'authMode']);
    const key = readString(value, ['OPENAI_API_KEY', 'openai_api_key', 'openaiApiKey']);
    if (mode?.toLowerCase() !== 'apikey' || !key) return item;
    // Never silently replace an explicit upstream in the bundle either.
    if (readString(value, ['base_url', 'baseUrl', 'api_base_url', 'apiBaseUrl'])) return item;
    const cockpitAccount = cockpitBundle || readString(value, ['api_provider_id', 'apiProviderId'])?.toLowerCase() === 'cockpit_api'
      || ['api_provider_name', 'apiProviderName', 'plan_type', 'planType', 'account_note', 'accountNote']
        .some(field => typeof value[field] === 'string' && value[field].toLowerCase().includes('cockpit api'));
    return {
      ...value,
      base_url: baseUrl,
      api_base_url: baseUrl,
      api_provider_mode: readString(value, ['api_provider_mode', 'apiProviderMode']) ?? 'custom',
      ...(cockpitAccount ? {
        api_provider_id: readString(value, ['api_provider_id', 'apiProviderId']) ?? 'cockpit_api',
        api_provider_name: readString(value, ['api_provider_name', 'apiProviderName']) ?? 'Cockpit Api',
        plan_type: readString(value, ['plan_type', 'planType']) ?? 'Cockpit Api',
      } : {}),
    };
  });
}
