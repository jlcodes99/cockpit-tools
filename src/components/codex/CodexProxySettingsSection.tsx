import { useTranslation } from 'react-i18next';
import { Activity, HardDrive, Lock, Route } from 'lucide-react';
import { CodexProxyEngineCard } from './CodexProxyEngineCard';
import { useCodexProxyWorkspace } from './CodexProxyWorkspaceContext';
import { useCodexProxyActivity } from './useCodexProxyActivity';
import { useCodexProxyAccountName } from './useCodexProxyExitEditor';
import '../../styles/pages/codex-proxy-resources-page.css';

/**
 * Workbench settings: engine installation, in-memory log capture for the selected account and the
 * standing boundaries (local entry, system proxy untouched, credentials encrypted locally).
 */
export function CodexProxySettingsSection() {
  const { t } = useTranslation();
  const { accounts, selectedId, goSection } = useCodexProxyWorkspace();
  const activity = useCodexProxyActivity(selectedId);
  const account = accounts.find((entry) => entry.id === selectedId);
  const resolveName = useCodexProxyAccountName();
  const name = resolveName(account);
  const snapshot = activity.snapshot;

  return <div className="codex-proxy-settings">
    <section className="codex-proxy-settings-card" aria-labelledby="codex-proxy-settings-engine">
      <header><h3 id="codex-proxy-settings-engine"><HardDrive size={17} aria-hidden="true" />{t('codex.proxy.engine.title')}</h3>
        <p>{t('codex.proxy.engine.description')}</p></header>
      <CodexProxyEngineCard />
    </section>

    <section className="codex-proxy-settings-card" aria-labelledby="codex-proxy-settings-capture">
      <header><h3 id="codex-proxy-settings-capture"><Activity size={17} aria-hidden="true" />{t('codex.proxy.settings.captureTitle')}</h3>
        <p>{name ? t('codex.proxy.overview.intro', { name }) : t('codex.proxy.settings.noAccount')}</p></header>
      <p className="codex-proxy-page-note">{t('codex.proxy.settings.captureHint')}</p>
      <div className="codex-proxy-settings-actions">
        <button type="button" className="btn btn-secondary" disabled={activity.busy || !snapshot?.supported}
          onClick={() => void activity.changeEnabled(!snapshot?.enabled)}>
          {t(snapshot?.enabled ? 'codex.proxy.activity.stop' : 'codex.proxy.activity.start')}
        </button>
        <button type="button" className="btn btn-secondary" disabled={activity.busy || !snapshot?.enabled}
          onClick={() => void activity.clear()}>{t('codex.proxy.activity.clear')}</button>
        <button type="button" className="btn btn-secondary" onClick={() => goSection('logs')}>{t('codex.proxy.workspace.logs')}</button>
      </div>
      {!activity.loading && !snapshot?.supported && <p className="codex-proxy-page-note">{t('codex.proxy.activity.unsupported')}</p>}
      {activity.error && <p className="codex-proxy-page-error" role="alert">{t('codex.proxy.activity.actionFailed')}</p>}
    </section>

    <section className="codex-proxy-settings-card" aria-labelledby="codex-proxy-settings-entry">
      <header><h3 id="codex-proxy-settings-entry"><Route size={17} aria-hidden="true" />{t('codex.proxy.settings.stabilityTitle')}</h3>
        <p>{t('codex.proxy.restartHint')}</p></header>
    </section>

    <section className="codex-proxy-settings-card" aria-labelledby="codex-proxy-settings-privacy">
      <header><h3 id="codex-proxy-settings-privacy"><Lock size={17} aria-hidden="true" />{t('codex.proxy.settings.privacyTitle')}</h3>
        <p>{t('codex.proxy.settings.privacyHint')}</p></header>
      <p className="codex-proxy-page-note">{t('codex.proxy.engine.privacy')}</p>
      <p className="codex-proxy-page-note">{t('codex.proxy.exportHint')}</p>
    </section>
  </div>;
}
