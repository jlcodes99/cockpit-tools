import { useEffect, useState } from 'react';
import {
  ANTIGRAVITY_RUNTIME_TARGET_CHANGED_EVENT,
  ANTIGRAVITY_RUNTIME_TARGET_STORAGE_KEY,
  AntigravityRuntimeTarget,
  getAntigravityRuntimeTarget,
  hasSavedAntigravityRuntimeTarget,
  normalizeAntigravityRuntimeTarget,
  setAntigravityRuntimeTarget,
} from '../utils/antigravityRuntimeTarget';
import { resolvePreferredAntigravityRuntimeTarget } from '../services/antigravityRuntimeService';

let antigravityRuntimeTargetAutoResolveStarted = false;

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

  useEffect(() => {
    if (antigravityRuntimeTargetAutoResolveStarted) {
      return;
    }
    antigravityRuntimeTargetAutoResolveStarted = true;

    // Installation discovery is a fallback, never an override of a saved choice.
    if (hasSavedAntigravityRuntimeTarget()) return;

    const initialTarget = getAntigravityRuntimeTarget();
    void resolvePreferredAntigravityRuntimeTarget(initialTarget)
      .then((preferredTarget) => {
        if (hasSavedAntigravityRuntimeTarget()) {
          return;
        }
        if (preferredTarget === initialTarget) {
          return;
        }
        if (getAntigravityRuntimeTarget() !== initialTarget) {
          return;
        }
        setAntigravityRuntimeTarget(preferredTarget);
      })
      .catch((error) => {
        console.warn('[AntigravityRuntime] failed to resolve preferred target:', error);
      });
  }, []);

  return target;
}
