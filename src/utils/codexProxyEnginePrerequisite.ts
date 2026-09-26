/** Only fixed host error codes may open the engine installer; never forward raw errors. */
export const CODEX_PROXY_ENGINE_REQUIRED_EVENT = 'codex-proxy-engine-required';

const prerequisiteKeys: Record<string, string> = {
  PROXY_ENGINE_MISSING: 'codex.proxy.engineMissing',
  ENGINE_INSTALL_VERIFY: 'codex.proxy.engine.repairHint',
  ENGINE_INSTALL_ARCHIVE: 'codex.proxy.engine.repairHint',
  ENGINE_INSTALL_TOO_LARGE: 'codex.proxy.engine.repairHint',
  ENGINE_INSTALL_IO: 'codex.proxy.engine.repairHint',
  ENGINE_INSTALL_UNSUPPORTED: 'codex.proxy.engine.unsupported',
  ENGINE_INSTALL_BUSY: 'codex.proxy.engine.busy',
  ENGINE_INSTALL_TIMEOUT: 'codex.proxy.engine.timeout',
  PROXY_ENGINE_VERSION: 'codex.proxy.engine.versionMismatch',
  PROXY_ENGINE_START_FAILED: 'codex.proxy.engine.startFailed',
  PROXY_ENGINE_TIMEOUT: 'codex.proxy.engine.startTimeout',
};

export function proxyEnginePrerequisiteCode(error: unknown): string | null {
  const code = String(error).replace(/^Error:\s*/, '');
  return Object.prototype.hasOwnProperty.call(prerequisiteKeys, code) ? code : null;
}

export function proxyEnginePrerequisiteKey(error: unknown): string | null {
  const code = proxyEnginePrerequisiteCode(error);
  return code ? prerequisiteKeys[code] : null;
}

export function presentProxyEnginePrerequisite(error: unknown): boolean {
  const code = proxyEnginePrerequisiteCode(error);
  if (!code) return false;
  if (typeof window !== 'undefined') {
    window.dispatchEvent(new CustomEvent(CODEX_PROXY_ENGINE_REQUIRED_EVENT, { detail: code }));
  }
  return true;
}

/** User actions retain their rejection and draft; the shared prompt offers installation. */
export async function withProxyEnginePrerequisite<T>(request: Promise<T>): Promise<T> {
  try {
    return await request;
  } catch (error) {
    presentProxyEnginePrerequisite(error);
    throw error;
  }
}
