import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { FolderOpen, Play, RefreshCw, Terminal } from 'lucide-react';
import { open } from '@tauri-apps/plugin-dialog';
import { openUrl } from '@tauri-apps/plugin-opener';
import { getAntigravityCliStatus, launchAntigravityCli, type AntigravityCliStatus } from '../services/antigravityCliService';
import { useAccountStore } from '../stores/useAccountStore';
import '../styles/components/antigravity-cli.css';

interface Props {
  active: boolean;
  desktopLabel: string;
  disabled: boolean;
  onChange: (active: boolean) => void;
}

export function AntigravityCliPanel({ active, desktopLabel, disabled, onChange }: Props) {
  const { t } = useTranslation();
  const [status, setStatus] = useState<AntigravityCliStatus | null>(null);
  const [loading, setLoading] = useState(false);
  const [launching, setLaunching] = useState(false);
  const [directory, setDirectory] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const currentId = useAccountStore((state) => state.currentAccountsByTarget.antigravity_cli?.id);
  const requestId = useRef(0);

  const refresh = useCallback(async () => {
    const id = ++requestId.current;
    setLoading(true);
    setError(null);
    try {
      const next = await getAntigravityCliStatus();
      if (id === requestId.current) setStatus(next);
    } catch (e) {
      if (id === requestId.current) { setError(String(e)); setStatus(null); }
    } finally {
      if (id === requestId.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (!active) return;
    void refresh();
    const onFocus = () => {
      void refresh();
      void useAccountStore.getState().fetchCurrentAccount('antigravity_cli');
    };
    window.addEventListener('focus', onFocus);
    return () => { ++requestId.current; window.removeEventListener('focus', onFocus); };
  }, [active, currentId, refresh]);

  const launch = async () => {
    setLaunching(true);
    setError(null);
    setNotice(null);
    try {
      await launchAntigravityCli(directory || undefined);
      setNotice(t('antigravityCli.launched'));
    } catch (e) { setError(String(e)); }
    finally { setLaunching(false); }
  };

  return (
    <section className="antigravity-cli-panel" aria-label={t('antigravityCli.management')}>
      <div className="antigravity-cli-mode" role="group" aria-label={t('antigravityCli.target')}>
        <button className={`filter-tab${!active ? ' active' : ''}`} aria-pressed={!active} disabled={disabled} onClick={() => onChange(false)}>{desktopLabel}</button>
        <button className={`filter-tab${active ? ' active' : ''}`} aria-pressed={active} disabled={disabled} onClick={() => onChange(true)}><Terminal size={16} /> Antigravity CLI</button>
      </div>
      {active && <div className="antigravity-cli-details">
        <div className="antigravity-cli-actions">
          <strong>{status?.version ? `agy ${status.version}` : t('antigravityCli.title')}</strong>
          <button className="btn btn-secondary" disabled={loading || disabled} onClick={() => { void refresh(); void useAccountStore.getState().fetchCurrentAccount('antigravity_cli'); }}><RefreshCw size={14} />{t('antigravityCli.refresh')}</button>
          <button className="btn btn-primary" disabled={launching || disabled || !status?.executable} onClick={() => void launch()}><Play size={14} />{t('antigravityCli.launch')}</button>
          {!loading && !status?.executable && <button className="btn btn-secondary" onClick={() => void openUrl('https://www.antigravity.google/docs/cli/install/').catch((e) => setError(String(e)))}>{t('antigravityCli.install')}</button>}
        </div>
        <p>{t('antigravityCli.scope')}</p>
        <p>{t('antigravityCli.sharedCredential')}</p>
        {status && <p role="status">{loading ? t('antigravityCli.loading') : status.credential_error || (
          status?.api_key_mode ? t('antigravityCli.apiKeyMode') : status?.current_account_id ? t('antigravityCli.managed') : status?.credential_present ? t('antigravityCli.notImported') : t('antigravityCli.notSignedIn')
        )}</p>}
        {loading && !status && <p role="status">{t('antigravityCli.loading')}</p>}
        {status?.executable && <p className="antigravity-cli-path">{status.executable}</p>}
        <div className="antigravity-cli-directory">
          <label htmlFor="antigravity-cli-directory">{t('antigravityCli.directory')}</label>
          <input id="antigravity-cli-directory" value={directory} placeholder={t('antigravityCli.homeDirectory')} onChange={(event) => setDirectory(event.target.value)} />
          <button className="btn btn-secondary" aria-label={t('antigravityCli.chooseDirectory')} onClick={() => void open({ directory: true, multiple: false }).then((path) => { if (typeof path === 'string') setDirectory(path); }).catch((e) => setError(String(e)))}><FolderOpen size={16} /></button>
        </div>
        {(error || status?.version_error) && <p role="alert" className="antigravity-cli-error">{error || status?.version_error}</p>}
        {notice && <p role="status">{notice}</p>}
      </div>}
    </section>
  );
}
