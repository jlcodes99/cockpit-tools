/** Reserve IDs at enqueue time, so overlapping list/page loads share one worker. */
export function createSerialAccountRefreshQueue(options: {
  shouldRun: (id: string) => boolean;
  run: (id: string) => Promise<void>;
  onError: (id: string, error: unknown) => void;
  retryIntervalMs: number;
  now?: () => number;
}) {
  const pending = new Map<string, { promise: Promise<void>; resolve: () => void }>();
  const lastAttempt = new Map<string, number>();
  const now = options.now ?? Date.now;
  let running = false;

  const eligible = (id: string) => options.shouldRun(id) &&
    (!lastAttempt.has(id) || now() - lastAttempt.get(id)! >= options.retryIntervalMs);

  async function drain() {
    running = true;
    let processed = 0;
    try {
      while (pending.size > 0) {
        const [id, task] = pending.entries().next().value!;
        try {
          // Recheck after waiting: an account may have been updated or removed.
          if (eligible(id)) {
            lastAttempt.set(id, now());
            await options.run(id);
          }
        } catch (error) {
          options.onError(id, error);
        } finally {
          pending.delete(id);
          task.resolve();
        }
        if (++processed % 10 === 0 && pending.size > 0) {
          await new Promise<void>((resolve) => setTimeout(resolve, 0));
        }
      }
    } finally {
      running = false;
    }
  }

  return {
    enqueue(ids: Iterable<string>): Promise<void> {
      const waits: Promise<void>[] = [];
      for (const id of ids) {
        let task = pending.get(id);
        if (!task && eligible(id)) {
          let resolve!: () => void;
          const promise = new Promise<void>((done) => { resolve = done; });
          task = { promise, resolve };
          pending.set(id, task);
        }
        if (task) waits.push(task.promise);
      }
      if (!running && pending.size > 0) void drain();
      return Promise.all(waits).then(() => undefined);
    },
  };
}
