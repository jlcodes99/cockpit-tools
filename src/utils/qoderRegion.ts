import type { TFunction } from 'i18next';
import type { QoderAccount } from '../types/qoder';
export type QoderRegion = 'global' | 'cn';
export type QoderClient = 'ide' | 'desktop' | 'cli';

/**
 * Only the IDE adapter exists for the global client on this branch; the desktop
 * and CLI adapters write into the CN clients' own credential stores.
 */
export function isQoderClientAvailable(
  region: QoderRegion,
  client: QoderClient,
  isWindows: boolean,
): boolean {
  if (client === 'ide') return region === 'global';
  if (region === 'global') return false;
  if (client === 'desktop') return isWindows;
  return true;
}

export function defaultQoderClient(region: QoderRegion, isWindows: boolean): QoderClient {
  if (region === 'global') return 'ide';
  return isWindows ? 'desktop' : 'cli';
}

/**
 * Mirrors `qoder_channel::source_of`: `qoder_cn` marks the public CN client,
 * while `qodercn` stays reserved for the retired CN IDE adapter.
 */
export function getQoderRegion(account: QoderAccount): QoderRegion {
  const info = account.auth_user_info_raw as { login_source?: string } | undefined;
  return info?.login_source === 'qoder_cn' ? 'cn' : 'global';
}

export function qoderRegionLabel(region: QoderRegion, t: TFunction): string {
  return region === 'cn'
    ? t('qoder.region.cn', 'Qoder CN · 国内版')
    : t('qoder.region.global', 'Qoder · 国际版');
}
