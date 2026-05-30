import { PlatformId } from '../types/platform';

export type AntigravityRuntimeTarget = Extract<PlatformId, 'antigravity' | 'antigravity_ide'>;

export const ANTIGRAVITY_RUNTIME_TARGET_STORAGE_KEY = 'agtools.antigravity.runtime_target.v1';
export const ANTIGRAVITY_RUNTIME_TARGET_CHANGED_EVENT = 'agtools-antigravity-runtime-target-changed';
export const DEFAULT_ANTIGRAVITY_RUNTIME_TARGET: AntigravityRuntimeTarget = 'antigravity_ide';

export function isAntigravityRuntimeTarget(value: unknown): value is AntigravityRuntimeTarget {
  return value === 'antigravity' || value === 'antigravity_ide';
}

export function normalizeAntigravityRuntimeTarget(value: unknown): AntigravityRuntimeTarget {
  return isAntigravityRuntimeTarget(value) ? value : DEFAULT_ANTIGRAVITY_RUNTIME_TARGET;
}

export function getAntigravityRuntimeTarget(): AntigravityRuntimeTarget {
  if (typeof window === 'undefined') {
    return DEFAULT_ANTIGRAVITY_RUNTIME_TARGET;
  }
  try {
    return normalizeAntigravityRuntimeTarget(
      window.localStorage.getItem(ANTIGRAVITY_RUNTIME_TARGET_STORAGE_KEY),
    );
  } catch {
    return DEFAULT_ANTIGRAVITY_RUNTIME_TARGET;
  }
}

export function setAntigravityRuntimeTarget(target: AntigravityRuntimeTarget): void {
  if (typeof window === 'undefined') {
    return;
  }
  try {
    window.localStorage.setItem(ANTIGRAVITY_RUNTIME_TARGET_STORAGE_KEY, target);
  } catch {
    // ignore persistence failures
  }
  window.dispatchEvent(
    new CustomEvent(ANTIGRAVITY_RUNTIME_TARGET_CHANGED_EVENT, { detail: target }),
  );
}

export function setAntigravityRuntimeTargetFromPlatform(platformId: PlatformId): void {
  if (!isAntigravityRuntimeTarget(platformId)) {
    return;
  }
  setAntigravityRuntimeTarget(platformId);
}

