import { engineInstallActive, type EngineInstallStatus } from '../services/codexProxyEngineService';

export type ProxyEngineReadiness = 'loading' | 'unknown' | 'unsupported' | 'installing' | 'failed' | 'missing' | 'ready';

/** An unreadable status or a partial installation is never reported as usable. */
export function proxyEngineReadiness(status: EngineInstallStatus | null, readFailed: boolean): ProxyEngineReadiness {
  if (readFailed) return 'unknown';
  if (!status) return 'loading';
  if (!status.supported) return 'unsupported';
  if (engineInstallActive(status)) return 'installing';
  if (status.phase === 'failed') return 'failed';
  return status.installedVersion === status.version ? 'ready' : 'missing';
}

export function proxySetupStage(readiness: ProxyEngineReadiness, hasUsableProxies: boolean | null): 'prepare' | 'pending' | 'add' | 'assign' {
  if (readiness !== 'ready') return 'prepare';
  if (hasUsableProxies === null) return 'pending';
  return hasUsableProxies ? 'assign' : 'add';
}
