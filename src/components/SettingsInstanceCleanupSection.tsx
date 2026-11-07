import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/core';
import './SettingsInstanceCleanupSection.css';

interface OrphanDirectory {
  path: string;
  platform: string;
  bytes: number | null;
  sizeError?: string | null;
}

interface CleanupResult {
  deleted: string[];
  failed: { path: string; error: string }[];
}

function formatSize(bytes: number, language: string): string {
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  const index = bytes > 0 ? Math.min(4, Math.floor(Math.log(bytes) / Math.log(1024))) : 0;
  return `${new Intl.NumberFormat(language, { maximumFractionDigits: 2 }).format(bytes / 1024 ** index)} ${units[index]}`;
}

export function SettingsInstanceCleanupSection() {
  const { t, i18n } = useTranslation();
  const [directories, setDirectories] = useState<OrphanDirectory[] | null>(null);
  const [busy, setBusy] = useState<'scan' | 'delete' | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [error, setError] = useState('');
  const [deletedCount, setDeletedCount] = useState<number | null>(null);
  const operation = useRef(false);
  const errorRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (error) {
      errorRef.current?.scrollIntoView({ block: 'nearest' });
      errorRef.current?.focus({ preventScroll: true });
    }
  }, [error]);

  async function scan() {
    if (operation.current) return;
    operation.current = true;
    setBusy('scan');
    setError('');
    setConfirming(false);
    setDeletedCount(null);
    setDirectories(null);
    try {
      setDirectories(await invoke<OrphanDirectory[]>('scan_orphan_instance_dirs'));
    } catch (cause) {
      setError(String(cause));
    } finally {
      operation.current = false;
      setBusy(null);
    }
  }

  async function remove() {
    if (operation.current || !confirming || !directories?.length) return;
    operation.current = true;
    setBusy('delete');
    setError('');
    setDeletedCount(null);
    try {
      const result = await invoke<CleanupResult>('delete_orphan_instance_dirs', {
        paths: directories.map(({ path }) => path),
      });
      const deleted = new Set(result.deleted);
      setDirectories((current) => current?.filter(({ path }) => !deleted.has(path)) ?? null);
      setDeletedCount(result.deleted.length);
      setConfirming(false);
      if (result.failed.length) {
        // Failed deletions may have moved data into a recoverable cleanup directory.
        // Require a fresh scan before retrying so we never submit stale paths.
        setDirectories(null);
        setError(result.failed.map(({ path, error: detail }) => `${path}: ${detail}`).join('\n'));
      }
    } catch (cause) {
      setError(String(cause));
    } finally {
      operation.current = false;
      setBusy(null);
    }
  }

  const knownSize = formatSize(directories?.reduce((sum, item) => sum + (item.bytes ?? 0), 0) ?? 0, i18n.language);
  const size = directories?.some((item) => item.bytes === null) ? `≥ ${knownSize}` : knownSize;
  return (
    <section className="instance-cleanup-section" aria-busy={busy !== null}>
      <div className="group-title">{t('settings.instanceCleanup.title')}</div>
      <div className="settings-group">
        <div className="settings-row">
          <div className="row-label">
            <div className="row-title">{t('settings.instanceCleanup.title')}</div>
            <div className="row-desc">{t('settings.instanceCleanup.description')}</div>
          </div>
          <div className="row-control">
            <button type="button" className="btn btn-secondary" disabled={busy !== null} onClick={() => void scan()}>
              {t(busy === 'scan' ? 'settings.instanceCleanup.scanning' : 'settings.instanceCleanup.scan')}
            </button>
          </div>
        </div>
        <div className="instance-cleanup-content">
          <div role="status" aria-live="polite">
            {deletedCount !== null && <p>{t('settings.instanceCleanup.done', { count: deletedCount })}</p>}
            {directories !== null && (
              <p>{directories.length
                ? t('settings.instanceCleanup.summary', { count: directories.length, size })
                : t('settings.instanceCleanup.empty')}</p>
            )}
          </div>
          {!!directories?.length && (
            <>
              <div className="instance-cleanup-list">
                {directories.map((item) => (
                  <div key={item.path} className="instance-cleanup-entry">
                    <span>{item.platform}</span>
                    <code>{item.path}</code>
                    <span>{item.bytes === null ? '—' : formatSize(item.bytes, i18n.language)}</span>
                    {item.sizeError && <small className="instance-cleanup-size-error">{item.sizeError}</small>}
                  </div>
                ))}
              </div>
              {confirming && <p className="instance-cleanup-warning">{t('settings.instanceCleanup.confirm')}</p>}
              <div className="instance-cleanup-actions">
                {confirming ? (
                  <>
                    <button type="button" className="btn btn-secondary" disabled={busy !== null}
                      onClick={() => { setConfirming(false); setError(''); }}>
                      {t('settings.instanceCleanup.cancel')}
                    </button>
                    <button type="button" className="btn btn-danger" disabled={busy !== null} onClick={() => void remove()}>
                      {t(busy === 'delete' ? 'settings.instanceCleanup.removing' : 'settings.instanceCleanup.confirmRemove')}
                    </button>
                  </>
                ) : (
                  <button type="button" className="btn btn-danger" disabled={busy !== null}
                    onClick={() => { setConfirming(true); setError(''); }}>
                    {t('settings.instanceCleanup.remove')}
                  </button>
                )}
              </div>
            </>
          )}
          {error && <div ref={errorRef} tabIndex={-1} role="alert" className="instance-cleanup-error">
            <strong>{t('settings.instanceCleanup.failed')}</strong>
            <div>{error}</div>
          </div>}
        </div>
      </div>
    </section>
  );
}
