import { useEffect, useId, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { FolderOpen } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { antigravityAppPathTarget, ANTIGRAVITY_CLIENT_LABELS, type AntigravityLaunchClient } from '../utils/antigravityRuntimeTarget';
import { ModalErrorMessage, useModalErrorState } from './ModalErrorMessage';

interface PathStatus {
  configuredPath: string | null;
  resolvedPath: string | null;
  usesConfiguredPath: boolean;
}

const inFlight = new Map<AntigravityLaunchClient, Promise<PathStatus>>();

function readPath(client: AntigravityLaunchClient) {
  const existing = inFlight.get(client);
  if (existing) return existing;
  const request = invoke<PathStatus>('get_antigravity_client_launch_path', { client });
  inFlight.set(client, request);
  const clear = () => { if (inFlight.get(client) === request) inFlight.delete(client); };
  void request.then(clear, clear);
  return request;
}

interface Props {
  client: AntigravityLaunchClient;
  disabled: boolean;
  recovering: boolean;
  onSaving: (saving: boolean) => void;
  onSaved: () => void;
}

export function AntigravityLaunchPathField({ client, disabled, recovering, onSaving, onSaved }: Props) {
  const { t } = useTranslation();
  const id = useId();
  const input = useRef<HTMLInputElement>(null);
  const saving = useRef(false);
  const [status, setStatus] = useState<PathStatus>();
  const [loadFailed, setLoadFailed] = useState(false);
  const [draft, setDraft] = useState<string>();
  const [revision, setRevision] = useState(0);
  const isCli = client === 'cli';
  const label = t('settings.general.antigravityAppPath', { appName: ANTIGRAVITY_CLIENT_LABELS[client] });
  const configuredInvalid = Boolean(status?.configuredPath && !status.usesConfiguredPath);
  const displayedPath = configuredInvalid ? status?.configuredPath : status?.resolvedPath;
  const error = useModalErrorState();

  useEffect(() => {
    let requestId = 0;
    const refresh = () => {
      const id = ++requestId;
      setLoadFailed(false);
      void readPath(client).then(
        (resolved) => { if (id === requestId) setStatus(resolved); },
        () => { if (id === requestId) setLoadFailed(true); },
      );
    };
    const configUpdated = () => {
      inFlight.delete(client);
      refresh();
    };
    refresh();
    window.addEventListener('config-updated', configUpdated);
    return () => { requestId++; window.removeEventListener('config-updated', configUpdated); };
  }, [client, revision]);

  useEffect(() => {
    if (recovering) input.current?.focus();
  }, [recovering]);

  const pickPath = async () => {
    try {
      const path = await open({ multiple: false, directory: false });
      if (typeof path === 'string') {
        error.clear();
        setDraft(path);
        input.current?.focus();
      }
    } catch (cause) {
      error.report(t('quickSettings.error.pickPathFailed', { error: String(cause) }));
    }
  };

  const savePath = async () => {
    if (draft === undefined || saving.current) return;
    saving.current = true;
    onSaving(true);
    error.clear();
    try {
      if (isCli) await invoke('save_cli_launch_path', { app: 'antigravity_cli', path: draft.trim() });
      else await invoke('set_app_path', { app: antigravityAppPathTarget(client), path: draft.trim() });
      setDraft(undefined);
      window.dispatchEvent(new Event('config-updated'));
      onSaved();
    } catch (cause) {
      error.report(t('quickSettings.error.saveFailed', { error: String(cause) }));
    } finally {
      saving.current = false;
      onSaving(false);
    }
  };

  return (
    <section className="antigravity-launch-path">
      <label htmlFor={id}>{label}</label>
      <div className="antigravity-launch-path-controls">
        <input
          id={id} ref={input} aria-describedby={`${id}-status`}
          aria-invalid={recovering || configuredInvalid}
          value={draft ?? displayedPath ?? ''} title={draft ?? displayedPath ?? ''}
          disabled={disabled} autoComplete="off"
          onChange={(event) => { setDraft(event.target.value); error.clear(); }}
        />
          <button type="button" className="btn btn-secondary" onClick={() => void pickPath()} disabled={disabled}>
            <FolderOpen size={16} aria-hidden="true" />{t('settings.general.codexPathSelect')}
          </button>
          <button type="button" className="btn btn-secondary" onClick={() => void savePath()} disabled={disabled || draft === undefined}>
            {t(recovering ? 'antigravityLaunch.pathSaveAndRetry' : 'common.save')}
          </button>
      </div>
      <div id={`${id}-status`} className="antigravity-launch-path-status" aria-live="polite">
        {recovering && <div className="antigravity-launch-path-required" role="alert">
          {t('appPath.missing.desc', { app: ANTIGRAVITY_CLIENT_LABELS[client] })}
        </div>}
        {loadFailed ? <>
          {t('antigravityLaunch.pathCheckFailed')}
          <button type="button" className="client-launch-text-button" disabled={disabled} onClick={() => setRevision((value) => value + 1)}>
            {t('common.retry')}
          </button>
        </> : !status ? t('common.loading') : <>
          {status.usesConfiguredPath ? t('antigravityLaunch.pathConfigured') :
            status.resolvedPath ? t('antigravityLaunch.pathDetected') :
              t(isCli ? 'antigravityLaunch.cliMissing' : 'appPath.missing.title')}
          {configuredInvalid && (
            <span title={status.resolvedPath ?? undefined}>
              {t('antigravityLaunch.pathInvalid')}{status.resolvedPath && ` · ${status.resolvedPath}`}
            </span>
          )}
        </>}
      </div>
      <ModalErrorMessage message={error.message} scrollKey={error.scrollKey} position="bottom" />
    </section>
  );
}
