import { subscribeTauriEvent } from './tauriEventListener';

/** Combine native and in-page notifications while a service snapshot is being read. */
export function subscribeCodexLocalAccessStateUpdates(refresh: () => Promise<unknown>) {
  const event = 'codex-local-access-state-updated';
  let running = false;
  let queued = false;
  let disposed = false;

  async function drain() {
    running = true;
    try {
      while (queued && !disposed) {
        queued = false;
        try {
          await refresh();
        } catch (error) {
          if (!disposed) console.error('Failed to refresh Codex API service state:', error);
        }
      }
    } finally {
      running = false;
    }
  }

  const onUpdated = () => {
    if (disposed) return;
    queued = true;
    if (!running) void drain();
  };
  window.addEventListener(event, onUpdated);
  const unsubscribe = subscribeTauriEvent(event, onUpdated);
  return () => {
    disposed = true;
    window.removeEventListener(event, onUpdated);
    unsubscribe();
  };
}
