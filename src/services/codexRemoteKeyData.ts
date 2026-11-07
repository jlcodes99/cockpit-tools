import { invoke } from '@tauri-apps/api/core';
import { useEffect, useState } from 'react';
import type { CodexRemoteKeyQuery, CodexRemoteKeyResponse } from '../types/codexRemoteService';

const flights = new Map<string, Promise<CodexRemoteKeyResponse>>();

/** Only mounted detail views query history. Credentials stay in memory. */
export function useCodexRemoteKeyData<T extends CodexRemoteKeyResponse>(
  baseUrl: string, apiKey: string, resource: 'stats' | 'requests', query: CodexRemoteKeyQuery, revision: number,
) {
  const identity = JSON.stringify([baseUrl, apiKey, resource, query]);
  const [state, setState] = useState<{ identity: string; data?: T; loading: boolean; error: boolean }>({ identity, loading: true, error: false });
  useEffect(() => {
    let active = true;
    let running = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const run = async () => {
      if (!active || running || document.visibilityState === 'hidden') return;
      running = true;
      clearTimeout(timer);
      setState(previous => ({ identity, data: previous.identity === identity ? previous.data : undefined, loading: true, error: false }));
      let flight = flights.get(identity);
      if (!flight) {
        flight = invoke<T>('codex_query_remote_key_data', { baseUrl, apiKey, resource, query });
        flights.set(identity, flight);
        void flight.finally(() => { if (flights.get(identity) === flight) flights.delete(identity); }).catch(() => {});
      }
      let delay = 30_000;
      try {
        const data = await flight as T;
        if (!active) return;
        setState(previous => ({ identity,
          data: !data.ready && previous.identity === identity && (previous.data?.ready || previous.data?.previousSnapshot)
            ? { ...previous.data, ready: false, previousSnapshot: true } : data,
          loading: false, error: false }));
        if (!data.ready) delay = 5_000;
      } catch {
        if (active) setState(previous => ({ ...previous, identity, loading: false, error: true }));
      }
      running = false;
      if (active) timer = setTimeout(() => { void run(); }, delay);
    };
    const visible = () => {
      if (document.visibilityState === 'hidden') { clearTimeout(timer); return; }
      clearTimeout(timer); void run();
    };
    void run();
    document.addEventListener('visibilitychange', visible);
    return () => { active = false; clearTimeout(timer); document.removeEventListener('visibilitychange', visible); };
    // identity contains every query field and both credentials.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [identity, revision]);
  return state.identity === identity ? state : { identity, loading: true, error: false, data: undefined };
}
