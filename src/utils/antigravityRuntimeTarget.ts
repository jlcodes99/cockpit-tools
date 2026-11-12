import { previewDurableUiSetting, readDurableUiSetting } from './durableUiSettings';
import { PlatformId } from '../types/platform';

export type AntigravityRuntimeTarget = Extract<PlatformId, 'antigravity' | 'antigravity_ide'>;

export const ANTIGRAVITY_RUNTIME_TARGET_STORAGE_KEY = 'agtools.antigravity.runtime_target.v1';
export const ANTIGRAVITY_RUNTIME_TARGET_CHANGED_EVENT = 'agtools-antigravity-runtime-target-changed';
export const DEFAULT_ANTIGRAVITY_RUNTIME_TARGET: AntigravityRuntimeTarget = 'antigravity_ide';

export function antigravityAppPathTarget(target: AntigravityRuntimeTarget): 'antigravity_legacy' | 'antigravity' {
  return target === 'antigravity' ? 'antigravity_legacy' : 'antigravity';
}

export function isAntigravityRuntimeTarget(value: unknown): value is AntigravityRuntimeTarget {
  return value === 'antigravity' || value === 'antigravity_ide';
}

export function normalizeAntigravityRuntimeTarget(value: unknown): AntigravityRuntimeTarget {
  return isAntigravityRuntimeTarget(value) ? value : DEFAULT_ANTIGRAVITY_RUNTIME_TARGET;
}

export function hasSavedAntigravityRuntimeTarget(): boolean {
  try {
    return typeof window !== 'undefined' && isAntigravityRuntimeTarget(
      readDurableUiSetting(ANTIGRAVITY_RUNTIME_TARGET_STORAGE_KEY),
    );
  } catch {
    return false;
  }
}

/** Startup preferences distinguish the two clients while sharing the account page. */
export function resolveAntigravityStartupTarget(value: unknown): AntigravityRuntimeTarget | null {
  if (typeof value !== 'string') return null;
  const normalized = value.trim().toLowerCase();
  if (normalized === 'antigravity') return 'antigravity';
  if (normalized === 'antigravity-ide') return 'antigravity_ide';
  return null;
}

export function getAntigravityRuntimeTarget(): AntigravityRuntimeTarget {
  if (typeof window === 'undefined') {
    return DEFAULT_ANTIGRAVITY_RUNTIME_TARGET;
  }
  try {
    return normalizeAntigravityRuntimeTarget(
      readDurableUiSetting(ANTIGRAVITY_RUNTIME_TARGET_STORAGE_KEY),
    );
  } catch {
    return DEFAULT_ANTIGRAVITY_RUNTIME_TARGET;
  }
}

export function setAntigravityRuntimeTarget(target: AntigravityRuntimeTarget): void {
  if (typeof window === 'undefined') {
    return;
  }
  previewDurableUiSetting(ANTIGRAVITY_RUNTIME_TARGET_STORAGE_KEY, target);
}

export function setAntigravityRuntimeTargetFromPlatform(platformId: PlatformId): void {
  if (!isAntigravityRuntimeTarget(platformId)) {
    return;
  }
  setAntigravityRuntimeTarget(platformId);
}
