import { singleFlightRead } from './codexProxyPreview';

export const PROXY_CARD_REFRESH_MS = 20_000;
const READ_TIMEOUT_MS = 10_000;
const BUSY_RETRY_MS = 250;
const TRANSIENT_READ_CODES = new Set(['PROXY_RUNTIME_LIMIT', 'PROXY_RUNTIME_BUSY',
  'PROXY_RUNTIME_STARTING', 'PROXY_RUNTIME_READ_TIMEOUT', 'PROXY_CARD_QUEUE_TIMEOUT']);

export type ProxyCardReadErrorKind = 'busy' | 'capacity' | 'failed';
const errorCode = (error: unknown) => String(error).replace(/^Error:\s*/, '').trim();
function readErrorKind(error: unknown): ProxyCardReadErrorKind {
  const code = errorCode(error);
  if (code === 'PROXY_RUNTIME_CAPACITY') return 'capacity';
  if (TRANSIENT_READ_CODES.has(code) && code !== 'PROXY_RUNTIME_READ_TIMEOUT') return 'busy';
  return 'failed';
}

/** Keep permits until the native operation settles, including after a UI timeout. */
export function createProxyCardReadPool(limit = 4, timeoutMs = READ_TIMEOUT_MS) {
  let active = 0;
  const queue: Array<() => void> = [];
  const drain = () => {
    while (active < limit && queue.length) queue.shift()!();
  };
  return <T>(read: () => Promise<T>, isActive: () => boolean): Promise<T> => new Promise((resolve, reject) => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const start = () => {
      clearTimeout(timer);
      if (!isActive()) { reject(new Error('PROXY_CARD_INACTIVE')); return; }
      active++;
      void Promise.resolve().then(read).then(resolve, reject).finally(() => { active--; drain(); });
    };
    queue.push(start);
    timer = setTimeout(() => {
      const index = queue.indexOf(start);
      if (index >= 0) { queue.splice(index, 1); reject(new Error('PROXY_CARD_QUEUE_TIMEOUT')); }
    }, timeoutMs);
    drain();
  });
}

export interface ProxyCardReadResult<T> { value: T | null; error: boolean; errorKind?: ProxyCardReadErrorKind }

/** Shared snapshot cache; failures preserve the last successful value. */
export function createProxyCardReadCache<T>(
  read: (accountId: string) => Promise<T>,
  pool: ReturnType<typeof createProxyCardReadPool>,
  timeoutMs = READ_TIMEOUT_MS,
  retryDelayMs = BUSY_RETRY_MS,
) {
  const snapshots = new Map<string, ProxyCardReadResult<T> & { checkedAt: number }>();
  const revisions = new Map<string, number>();
  const consumers = new Map<string, Set<() => boolean>>();
  const nativeReads = singleFlightRead(async (key) => {
    const accountId = JSON.parse(JSON.parse(key)[0])[0] as string;
    const active = () => [...(consumers.get(key) ?? [])].some((check) => check());
    const deadline = Date.now() + timeoutMs;
    try {
      return await pool(() => read(accountId), active);
    } catch (error) {
      // One shared retry hides short-lived pressure without creating a retry storm.
      // Never release native permits early or retry after every consumer has left.
      if (!TRANSIENT_READ_CODES.has(errorCode(error)) || !active() || Date.now() + retryDelayMs >= deadline) throw error;
      await new Promise<void>((resolve) => setTimeout(resolve, retryDelayMs));
      if (!active() || Date.now() >= deadline) throw error;
      return pool(() => read(accountId), active);
    }
  }, timeoutMs);
  return {
    invalidateAccount(accountId: string) {
      revisions.set(accountId, (revisions.get(accountId) ?? 0) + 1);
      for (const key of snapshots.keys()) {
        if (JSON.parse(key)[0] === accountId) snapshots.delete(key);
      }
    },
    has(key: string): boolean { return snapshots.has(key); },
    peek(key: string): ProxyCardReadResult<T> {
      return snapshots.get(key) ?? { value: null, error: false };
    },
    async read(key: string, isActive: () => boolean, force = false): Promise<ProxyCardReadResult<T>> {
      const accountId = JSON.parse(key)[0] as string;
      const revision = revisions.get(accountId) ?? 0;
      const nativeKey = JSON.stringify([key, revision]);
      const current = () => isActive() && revision === (revisions.get(accountId) ?? 0);
      const cached = snapshots.get(key);
      if (!force && cached && Date.now() - cached.checkedAt < PROXY_CARD_REFRESH_MS) return cached;
      const checks = consumers.get(nativeKey) ?? new Set<() => boolean>();
      checks.add(current);
      consumers.set(nativeKey, checks);
      const checkedAt = Date.now();
      try {
        const value = await nativeReads(nativeKey);
        if (!current()) return this.peek(key);
        const next = { value, error: false, checkedAt };
        snapshots.delete(key);
        snapshots.set(key, next);
        // Do not retain every account/binding ever visited for the entire session.
        if (snapshots.size > 256) snapshots.delete(snapshots.keys().next().value!);
        return next;
      } catch (error) {
        if (!current()) return this.peek(key);
        const next = { value: snapshots.get(key)?.value ?? null, error: true, errorKind: readErrorKind(error), checkedAt };
        snapshots.delete(key);
        snapshots.set(key, next);
        if (snapshots.size > 256) snapshots.delete(snapshots.keys().next().value!);
        return next;
      } finally {
        checks.delete(current);
        if (!checks.size) consumers.delete(nativeKey);
      }
    },
  };
}
