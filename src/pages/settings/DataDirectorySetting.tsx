import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { FolderOpen } from 'lucide-react';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { openDataFolder } from '../../services/accountService';

interface DataDirectorySettings {
  currentPath: string | null;
  configuredPath: string | null;
  environmentOverride: boolean;
  restartRequired: boolean;
  error: string | null;
}

export function DataDirectorySetting() {
  const { t } = useTranslation();
  const [settings, setSettings] = useState<DataDirectorySettings>();
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let active = true;
    invoke<DataDirectorySettings>('get_data_directory_settings')
      .then(result => { if (active) setSettings(result); })
      .catch(reason => { if (active) setError(String(reason)); });
    return () => { active = false; };
  }, []);

  async function update(reset: boolean) {
    setBusy(true);
    setError('');
    try {
      const path = reset ? null : await open({
        directory: true,
        multiple: false,
        defaultPath: settings?.configuredPath ?? settings?.currentPath ?? undefined,
      });
      if (!reset && typeof path !== 'string') return;
      const result = await invoke<DataDirectorySettings>('set_data_directory', { path });
      setSettings(result);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  const message = error || settings?.error;
  return (
    <div className="settings-row settings-data-directory">
      <div className="row-label">
        <div className="row-title">{t('settings.general.dataDir')}</div>
        <div className="row-desc">{t('settings.general.dataDirDesc')}</div>
        {settings?.currentPath && <code className="row-desc">{settings.currentPath}</code>}
        {settings?.environmentOverride && <div className="row-desc" role="status">
          {t('settings.general.dataDirEnvironment')}
        </div>}
        {settings?.restartRequired && <div className="row-desc" role="status">
          {settings.configuredPath
            ? t('settings.general.dataDirPending', { path: settings.configuredPath })
            : t('settings.general.dataDirPendingDefault')}
        </div>}
        {message && <div className="settings-app-path-error" role="alert">{message}</div>}
      </div>
      <div className="row-control">
        <button className="btn btn-secondary" type="button" disabled={busy}
          onClick={() => void openDataFolder().catch(reason => setError(String(reason)))}>
          <FolderOpen size={16} />{t('common.open')}
        </button>
        <button className="btn btn-secondary" type="button" disabled={busy || !settings}
          onClick={() => void update(false)}>
          {t('instances.launchDialog.chooseDirectory')}
        </button>
        {(settings?.configuredPath || settings?.error) && (
          <button className="btn btn-secondary" type="button" disabled={busy}
            onClick={() => void update(true)}>
            {t('platformLayout.reset')}
          </button>
        )}
      </div>
    </div>
  );
}
