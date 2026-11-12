import { useCallback, useEffect, useRef, useState } from 'react';
import { loadCodexResetHistory, type ResetHistoryEntry, type ResetHistorySource } from '../services/codexResetHistoryService';
import { subscribeTauriEvent } from '../utils/tauriEventListener';

export function useCodexResetHistory(accountId: string, source: ResetHistorySource) {
  const [data, setData] = useState<{ entries: ResetHistoryEntry[]; cursor: string | number | null }>({ entries: [], cursor: null });
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [errorKey, setErrorKey] = useState(0);
  const generation = useRef(0);
  const lastRequest = useRef<{ cursor: string | number | null; preserve: boolean }>({ cursor: null, preserve: false });
  const load = useCallback(async (nextCursor: string | number | null = null, preserveLoaded = false) => {
    const version = ++generation.current;
    lastRequest.current = { cursor: nextCursor, preserve: preserveLoaded };
    setLoading(true); setError(null);
    try {
      const page = await loadCodexResetHistory(accountId, source, nextCursor);
      if (version !== generation.current) return;
      setData(previous => {
        if (preserveLoaded && previous.entries.length > 0) {
          const latestIds = new Set(page.entries.map(entry => entry.id));
          return { entries: [...page.entries, ...previous.entries.filter(entry => !latestIds.has(entry.id))], cursor: previous.cursor };
        }
        return {
          entries: nextCursor === null ? page.entries
            : [...new Map([...previous.entries, ...page.entries].map(entry => [entry.id, entry])).values()],
          cursor: page.nextCursor,
        };
      });
    } catch (error) {
      if (version !== generation.current) return;
      setError(String(error)); setErrorKey(key => key + 1);
    } finally {
      if (version === generation.current) setLoading(false);
    }
  }, [accountId, source]);

  useEffect(() => {
    setData({ entries: [], cursor: null });
    void load();
    const stop = source === 'local'
      ? subscribeTauriEvent('codex:reset-automation-updated', () => { void load(null, true); }) : undefined;
    return () => { generation.current++; stop?.(); };
  }, [load, source]);

  return { ...data, loading, error, errorKey, refresh: () => load(), more: () => load(data.cursor), retry: () => load(lastRequest.current.cursor, lastRequest.current.preserve) };
}
