import { useEffect, useState } from 'react';
import { getApiKeyFunCampaignRefreshDelay, isApiKeyFunCampaignActive } from '../utils/apiKeyFunCampaign';

export function useApiKeyFunCampaign(): boolean {
  const [active, setActive] = useState(() => isApiKeyFunCampaignActive());

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout>;
    const refresh = () => {
      clearTimeout(timer);
      setActive(isApiKeyFunCampaignActive());
      timer = setTimeout(refresh, getApiKeyFunCampaignRefreshDelay());
    };
    refresh();
    window.addEventListener('focus', refresh);
    document.addEventListener('visibilitychange', refresh);
    return () => {
      clearTimeout(timer);
      window.removeEventListener('focus', refresh);
      document.removeEventListener('visibilitychange', refresh);
    };
  }, []);

  return active;
}
