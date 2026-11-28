import type { TFunction } from 'i18next';
import type { QoderAccount } from '../types/qoder';
export type QoderRegion = 'global' | 'cn';
export type QoderClient = 'ide' | 'desktop' | 'cli';

/**
 * Native targets available for the corresponding realm and host OS.
 */
export function isQoderClientAvailable(
  region: QoderRegion,
  client: QoderClient,
  isWindows: boolean,
  isMac = false,
): boolean {
  if (client === 'ide') return region === 'global' || isMac;
  if (client === 'desktop') return isWindows || isMac;
  return true;
}

export function defaultQoderClient(region: QoderRegion, isWindows: boolean, isMac = false): QoderClient {
  if (region === 'global') return 'ide';
  return isWindows || isMac ? 'desktop' : 'cli';
}

/**
 * Cockpit's public CN account store uses `qoder_cn`. The IDE's native
 * `qodercn` marker is translated when writing that client's profile.
 *
 * The marker may sit on either raw payload — the backend resolves both and
 * rejects a session whose two halves disagree — so reading only
 * `auth_user_info_raw` would list a plan-stamped CN session as global and then
 * fail the switch with `QODER_PLATFORM_MISMATCH`.
 */
export function getQoderRegion(account: QoderAccount): QoderRegion {
  const info = account.auth_user_info_raw as { login_source?: string } | undefined;
  const plan = account.auth_user_plan_raw as { login_source?: string } | undefined;
  const marker = info?.login_source ?? plan?.login_source;
  return marker === 'qoder_cn' ? 'cn' : 'global';
}

export function qoderRegionLabel(region: QoderRegion, t: TFunction): string {
  return region === 'cn'
    ? t('qoder.region.cn', 'Qoder CN · 国内版')
    : t('qoder.region.global', 'Qoder · 国际版');
}
