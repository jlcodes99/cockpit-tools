import { useEffect, useRef, useState } from 'react';
import { getProxyActivitySummary } from '../../services/codexProxyActivityService';
import {
  applyProxyTrafficReading,
  createProxyTraffic,
  sumProxyActivitySummary,
  type CodexProxyTraffic,
  type ProxyTrafficReading,
} from '../../utils/codexProxyTraffic';

export const PROXY_TRAFFIC_POLL_MS = 3000;

/**
 * Polls the all-account activity summary for the traffic bar.
 * The first frame is requested immediately; the timer is scheduled only after a poll settled,
 * so a slow command can never stack requests. Account switching must not reset the session
 * totals, so the polling lifetime is the provider mount, not the selected account.
 */
export function useCodexProxyTraffic(active = true): CodexProxyTraffic {
  const [traffic, setTraffic] = useState(createProxyTraffic);
  const lastReading = useRef<ProxyTrafficReading | null>(null);
  const generation = useRef(0);

  useEffect(() => {
    if (!active) return;
    const current = ++generation.current;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const read = async () => {
      let reading: ProxyTrafficReading;
      try {
        const entries = await getProxyActivitySummary();
        reading = { at: Date.now(), totals: sumProxyActivitySummary(entries) };
      } catch {
        reading = { at: Date.now(), totals: null };
      }
      if (generation.current !== current) return;
      const previousReading = lastReading.current;
      setTraffic((previous) => applyProxyTrafficReading(previous, previousReading, reading));
      if (reading.totals) lastReading.current = reading;
      timer = setTimeout(() => { void read(); }, PROXY_TRAFFIC_POLL_MS);
    };
    void read();
    return () => {
      generation.current += 1;
      if (timer) clearTimeout(timer);
    };
  }, [active]);

  return traffic;
}
