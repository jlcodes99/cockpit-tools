/** Visual treatment follows the actual endpoint, independent of the editable provider name. */
export function resolveCodexAccountCardSkin(baseUrl?: string | null): string {
  if (!baseUrl?.trim()) return '';
  try {
    const url = new URL(baseUrl.trim());
    if (url.protocol !== 'https:' && url.protocol !== 'http:') return '';
    const hostname = url.hostname.toLowerCase().replace(/\.$/, '');
    if (['apikey.fan', 'apikey.fun'].some(domain => hostname === domain || hostname.endsWith(`.${domain}`))) {
      return 'codex-card-skin-apikey-fun';
    }
    if (hostname === 'ai.ahzm.top' || hostname.endsWith('.ai.ahzm.top')) {
      return 'codex-card-skin-ahzm';
    }
    return hostname === 'yunmoor.com' || hostname.endsWith('.yunmoor.com')
      ? 'codex-card-skin-yunmoor'
      : '';
  } catch {
    return '';
  }
}
