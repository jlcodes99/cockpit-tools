import { invoke } from '@tauri-apps/api/core';
import { withProxyEnginePrerequisite } from '../utils/codexProxyEnginePrerequisite';

export interface EngineInstallStatus {
  supported: boolean;
  version: string;
  installedVersion: string | null;
  assetName: string | null;
  archiveBytes: number | null;
  jobId: string | null;
  phase: 'idle' | 'downloading' | 'importing' | 'verifying' | 'extracting' | 'checking' | 'installing' | 'completed' | 'cancelled' | 'failed';
  receivedBytes: number;
  totalBytes: number | null;
  error: string | null;
}

export function engineInstallActive(status: EngineInstallStatus | null): boolean {
  return !!status && ['downloading', 'importing', 'verifying', 'extracting', 'checking', 'installing'].includes(status.phase);
}

function withTimeout<T>(request: Promise<T>, timeoutMs = 10_000): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('ENGINE_INSTALL_TIMEOUT')), timeoutMs);
    request.then(resolve, reject).finally(() => clearTimeout(timer));
  });
}

// Keep the underlying request single-flight even if an individual waiter times out.
let statusRequest: Promise<EngineInstallStatus> | null = null;
export function getCodexProxyEngineStatus(): Promise<EngineInstallStatus> {
  if (!statusRequest) {
    const request = invoke<EngineInstallStatus>('codex_proxy_engine_status');
    statusRequest = request;
    void request.then(() => { if (statusRequest === request) statusRequest = null; },
      () => { if (statusRequest === request) statusRequest = null; });
  }
  return withTimeout(statusRequest);
}

export function installCodexProxyEngine(archivePath: string | null): Promise<EngineInstallStatus> {
  return withTimeout(invoke('codex_proxy_engine_install', { archivePath }));
}

export function cancelCodexProxyEngineInstall(jobId: string): Promise<void> {
  return withTimeout(invoke('codex_proxy_engine_cancel', { jobId }));
}

/** Explicit action only: page/status reads never execute the engine. */
export function preflightCodexProxyEngine(showPrompt = true): Promise<void> {
  const request = withTimeout(invoke<void>('codex_proxy_engine_preflight'), 20_000);
  return showPrompt ? withProxyEnginePrerequisite(request) : request;
}

/** Run before a frontend restart stops the old client; the backend resolves its actual account. */
export function preflightCodexProxyInstance(instanceId: string): Promise<void> {
  return withProxyEnginePrerequisite(withTimeout(invoke<void>('codex_proxy_instance_preflight', { instanceId }), 20_000));
}

/** Never display backend errors or a local archive path verbatim. */
export function engineInstallErrorKey(error: unknown): string {
  const code = String(error).replace(/^Error:\s*/, '');
  const keys: Record<string, string> = {
    ENGINE_INSTALL_UNSUPPORTED: 'unsupported', ENGINE_INSTALL_BUSY: 'busy',
    ENGINE_INSTALL_CANCELLED: 'cancelled', ENGINE_INSTALL_TIMEOUT: 'timeout',
    ENGINE_INSTALL_DOWNLOAD: 'downloadFailed', ENGINE_INSTALL_TOO_LARGE: 'invalidArchive',
    ENGINE_INSTALL_CHECKSUM: 'invalidArchive', ENGINE_INSTALL_ARCHIVE: 'invalidArchive',
    ENGINE_INSTALL_VERIFY: 'verificationFailed', ENGINE_INSTALL_IO: 'failed',
    ENGINE_INSTALL_START_TIMEOUT: 'startTimeout', ENGINE_INSTALL_START_FAILED: 'startFailed',
    ENGINE_INSTALL_VERSION: 'versionMismatch',
  };
  return `codex.proxy.engine.${keys[code] ?? 'failed'}`;
}
