import { useEffect, useId, useRef, useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { openUrl } from '@tauri-apps/plugin-opener';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';
import { Check, Copy, ExternalLink, FolderOpen, ScanSearch } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  PLATFORM_LAUNCH_PATHS, readPlatformLaunchPath, savePlatformLaunchPath, scanPlatformLaunchPaths,
  type PlatformLaunchPathTarget,
} from '../utils/platformLaunchPath';
import { ModalErrorMessage, useModalErrorState } from './ModalErrorMessage';
import './PlatformLaunchPathField.css';

interface Props {
  target: PlatformLaunchPathTarget;
  disabled?: boolean;
  recovery?: number | boolean;
  onSaving?: (saving: boolean) => void;
  onSaved?: () => void;
}

export function PlatformLaunchPathField({ target, disabled, recovery, onSaving, onSaved }: Props) {
  const { t } = useTranslation();
  const id = useId();
  const input = useRef<HTMLInputElement>(null);
  const { label: appName, guide } = PLATFORM_LAUNCH_PATHS[target];
  const app = target === 'qoder_desktop' ? t('qoder.clientTarget.desktop') : appName;
  const [status, setStatus] = useState<Awaited<ReturnType<typeof readPlatformLaunchPath>>>();
  const [loadFailed, setLoadFailed] = useState(false);
  const [draft, setDraft] = useState<string>();
  const [candidates, setCandidates] = useState<string[]>([]);
  const [scanEmpty, setScanEmpty] = useState(false);
  const [installCopied, setInstallCopied] = useState(false);
  const [busy, setBusy] = useState<'pick' | 'scan' | 'save'>();
  const editRevision = useRef(0);
  const error = useModalErrorState();
  const value = draft ?? status?.configuredPath ?? status?.resolvedPath ?? '';
  const configuredInvalid = Boolean(status?.configuredPath && !status.usesConfiguredPath);
  const showGuide = scanEmpty || Boolean(status && !status.resolvedPath) || Boolean(recovery);
  const label = t('appPath.missing.pathLabel', { app });

  useEffect(() => {
    let request = 0;
    const refresh = () => {
      const revision = ++request;
      void readPlatformLaunchPath(target).then(
        (next) => {
          if (request !== revision) return;
          setStatus(next);
          setLoadFailed(false);
        },
        () => { if (request === revision) setLoadFailed(true); },
      );
    };
    refresh();
    const configUpdated = (event: Event) => {
      const changedTarget = (event as CustomEvent<{ launchPathTarget?: string }>).detail?.launchPathTarget;
      if (!changedTarget || changedTarget === target) refresh();
    };
    window.addEventListener('config-updated', configUpdated);
    return () => {
      request++;
      window.removeEventListener('config-updated', configUpdated);
    };
  }, [target]);

  useEffect(() => {
    if (recovery) input.current?.focus();
  }, [recovery]);

  const updateDraft = (path: string) => {
    editRevision.current++;
    setDraft(path);
    setScanEmpty(false);
    error.clear();
  };

  const choose = async () => {
    setBusy('pick');
    error.clear();
    try {
      const path = await open({ multiple: false, directory: false, defaultPath: value || undefined });
      if (typeof path === 'string') {
        updateDraft(path);
        setCandidates([]);
        input.current?.focus();
      }
    } catch (cause) {
      error.report(t('quickSettings.error.pickPathFailed', { error: String(cause) }));
    } finally {
      setBusy(undefined);
    }
  };

  const scan = async () => {
    const revision = editRevision.current;
    setBusy('scan');
    setCandidates([]);
    setScanEmpty(false);
    error.clear();
    try {
      const paths = await scanPlatformLaunchPaths(target);
      if (paths.length === 0) {
        setScanEmpty(true);
        return;
      }
      if (editRevision.current === revision) updateDraft(paths[0]);
      if (paths.length > 1) setCandidates(paths);
    } catch (cause) {
      error.report(t('messages.actionFailed', { action: t('appPath.missing.autoDetect'), error: String(cause) }));
    } finally {
      setBusy(undefined);
    }
  };

  const save = async () => {
    if (draft === undefined) return;
    setBusy('save');
    onSaving?.(true);
    error.clear();
    let saved = false;
    try {
      await savePlatformLaunchPath(target, draft);
      const path = draft.trim();
      setStatus((previous) => ({
        configuredPath: path || null,
        resolvedPath: path || (previous?.usesConfiguredPath ? null : previous?.resolvedPath ?? null),
        usesConfiguredPath: Boolean(path),
      }));
      setLoadFailed(false);
      setDraft(undefined);
      setCandidates([]);
      setScanEmpty(false);
      saved = true;
    } catch (cause) {
      const code = String(cause).replace(/^Error: /, '');
      error.report(code === `APP_PATH_NOT_FOUND:${target}` || code === `CLI_PATH_NOT_FOUND:${target}`
        ? t('appPath.missing.invalidPath', { app })
        : t('quickSettings.error.saveFailed', { error: code }));
      input.current?.focus();
    } finally {
      setBusy(undefined);
      onSaving?.(false);
    }
    if (saved) onSaved?.();
  };

  const openGuide = async () => {
    error.clear();
    try {
      await openUrl(guide);
    } catch (cause) {
      error.report(t('messages.actionFailed', { action: t('appPath.missing.installationGuide'), error: String(cause) }));
    }
  };

  const copyInstall = async () => {
    try {
      await writeText('npm install -g @tencent-ai/codebuddy-code');
      setInstallCopied(true);
    } catch {
      error.report(t('common.shared.export.copyFailed'));
    }
  };

  return (
    <section className="platform-launch-path">
      <label htmlFor={id}>{label}</label>
      <div className="platform-launch-path-controls">
        <input
          id={id} ref={input} autoComplete="off"
          aria-describedby={`${id}-status`}
          aria-invalid={draft === undefined && (Boolean(recovery) || configuredInvalid)}
          value={value} title={value || undefined}
          placeholder={t('settings.general.codexAppPathPlaceholder')}
          disabled={disabled || busy === 'save'}
          onChange={(event) => updateDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && draft !== undefined) {
              event.preventDefault();
              if (!disabled && !busy) void save();
            }
          }}
        />
        <div className="platform-launch-path-actions">
          <button type="button" className="btn btn-secondary" onClick={() => void choose()} disabled={disabled || !!busy}>
            <FolderOpen size={15} aria-hidden="true" />{t('settings.general.codexPathSelect')}
          </button>
          <button type="button" className="btn btn-secondary" onClick={() => void scan()} disabled={disabled || !!busy} aria-busy={busy === 'scan'}>
            <ScanSearch size={15} aria-hidden="true" />{t('appPath.missing.autoDetect')}
          </button>
          <button type="button" className={`btn ${draft !== undefined ? 'btn-primary' : 'btn-secondary'}`} onClick={() => void save()} disabled={disabled || !!busy || draft === undefined} aria-busy={busy === 'save'}>
            {t(recovery ? 'appPath.missing.saveAndRetry' : 'common.save')}
          </button>
        </div>
      </div>
      {candidates.length > 1 && (
        <div className="platform-launch-path-candidates">
          {candidates.map((candidate) => (
            <button type="button" key={candidate} className="btn btn-ghost" title={candidate}
              aria-pressed={candidate === value} disabled={disabled || !!busy}
              onClick={() => updateDraft(candidate)}>
              {candidate === value && <Check size={14} aria-hidden="true" />}
              <span>{candidate}</span>
            </button>
          ))}
        </div>
      )}
      <div id={`${id}-status`} className="platform-launch-path-status" role="status">
        <span>
          {busy === 'scan' || busy === 'save' ? t('common.loading')
            : scanEmpty ? t('appPath.missing.noInstallation', { app })
            : draft !== undefined ? t('appPath.missing.configureHint')
            : loadFailed ? t('appPath.missing.checkFailed')
            : !status ? t('common.loading')
            : configuredInvalid || (recovery && status.configuredPath) ? t('appPath.missing.invalidPath', { app })
            : recovery ? t('appPath.missing.noInstallation', { app })
            : status.usesConfiguredPath ? t('appPath.missing.configured')
            : status.resolvedPath ? t('appPath.missing.detected')
            : t('appPath.missing.noInstallation', { app })}
        </span>
        {Boolean(draft ?? status?.configuredPath) && (
          <button type="button" className="btn btn-ghost" disabled={disabled || !!busy} onClick={() => updateDraft('')}>
            {t('common.clear')}
          </button>
        )}
        {showGuide && (
          <button type="button" className="btn btn-ghost" disabled={disabled} onClick={() => void openGuide()}>
            <ExternalLink size={13} aria-hidden="true" />{t('appPath.missing.installationGuide')}
          </button>
        )}
        {showGuide && target === 'codebuddy_cli' && (
          <button type="button" className="btn btn-ghost" disabled={disabled} onClick={() => void copyInstall()}>
            {installCopied ? <Check size={13} aria-hidden="true" /> : <Copy size={13} aria-hidden="true" />}
            {t(installCopied ? 'common.copied' : 'appPath.missing.copyInstallCommand')}
          </button>
        )}
      </div>
      <ModalErrorMessage message={error.message} scrollKey={error.scrollKey} position="bottom" />
    </section>
  );
}
