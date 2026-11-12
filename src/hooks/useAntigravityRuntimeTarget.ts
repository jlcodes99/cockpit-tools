import { hydrateDurableUiSettings } from '../utils/durableUiSettings';
import { useEffect, useState } from 'react';
import {
  ANTIGRAVITY_RUNTIME_TARGET_CHANGED_EVENT,
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
      setTarget(normalizeAntigravityRuntimeTarget((event as CustomEvent).detail));
    };

    window.addEventListener(ANTIGRAVITY_RUNTIME_TARGET_CHANGED_EVENT, handleChange);
    return () => {
      window.removeEventListener(ANTIGRAVITY_RUNTIME_TARGET_CHANGED_EVENT, handleChange);
    };
  }, []);

  useEffect(() => {
    if (antigravityRuntimeTargetAutoResolveStarted) {
      return;
    }
    antigravityRuntimeTargetAutoResolveStarted = true;

    // Only discover a fallback after the Rust configuration has been read.
    void hydrateDurableUiSettings().then(async () => {
      if (hasSavedAntigravityRuntimeTarget()) return;
      const initialTarget = getAntigravityRuntimeTarget();
      const preferredTarget = await resolvePreferredAntigravityRuntimeTarget(initialTarget);
      if (!hasSavedAntigravityRuntimeTarget()
          && preferredTarget !== initialTarget
          && getAntigravityRuntimeTarget() === initialTarget) {
        setAntigravityRuntimeTarget(preferredTarget);
      }
    })
      .catch((error) => {
        antigravityRuntimeTargetAutoResolveStarted = false;
        console.warn('[AntigravityRuntime] failed to resolve preferred target:', error);
      });
  }, []);

  return target;
}
