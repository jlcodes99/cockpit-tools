import { useCallback, useEffect, useRef, useState } from 'react';
import { subscribeTauriEvent } from '../utils/tauriEventListener';

export interface LocalHistoryRead<T> {
  records: T[];
  migrationPending: boolean;
  migrationError: string | null;
}

const POLL_MS = 750;
const READ_TIMEOUT_MS = 5000;
export const LOCAL_HISTORY_TIMEOUT = 'local_history_timeout';

const emptyHistory = <T,>(): LocalHistoryRead<T> => ({
  records: [], migrationPending: false, migrationError: null,
});

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function useLocalHistory<T>(
  read: (retry: boolean) => Promise<LocalHistoryRead<T>>,
  changedEvent?: string,
  prepare?: () => Promise<void>,
  enabled = true,
) {
  const [snapshot, setSnapshot] = useState(emptyHistory<T>);
  const [loading, setLoading] = useState(true);
  const [prepareState, setPrepareState] = useState<{ pending: boolean; error: string | null }>(
    { pending: Boolean(prepare), error: null },
  );
  const actions = useRef({
    refresh: (_retry = false) => {},
    cleared: () => {},
    update: (_edit: (records: T[]) => T[]) => {},
  });
  const refreshHistory = useCallback((retry = false) => actions.current.refresh(retry), []);
  const clearVisibleHistory = useCallback(() => actions.current.cleared(), []);
  const updateRecords = useCallback((edit: (records: T[]) => T[]) => actions.current.update(edit), []);

  useEffect(() => {
    if (!enabled) return;
    setLoading(true);
    let disposed = false;
    let reading = false;
    let queued = false;
    let queuedRetry = false;
    let generation = 0;
    let preparePending = false;
    let prepareDone = false;
    let poll: ReturnType<typeof setTimeout> | undefined;
    let timeout: ReturnType<typeof setTimeout> | undefined;
    let prepareTimeout: ReturnType<typeof setTimeout> | undefined;

    const refresh = (retry = false) => {
      if (disposed) return;
      clearTimeout(poll);
      queued = true;
      queuedRetry ||= retry;
      if (!reading) void readNext();
    };

    async function readNext() {
      reading = true;
      queued = false;
      const retry = queuedRetry;
      queuedRetry = false;
      const started = generation;
      const prepared = prepareDone;
      const current = () => !disposed && started === generation;
      let needsPoll = false;
      timeout = setTimeout(() => {
        if (!current()) return;
        setLoading(false);
        setSnapshot(previous => ({
          ...previous, migrationPending: false, migrationError: LOCAL_HISTORY_TIMEOUT,
        }));
      }, READ_TIMEOUT_MS);
      try {
        const result = await read(retry);
        if (current()) {
          setSnapshot(result);
          needsPoll = result.migrationPending;
        }
      } catch (error) {
        if (current()) setSnapshot(previous => ({
          ...previous, migrationPending: false, migrationError: errorMessage(error),
        }));
      } finally {
        clearTimeout(timeout);
        reading = false;
        if (current()) {
          setLoading(false);
          if (prepared) setPrepareState({ pending: false, error: null });
        }
        if (!disposed) {
          if (queued) void readNext();
          else if (needsPoll) poll = setTimeout(() => refresh(), POLL_MS);
        }
      }
    }

    function prepareLegacy() {
      if (!prepare || prepareDone || preparePending) return;
      preparePending = true;
      setPrepareState({ pending: true, error: null });
      prepareTimeout = setTimeout(() => {
        if (!disposed && !prepareDone) setPrepareState({ pending: false, error: LOCAL_HISTORY_TIMEOUT });
      }, READ_TIMEOUT_MS);
      void prepare().then(() => {
        if (disposed || prepareDone) return;
        prepareDone = true;
        refresh();
      }, error => {
        if (!disposed && !prepareDone) setPrepareState({ pending: false, error: errorMessage(error) });
      }).finally(() => {
        clearTimeout(prepareTimeout);
        preparePending = false;
      });
    }

    actions.current = {
      refresh(retry = false) {
        if (retry) prepareLegacy();
        refresh(retry);
      },
      cleared() {
        if (disposed) return;
        generation++;
        prepareDone = true;
        setPrepareState({ pending: false, error: null });
        setSnapshot(emptyHistory<T>());
        setLoading(false);
        refresh();
      },
      update(edit) {
        if (disposed) return;
        generation++;
        setSnapshot(previous => ({ ...previous, records: edit(previous.records) }));
        refresh();
      },
    };
    const unsubscribe = changedEvent ? subscribeTauriEvent(changedEvent, () => refresh()) : undefined;
    refresh();
    prepareLegacy();
    return () => {
      disposed = true;
      clearTimeout(poll);
      clearTimeout(timeout);
      clearTimeout(prepareTimeout);
      unsubscribe?.();
    };
  }, [changedEvent, enabled, prepare, read]);

  return {
    records: snapshot.records,
    loading,
    migrationPending: snapshot.migrationPending || prepareState.pending,
    error: prepareState.error ?? snapshot.migrationError,
    refresh: refreshHistory,
    cleared: clearVisibleHistory,
    updateRecords,
  };
}
