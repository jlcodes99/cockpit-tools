export interface LatencyValue { latencyMs: number; checkedAt: number }
export type LatencyState = { status: 'queued' | 'running' | 'cancelled' } | { status: 'success'; value: LatencyValue } | { status: 'error'; error: unknown };
export const LATENCY_FRESH_MS = 60_000;
export function latencyFresh(result: LatencyState | undefined, now = Date.now()): boolean {
  return result?.status === 'success' && now >= result.value.checkedAt && now - result.value.checkedAt < LATENCY_FRESH_MS;
}
/** Three workers, explicit cancellation, no selection changes and no retries. */
export function startLatencyBatch(ids: string[], deps: {
  id: () => string;
  measure: (nodeId: string, requestId: string) => Promise<LatencyValue>;
  cancel: (requestId: string) => Promise<unknown>;
  update: (nodeId: string, state: LatencyState) => void;
}) {
  const queue = [...new Set(ids)];
  const active = new Map<string, string>();
  const finished = new Set<string>();
  let next = 0; let cancelled = false;
  queue.forEach((id) => deps.update(id, { status: 'queued' }));
  const worker = async () => {
    while (!cancelled && next < queue.length) {
      const nodeId = queue[next++]; const requestId = deps.id();
      active.set(requestId, nodeId); deps.update(nodeId, { status: 'running' });
      try {
        const value = await deps.measure(nodeId, requestId);
        if (!cancelled) deps.update(nodeId, { status: 'success', value });
      } catch (error) {
        if (!cancelled) deps.update(nodeId, { status: 'error', error });
      } finally { active.delete(requestId); finished.add(nodeId); }
    }
  };
  const done = Promise.all(Array.from({ length: Math.min(3, queue.length) }, () => worker())).then(() => {});
  return {
    done,
    cancel() {
      if (cancelled) return; cancelled = true;
      for (const id of queue) if (!finished.has(id)) deps.update(id, { status: 'cancelled' });
      for (const requestId of active.keys()) void deps.cancel(requestId).catch(() => {});
    },
  };
}
