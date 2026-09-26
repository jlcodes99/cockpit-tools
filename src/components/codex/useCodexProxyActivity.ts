import { useCallback, useEffect, useRef, useState } from 'react';
import {
  clearProxyActivity,
  getProxyActivity,
  setProxyActivityEnabled,
  type ProxyActivitySnapshot,
} from '../../services/codexProxyActivityService';

export function useCodexProxyActivity(accountId: string, active = true) {
  const [snapshot, setSnapshot] = useState<ProxyActivitySnapshot | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [paused, setPaused] = useState(false);
  const [error, setError] = useState(false);
  const [revision, setRevision] = useState(0);
  const generation = useRef(0);
  const operationGeneration = useRef(0);
  const running = useRef(false);

  const refresh = useCallback(() => setRevision((value) => value + 1), []);

  useEffect(() => {
    operationGeneration.current += 1;
    running.current = false;
    setBusy(false);
    setSnapshot(null);
    setLoading(true);
    setError(false);
    setPaused(false);
    return () => { operationGeneration.current += 1; running.current = false; };
  }, [accountId, active]);

  useEffect(() => {
    if (!active || busy) return;
    const current = ++generation.current;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const read = async () => {
      try {
        const next = await getProxyActivity(accountId);
        if (generation.current === current) {
          setSnapshot(next);
          setError(false);
        }
      } catch {
        if (generation.current === current) setError(true);
      } finally {
        if (generation.current === current) {
          setLoading(false);
          if (!paused) timer = setTimeout(read, 2500);
        }
      }
    };
    void read();
    return () => {
      generation.current += 1;
      if (timer) clearTimeout(timer);
    };
  }, [accountId, active, busy, paused, revision]);

  const mutate = async (action: () => Promise<unknown>) => {
    if (!active || !accountId || running.current) return;
    const current = operationGeneration.current;
    const isCurrent = () => current === operationGeneration.current;
    running.current = true;
    // A pre-write poll cannot overwrite the mutation's authoritative snapshot.
    generation.current += 1;
    setBusy(true);
    setError(false);
    try {
      await action();
      if (!isCurrent()) return;
      const next = await getProxyActivity(accountId);
      if (!isCurrent()) return;
      setSnapshot(next);
    } catch {
      if (isCurrent()) setError(true);
    } finally {
      if (isCurrent()) {
        running.current = false;
        setBusy(false);
        refresh();
      }
    }
  };

  const changeEnabled = (enabled: boolean) => mutate(() => setProxyActivityEnabled(accountId, enabled));
  const clear = () => mutate(() => clearProxyActivity(accountId));

  return { snapshot, loading, busy, paused, setPaused, error, refresh, changeEnabled, clear };
}
