import { Check, Link2, Plus } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import { CodexProxyEnginePanel } from './CodexProxyEngineCard';
import { useCodexProxyEngine } from './useCodexProxyEngine';
import { proxySetupStage } from '../../utils/codexProxySetup';
import '../../styles/pages/codex-proxy-setup.css';

/** Existing lists stay usable while the shared engine installer reads or downloads. */
export function CodexProxySetupGuide({ empty, hasUsableProxies, onAdd, addDisabled }: {
  empty: boolean;
  hasUsableProxies: boolean | null;
  onAdd: () => void;
  addDisabled: boolean;
}) {
  const { t } = useTranslation();
  const engine = useCodexProxyEngine();
  const stage = proxySetupStage(engine.readiness, hasUsableProxies);
  const missing = !empty && hasUsableProxies !== null && ['missing', 'failed'].includes(engine.readiness);
  return <div className={`codex-proxy-setup${empty ? ' is-first-use' : ''}`}>
    {empty && <header className="codex-proxy-setup-heading"><h2>{t('codex.proxy.setup.title')}</h2><p>{t('codex.proxy.setup.intro')}</p></header>}
    {missing && <p className="codex-proxy-setup-notice" role="status">{t('codex.proxy.setup.missing')}</p>}
    <div className="codex-proxy-setup-step">
      {empty && <span className={`codex-proxy-setup-number${engine.readiness === 'ready' ? ' is-done' : ''}`} aria-hidden="true">{engine.readiness === 'ready' ? <Check size={16} /> : '1'}</span>}
      <CodexProxyEnginePanel engine={engine} compactWhenReady
        title={empty ? t('codex.proxy.setup.engineStep') : undefined} description={t('codex.proxy.setup.engineHint')} />
    </div>
    {empty && <div className="codex-proxy-setup-step">
      <span className="codex-proxy-setup-number" aria-hidden="true">2</span>
      <div className="codex-proxy-setup-add"><Link2 size={22} aria-hidden="true" /><div><h3>{t('codex.proxy.managerResources.add')}</h3><p>{t('codex.proxy.setup.addHint')}</p></div>
        <button type="button" className="btn btn-primary" disabled={addDisabled} onClick={onAdd}><Plus size={15} />{t('codex.proxy.managerResources.add')}</button>
      </div>
    </div>}
    {stage === 'assign' && <p className="codex-proxy-setup-next" role="status">{t('codex.proxy.setup.assignHint')}</p>}
  </div>;
}
