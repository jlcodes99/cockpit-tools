import { useCallback, useEffect, useRef, useState } from 'react';
import {
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
  const saveSequence = useRef(0);
  const saving = useRef(false);
  const changeLayout = useCallback((layout: CodexTopLayoutPreference) => {
    const normalized = normalizeCodexTopLayout(layout);
    const sequence = ++saveSequence.current;
    saving.current = true;
    current.current = { layout: normalized, storageError: null };
    setState(current.current);
    const finish = (storageError: StorageError) => {
      if (sequence !== saveSequence.current) return;
      saving.current = false;
      current.current = { layout: normalized, storageError };
      setState(current.current);
    };
    void writeCodexTopLayoutPreference(normalized).then(
      () => finish(null),
      () => finish('save'),
    );
  }, []);
  const reload = useCallback(() => {
    if (saving.current || current.current.storageError === 'save') return;
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
    window.addEventListener(CODEX_TOP_LAYOUT_UPDATED_EVENT, reload);
    return () => window.removeEventListener(CODEX_TOP_LAYOUT_UPDATED_EVENT, reload);
  }, [reload]);
  return { ...state, changeLayout, retry };
}
