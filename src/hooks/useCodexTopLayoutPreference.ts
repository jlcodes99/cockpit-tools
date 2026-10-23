import { useCallback, useEffect, useRef, useState } from 'react';
import {
  CODEX_TOP_LAYOUT_STORAGE_KEY,
  CODEX_TOP_LAYOUT_UPDATED_EVENT,
  createDefaultCodexTopLayout,
  loadCodexTopLayoutPreference,
  normalizeCodexTopLayout,
  writeCodexTopLayoutPreference,
  type CodexTopLayoutPreference,
} from '../utils/codexTopLayoutPreferences';

type StorageError = 'read' | 'save' | null;

function loadState(): { layout: CodexTopLayoutPreference; storageError: StorageError } {
  try {
    return { layout: loadCodexTopLayoutPreference(), storageError: null };
  } catch {
    return { layout: createDefaultCodexTopLayout(), storageError: 'read' };
  }
}

/** Immediate preview with explicit persistence failures; passive reloads never write back. */
export function useCodexTopLayoutPreference() {
  const [state, setState] = useState(loadState);
  const current = useRef(state);
  current.current = state;
  const changeLayout = useCallback((layout: CodexTopLayoutPreference) => {
    const normalized = normalizeCodexTopLayout(layout);
    try {
      writeCodexTopLayoutPreference(normalized);
      current.current = { layout: normalized, storageError: null };
      setState(current.current);
    } catch {
      current.current = { layout: normalized, storageError: 'save' };
      setState(current.current);
    }
  }, []);
  const reload = useCallback(() => {
    if (current.current.storageError === 'save') return;
    try {
      current.current = { layout: loadCodexTopLayoutPreference(), storageError: null };
      setState(current.current);
    } catch {
      // A failed passive reload must not replace an existing usable preview.
      current.current = { ...current.current, storageError: 'read' };
      setState(current.current);
    }
  }, []);
  const retry = useCallback(() => {
    if (current.current.storageError === 'save') changeLayout(current.current.layout);
    else reload();
  }, [changeLayout, reload]);

  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.key === CODEX_TOP_LAYOUT_STORAGE_KEY || event.key === null) reload();
    };
    window.addEventListener(CODEX_TOP_LAYOUT_UPDATED_EVENT, reload);
    window.addEventListener('storage', onStorage);
    return () => {
      window.removeEventListener(CODEX_TOP_LAYOUT_UPDATED_EVENT, reload);
      window.removeEventListener('storage', onStorage);
    };
  }, [reload]);
  return { ...state, changeLayout, retry };
}
