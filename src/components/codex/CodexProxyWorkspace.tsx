import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { ArrowDown, ArrowLeft, ArrowUp, Globe2, Settings2 } from 'lucide-react';
import { useCodexProxyWorkspace } from './CodexProxyWorkspaceContext';
import { PROXY_DIAGNOSTIC_IDS, PROXY_SECTION_COMPONENTS, PROXY_SECTION_ICONS, PROXY_SECTION_IDS, PROXY_SECTION_LABEL_KEYS } from './codexProxySections';
import { formatTrafficBytes } from '../../utils/codexProxyTraffic';
import { canUseCodexAccountProxy } from '../../utils/codexAccountProxy';
import { useCodexProxyAccountName } from './useCodexProxyExitEditor';
import { SingleSelectDropdown } from '../SingleSelectDropdown';
import '../../styles/pages/codex-proxy-workspace.css';

/** Full-width task lists; operational tools live one level below the two primary tabs. */
export function CodexProxyWorkspace({ onBack }: { onBack: () => void }) {
  const { t } = useTranslation();
  const { section, goSection, traffic, accounts, selectedId, selectAccount } = useCodexProxyWorkspace();
  const [returnTo, setReturnTo] = useState<'resources' | 'accounts'>(section === 'accounts' ? 'accounts' : 'resources');
  const resolveName = useCodexProxyAccountName();
  const primary = section === 'resources' || section === 'accounts';
  useEffect(() => { if (section === 'resources' || section === 'accounts') setReturnTo(section); }, [section]);
  const Section = PROXY_SECTION_COMPONENTS[section];
  const eligible = accounts.filter(canUseCodexAccountProxy);
  const rate = (value: number) => !traffic.ready || traffic.unavailable ? '—' : `${formatTrafficBytes(value)}/s`;

  return <main className="codex-tool-page codex-proxy-workspace" aria-labelledby="codex-proxy-workspace-title">
    <header className="codex-proxy-manager-header">
      <div className="codex-proxy-manager-heading">
        <span className="codex-proxy-manager-icon" aria-hidden="true"><Globe2 size={22} /></span>
        <div><h1 id="codex-proxy-workspace-title">{t('codex.proxy.management')}</h1>
          <p>{t('codex.proxy.manager.subtitle')}</p></div>
      </div>
      <div className="codex-proxy-manager-tools">
        <button type="button" className="btn btn-secondary" aria-pressed={!primary} onClick={() => goSection(primary ? 'settings' : returnTo)}>
          <Settings2 size={15} />{t('codex.proxy.manager.tools')}</button>
        <button type="button" className="btn btn-secondary" onClick={onBack}>
          <ArrowLeft size={15} />{t('codex.proxy.backToAccounts')}</button>
      </div>
    </header>
    <nav className="codex-proxy-manager-tabs" aria-label={t('codex.proxy.workspace.navLabel')}>
      {PROXY_SECTION_IDS.map((id) => {
        const Icon = PROXY_SECTION_ICONS[id];
        return <button key={id} type="button" className={`btn codex-proxy-manager-tab${section === id ? ' is-active' : ''}`}
          aria-current={section === id ? 'page' : undefined} onClick={() => goSection(id)}>
          <Icon size={17} /><span>{t(PROXY_SECTION_LABEL_KEYS[id])}</span>
        </button>;
      })}
    </nav>
    {!primary && <div className="codex-proxy-manager-diagnostics">
      <header className="codex-proxy-manager-diagnostics-head">
        <button type="button" className="btn btn-secondary compact" onClick={() => goSection(returnTo)}>
          <ArrowLeft size={14} />{t(PROXY_SECTION_LABEL_KEYS[returnTo])}</button>
        <h2>{t('codex.proxy.manager.tools')}</h2>
        {(section === 'logs' || section === 'connections') && <div className="codex-proxy-manager-traffic" aria-label={t('codex.proxy.workspace.trafficSession')}>
          <span title={t('codex.proxy.workspace.trafficUpload')}><ArrowUp size={13} />{rate(traffic.upPerSecond)}</span>
          <span title={t('codex.proxy.workspace.trafficDownload')}><ArrowDown size={13} />{rate(traffic.downPerSecond)}</span>
        </div>}
      </header>
      <div className="codex-proxy-manager-diagnostics-toolbar">
        <nav aria-label={t('codex.proxy.manager.tools')}>
          {PROXY_DIAGNOSTIC_IDS.map((id) => <button key={id} type="button"
            className={`btn btn-secondary compact${section === id ? ' is-active' : ''}`}
            aria-current={section === id ? 'page' : undefined} onClick={() => goSection(id)}>{t(PROXY_SECTION_LABEL_KEYS[id])}</button>)}
        </nav>
        {eligible.length > 0 && <SingleSelectDropdown value={selectedId} onChange={selectAccount}
          ariaLabel={t('codex.proxy.accountsTitle')} options={eligible.map((entry) => ({ value: entry.id, label: resolveName(entry) }))} />}
      </div>
    </div>}
    <section className={`codex-proxy-manager-content${primary ? '' : ' is-diagnostic'}`} aria-label={t(PROXY_SECTION_LABEL_KEYS[section])}>
      <Section />
    </section>
  </main>;
}
