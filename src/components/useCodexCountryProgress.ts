import { useEffect, type Dispatch, type SetStateAction } from 'react';
import { listen } from '@tauri-apps/api/event';
import { applyCountryProgress } from '../utils/codexCountryProgress';
import type { CodexCountryCheck } from '../services/codexProxyCountryService';
type State = Parameters<typeof applyCountryProgress>[0];
export function useCodexCountryProgress<T extends NonNullable<State>>(setState: Dispatch<SetStateAction<T | null>>) {
  useEffect(() => {
    let alive = true; let unlisten: (() => void) | undefined;
    void listen<CodexCountryCheck>('codex:country-check', event => {
      if (alive) setState(previous => applyCountryProgress(previous, event.payload));
    }).then(cleanup => { if (alive) unlisten = cleanup; else cleanup(); });
    return () => { alive = false; unlisten?.(); };
  }, [setState]);
}
