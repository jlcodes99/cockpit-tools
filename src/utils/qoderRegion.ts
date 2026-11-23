import type { TFunction } from 'i18next';
import type { QoderAccount } from '../types/qoder';
export type QoderRegion = 'global' | 'cn';
export type QoderClient = 'ide' | 'desktop' | 'cli';

/**
 * Both realms ship a desktop app, but its credential file can only be read and
 * written on Windows. The `ide` target is the global IDE's own database, so it
 * exists only there; the CLI is cross-platform on both sides.
 */
export function isQoderClientAvailable(
  region: QoderRegion,
  client: QoderClient,
  isWindows: boolean,
): boolean {
  if (client === 'ide') return region === 'global';
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
