import type { CodexPelicanBatch } from '../../../types/codexPelican.ts';

/** Full snapshots can cross in-flight invoke responses. Never roll an active job back. */
export function mergePelicanSnapshot(
  active: CodexPelicanBatch | null,
  batch: CodexPelicanBatch,
  dismissedIds: ReadonlySet<string>,
): CodexPelicanBatch | null {
  if (dismissedIds.has(batch.id)) return active;
  if (!active) return batch;
  if (active.id === batch.id) return batch.revision >= active.revision ? batch : active;
  return batch.createdAt > active.createdAt ? batch : active;
}

/** Serialize all result-card tag/group writes, including reads used to append values. */
export function createPelicanWriteQueue() {
  let tail: Promise<unknown> = Promise.resolve();
  return <T>(operation: () => Promise<T>): Promise<T> => {
    const result = tail.then(operation);
    tail = result.catch(() => undefined);
    return result;
  };
}

export const queuePelicanOrganization = createPelicanWriteQueue();
