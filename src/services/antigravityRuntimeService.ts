import { invoke } from '@tauri-apps/api/core';
import { AntigravityRuntimeTarget } from '../utils/antigravityRuntimeTarget';

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
