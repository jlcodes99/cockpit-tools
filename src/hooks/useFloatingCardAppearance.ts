import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, TauriEvent } from '@tauri-apps/api/event';
import { FLOATING_CARD_APPEARANCE_CHANGED_EVENT, updateFloatingCardAppearance, type FloatingCardAppearance } from '../services/floatingCardService';
import { normalizeFloatingCardOpacity } from '../utils/floatingCardAppearance';

/** Both settings and floating windows use the same persisted, partial update path. */
export function useFloatingCardAppearance() {
  const [appearance, setAppearance] = useState<FloatingCardAppearance>({ minimal: false, backgroundOpacity: 1 });
  const [loaded, setLoaded] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState('');
  const saving = useRef(false);
  const revision = useRef(0);
  const active = useRef(true);
  useEffect(() => {
    active.current = true;
    let disposed = false;
    const cleanups: Array<() => void> = [];
    const addListener = async (event: string, callback: Parameters<typeof listen>[1]) => {
      const cleanup = await listen(event, callback);
      if (disposed) cleanup(); else cleanups.push(cleanup);
    };
    const load = async () => {
      const before = revision.current;
      try {
        const config = await invoke<{ floating_card_minimal?: boolean; floating_card_background_opacity?: number }>('get_general_config');
        if (!disposed && before === revision.current) {
          setAppearance({ minimal: config.floating_card_minimal === true, backgroundOpacity: normalizeFloatingCardOpacity(config.floating_card_background_opacity) });
          setError('');
        }
      } catch (cause) {
        if (!disposed) setError(String(cause));
      } finally {
        if (!disposed) setLoaded(true);
      }
    };
    void (async () => {
      try {
        await addListener(FLOATING_CARD_APPEARANCE_CHANGED_EVENT, (event) => {
          if (disposed) return;
          const next = event.payload as FloatingCardAppearance;
          revision.current += 1;
          setAppearance({ minimal: next.minimal === true, backgroundOpacity: normalizeFloatingCardOpacity(next.backgroundOpacity) });
          setError('');
        });
        await addListener(TauriEvent.WINDOW_FOCUS, () => { void load(); });
      } catch (cause) {
        if (!disposed) setError(String(cause));
      }
      if (!disposed) await load();
    })();
    return () => { disposed = true; active.current = false; cleanups.forEach((cleanup) => cleanup()); };
  }, []);
  const update = useCallback(async (patch: Partial<FloatingCardAppearance>) => {
    if (saving.current) return;
    saving.current = true;
    const requestRevision = ++revision.current;
    setPending(true);
    setError('');
    try {
      const next = await updateFloatingCardAppearance(patch);
      // A newer cross-window event wins over a delayed IPC response.
      if (active.current && revision.current === requestRevision) setAppearance(next);
    } catch (cause) {
      if (active.current) setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      saving.current = false;
      if (active.current) setPending(false);
    }
  }, []);
  return { ...appearance, busy: !loaded || pending, error, update };
}
