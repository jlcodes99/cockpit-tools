import type { PlatformId } from '../types/platform.ts';

export const ANTIGRAVITY_RUNTIME_TARGETS = [
  'antigravity',
  'antigravity_ide',
  'antigravity_cli',
] as const;

export type AntigravityRuntimeTarget = (typeof ANTIGRAVITY_RUNTIME_TARGETS)[number];

export const ANTIGRAVITY_RUNTIME_TARGET_STORAGE_KEY = 'agtools.antigravity.runtime_target.v1';
export const ANTIGRAVITY_RUNTIME_TARGET_CHANGED_EVENT = 'agtools-antigravity-runtime-target-changed';
export const DEFAULT_ANTIGRAVITY_RUNTIME_TARGET: AntigravityRuntimeTarget = 'antigravity_ide';

export function isAntigravityRuntimeTarget(value: unknown): value is AntigravityRuntimeTarget {
  return typeof value === 'string' && (ANTIGRAVITY_RUNTIME_TARGETS as readonly string[]).includes(value);
}

export function normalizeAntigravityRuntimeTarget(value: unknown): AntigravityRuntimeTarget {
  if (typeof value !== 'string') {
    return DEFAULT_ANTIGRAVITY_RUNTIME_TARGET;
  }
  const normalized = value.trim().toLowerCase();
  if (normalized === 'antigravity') {
    return 'antigravity';
  }
  if (
    normalized === 'antigravity_cli' ||
    normalized === 'antigravity-cli' ||
    normalized === 'cli' ||
    normalized === 'agy'
  ) {
    return 'antigravity_cli';
  }
  return DEFAULT_ANTIGRAVITY_RUNTIME_TARGET;
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

export function buildEmptyAntigravityCurrentAccounts<T = null>(
  defaultValue: T = null as unknown as T,
): Record<AntigravityRuntimeTarget, T> {
  const result = {} as Record<AntigravityRuntimeTarget, T>;
  for (const target of ANTIGRAVITY_RUNTIME_TARGETS) {
    result[target] = defaultValue;
  }
  return result;
}

