import type { ProxyEngineReadiness } from './codexProxySetup';

/** A source stores its last refresh failure, not the engine's current state.
 * Only installation availability errors are superseded by confirmed readiness;
 * startup, configuration and network failures still need their own retry.
 */
export function proxySourceDisplayError(error: string | null | undefined, readiness: ProxyEngineReadiness): string | null {
  if (readiness === 'ready' && (error === 'PROXY_ENGINE_MISSING'
    || error === 'PROXY_ENGINE_VERSION' || error === 'ENGINE_INSTALL_BUSY')) return null;
  return error ?? null;
}
