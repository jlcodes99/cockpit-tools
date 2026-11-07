import type { CodexResetCreditsSnapshot } from '../types/codex';

type RecordEntry = { count: number; snapshot: CodexResetCreditsSnapshot; refreshAt: number };
type Job = {
  id: string; count: number; readyAt: number; manual: boolean;
  promise: Promise<CodexResetCreditsSnapshot>;
  resolve: (value: CodexResetCreditsSnapshot) => void; reject: (error: unknown) => void;
};
type Options = {
  now?: () => number; random?: () => number;
  storage?: Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;
  setTimer?: (callback: () => void, delay: number) => ReturnType<typeof setTimeout>;
  clearTimer?: (timer: ReturnType<typeof setTimeout>) => void;
};
const HOUR = 60 * 60_000;
const PREFIX = 'cockpit.codex.reset-expiry.v1.';

/** Persist details, spread automatic refreshes, and share requests across views. */
export function createResetCreditLoader(
  fetchSnapshot: (accountId: string) => Promise<CodexResetCreditsSnapshot>,
  options: Options = {},
) {
  const now = options.now ?? Date.now;
  const random = options.random ?? Math.random;
  const setTimer = options.setTimer ?? setTimeout;
  const clearTimer = options.clearTimer ?? clearTimeout;
  const records = new Map<string, RecordEntry>();
  const listeners = new Map<string, Set<() => void>>();
  const queued = new Map<string, Job>();
  const running = new Map<string, Job>();
  const retryAt = new Map<string, number>();
  let timer: ReturnType<typeof setTimeout> | undefined;
  let nextAutomaticStart = 0;

  function read(id: string) {
    if (!records.has(id)) {
      try {
        const raw = options.storage?.getItem(PREFIX + id);
        if (raw) {
          const value = JSON.parse(raw) as RecordEntry;
          if (Number.isFinite(value.refreshAt) && Array.isArray(value.snapshot?.credits)) records.set(id, value);
        }
      } catch { /* A missing or unreadable cache must not block account cards. */ }
    }
    return records.get(id);
  }
  function peek(id: string, count: number) {
    const entry = read(id);
    return entry?.count === count ? entry.snapshot : undefined;
  }
  function publish(id: string) { listeners.get(id)?.forEach(listener => listener()); }
  function save(id: string, count: number, snapshot: CodexResetCreditsSnapshot) {
    // Each account gets its own 5–7 hour deadline, preserved across restarts.
    const current = now();
    const expiry = (snapshot.next_expires_at ?? 0) * 1000;
    const refreshAt = Math.min(current + (5 + random() * 2) * HOUR,
      expiry > current ? expiry + random() * 3 * 60_000 : Infinity);
    const entry = { count, snapshot, refreshAt };
    records.set(id, entry);
    retryAt.delete(id);
    try { options.storage?.setItem(PREFIX + id, JSON.stringify(entry)); } catch { /* Optional cache. */ }
    publish(id);
  }
  function schedule() {
    if (timer !== undefined) clearTimer(timer);
    timer = undefined;
    if (running.size >= 2 || !queued.size) return;
    const due = Math.min(...Array.from(queued.values(), job =>
      job.manual ? job.readyAt : Math.max(job.readyAt, nextAutomaticStart)));
    timer = setTimer(pump, Math.max(0, due - now()));
  }
  function pump() {
    timer = undefined;
    const job = Array.from(queued.values())
      .filter(item => item.readyAt <= now() && (item.manual || nextAutomaticStart <= now()))
      .sort((a, b) => Number(b.manual) - Number(a.manual) || a.readyAt - b.readyAt)[0];
    if (!job || running.size >= 2) { schedule(); return; }
    queued.delete(job.id);
    running.set(job.id, job);
    if (!job.manual) nextAutomaticStart = now() + 1000 + random() * 1000;
    void Promise.resolve().then(() => fetchSnapshot(job.id)).then(snapshot => {
      save(job.id, job.count, snapshot);
      job.resolve(snapshot);
    }, error => {
      // Failed background reads retry later, without discarding the last expiry.
      retryAt.set(job.id, now() + (10 + random() * 5) * 60_000);
      job.reject(error);
    }).finally(() => {
      running.delete(job.id);
      schedule();
    });
    schedule();
  }
  function load(id: string, count: number, force = false): Promise<CodexResetCreditsSnapshot> {
    const active = running.get(id);
    if (active) return active.promise;
    const pending = queued.get(id);
    if (pending) {
      pending.count = count;
      if (force) { pending.manual = true; pending.readyAt = now(); schedule(); }
      return pending.promise;
    }
    const entry = read(id);
    if (!force && entry?.count === count && entry.refreshAt > now()) return Promise.resolve(entry.snapshot);
    if (!force && (retryAt.get(id) ?? 0) > now()) {
      return entry ? Promise.resolve(entry.snapshot) : Promise.reject(new Error('Reset details retry scheduled'));
    }
    let resolve!: Job['resolve'];
    let reject!: Job['reject'];
    const promise = new Promise<CodexResetCreditsSnapshot>((yes, no) => { resolve = yes; reject = no; });
    queued.set(id, { id, count, manual: force,
      // Overdue caches found together on startup still refresh at different times.
      readyAt: now() + (force ? 0 : random() * (entry ? 60_000 : 1500)),
      promise, resolve, reject });
    schedule();
    return promise;
  }
  return {
    peek, load,
    seed(id: string, count: number, snapshot: CodexResetCreditsSnapshot) {
      if (!read(id)) save(id, count, snapshot);
    },
    invalidate(id: string) {
      records.delete(id);
      retryAt.delete(id);
      try { options.storage?.removeItem(PREFIX + id); } catch { /* Optional cache. */ }
      publish(id);
    },
    subscribe(id: string, listener: () => void) {
      const group = listeners.get(id) ?? new Set<() => void>();
      group.add(listener);
      listeners.set(id, group);
      return () => { group.delete(listener); if (!group.size) listeners.delete(id); };
    },
  };
}
