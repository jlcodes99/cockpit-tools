import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { open } from '@tauri-apps/plugin-dialog';
import {
  cancelCodexProxyEngineInstall, engineInstallActive, engineInstallErrorKey,
  getCodexProxyEngineStatus, installCodexProxyEngine, type EngineInstallStatus,
} from '../../services/codexProxyEngineService';
import { proxyEngineReadiness } from '../../utils/codexProxySetup';
import { proxyEnginePrerequisiteCode, proxyEnginePrerequisiteKey } from '../../utils/codexProxyEnginePrerequisite';

/** Shared installer behavior for first-use guidance and the existing settings entry. */
export function useCodexProxyEngineController() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<EngineInstallStatus | null>(null);
  const [errorKey, setErrorKey] = useState('');
  const [reading, setReading] = useState(false);
  const [action, setAction] = useState<'install' | 'import' | 'cancel' | null>(null);
  const [revision, setRevision] = useState(0);
  const [preflightFailure, setPreflightFailure] = useState<string | null>(null);
  const mounted = useRef(false);
  const locked = useRef(false);
  const generation = useRef(0);
  const observedInstallJob = useRef<string | null>(null);
  const observeInstall = (next: EngineInstallStatus) => {
    if (engineInstallActive(next)) observedInstallJob.current = next.jobId;
    else setPreflightFailure((previous) => previous === 'ENGINE_INSTALL_BUSY' ? null : previous);
    if (next.phase === 'completed' && next.jobId && observedInstallJob.current === next.jobId) {
      observedInstallJob.current = null;
      setPreflightFailure(null);
    }
  };

  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => {
    const current = ++generation.current;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const refresh = async () => {
      if (disposed || current !== generation.current) return;
      setReading(true);
      try {
        const next = await getCodexProxyEngineStatus();
        if (disposed || current !== generation.current) return;
        observeInstall(next);
        setStatus(next); setErrorKey((previous) => previous === 'codex.proxy.engine.statusUnavailable' ? '' : previous);
        if (engineInstallActive(next)) timer = setTimeout(() => void refresh(), 800);
      } catch {
        if (!disposed && current === generation.current) setErrorKey('codex.proxy.engine.statusUnavailable');
      } finally {
        if (!disposed && current === generation.current) setReading(false);
      }
    };
    void refresh();
    return () => { disposed = true; if (timer) clearTimeout(timer); };
  }, [revision]);

  const run = async (kind: 'install' | 'import' | 'cancel') => {
    if (locked.current) return;
    locked.current = true; setAction(kind); setErrorKey('');
    ++generation.current; // Ignore snapshots taken before this mutation.
    try {
      if (kind === 'cancel') {
        if (status?.jobId) await cancelCodexProxyEngineInstall(status.jobId);
      } else {
        const archivePath = kind === 'import'
          ? await open({ multiple: false, directory: false, title: t('codex.proxy.engine.importTitle'),
            filters: [{ name: t('codex.proxy.engine.archive'), extensions: ['zip', 'gz'] }] }) : null;
        if (!mounted.current) return;
        if (kind === 'import' && typeof archivePath !== 'string') return;
        const next = await installCodexProxyEngine(typeof archivePath === 'string' ? archivePath : null);
        if (mounted.current) {
          observeInstall(next);
          setStatus((previous) => ({ ...next, installedVersion: next.installedVersion ?? previous?.installedVersion ?? null }));
        }
      }
    } catch (caught) {
      if (mounted.current) setErrorKey(engineInstallErrorKey(caught));
    } finally {
      locked.current = false;
      if (mounted.current) { setAction(null); setRevision((value) => value + 1); }
    }
  };

  const active = engineInstallActive(status);
  const busy = action !== null || active;
  const error = errorKey || (status?.phase === 'failed' ? engineInstallErrorKey(status.error) : '')
    || (!active && preflightFailure ? proxyEnginePrerequisiteKey(preflightFailure) ?? '' : '');
  const phase = status?.phase;
  const readiness = !active && preflightFailure
    ? preflightFailure === 'PROXY_ENGINE_MISSING' ? 'missing' : 'failed'
    : proxyEngineReadiness(status, errorKey === 'codex.proxy.engine.statusUnavailable');
  const phaseKey = phase === 'downloading' ? 'downloading' : phase === 'importing' ? 'importing'
    : phase === 'checking' ? 'checking' : active ? 'preparing' : phase === 'cancelled' ? 'cancelled' : readiness;
  const percent = status?.totalBytes ? Math.min(100, Math.max(0, status.receivedBytes / status.totalBytes * 100)) : undefined;

  const refresh = () => { setErrorKey(''); setRevision((value) => value + 1); };
  const reportPreflightFailure = (error: unknown) => {
    const code = proxyEnginePrerequisiteCode(error);
    if (code) setPreflightFailure(code);
  };
  const clearPreflightFailure = () => setPreflightFailure(null);
  return { status, errorKey, error, reading, action, active, busy, phase, phaseKey, percent, readiness, run, refresh,
    reportPreflightFailure, clearPreflightFailure };
}
