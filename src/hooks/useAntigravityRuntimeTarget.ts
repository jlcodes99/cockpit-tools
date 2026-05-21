import { useEffect, useState } from 'react';
import {
  ANTIGRAVITY_RUNTIME_TARGET_CHANGED_EVENT,
  ANTIGRAVITY_RUNTIME_TARGET_STORAGE_KEY,
  AntigravityRuntimeTarget,
  getAntigravityRuntimeTarget,
  normalizeAntigravityRuntimeTarget,
} from '../utils/antigravityRuntimeTarget';

export function useAntigravityRuntimeTarget(): AntigravityRuntimeTarget {
  const [target, setTarget] = useState<AntigravityRuntimeTarget>(() =>
    getAntigravityRuntimeTarget(),
  );

  useEffect(() => {
    const handleChange = (event: Event) => {
      if (event instanceof StorageEvent) {
        if (event.key !== ANTIGRAVITY_RUNTIME_TARGET_STORAGE_KEY) {
          return;
        }
        setTarget(normalizeAntigravityRuntimeTarget(event.newValue));
        return;
      }
      setTarget(normalizeAntigravityRuntimeTarget((event as CustomEvent).detail));
    };

    window.addEventListener(ANTIGRAVITY_RUNTIME_TARGET_CHANGED_EVENT, handleChange);
    window.addEventListener('storage', handleChange);
    return () => {
      window.removeEventListener(ANTIGRAVITY_RUNTIME_TARGET_CHANGED_EVENT, handleChange);
      window.removeEventListener('storage', handleChange);
    };
  }, []);

  return target;
}
