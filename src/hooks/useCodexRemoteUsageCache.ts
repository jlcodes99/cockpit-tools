import { useEffect, useState } from 'react';
import { CODEX_API_KEY_USAGE_REFRESHED_EVENT, readCodexApiKeyUsageCache } from '../services/codexApiKeyUsageRefreshService';

/** Read once per snapshot event, never once per account or rendered option. */
export function useCodexRemoteUsageCache() {
  const [cache, setCache] = useState(readCodexApiKeyUsageCache);
  useEffect(() => {
    const refresh = () => setCache(readCodexApiKeyUsageCache());
    window.addEventListener(CODEX_API_KEY_USAGE_REFRESHED_EVENT, refresh);
    refresh();
    return () => window.removeEventListener(CODEX_API_KEY_USAGE_REFRESHED_EVENT, refresh);
  }, []);
  return cache;
}
