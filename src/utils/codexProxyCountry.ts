import countryCodes from '../data/codexCountryCodes.json';

type RegionNames = { of(code: string): string | undefined };
type RegionNamesConstructor = new (locales: string[], options: { type: 'region' }) => RegionNames;
export function codexCountryOptions(language: string): { value: string; label: string; searchText: string }[] {
  const DisplayNames = (Intl as unknown as { DisplayNames?: RegionNamesConstructor }).DisplayNames;
  let names: RegionNames | undefined;
  try { names = DisplayNames ? new DisplayNames([language], { type: 'region' }) : undefined; } catch { /* ISO codes remain usable. */ }
  let english: RegionNames | undefined;
  try { english = DisplayNames ? new DisplayNames(['en'], { type: 'region' }) : undefined; } catch { /* Codes remain searchable. */ }
  return countryCodes.map(code => ({ value: code, label: `${names?.of(code) ?? code} (${code})`, searchText: code + ' ' + (english?.of(code) ?? '') }))
    .sort((a, b) => a.label.localeCompare(b.label));
}
export function codexCountryErrorValues(error: unknown): Record<string, string> {
  const match = /^PROXY_COUNTRY_MISMATCH:([A-Z]{2}):([A-Z]{2})$/.exec(String(error ?? '').replace(/^Error:\s*/, ''));
  return match ? { expected: match[1], actual: match[2] } : {};
}
export function codexCountryErrorKey(error: unknown): string | null {
  const code = String(error ?? '').replace(/^Error:\s*/, '');
  if (/^PROXY_COUNTRY_MISMATCH:[A-Z]{2}:[A-Z]{2}$/.test(code)) return 'codex.proxy.country.mismatch';
  const keys: Record<string, string> = {
    PROXY_COUNTRY_UNAVAILABLE: 'unavailable', PROXY_COUNTRY_STORAGE: 'storage',
    PROXY_COUNTRY_INVALID: 'invalid', PROXY_COUNTRY_CHANGED: 'changed',
    PROXY_COUNTRY_ROUTE_UNKNOWN: 'routeUnknown',
  };
  return Object.prototype.hasOwnProperty.call(keys, code) ? `codex.proxy.country.${keys[code]}` : null;
}
