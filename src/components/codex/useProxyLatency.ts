import { useCallback, useLayoutEffect, useRef, useState } from 'react';
import { cancelProxyCatalog, catalogErrorKey, measureProxyLatency, type ProxyCatalogSource } from '../../services/codexProxyCatalogService';
import { preflightCodexProxyEngine } from '../../services/codexProxyEngineService';
import { latencyFresh, startLatencyBatch, type LatencyState } from '../../utils/codexProxyLatency';

/** One cancellable queue per picker. A replacement waits for old native checks to stop. */
export function useProxyLatency(source: ProxyCatalogSource | undefined) {
  const key = source ? source.id + ':' + source.revision : '';
  const [scope, setScope] = useState('');
  const [cache, setCache] = useState<{ key: string; scopes: Record<string, Record<string, LatencyState>> }>({ key, scopes: {} });
  const results = cache.key === key ? cache.scopes[scope] ?? {} : {};
  const latest = useRef(cache); latest.current = cache;
  const [running, setRunning] = useState(false);
  const [total, setTotal] = useState(0);
  const [completed, setCompleted] = useState(0);
  const [errorKey, setErrorKey] = useState('');
  const active = useRef<{ cancel(): void; done: Promise<void> } | null>(null);
  const generation = useRef(0);
  const cancel = useCallback(() => { active.current?.cancel(); }, []);
  useLayoutEffect(() => {
    generation.current++;
    active.current?.cancel();
    setCache({ key, scopes: {} }); setScope(''); setRunning(false); setTotal(0); setCompleted(0); setErrorKey('');
    return () => { generation.current++; active.current?.cancel(); };
  }, [key]);

  const measure = useCallback((ids: string[], onlyStale = false, groupId?: string) => {
    if (!source) return;
    const nextScope = groupId ?? '';
    const previousResults = latest.current.key === key ? latest.current.scopes[nextScope] ?? {} : {};
    const requested = [...new Set(ids)].filter((id) => source.nodes.some((node) => node.id === id && node.supported));
    const pending = requested.filter((id) => !onlyStale || !latencyFresh(previousResults[id]));
    const previous = active.current;
    previous?.cancel();
    const current = ++generation.current;
    let cancelled = false;
    let batch: ReturnType<typeof startLatencyBatch> | undefined;
    const finished = new Set<string>();
    const update = (id: string, state: LatencyState) => {
      if (generation.current !== current || cancelled) return;
      setCache((old) => {
        const scopes = old.key === key ? old.scopes : {};
        return { key, scopes: { ...scopes, [nextScope]: { ...scopes[nextScope], [id]: state } } };
      });
      if (!['queued', 'running'].includes(state.status)) { finished.add(id); setCompleted(finished.size); }
    };
    setScope(nextScope); setRunning(pending.length > 0); setTotal(pending.length); setCompleted(0); setErrorKey('');
    const task = {
      done: Promise.resolve(),
      cancel() {
        if (cancelled) return;
        batch?.cancel();
        pending.filter((id) => !finished.has(id)).forEach((id) => update(id, { status: 'cancelled' }));
        cancelled = true;
        if (generation.current === current) setRunning(false);
      },
    };
    active.current = task;
    task.done = (async () => {
      await previous?.done;
      if (cancelled || generation.current !== current || !pending.length) return;
      // Check the batch before creating node results. Cached/background refreshes
      // show one inline setup error; only explicit checks open the installer prompt.
      await preflightCodexProxyEngine(!onlyStale);
      if (cancelled || generation.current !== current) return;
      pending.forEach((id) => update(id, { status: 'queued' }));
      batch = startLatencyBatch(pending, {
        id: () => crypto.randomUUID(),
        measure: (nodeId, requestId) => measureProxyLatency(requestId, source.id, nodeId, source.revision, groupId),
        cancel: cancelProxyCatalog, update,
      });
      await batch.done;
    })().catch((error) => {
      if (generation.current === current && !cancelled) setErrorKey(catalogErrorKey(error));
    }).finally(() => {
      if (active.current === task) active.current = null;
      if (generation.current === current) setRunning(false);
    });
  }, [key, source]);
  return { results, running, total, completed, errorKey, measure, cancel };
}
