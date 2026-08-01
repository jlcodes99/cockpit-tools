import { invoke } from '@tauri-apps/api/core';
import type { AntigravityRuntimeTarget } from '../utils/antigravityRuntimeTarget';

export interface AntigravityInstalledVersionInfo {
  product_name: string;
  version: string;
  app_path: string;
  source: string;
}

export type AntigravityInstalledVersionScanMode = 'quick' | 'full';

export async function getAntigravityInstalledVersionInfo(
  target?: AntigravityRuntimeTarget,
  scanMode: AntigravityInstalledVersionScanMode = 'quick',
): Promise<AntigravityInstalledVersionInfo | null> {
  return invoke<AntigravityInstalledVersionInfo | null>('get_antigravity_installed_version_info', {
    target,
    scanMode,
  });
}

function getAlternateAntigravityRuntimeTarget(
  target: AntigravityRuntimeTarget,
): AntigravityRuntimeTarget | null {
  if (target === 'antigravity_cli') return null;
  return target === 'antigravity_ide' ? 'antigravity' : 'antigravity_ide';
}

async function detectTargetVersion(
  target: AntigravityRuntimeTarget,
  scanMode: AntigravityInstalledVersionScanMode,
): Promise<boolean> {
  try {
    const info = await getAntigravityInstalledVersionInfo(target, scanMode);
    return !!info?.version;
  } catch (error) {
    console.warn(
      `[AntigravityRuntime] failed to detect ${target} ${scanMode} version:`,
      error,
    );
    return false;
  }
}

export async function resolvePreferredAntigravityRuntimeTarget(
  currentTarget: AntigravityRuntimeTarget,
): Promise<AntigravityRuntimeTarget> {
  const alternateTarget = getAlternateAntigravityRuntimeTarget(currentTarget);
  if (!alternateTarget) {
    return currentTarget;
  }

  const [currentQuickAvailable, alternateQuickAvailable] = await Promise.all([
    detectTargetVersion(currentTarget, 'quick'),
    detectTargetVersion(alternateTarget, 'quick'),
  ]);
  if (currentQuickAvailable) {
    return currentTarget;
  }
  if (alternateQuickAvailable) {
    return alternateTarget;
  }

  const [currentFullAvailable, alternateFullAvailable] = await Promise.all([
    detectTargetVersion(currentTarget, 'full'),
    detectTargetVersion(alternateTarget, 'full'),
  ]);
  if (currentFullAvailable) {
    return currentTarget;
  }
  if (alternateFullAvailable) {
    return alternateTarget;
  }
  return currentTarget;
}
